; Inno Setup script. Called by build_windows.ps1:
;   ISCC /DAppVersion=1.0.0 /DArch=x64 /DSourceDir=...\stage\RadioMoon-MPX /DOutDir=...\dist installer\radiomoon-mpx.iss
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Arch
  #define Arch "x64"
#endif
#ifndef SourceDir
  #define SourceDir "..\stage\RadioMoon-MPX"
#endif
#ifndef OutDir
  #define OutDir "..\dist"
#endif

[Setup]
AppId={{4C9E2B71-8A35-4E0D-B6F2-7D1A93C5E0B4}
AppName=RadioMoon MPX
AppVersion={#AppVersion}
AppPublisher=RadioMoon
DefaultDirName={autopf}\RadioMoon MPX
DefaultGroupName=RadioMoon MPX
PrivilegesRequired=admin
OutputDir={#OutDir}
OutputBaseFilename=RadioMoon-MPX-{#AppVersion}-windows-{#Arch}-setup
SetupIconFile={#SourcePath}..\packaging\transmitter.ico
UninstallDisplayIcon={app}\RadioMoon-MPX-Transmitter.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
DisableProgramGroupPage=yes
#if Arch == "x64"
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#elif Arch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "clap"; Description: "Install the CLAP plug-in (RadioMoon MPX Transmitter) for audio programs: Common Files\CLAP"; GroupDescription: "Plug-in:"

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion
; the standard CLAP folder, where REAPER, Bitwig, FL Studio, Studio One ... look for plug-ins
Source: "{#SourceDir}\CLAP\RadioMoon-MPX-Transmitter.clap"; DestDir: "{commoncf}\CLAP"; Tasks: clap; Flags: ignoreversion

[Dirs]
; settings shared by all users of the PC (mpx-transmitter.json, mpx-receiver.json)
Name: "{commonappdata}\RadioMoon"; Permissions: users-modify

[Icons]
Name: "{group}\RadioMoon MPX Transmitter"; Filename: "{app}\RadioMoon-MPX-Transmitter.exe"
Name: "{group}\RadioMoon MPX Receiver"; Filename: "{app}\RadioMoon-MPX-Receiver.exe"
Name: "{autodesktop}\RadioMoon MPX Transmitter"; Filename: "{app}\RadioMoon-MPX-Transmitter.exe"; Tasks: desktopicon
Name: "{autodesktop}\RadioMoon MPX Receiver"; Filename: "{app}\RadioMoon-MPX-Receiver.exe"; Tasks: desktopicon
