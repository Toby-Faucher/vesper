; The Windows installer (Inno Setup 6), built by .github/workflows/desktop-package.yml:
;   ISCC /DAppVersion=2.3.0 /DSource=<folder with Vesper.exe and the text files> /O<out> vesper.iss
; Per user, no admin: the app goes to %LOCALAPPDATA%\Programs\Vesper, where the app's own updater can swap Vesper.exe.
; It adds a Start menu entry, an optional desktop shortcut, the vesper:// link handler and an entry in Installed apps.
; Verified by CI only.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Source
  #define Source "..\..\..\dist\Vesper"
#endif

[Setup]
; Never change the AppId: Windows finds the installed copy by it.
AppId={{6C1C8E4E-5B7A-4C59-9C1E-0D3F2A9B7E41}
AppName=Vesper
AppVersion={#AppVersion}
AppPublisher=Vesper
AppPublisherURL=https://github.com/Toby-Faucher/vesper
DefaultDirName={autopf}\Vesper
DefaultGroupName=Vesper
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputBaseFilename=vesper-desktop-windows-x64-setup
SetupIconFile=vesper.ico
UninstallDisplayIcon={app}\Vesper.exe
UninstallDisplayName=Vesper
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; A running Vesper is asked to close before its files are replaced.
CloseApplications=yes
LicenseFile={#Source}\LICENSE.txt

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Source}\Vesper.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\README.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\THIRD-PARTY.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Vesper"; Filename: "{app}\Vesper.exe"
Name: "{autodesktop}\Vesper"; Filename: "{app}\Vesper.exe"; Tasks: desktopicon

[Registry]
; vesper:// links open Vesper, which hands them to the copy already running.
Root: HKCU; Subkey: "Software\Classes\vesper"; ValueType: string; ValueName: ""; ValueData: "URL:Vesper"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\vesper"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\vesper\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\Vesper.exe"",0"
Root: HKCU; Subkey: "Software\Classes\vesper\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\Vesper.exe"" ""%1"""

[UninstallDelete]
; The copy the updater keeps for a rollback.
Type: files; Name: "{app}\Vesper.exe.old"

[Run]
Filename: "{app}\Vesper.exe"; Description: "{cm:LaunchProgram,Vesper}"; Flags: nowait postinstall skipifsilent
