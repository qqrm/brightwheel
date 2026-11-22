#requires -Version 7.0
[CmdletBinding()]
param(
    [ValidatePattern('^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$')]
    [string]$Version,
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot '../target/release'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../target/dist'),
    [string]$InstallerUrl,
    [string]$CompilerPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $Version) {
    Push-Location $root
    try {
        $metadata = & cargo metadata --no-deps --format-version 1
        if ($LASTEXITCODE -ne 0) { throw 'Could not read the Cargo package version.' }
        $package = ($metadata | ConvertFrom-Json).packages | Where-Object name -eq 'brightwheel'
        $Version = $package.version
    } finally { Pop-Location }
}
if ($Version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') {
    throw 'The release version must be X.Y.Z or X.Y.Z-prerelease.'
}
$stable = -not $Version.Contains('-')

$BinaryDirectory = (Resolve-Path -LiteralPath $BinaryDirectory).Path
foreach ($name in @('brightwheel.exe', 'bright.exe')) {
    if (-not (Test-Path -LiteralPath (Join-Path $BinaryDirectory $name) -PathType Leaf)) {
        throw "Missing production binary: $name"
    }
}
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$OutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$managedArtifacts = @()
if ($stable) {
    if (-not $CompilerPath) {
        $compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue
        if ($compiler) { $CompilerPath = $compiler.Source }
        else {
            $CompilerPath = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6/ISCC.exe'
        }
    }
    if (-not (Test-Path -LiteralPath $CompilerPath -PathType Leaf)) {
        throw 'Inno Setup 6 is required. Install it or supply -CompilerPath.'
    }
    if ([Diagnostics.FileVersionInfo]::GetVersionInfo($CompilerPath).FileMajorPart -ne 6) {
        throw 'The native lifecycle declarations require the 32-bit Inno Setup 6 compiler.'
    }

    & $CompilerPath /Qp "/DAppVersion=$Version" "/DBinaryDirectory=$BinaryDirectory" "/DOutputDirectory=$OutputDirectory" (Join-Path $root 'packaging/windows/brightwheel.iss')
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed with exit code $LASTEXITCODE." }
    $installer = Join-Path $OutputDirectory 'brightwheel-setup.exe'
    if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) { throw 'Inno Setup did not produce the installer.' }
    if (-not $InstallerUrl) {
        $InstallerUrl = "https://github.com/qqrm/brightwheel/releases/download/v$Version/brightwheel-setup.exe"
    }
    & (Join-Path $PSScriptRoot 'new-winget-manifest.ps1') -Version $Version -InstallerPath $installer -InstallerUrl $InstallerUrl -OutputDirectory (Join-Path $OutputDirectory 'winget')
    $manifestZip = Join-Path $OutputDirectory 'brightwheel-winget-manifests.zip'
    Compress-Archive -Path (Join-Path $OutputDirectory 'winget/*.yaml') -DestinationPath $manifestZip -Force
    $managedArtifacts = @($installer, $manifestZip)
} else {
    # Keep the existing portable prerelease channel. Stable setup identities and
    # WinGet manifests must not accidentally distribute a prerelease as stable.
    foreach ($name in @('brightwheel-setup.exe', 'brightwheel-winget-manifests.zip', 'winget')) {
        if (Test-Path -LiteralPath (Join-Path $OutputDirectory $name)) {
            throw 'Use a clean output directory when packaging a portable prerelease.'
        }
    }
    Write-Host 'Prerelease: producing portable assets only; no installer or WinGet publication.'
}

foreach ($name in @('brightwheel.exe', 'bright.exe')) {
    $source = Join-Path $BinaryDirectory $name
    $destination = Join-Path $OutputDirectory $name
    if ($source -ine $destination) { Copy-Item -LiteralPath $source -Destination $destination -Force }
}
$zip = Join-Path $OutputDirectory "brightwheel-$Version-x86_64-pc-windows-msvc.zip"
Compress-Archive -LiteralPath (Join-Path $OutputDirectory 'brightwheel.exe'), (Join-Path $OutputDirectory 'bright.exe') -DestinationPath $zip -Force
$files = @(
    (Join-Path $OutputDirectory 'brightwheel.exe'),
    (Join-Path $OutputDirectory 'bright.exe'),
    $zip
) + $managedArtifacts
$checksums = foreach ($file in $files) {
    $hash = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($file))"
}
$checksums | Set-Content (Join-Path $OutputDirectory 'SHA256SUMS.txt') -Encoding utf8NoBOM
Write-Host "Packaged BrightWheel $Version in $OutputDirectory"
