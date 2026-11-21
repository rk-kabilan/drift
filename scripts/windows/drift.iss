; Inno Setup script for the Drift installer. scripts\package-windows.ps1 runs it
; with the version and architecture; paths are relative to this file.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Arch
  #define Arch "x64"
#endif
#if Arch == "arm64"
  #define Allowed "arm64"
#else
  #define Allowed "x64compatible"
#endif

[Setup]
AppId={{6BD9B089-85AC-46E4-B8F7-1A5B0A92EA42}
AppName=Drift
AppVersion={#AppVersion}
AppVerName=Drift {#AppVersion}
AppPublisher=Drift
AppPublisherURL=https://github.com/rk-kabilan/drift
DefaultDirName={autopf}\Drift
DefaultGroupName=Drift
DisableProgramGroupPage=yes
; Installs for the current user without admin rights, or for everyone if they choose.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed={#Allowed}
ArchitecturesInstallIn64BitMode={#Allowed}
SetupIconFile=..\..\assets\app-icon\windows\drift.ico
UninstallDisplayIcon={app}\drift.exe
UninstallDisplayName=Drift
OutputBaseFilename=Drift-{#AppVersion}-setup-{#Arch}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\target\release\drift.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Drift"; Filename: "{app}\drift.exe"
Name: "{autodesktop}\Drift"; Filename: "{app}\drift.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\drift.exe"; Description: "{cm:LaunchProgram,Drift}"; Flags: nowait postinstall skipifsilent
