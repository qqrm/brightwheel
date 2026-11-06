#requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$TargetPath,
    [Parameter(Mandatory = $true)][string]$CurrentVersion,
    [Parameter(Mandatory = $true)][int]$ParentProcessId,
    [Parameter(Mandatory = $true)][string]$LogPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$workDir = Split-Path -Parent $PSCommandPath
$stagedEntries = @()
$rollbackEntries = New-Object 'System.Collections.Generic.List[object]'
$quitRequested = $false
$startedNew = $false

function Write-UpdateLog([string]$message) {
    try {
        New-Item -ItemType Directory -Path (Split-Path -Parent $LogPath) -Force | Out-Null
        Add-Content -LiteralPath $LogPath -Value ("{0:u} {1}" -f (Get-Date), $message) -Encoding UTF8
    } catch {
        # Update and rollback must not depend on a writable log.
    }
}

function Read-Version([string]$value) {
    if ($value -cnotmatch '^v?(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {
        throw "Invalid release version: $value"
    }
    return [version]($value.TrimStart('v'))
}

function Verified-Asset([object]$release, [string]$tag, [string]$name) {
    $assets = @($release.assets | Where-Object { $_.name -ceq $name })
    if ($assets.Count -ne 1) {
        throw "Expected exactly one release asset: $name"
    }
    $asset = $assets[0]
    $url = "https://github.com/qqrm/brightwheel/releases/download/$tag/$name"
    if ([string]$asset.browser_download_url -cne $url) {
        throw "Unexpected release asset URL: $name"
    }
    $digest = [string]$asset.digest
    if ($digest -cnotmatch '^sha256:[0-9a-f]{64}$') {
        throw "Release asset has no valid SHA-256 digest: $name"
    }
    $size = [long]$asset.size
    if ($size -lt 1024 -or $size -gt 104857600) {
        throw "Release asset size is invalid: $name"
    }
    return [pscustomobject]@{
        Name = $name
        Uri = $url
        Hash = $digest.Substring(7)
        Size = $size
    }
}

function Assert-Verified([string]$path, [object]$asset) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Missing staged executable: $($asset.Name)"
    }
    if ((Get-Item -LiteralPath $path).Length -ne $asset.Size) {
        throw "Executable size mismatch: $($asset.Name)"
    }
    $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -cne $asset.Hash) {
        throw "Executable SHA-256 mismatch: $($asset.Name)"
    }
}

try {
    if ([IO.Path]::GetFileName($TargetPath) -cne 'brightwheel.exe') {
        throw "Update target must be brightwheel.exe"
    }
    if (-not (Test-Path -LiteralPath $TargetPath -PathType Leaf) -or $ParentProcessId -le 0) {
        throw "Invalid running BrightWheel process or executable"
    }
    $directory = Split-Path -Parent $TargetPath
    $current = Read-Version $CurrentVersion
    Write-UpdateLog "Checking for updates ($CurrentVersion)"
    $headers = @{ 'User-Agent' = 'BrightWheel-Updater' }
    $release = Invoke-RestMethod -Uri 'https://api.github.com/repos/qqrm/brightwheel/releases/latest' -Headers $headers -TimeoutSec 45
    if ($release.draft -or $release.prerelease) {
        throw "Latest release is not a stable published release"
    }
    $tag = [string]$release.tag_name
    $latest = Read-Version $tag
    if ($latest -le $current) {
        Write-UpdateLog "Already up to date ($CurrentVersion)"
        exit 10
    }

    # A Cargo installation contains both binaries. Only replace the optional CLI
    # when it is present in the same directory as the running tray executable.
    $names = @('brightwheel.exe')
    $cliPath = Join-Path $directory 'bright.exe'
    if (Test-Path -LiteralPath $cliPath -PathType Leaf) {
        $names += 'bright.exe'
    }

    $suffix = [guid]::NewGuid().ToString('N')
    foreach ($name in $names) {
        $asset = Verified-Asset $release $tag $name
        $staged = Join-Path $workDir "$name.download"
        Invoke-WebRequest -Uri $asset.Uri -Headers $headers -OutFile $staged -UseBasicParsing -TimeoutSec 180 | Out-Null
        Assert-Verified $staged $asset

        $target = Join-Path $directory $name
        $candidate = Join-Path $directory ".$name.update-$suffix"
        $backup = Join-Path $directory ".$name.backup-$suffix"
        Copy-Item -LiteralPath $staged -Destination $candidate -ErrorAction Stop
        Assert-Verified $candidate $asset
        $stagedEntries += [pscustomobject]@{
            Target = $target
            Candidate = $candidate
            Backup = $backup
            Asset = $asset
        }
    }

    Write-UpdateLog "Verified release $tag; requesting graceful shutdown"
    $quitRequested = $true
    Start-Process -FilePath $TargetPath -ArgumentList '--quit-for-update' -Wait -WindowStyle Hidden -ErrorAction Stop
    $running = Get-Process -Id $ParentProcessId -ErrorAction SilentlyContinue
    if ($null -ne $running -and -not $running.WaitForExit(90000)) {
        throw "BrightWheel did not exit in 90 seconds; originals were not changed"
    }

    foreach ($entry in $stagedEntries) {
        [IO.File]::Move($entry.Target, $entry.Backup)
        $rollbackEntries.Add($entry)
        Assert-Verified $entry.Candidate $entry.Asset
        [IO.File]::Move($entry.Candidate, $entry.Target)
    }

    # Do not re-enter the automatic startup check while this helper is still
    # committing the update (or repeatedly retry a failed update after rollback).
    $newProcess = Start-Process -FilePath $TargetPath -ArgumentList '--skip-startup-update' -WorkingDirectory $directory -PassThru -ErrorAction Stop
    if ($newProcess.WaitForExit(1000)) {
        throw "Updated BrightWheel exited immediately"
    }
    $startedNew = $true
    Write-UpdateLog "Updated to $tag"

    foreach ($entry in $rollbackEntries) {
        Remove-Item -LiteralPath $entry.Backup -Force -ErrorAction SilentlyContinue
    }
} catch {
    Write-UpdateLog ("Update failed: " + $_.Exception.Message)
    $entries = @($rollbackEntries.ToArray())
    [array]::Reverse($entries)
    foreach ($entry in $entries) {
        try {
            if (Test-Path -LiteralPath $entry.Target -PathType Leaf) {
                Remove-Item -LiteralPath $entry.Target -Force -ErrorAction Stop
            }
            [IO.File]::Move($entry.Backup, $entry.Target)
        } catch {
            Write-UpdateLog ("Rollback failed for $($entry.Target): " + $_.Exception.Message)
        }
    }
    if ($quitRequested -and -not $startedNew -and (Test-Path -LiteralPath $TargetPath -PathType Leaf)) {
        $running = Get-Process -Id $ParentProcessId -ErrorAction SilentlyContinue
        if ($null -eq $running) {
            try {
                Start-Process -FilePath $TargetPath -ArgumentList '--skip-startup-update' -ErrorAction Stop
            } catch {
                Write-UpdateLog ("Restart failed: " + $_.Exception.Message)
            }
        }
    }
    exit 1
} finally {
    foreach ($entry in $stagedEntries) {
        Remove-Item -LiteralPath $entry.Candidate -Force -ErrorAction SilentlyContinue
    }
    Remove-Item -LiteralPath $workDir -Recurse -Force -ErrorAction SilentlyContinue
}
