; Pagify Desktop — Windows installer.
;
; Built with Inno Setup rather than a hand-copied folder of loose files.
; Structure choices here are deliberately the ones a mainstream, trusted
; installer uses, since an unsigned exe's biggest strike against it with
; Windows is looking like nothing else Windows has ever seen:
;   - a per-user install under %LocalAppData%\Programs, the same place VS
;     Code, Discord and most other modern Windows apps install to — no admin
;     prompt, no UAC elevation, nothing that reads as "this wants to change
;     your whole machine".
;   - a real Start Menu entry and an Add/Remove Programs uninstall entry,
;     both with a proper publisher name, product name and version — the
;     metadata a bare renamed .exe never carries.
;   - the installer's own icon set explicitly, so it is not one more
;     nameless grey-icon .exe in Downloads.
;   - no packer, no obfuscation: Inno Setup's own installer stub is one of
;     the most common on Windows and is not what "packed to evade AV" looks
;     like structurally.
;
; None of this replaces a real code-signing certificate — that is the actual
; fix for the SmartScreen "Windows protected your PC" prompt, and nothing
; about folder layout changes that. This gets everything else right so that,
; once a certificate exists, signing is the only remaining step — see
; SIGNING_THUMBPRINT in build.ps1, which this can be pointed at the same way.

#define MyAppName "Pagify"
#define MyAppVersion "0.1.47"
#define MyAppPublisher "HSI Lighting"
#define MyAppExeName "Pagify.exe"

[Setup]
; Fixed once, kept stable across versions — this is what tells Windows a
; new installer is an *update* to the same app rather than a different one.
AppId={{7C9F3C9E-6C37-4A0B-9E0B-6E4E4B9E9C7A}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
; Per-user by default (no UAC prompt at all) but lets someone who wants an
; all-users install choose that instead, rather than forcing either one.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DisableProgramGroupPage=yes
OutputDir=C:\Users\hsili\Desktop\Pgify Installation
OutputBaseFilename=PagifyDesktopSetup
SetupIconFile=..\..\assets\pagify-logo.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Files]
Source: "..\..\target\release\pagify_app.exe"; DestDir: "{app}"; DestName: "{#MyAppExeName}"; Flags: ignoreversion
Source: "..\..\third_party\pdfium\pdfium-win-x64\bin\pdfium.dll"; DestDir: "{app}"; Flags: ignoreversion
; The OCR models (`pagify_shell::models` — "beside the executable… is what an
; installed app uses") were missing from here entirely: fonts and the
; dictionary are embedded in the exe itself (`include_bytes!`/`include_str!`),
; but these two are loaded from disk, so `extracttext` silently had nothing
; to load on any machine that only ever ran this installer. Reported from a
; fresh install that got past the earlier VCRUNTIME/pdfium.dll failures and
; still would have hit this one next.
Source: "..\..\third_party\ocr\text-detection.rten"; DestDir: "{app}\ocr"; Flags: ignoreversion
Source: "..\..\third_party\ocr\text-recognition.rten"; DestDir: "{app}\ocr"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch {#MyAppName}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: filesandordirs; Name: "{app}"
