#requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$Version,
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^https?://[^\r\n]+$')]
    [string]$InstallerUrl,
    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$hash = (Get-FileHash -LiteralPath $InstallerPath -Algorithm SHA256).Hash
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
# Both product identities are needed: WinGet must select this Inno installer
# while the installed version is still the original portable package.
$installerManifest = @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.installer.1.28.0.schema.json
PackageIdentifier: QQRM.BrightWheel
PackageVersion: $Version
InstallerType: inno
Scope: user
UpgradeBehavior: install
InstallModes:
- interactive
- silent
- silentWithProgress
AppsAndFeaturesEntries:
- DisplayName: BrightWheel
  Publisher: qqrm
  ProductCode: QQRM.BrightWheel_is1
  InstallerType: inno
- ProductCode: QQRM.BrightWheel_Microsoft.Winget.Source_8wekyb3d8bbwe
  InstallerType: portable
Installers:
- Architecture: x64
  InstallerUrl: $InstallerUrl
  InstallerSha256: $hash
ManifestType: installer
ManifestVersion: 1.28.0
"@
$localeManifest = @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.defaultLocale.1.28.0.schema.json
PackageIdentifier: QQRM.BrightWheel
PackageVersion: $Version
PackageLocale: en-US
Publisher: qqrm
PublisherUrl: https://github.com/qqrm
PackageName: BrightWheel
PackageUrl: https://github.com/qqrm/brightwheel
License: MIT
LicenseUrl: https://github.com/qqrm/brightwheel/blob/main/LICENSE
ShortDescription: Windows tray utility for DDC/CI monitor brightness and HDR control
ReleaseNotesUrl: https://github.com/qqrm/brightwheel/releases/tag/v$Version
ManifestType: defaultLocale
ManifestVersion: 1.28.0
"@
$versionManifest = @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.version.1.28.0.schema.json
PackageIdentifier: QQRM.BrightWheel
PackageVersion: $Version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.28.0
"@
$installerManifest | Set-Content (Join-Path $OutputDirectory 'QQRM.BrightWheel.installer.yaml') -Encoding utf8NoBOM
$localeManifest | Set-Content (Join-Path $OutputDirectory 'QQRM.BrightWheel.locale.en-US.yaml') -Encoding utf8NoBOM
$versionManifest | Set-Content (Join-Path $OutputDirectory 'QQRM.BrightWheel.yaml') -Encoding utf8NoBOM
Write-Host "Generated WinGet manifests in $OutputDirectory"
