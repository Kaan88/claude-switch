{ Read and edit only the user's PATH. Keep the raw text, registry type and unrelated tokens. }
const
  PathValue = 'Path';
  RegMissing = 0;
  RegString = 1;
  RegExpandString = 2;

function QueryPathValue(Key: HKEY; SubKey, ValueName: String; Flags: DWORD;
  var ValueType: DWORD; Data: String; var Bytes: DWORD): LONG;
external 'RegGetValueW@advapi32.dll stdcall';

function ExpandPathString(Source, Dest: String; Capacity: DWORD): DWORD;
external 'ExpandEnvironmentStringsW@kernel32.dll stdcall';

function SendEnvironmentChange(Window: HWND; Msg: UINT; WParam: WPARAM;
  Setting: String; Flags, Timeout: UINT; var MessageResult: ULONG_PTR): LRESULT;
external 'SendMessageTimeoutW@user32.dll stdcall';

procedure NotifyEnvironmentChanged;
var
  MessageResult: ULONG_PTR;
begin
  { HWND_BROADCAST, WM_SETTINGCHANGE, SMTO_ABORTIFHUNG. Existing shells keep their own environment. }
  SendEnvironmentChange($FFFF, $001A, 0, 'Environment', 2, 200, MessageResult);
end;

function ReadUserPath(Key: String; var Raw: String; var Kind: DWORD): Boolean;
var
  Code: LONG;
  Bytes: DWORD;
  Buffer: String;
begin
  Raw := '';
  Kind := RegMissing;
  Result := False;
  SetLength(Buffer, 32768);
  Bytes := Length(Buffer) * 2;
  { Raw HKCU handle; RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND. }
  Code := QueryPathValue(HKEY(-2147483647), Key, PathValue, $10000006, Kind, Buffer, Bytes);
  if Code = 2 then begin
    Kind := RegMissing;
    Result := True;
  end else if (Code = 0) and (Bytes >= 2) and (Bytes <= Length(Buffer) * 2) and (Bytes mod 2 = 0) then begin
    Raw := Copy(Buffer, 1, Bytes div 2 - 1);
    Result := (Buffer[Bytes div 2] = #0) and (Pos(#0, Raw) = 0);
  end;
end;

function WriteUnchangedPath(Key, Before, After: String; Kind: DWORD; DeleteIfEmpty: Boolean): Boolean;
var
  Current: String;
  CurrentKind: DWORD;
begin
  Result := False;
  { Do not replace a freshly observed edit or change the current registry value type. }
  if not ReadUserPath(Key, Current, CurrentKind) or (CurrentKind <> Kind) or (Current <> Before) then Exit;
  if DeleteIfEmpty and (After = '') and (Kind = RegExpandString) then
    Result := RegDeleteValue(HKCU, Key, PathValue)
  else if Kind = RegString then
    Result := RegWriteStringValue(HKCU, Key, PathValue, After)
  else
    Result := RegWriteExpandStringValue(HKCU, Key, PathValue, After);
end;

function PlainPathToken(Path: String): Boolean;
begin
  Result := (Path <> '') and (Length(Path) <= 1024) and (Path = Trim(Path)) and
    (Pos(';', Path) = 0) and (Pos('"', Path) = 0) and (Pos('%', Path) = 0) and
    (Pos(#13, Path) = 0) and (Pos(#10, Path) = 0) and (Pos(#0, Path) = 0);
end;

function PathKey(Path: String; Kind: DWORD): String;
var
  Expanded: String;
  Count: DWORD;
begin
  if (Kind = RegExpandString) and (Pos('%', Path) > 0) then begin
    SetLength(Expanded, 32768);
    Count := ExpandPathString(Path, Expanded, Length(Expanded));
    if (Count > 0) and (Count <= Length(Expanded)) then Path := Copy(Expanded, 1, Count - 1);
  end;
  Result := Trim(Path);
  if (Length(Result) >= 2) and (Result[1] = '"') and (Result[Length(Result)] = '"') then
    Result := Copy(Result, 2, Length(Result) - 2);
  StringChangeEx(Result, '/', '\', True);
  while (Length(Result) > 3) and (Result[Length(Result)] = '\') do
    SetLength(Result, Length(Result) - 1);
end;

{ Find exact and equivalent tokens; never treat a path substring as an entry. }
function PathMatches(Raw, Dir: String; Kind: DWORD; var ExactStart, ExactEnd: Integer): Integer;
var
  I, Start: Integer;
  Entry, Want: String;
begin
  Result := 0;
  ExactStart := 0;
  ExactEnd := 0;
  Want := PathKey(Dir, RegString);
  Start := 1;
  for I := 1 to Length(Raw) + 1 do begin
    if I <= Length(Raw) then
      if Raw[I] <> ';' then Continue;
    Entry := Copy(Raw, Start, I - Start);
    if CompareText(PathKey(Entry, Kind), Want) = 0 then begin
      Result := Result + 1;
      if Entry = Dir then begin
        ExactStart := Start;
        ExactEnd := I;
      end;
    end;
    Start := I + 1;
  end;
end;

function AddUserPath(EnvironmentKey, OwnershipKey, Dir: String; var Changed: Boolean): Boolean;
var
  Before, After: String;
  Kind, WrittenKind, Owned, Marker: DWORD;
  Start, Stop: Integer;
  HadMarker: Boolean;
begin
  Changed := False;
  Result := False;
  if not PlainPathToken(Dir) or not ReadUserPath(EnvironmentKey, Before, Kind) then Exit;
  if PathMatches(Before, Dir, Kind, Start, Stop) > 0 then begin
    Result := True;
    Exit; { A pre-existing equivalent entry never becomes ours. }
  end;
  After := Dir;
  if Before <> '' then After := Before + ';' + Dir;
  if Length(After) >= 32767 then Exit;
  HadMarker := RegQueryDWordValue(HKCU, OwnershipKey, Dir, Owned) and ((Owned = 1) or (Owned = 2));
  if not HadMarker and RegValueExists(HKCU, OwnershipKey, Dir) then Exit;
  if not WriteUnchangedPath(EnvironmentKey, Before, After, Kind, False) then Exit;
  Changed := True;
  Marker := 1;
  WrittenKind := Kind;
  if Kind = RegMissing then begin
    Marker := 2;
    WrittenKind := RegExpandString;
  end;
  { Claim ownership only after writing PATH. If recording fails, roll back to the exact previous
    value only if PATH is still what we just wrote; never publish a false ownership record. }
  Result := HadMarker;
  if not Result then Result := RegWriteDWordValue(HKCU, OwnershipKey, Dir, Marker);
  if not Result then
    if WriteUnchangedPath(EnvironmentKey, After, Before, WrittenKind, Marker = 2) then Changed := False;
end;

function RemoveUserPaths(EnvironmentKey, OwnershipKey: String; var Changed: Boolean): Boolean;
var
  Names: TArrayOfString;
  I, Start, Stop, Count: Integer;
  Kind, Owned: DWORD;
  Before, After, Dir: String;
begin
  Changed := False;
  Result := False;
  if not RegGetValueNames(HKCU, OwnershipKey, Names) then begin
    Result := not RegKeyExists(HKCU, OwnershipKey);
    Exit;
  end;
  Result := True;
  for I := 0 to GetArrayLength(Names) - 1 do begin
    Dir := Names[I];
    if not PlainPathToken(Dir) or not RegQueryDWordValue(HKCU, OwnershipKey, Dir, Owned) or
      ((Owned <> 1) and (Owned <> 2)) then
      Continue;
    if not ReadUserPath(EnvironmentKey, Before, Kind) then begin
      Result := False;
      Continue;
    end;
    Count := PathMatches(Before, Dir, Kind, Start, Stop);
    if Count > 0 then begin
      if (Count <> 1) or (Start = 0) then begin
        Result := False; { Edited or ambiguous: leave user data alone. }
        Continue;
      end;
      if Stop <= Length(Before) then
        Stop := Stop + 1
      else if Start > 1 then
        Start := Start - 1;
      After := Copy(Before, 1, Start - 1) + Copy(Before, Stop, Length(Before));
      if not WriteUnchangedPath(EnvironmentKey, Before, After, Kind, (Owned = 2) and (Before = Dir)) then begin
        Result := False;
        Continue;
      end;
      Changed := True;
    end;
    if not RegDeleteValue(HKCU, OwnershipKey, Dir) then Result := False;
  end;
  RegDeleteKeyIfEmpty(HKCU, OwnershipKey);
end;
