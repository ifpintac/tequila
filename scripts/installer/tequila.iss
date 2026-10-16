; Tequila's Windows installer, for Inno Setup (jrsoftware.org).
;
; Built by scripts/package-installer.ps1 from a package that
; scripts/package-windows.ps1 made, which defines SourceDir, AppVersion,
; NumericVersion and OutputDir on the command line.
;
; Installs for the person running it, under %LOCALAPPDATA%\Programs, so it
; needs no administrator rights - the usual position on a work PC. Later
; versions install over earlier ones (same AppId) and keep what is the
; user's: their own materials, plugins and settings live in %APPDATA%\Tequila,
; never in the program folder.

#ifndef SourceDir
  #error SourceDir is not defined: build with scripts/package-installer.ps1
#endif
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef NumericVersion
  #define NumericVersion "0.0.0.0"
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif

[Setup]
; Never change: it is how a later installer finds this installation.
AppId={{EDD88596-57CD-4475-98DD-4D3B719E8AB3}
AppName=Tequila
AppVersion={#AppVersion}
AppVerName=Tequila {#AppVersion}
AppPublisher=Tequila
VersionInfoVersion={#NumericVersion}
VersionInfoDescription=Tequila installer
DefaultDirName={autopf}\Tequila
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=Tequila-Setup-{#AppVersion}
SetupIconFile=tequila.ico
UninstallDisplayIcon={app}\tequila.ico
UninstallDisplayName=Tequila
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
LZMAUseSeparateProcess=yes
ChangesAssociations=yes
; A Tequila left open is offered to be closed before its files are
; replaced; it asks about unsaved work as it closes.
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "association"; Description: "Open .tequila project files with Tequila"; GroupDescription: "Files:"

[Dirs]
; Where renders go until a project is saved somewhere (the shortcuts start
; Tequila here). Kept on uninstall: it holds the user's pictures.
Name: "{code:PicturesDir}\Tequila"; Flags: uninsneveruninstall

[InstallDelete]
; What a newer version replaces wholesale, so a file an older one shipped
; and this one does not cannot linger and be loaded.
Type: filesandordirs; Name: "{app}\assets"
Type: filesandordirs; Name: "{app}\cycles"
Type: filesandordirs; Name: "{app}\licenses"
Type: filesandordirs; Name: "{app}\plugins"
Type: filesandordirs; Name: "{app}\python"
Type: files; Name: "{app}\*.dll"

[Files]
; The portable package's own read-me is about unzipping it, which an
; installed copy has no use for.
Source: "{#SourceDir}\*"; DestDir: "{app}"; Excludes: "START-HERE.txt"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "tequila.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Tequila"; Filename: "{app}\Tequila.exe"; WorkingDir: "{code:PicturesDir}\Tequila"; IconFilename: "{app}\tequila.ico"; Comment: "Render products in Tequila"
Name: "{autodesktop}\Tequila"; Filename: "{app}\Tequila.exe"; WorkingDir: "{code:PicturesDir}\Tequila"; IconFilename: "{app}\tequila.ico"; Comment: "Render products in Tequila"; Tasks: desktopicon

[Registry]
; HKA is the current user's classes here, as the install is per user.
Root: HKA; Subkey: "Software\Classes\.tequila"; ValueType: string; ValueName: ""; ValueData: "Tequila.Project"; Flags: uninsdeletevalue; Tasks: association
Root: HKA; Subkey: "Software\Classes\.tequila\OpenWithProgids"; ValueType: string; ValueName: "Tequila.Project"; ValueData: ""; Flags: uninsdeletevalue; Tasks: association
Root: HKA; Subkey: "Software\Classes\Tequila.Project"; ValueType: string; ValueName: ""; ValueData: "Tequila project"; Flags: uninsdeletekey; Tasks: association
Root: HKA; Subkey: "Software\Classes\Tequila.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\tequila.ico,0"; Tasks: association
Root: HKA; Subkey: "Software\Classes\Tequila.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\Tequila.exe"" ""%1"""; Tasks: association

[Run]
Filename: "{app}\Tequila.exe"; WorkingDir: "{code:PicturesDir}\Tequila"; Description: "{cm:LaunchProgram,Tequila}"; Flags: nowait postinstall skipifsilent

[Code]
// The user's Pictures folder, wherever it is: Windows records its real
// place, OneDrive's included, under Shell Folders. Inno Setup has no
// constant for it.
function PicturesDir(Param: String): String;
begin
  if not RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders', 'My Pictures', Result)
    or (Result = '') or not DirExists(Result) then
    Result := ExpandConstant('{%USERPROFILE}\Pictures');
end;
