; Compile only from the complete, already-verified stage in package-windows.ps1.
#ifndef SourceDir
  #error SourceDir must name the verified Windows package stage
#endif
#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif
#ifndef OutputName
  #error OutputName is required
#endif

[Setup]
AppId={{17F3D01D-07B1-45AC-8A65-4304FDCE9158}
AppName=Clarora GPUI
AppVersion={#AppVersion}
AppPublisher=Clarora
DefaultDirName={localappdata}\Programs\Clarora GPUI
DefaultGroupName=Clarora GPUI
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir={#OutputDir}
OutputBaseFilename={#OutputName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\clarora-gpui.exe
CloseApplications=yes
RestartApplications=no
LicenseFile={#SourceDir}\LICENSE

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[InstallDelete]
; Remove obsolete runtime names during upgrades, before installing the new tree.
; {app} contains program files only; learning data is in %LOCALAPPDATA%\Clarora-GPUI.
Type: files; Name: "{app}\*.dll"
Type: filesandordirs; Name: "{app}\tools"
Type: filesandordirs; Name: "{app}\licenses"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Icons]
Name: "{group}\Clarora GPUI"; Filename: "{app}\clarora-gpui.exe"
Name: "{autodesktop}\Clarora GPUI"; Filename: "{app}\clarora-gpui.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\clarora-gpui.exe"; Description: "Launch Clarora GPUI"; Flags: nowait postinstall skipifsilent

; Application data is intentionally outside {app}; upgrades/uninstall preserve it.
