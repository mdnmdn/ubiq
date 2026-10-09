; Ubiq per-user installer. Build: iscc /DMyAppVersion=1.2.3 _devops/windows/ubiq.iss
; Silent install (what the in-app updater runs): /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CURRENTUSER
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif

[Setup]
AppId={{6F1C2B7A-3D4E-4B8F-9A51-7C0E2D9B4A13}
AppName=Ubiq
AppVersion={#MyAppVersion}
AppPublisher=Ubiq
DefaultDirName={localappdata}\Programs\Ubiq
DefaultGroupName=Ubiq
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline
CloseApplications=yes
RestartApplications=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\ubiq.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SourceDir=..\..
OutputDir=.
OutputBaseFilename=Ubiq-Setup-x86_64

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop icon"; GroupDescription: "Additional icons:"
Name: "foldermenu"; Description: "Add ""Open with Ubiq"" to the folder context menu"; GroupDescription: "Shell integration:"; Flags: unchecked

[Files]
Source: "target\ubiq-windows-x86_64\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Ubiq"; Filename: "{app}\ubiq.exe"
Name: "{autodesktop}\Ubiq"; Filename: "{app}\ubiq.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Ubiq"; ValueType: string; ValueName: ""; ValueData: "Open with Ubiq"; Flags: uninsdeletekey; Tasks: foldermenu
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Ubiq"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\ubiq.exe"; Tasks: foldermenu
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Ubiq\command"; ValueType: string; ValueName: ""; ValueData: """{app}\ubiq.exe"" ""%1"""; Tasks: foldermenu

[Run]
Filename: "{app}\ubiq.exe"; Description: "Launch Ubiq"; Flags: nowait postinstall skipifsilent
