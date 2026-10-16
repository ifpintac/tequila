; An update for an installed Tequila: only the files that changed since
; the version it was made from, for Inno Setup (jrsoftware.org).
;
; Built by scripts/package-installer.ps1 -PatchFrom <the installed version's
; package>, which stages the changed and new files in SourceDir, lists the
; files the new version no longer has in RemovedList, and defines
; AppVersion, NumericVersion, BaseVersion and OutputDir.
;
; It updates the installation in place, wherever it is, and refuses any
; other version: files it does not carry are taken to be the base
; version's, so on anything else it would leave a mix. The same AppId as
; the full installer, so Settings > Apps shows the new version and the
; uninstaller still removes everything.

#ifndef SourceDir
  #error SourceDir is not defined: build with scripts/package-installer.ps1 -PatchFrom
#endif
#ifndef BaseVersion
  #error BaseVersion is not defined
#endif

[Setup]
AppId={{EDD88596-57CD-4475-98DD-4D3B719E8AB3}
AppName=Tequila
AppVersion={#AppVersion}
AppVerName=Tequila {#AppVersion}
AppPublisher=Tequila
VersionInfoVersion={#NumericVersion}
VersionInfoDescription=Tequila update from {#BaseVersion}
DefaultDirName={autopf}\Tequila
UsePreviousAppDir=yes
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableWelcomePage=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=Tequila-Update-{#BaseVersion}-to-{#AppVersion}
SetupIconFile=tequila.ico
UninstallDisplayIcon={app}\tequila.ico
UninstallDisplayName=Tequila
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Messages]
WelcomeLabel2=This updates Tequila from version {#BaseVersion} to {#AppVersion}, replacing only what changed. Your materials, plugins and settings stay as they are.

[InstallDelete]
#include RemovedList

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Run]
Filename: "{app}\Tequila.exe"; Description: "{cm:LaunchProgram,Tequila}"; Flags: nowait postinstall skipifsilent

[Code]
const
  Uninstall = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{EDD88596-57CD-4475-98DD-4D3B719E8AB3}_is1';

// Only onto the version it was made from: anything it does not carry is
// taken to be that version's.
function InitializeSetup(): Boolean;
var
  Installed: String;
begin
  Result := False;
  if not RegQueryStringValue(HKCU, Uninstall, 'DisplayVersion', Installed) then
  begin
    SuppressibleMsgBox('Tequila is not installed for this Windows account, so there is nothing to update. ' +
      'Run the full installer (Tequila-Setup) instead.', mbError, MB_OK, IDOK);
    exit;
  end;
  if Installed <> '{#BaseVersion}' then
  begin
    SuppressibleMsgBox('This update is for Tequila {#BaseVersion}, and this PC has Tequila ' + Installed + '. ' +
      'Run the full installer (Tequila-Setup) instead: it updates any version in place.', mbError, MB_OK, IDOK);
    exit;
  end;
  Result := True;
end;
