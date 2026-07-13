; K4 Remote — Windows installer (Inno Setup 6).
; Produces a setup.exe that installs k4remote.exe with Start-menu / optional
; desktop shortcuts and an uninstaller. The application version is passed in by
; the release workflow: ISCC.exe /DMyAppVersion=0.2.1 packaging\windows\k4remote.iss
; Source/output paths are relative to this script's directory (packaging\windows).

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif
#define MyAppName "K4 Remote"
#define MyAppPublisher "Simon Keimer (DC0SK)"
#define MyAppURL "https://github.com/dc0sk/K4remote"
#define MyAppExeName "k4remote.exe"

[Setup]
; A stable AppId so upgrades replace the previous install (never change this).
AppId={{8F2C4E7A-3B9D-4C1E-A5F6-2D8E0B1C3A4F}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={autopf}\K4Remote
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
LicenseFile=..\..\LICENSE
OutputDir=..\..
OutputBaseFilename=k4remote-windows-x86_64-setup
SetupIconFile=..\icons\k4remote.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; 64-bit only (matches the x86_64 build).
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
