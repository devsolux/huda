; Native, per-user installer. Identity/version come from Cargo.toml via package_desktop.py.
#ifndef AppVersion
  #error "Invoke scripts/package_desktop.py; do not invent a product version"
#endif
[Setup]
AppId={#AppIdentifier}
AppName=Huda
AppVersion={#AppVersion}
AppPublisher=DevSolux
DefaultDirName={localappdata}\Programs\Huda
DefaultGroupName=Huda
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename={#OutputName}
SetupIconFile={#StageDir}\huda.ico
UninstallDisplayIcon={app}\huda-desktop.exe
LicenseFile={#StageDir}\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=no
RestartApplications=no

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Huda"; Filename: "{app}\huda-desktop.exe"; AppUserModelID: "{#AppIdentifier}"

; Deliberately no Run/startup task, no service, no profile deletion and no auto-launch after install.
[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Value, ExecutablePath: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    ExecutablePath := Lowercase(ExpandConstant('{app}\huda-desktop.exe'));
    if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', '{#AppIdentifier}', Value) then
    begin
      Value := Lowercase(Value);
      { Remove only this installation's per-user login entry, not an entry pointing elsewhere. }
      if (Pos('"' + ExecutablePath + '"', Value) = 1) or
         (Pos(ExecutablePath + ' ', Value) = 1) or (Value = ExecutablePath) then
        RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', '{#AppIdentifier}');
    end;
  end;
end;
