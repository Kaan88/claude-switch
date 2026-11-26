#ifndef AppVersion
  #define AppVersion "0.1.1"
#endif
#ifndef Exe
  #define Exe "..\target\release\claude-switch.exe"
#endif
#ifndef Notices
  #define Notices "..\THIRD-PARTY-NOTICES"
#endif

[Setup]
AppId={{4F2C369B-84AF-4A46-AE8B-D22103025F78}
AppName=Claude Switch
AppVersion={#AppVersion}
AppPublisher=Kaan
AppPublisherURL=https://github.com/Kaan88/claude-switch
DefaultDirName={localappdata}\Programs\Claude Switch
PrivilegesRequired=lowest
; Quit through the app so it can ask about running terminal tabs.
CloseApplications=no
DisableProgramGroupPage=yes
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\icon.ico
Uninstallable=yes
OutputDir=..\dist
OutputBaseFilename=claude-switch-setup
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "userpath"; Description: "Add Claude Switch to PATH"

[Files]
Source: "{#Exe}"; DestDir: "{app}"; DestName: "claude-switch.exe"; Flags: ignoreversion
Source: "..\assets\icon.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Notices}"; DestDir: "{app}"; DestName: "THIRD-PARTY-NOTICES"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Claude Switch"; Filename: "{app}\claude-switch.exe"; WorkingDir: "{app}"; IconFilename: "{app}\icon.ico"
Name: "{autodesktop}\Claude Switch"; Filename: "{app}\claude-switch.exe"; WorkingDir: "{app}"; IconFilename: "{app}\icon.ico"; Tasks: desktopicon

; User settings and Claude profiles are not installed or uninstalled by this script.

[Code]
#include "user-path.iss"

const
  UserEnvironment = 'Environment';
  PathOwnership = 'Software\Claude Switch\SetupPath';

procedure CurStepChanged(CurStep: TSetupStep);
var
  Changed: Boolean;
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('userpath') then begin
    if not AddUserPath(UserEnvironment, PathOwnership, ExpandConstant('{app}'), Changed) then
      SuppressibleMsgBox('Claude Switch was installed, but Setup could not finish updating the current user PATH.',
        mbInformation, MB_OK, IDOK);
    if Changed then NotifyEnvironmentChanged;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Changed: Boolean;
begin
  if CurUninstallStep = usPostUninstall then begin
    if not RemoveUserPaths(UserEnvironment, PathOwnership, Changed) then
      SuppressibleMsgBox('Setup could not safely remove all of its PATH entries. Check your user PATH in Windows Environment Variables.',
        mbInformation, MB_OK, IDOK);
    if Changed then NotifyEnvironmentChanged;
  end;
end;
