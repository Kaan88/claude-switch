#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef Exe
  #define Exe "..\target\release\claude-switch.exe"
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

[Files]
Source: "{#Exe}"; DestDir: "{app}"; DestName: "claude-switch.exe"; Flags: ignoreversion
Source: "..\assets\icon.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Claude Switch"; Filename: "{app}\claude-switch.exe"; WorkingDir: "{app}"; IconFilename: "{app}\icon.ico"
Name: "{autodesktop}\Claude Switch"; Filename: "{app}\claude-switch.exe"; WorkingDir: "{app}"; IconFilename: "{app}\icon.ico"; Tasks: desktopicon

; The uninstaller removes installed files and shortcuts only. User settings
; and Claude profiles are not installed or uninstalled by this script.
