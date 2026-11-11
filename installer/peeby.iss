; Inno Setup script of the Peeby installer (ADR 0027, docs/release.md).
;
;   iscc /DAppVersion=1.0.0 installer\peeby.iss
;
; Expects a release build in target\release\peeby.exe, libmpv in third_party\mpv\
; (scripts/fetch-mpv.ps1) and yt-dlp + Deno in third_party\yt-dlp\ (scripts/fetch-tools.ps1);
; writes target\installer\peeby-setup-<version>-x64.exe, the name the updater looks for
; (crates/peeby-core/src/update/release.rs).

#ifndef AppVersion
  #error Pass the version: iscc /DAppVersion=x.y.z installer\peeby.iss
#endif

#define AppName "Peeby"
#define AppExe "peeby.exe"
; SourcePath is this script's folder, with a trailing backslash.
#define Root SourcePath + ".."

[Setup]
; Identifies the app for upgrades and uninstall: never change it.
AppId={{550B395A-8235-4DF4-BD46-1703B3C95941}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppName}
AppPublisherURL=https://github.com/Astalfr/peeby
VersionInfoVersion={#AppVersion}
; Per user only (%LOCALAPPDATA%\Programs\Peeby): no admin rights, and the folder stays writable
; for `yt-dlp -U`, which replaces tools\yt-dlp.exe (ADR 0005). An install in Program Files
; would break it, so the dialog offering one is off.
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
SetupIconFile={#Root}\crates\peeby\assets\peeby.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
LicenseFile={#Root}\LICENSE
OutputDir={#Root}\target\installer
OutputBaseFilename=peeby-setup-{#AppVersion}-x64
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; The wizard's language follows Windows', as Peeby's does (ADR 0014).
ShowLanguageDialog=no
; Closing Peeby's window only hides it in the notification area (ADR 0020), so Restart
; Manager's polite request can fail: `force` ends it, and the yt-dlp or Deno it started, which
; hold files of tools\. The updater has already quit it (crates/peeby-core/src/update).
CloseApplications=force
; Peeby is started again by [Run] below, not by Restart Manager (twice would hand the second
; launch to the first, ADR 0020).
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; The exe embeds its assets (icons, fonts, logo, sound).
Source: "{#Root}\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
; libmpv, loaded at runtime from next to the exe (peeby ADR 0007).
Source: "{#Root}\third_party\mpv\libmpv-2.dll"; DestDir: "{app}"; Flags: ignoreversion
; The VC++ runtime peeby.exe links against, app-local: Windows does not ship it
; (scripts/fetch-vcruntime.ps1).
Source: "{#Root}\third_party\vcruntime\vcruntime140.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\third_party\vcruntime\vcruntime140_1.dll"; DestDir: "{app}"; Flags: ignoreversion
; yt-dlp and Deno in tools\ (ADR 0005). yt-dlp updates itself once a day: an update of Peeby
; never puts the older pinned copy back over it.
Source: "{#Root}\third_party\yt-dlp\yt-dlp.exe"; DestDir: "{app}\tools"; Flags: onlyifdoesntexist
Source: "{#Root}\third_party\yt-dlp\deno.exe"; DestDir: "{app}\tools"; Flags: ignoreversion
Source: "{#Root}\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; What Peeby writes itself, removed with it: "Start with Windows" (the `Run` value,
; desktop/autostart.rs) and the toasts' registration (desktop/toast.rs).
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "{#AppName}"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\AppUserModelId\Peeby.Peeby"; ValueType: none; Flags: uninsdeletekey dontcreatekey

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
; An update started from Peeby runs silently and asks for Peeby back.
Filename: "{app}\{#AppExe}"; Parameters: "{code:RelaunchParameters}"; Flags: nowait runasoriginaluser; Check: Relaunch

[UninstallDelete]
; What yt-dlp's own update leaves next to it.
Type: filesandordirs; Name: "{app}\tools"

; Uninstalling leaves the user's data in place (%APPDATA%\peeby: settings and the encrypted
; cookie jar; %LOCALAPPDATA%\peeby: caches and logs; the Credential Manager's entries), so a
; reinstall keeps the sessions.

[Code]
// Peeby's updater passes /relaunch=1 (crates/peeby-core/src/update/github.rs).
function Relaunch: Boolean;
begin
  Result := WizardSilent and (ExpandConstant('{param:relaunch|0}') = '1');
end;

// /background=1: the update ran at a start with Windows, so Peeby comes back in the notification
// area only (crates/peeby-core/src/update/startup.rs).
function RelaunchParameters(Param: String): String;
begin
  if ExpandConstant('{param:background|0}') = '1' then
    Result := '--background'
  else
    Result := '';
end;
