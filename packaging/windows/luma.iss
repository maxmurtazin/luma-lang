#define AppVersion "0.1.0"
[Setup]
AppId={{645701D5-8D27-43BB-9AE5-215EFB75CA84}
AppName=Luma
AppVersion={#AppVersion}
AppPublisher=maxmurtazin
DefaultDirName={localappdata}\Programs\Luma
DisableDirPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\dist
OutputBaseFilename=Luma-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
ChangesEnvironment=no
UninstallDisplayIcon={app}\bin\luma.exe
[Tasks]
Name: "vscode"; Description: "Install Graph Explorer extension in VS Code (if available)"
[Files]
Source: "..\..\target\x86_64-pc-windows-msvc\release\luma.exe"; DestDir: "{app}\bin"; Flags: ignoreversion
Source: "..\..\editors\vscode\luma-language-tools-0.1.0.vsix"; DestDir: "{app}"; Flags: ignoreversion
Source: "Install-Extension.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\docs\windows-installation.md"; DestDir: "{app}"; Flags: ignoreversion
[Run]
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\Install-Extension.ps1"" -InstallDir ""{app}"""; Flags: runhidden waituntilterminated; Tasks: vscode
Filename: "{app}\extension-install.log"; Description: "View VS Code extension installation result"; Flags: shellexec postinstall skipifsilent unchecked; Tasks: vscode
[Code]
procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and not WizardSilent then
    MsgBox('Luma CLI is installed. Restart or reload VS Code after setup. If extension installation fails or VS Code is missing, install the bundled VSIX manually from the Luma folder. See windows-installation.md in that folder.', mbInformation, MB_OK);
end;
