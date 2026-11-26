; Compile with ISCC /O<temp> and run /VERYSILENT /SUPPRESSMSGBOXES /LOG=<temp log>.
; This fixture aborts Setup before installation and never uses HKCU\Environment.
[Setup]
AppId=ClaudeSwitchUserPathTests
AppName=Claude Switch User PATH Tests
AppVersion=1.0
DefaultDirName={tmp}\ClaudeSwitchUserPathTests
PrivilegesRequired=lowest
Uninstallable=no
OutputBaseFilename=test-user-path

[Code]
#include "..\user-path.iss"

const
  Fixture = 'Software\ClaudeSwitchUserPathFixture-2f65b2ba8804';
  Env = Fixture + '\Env';
  Ownership = Fixture + '\Ownership';
  Dir = 'C:\Synthetic Apps\Claude Switch';
  Dir2 = 'C:\Other Synthetic Apps\Claude Switch';

procedure Check(Condition: Boolean; LabelText: String);
begin
  if not Condition then RaiseException('PATH test failed: ' + LabelText);
end;

procedure ExpectPath(Want: String; WantKind: DWORD);
var
  Raw: String;
  Kind: DWORD;
begin
  Check(ReadUserPath(Env, Raw, Kind), 'read fixture PATH');
  Check((Raw = Want) and (Kind = WantKind), Format('wanted "%s" (%d), found "%s" (%d)', [Want, WantKind, Raw, Kind]));
end;

procedure TestPath;
var
  Before: String;
  Changed: Boolean;
  NonString: DWORD;
begin
  Check(AddUserPath(Env, Ownership, Dir, Changed) and Changed, 'add to missing PATH');
  ExpectPath(Dir, RegExpandString);
  Check(RemoveUserPaths(Env, Ownership, Changed) and Changed, 'uninstall backs out missing PATH addition');
  ExpectPath('', RegMissing);

  Before := ' C:\Other ;"C:/Synthetic Apps/CLAUDE SWITCH/";';
  Check(RegWriteStringValue(HKCU, Env, PathValue, Before), 'seed pre-existing entry');
  Check(AddUserPath(Env, Ownership, Dir, Changed) and not Changed, 'equivalent entry is not duplicated');
  Check(RemoveUserPaths(Env, Ownership, Changed) and not Changed, 'pre-existing entry is not owned');
  ExpectPath(Before, RegString);

  Before := '%SystemRoot%\System32;;C:\Keep\';
  Check(RegWriteExpandStringValue(HKCU, Env, PathValue, Before), 'seed expandable PATH');
  Check(AddUserPath(Env, Ownership, GetEnv('SystemRoot') + '\System32', Changed) and not Changed,
    'expandable entry is not duplicated');
  ExpectPath(Before, RegExpandString);
  Check(AddUserPath(Env, Ownership, Dir, Changed) and Changed, 'append to expandable PATH');
  ExpectPath(Before + ';' + Dir, RegExpandString);
  Check(not WriteUnchangedPath(Env, Before, 'stale edit', RegExpandString, False), 'refuse stale read');
  Check(RemoveUserPaths(Env, Ownership, Changed) and Changed, 'remove from expandable PATH');
  ExpectPath(Before, RegExpandString);

  Before := 'C:\Before;;';
  Check(RegDeleteValue(HKCU, Env, PathValue), 'remove synthetic value so the string writer can seed a new type');
  Check(RegWriteStringValue(HKCU, Env, PathValue, Before), 'seed literal PATH with empty tokens');
  Check(AddUserPath(Env, Ownership, Dir, Changed) and Changed, 'append one entry');
  Check(AddUserPath(Env, Ownership, Dir, Changed) and not Changed, 'reinstall is idempotent');
  ExpectPath(Before + ';' + Dir, RegString);
  Check(AddUserPath(Env, Ownership, Dir2, Changed) and Changed, 'track another install folder');
  Check(RegWriteStringValue(HKCU, Env, PathValue, Before + ';' + Dir + ';' + Dir2 + ';C:\Added later'),
    'simulate a subsequent user edit');
  Check(RemoveUserPaths(Env, Ownership, Changed) and Changed, 'remove only app-owned entries');
  ExpectPath(Before + ';C:\Added later', RegString);

  Check(AddUserPath(Env, Ownership, Dir, Changed) and Changed, 'add again');
  Check(RegWriteStringValue(HKCU, Env, PathValue, Dir + ';"' + Dir + '"'), 'simulate a duplicate');
  Check(not RemoveUserPaths(Env, Ownership, Changed) and not Changed, 'leave ambiguous duplicates alone');
  ExpectPath(Dir + ';"' + Dir + '"', RegString);
  Check(RegWriteStringValue(HKCU, Env, PathValue, 'C:\New'), 'simulate a user removal');
  Check(RemoveUserPaths(Env, Ownership, Changed) and not Changed, 'drop obsolete ownership');
  ExpectPath('C:\New', RegString);

  Check(RegWriteStringValue(HKCU, Ownership, Dir, 'not an ownership record'), 'seed an unknown marker');
  Check(not AddUserPath(Env, Ownership, Dir, Changed) and not Changed, 'refuse conflicting marker');
  ExpectPath('C:\New', RegString);
  Check(RegDeleteValue(HKCU, Ownership, Dir), 'remove synthetic marker');

  Check(RegWriteDWordValue(HKCU, Env, PathValue, 123), 'seed unsupported value type');
  Check(not AddUserPath(Env, Ownership, Dir, Changed) and not Changed, 'refuse to replace unsupported type');
  Check(RegQueryDWordValue(HKCU, Env, PathValue, NonString) and (NonString = 123), 'unsupported type unchanged');
end;

function InitializeSetup: Boolean;
begin
  Result := False;
  { A preexisting key may be someone else's. Never enter or clean it up. }
  Check(not RegKeyExists(HKCU, Fixture), 'fixture key already in use');
  try
    TestPath;
    Log('PATH tests passed (isolated HKCU fixture).');
  finally
    RegDeleteValue(HKCU, Env, PathValue);
    RegDeleteKeyIfEmpty(HKCU, Env);
    RegDeleteValue(HKCU, Ownership, Dir);
    RegDeleteValue(HKCU, Ownership, Dir2);
    RegDeleteKeyIfEmpty(HKCU, Ownership);
    RegDeleteKeyIfEmpty(HKCU, Fixture);
  end;
end;
