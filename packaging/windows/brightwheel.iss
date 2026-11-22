#ifndef AppVersion
  #error AppVersion must be supplied by scripts/package-windows.ps1
#endif
#ifndef BinaryDirectory
  #error BinaryDirectory must point to the production executables
#endif
#ifndef OutputDirectory
  #error OutputDirectory must be supplied
#endif

#define AppName "BrightWheel"
#define AppExeName "brightwheel.exe"
#define PackageIdentifier "QQRM.BrightWheel"
#define RunValueName "BrightWheel"
#define AppId "QQRM.BrightWheel"
#define RestartArguments "--skip-startup-update --skip-if-running"
#define QuitPidArgument "--quit-for-update="
#define AutostartArguments ""
#define AdditionalPayloadName "bright.exe"
#define DefaultInstallDirectory "{localappdata}\Programs\BrightWheel"

[Setup]
AppId={#AppId}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=qqrm
AppPublisherURL=https://github.com/qqrm
AppSupportURL=https://github.com/qqrm/brightwheel/issues
AppUpdatesURL=https://github.com/qqrm/brightwheel/releases/latest
DefaultDirName={code:GetDefaultInstallDirectory}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
SetupMutex=Local\QQRM.BrightWheel.Setup
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
UninstallDisplayIcon={app}\{#AppExeName}
SetupIconFile=..\..\assets\brightwheel.ico
LicenseFile=..\..\LICENSE
OutputDir={#OutputDirectory}
OutputBaseFilename=brightwheel-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=no
RestartApplications=no
ChangesEnvironment=yes
VersionInfoVersion={#AppVersion}

[Files]
Source: "{#BinaryDirectory}\brightwheel.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinaryDirectory}\bright.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\{#AppName}"; Filename: "{app}\{#AppExeName}"

[Code]
#include "app-lifecycle.iss"
