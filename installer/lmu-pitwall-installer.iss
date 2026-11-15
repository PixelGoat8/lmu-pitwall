; LMU Pitwall — Windows installer (Inno Setup 6).
;
; Windows: installer\build.ps1 compiles the dashboard and the bridge, then
; this script. Linux: scripts/release.sh, through Wine, with the sources
; passed in (/DExeSource=... /DLicenseSource=... /DIconSource=...).
;
; One installer for everyone: a driver keeps the defaults; the engineer also
; ticks "Pitwall Hub".

; Kept in step with bridge/Cargo.toml by scripts/bump-version.py.
#ifndef MyAppVersion
  #define MyAppVersion "1.3.0"
#endif
#ifndef ExeSource
  #define ExeSource "..\bridge\target\release\lmu-pitwall.exe"
#endif
#ifndef LicenseSource
  #define LicenseSource "..\LICENSE"
#endif
#ifndef IconSource
  #define IconSource "..\bridge\assets\icon.ico"
#endif
#define AppName "LMU Pitwall"
#define AppExe "lmu-pitwall.exe"
#define AppUrl "https://github.com/PixelGoat8/lmu-pitwall"
#define GuideUrl "https://github.com/PixelGoat8/lmu-pitwall/blob/remote-engineer/docs/guida-pilota.md"
#define FirewallRule "LMU Pitwall"

[Setup]
; A fixed id: a newer installer upgrades the installed copy in place.
AppId={{6F3C2B9E-0E57-4C61-9F1A-7B0D5C2A4E11}
AppName={#AppName}
AppVersion={#MyAppVersion}
AppVerName={#AppName} {#MyAppVersion}
AppPublisher=PixelGoat8
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
; Admin rights only for the firewall rule; settings and history stay in each
; user's %APPDATA%\LMUPitwall.
PrivilegesRequired=admin
UsedUserAreasWarning=no
LicenseFile={#LicenseSource}
SetupIconFile={#IconSource}
UninstallDisplayIcon={app}\{#AppExe}
OutputDir=Output
OutputBaseFilename=LMU-Pitwall-Setup-{#MyAppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; A running Pitwall is closed so the program file can be replaced.
CloseApplications=force
RestartApplications=no

[Languages]
Name: "italian"; MessagesFile: "compiler:Languages\Italian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
italian.TaskShortcuts=Icone:
english.TaskShortcuts=Shortcuts:
italian.TaskDesktop=Icona di LMU Pitwall sul desktop
english.TaskDesktop=LMU Pitwall icon on the desktop
italian.TaskHub=Icona "Pitwall Hub" (per l'ingegnere che segue tutta la squadra)
english.TaskHub="Pitwall Hub" icon (for the engineer following the whole team)
italian.TaskOptions=Opzioni:
english.TaskOptions=Options:
italian.TaskFirewall=Consenti le connessioni da Tailscale e dalla rete di casa (consigliato)
english.TaskFirewall=Allow connections from Tailscale and the home network (recommended)
italian.TaskAutostart=Avvia Pitwall con Windows (resta acceso in background)
english.TaskAutostart=Start Pitwall with Windows (stays on in the background)
italian.HubName=Pitwall Hub
english.HubName=Pitwall Hub
italian.GuideName=Guida per i piloti
english.GuideName=Driver guide
italian.RunNow=Avvia LMU Pitwall adesso
english.RunNow=Start LMU Pitwall now
italian.OpenGuide=Apri la guida
english.OpenGuide=Open the guide

[Tasks]
Name: "desktopicon"; Description: "{cm:TaskDesktop}"; GroupDescription: "{cm:TaskShortcuts}"
Name: "hub"; Description: "{cm:TaskHub}"; GroupDescription: "{cm:TaskShortcuts}"; Flags: unchecked
Name: "firewall"; Description: "{cm:TaskFirewall}"; GroupDescription: "{cm:TaskOptions}"
Name: "autostart"; Description: "{cm:TaskAutostart}"; GroupDescription: "{cm:TaskOptions}"; Flags: unchecked

[Files]
Source: "{#ExeSource}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#LicenseSource}"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autoprograms}\{cm:HubName}"; Filename: "{app}\{#AppExe}"; Parameters: "--hub"; Tasks: hub
Name: "{autoprograms}\{cm:GuideName}"; Filename: "{#GuideUrl}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon
Name: "{autodesktop}\{cm:HubName}"; Filename: "{app}\{#AppExe}"; Parameters: "--hub"; Tasks: hub and desktopicon
; At login: no browser window, and no exit after ten idle minutes (it waits
; for the game instead).
Name: "{userstartup}\{#AppName}"; Filename: "{app}\{#AppExe}"; Parameters: "--no-browser --keep-alive"; Tasks: autostart

[Run]
; Inbound only from Tailscale (100.64.0.0/10) and the local network (the
; tablet at home); the bridge still asks remote clients for the token.
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""{#FirewallRule}"""; Flags: runhidden; Tasks: firewall
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""{#FirewallRule}"" dir=in action=allow program=""{app}\{#AppExe}"" enable=yes profile=any remoteip=100.64.0.0/10,LocalSubnet"; Flags: runhidden; Tasks: firewall
Filename: "{app}\{#AppExe}"; Description: "{cm:RunNow}"; Flags: postinstall nowait skipifsilent runasoriginaluser
Filename: "{#GuideUrl}"; Description: "{cm:OpenGuide}"; Flags: postinstall shellexec nowait skipifsilent unchecked runasoriginaluser

[UninstallRun]
; A running Pitwall would keep its program file locked.
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im {#AppExe}"; Flags: runhidden; RunOnceId: "StopPitwall"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""{#FirewallRule}"""; Flags: runhidden; RunOnceId: "DelFirewallRule"
