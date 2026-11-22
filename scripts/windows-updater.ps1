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
$parentProcess = $null
$newProcess = $null
$updateGate = $null
$gateOwned = $false

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class BrightWheelUpdateReady {
    [StructLayout(LayoutKind.Sequential)]
    private struct Identifier { public uint Size; public IntPtr Window; public uint Id; public Guid Guid; }
    [StructLayout(LayoutKind.Sequential)]
    private struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string className, string title);
    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("shell32.dll")]
    private static extern int Shell_NotifyIconGetRect(ref Identifier identifier, out Rect rect);
    public static uint ReadyPid() {
        var window = FindWindowEx(IntPtr.Zero, IntPtr.Zero, "BrightWheel.HiddenWindow", "BrightWheel");
        if (window == IntPtr.Zero) return 0;
        uint pid;
        GetWindowThreadProcessId(window, out pid);
        var identifier = new Identifier { Size = (uint)Marshal.SizeOf(typeof(Identifier)), Window = window, Id = 1 };
        Rect rect;
        return Shell_NotifyIconGetRect(ref identifier, out rect) == 0 && rect.Right > rect.Left && rect.Bottom > rect.Top ? pid : 0;
    }
}
'@

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

function Release-UpdateGate {
    if ($script:gateOwned) {
        $script:updateGate.ReleaseMutex()
        $script:gateOwned = $false
    }
}

function Enter-UpdateGate {
    if ($script:gateOwned) { return }
    if ($null -eq $script:updateGate) {
        $script:updateGate = [System.Threading.Mutex]::new($false, 'Local\BrightWheel.Startup')
    }
    try { $script:gateOwned = $script:updateGate.WaitOne(10000) }
    catch [System.Threading.AbandonedMutexException] { $script:gateOwned = $true }
    if (-not $script:gateOwned) { throw 'Another BrightWheel launch is still starting' }
}

function Wait-ReadyTray {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        $readyPid = [BrightWheelUpdateReady]::ReadyPid()
        if ($readyPid -gt 0 -and $readyPid -ne $ParentProcessId) {
            $ready = Get-Process -Id $readyPid -ErrorAction SilentlyContinue
            if ($null -ne $ready) {
                try {
                    if ([string]::Equals($ready.Path, $TargetPath, [StringComparison]::OrdinalIgnoreCase)) {
                        return
                    }
                } finally { $ready.Dispose() }
            }
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Updated BrightWheel did not register a usable tray icon'
}

try {
    if ([IO.Path]::GetFileName($TargetPath) -cne 'brightwheel.exe') {
        throw "Update target must be brightwheel.exe"
    }
    if (-not (Test-Path -LiteralPath $TargetPath -PathType Leaf) -or $ParentProcessId -le 0) {
        throw "Invalid running BrightWheel process or executable"
    }
    $directory = Split-Path -Parent $TargetPath
    # Hold the original process handle across downloads. A user relaunch can
    # replace it; PID-only lookups afterwards must not target the replacement.
    $parentProcess = Get-Process -Id $ParentProcessId -ErrorAction Stop
    $null = $parentProcess.Handle
    if (-not [string]::Equals($parentProcess.Path, $TargetPath, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Updater parent executable does not match its target'
    }
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

    # Serialize the short destructive phase with user relaunches. Downloads do
    # not hold the gate; restart readiness must not hold it either.
    Enter-UpdateGate
    if ($parentProcess.HasExited) {
        Write-UpdateLog 'Update cancelled: the original BrightWheel instance was replaced'
        exit 0
    }
    Write-UpdateLog "Verified release $tag; requesting graceful shutdown"
    $quitRequested = $true
    Start-Process -FilePath $TargetPath -ArgumentList "--quit-for-update=$ParentProcessId" -Wait -WindowStyle Hidden -ErrorAction Stop
    if (-not $parentProcess.WaitForExit(10000)) {
        throw "BrightWheel did not exit in 10 seconds; originals were not changed"
    }

    foreach ($entry in $stagedEntries) {
        [IO.File]::Move($entry.Target, $entry.Backup)
        $rollbackEntries.Add($entry)
        Assert-Verified $entry.Candidate $entry.Asset
        [IO.File]::Move($entry.Candidate, $entry.Target)
    }

    # Do not re-enter the automatic startup check while this helper is still
    # committing the update (or repeatedly retry a failed update after rollback).
    Release-UpdateGate
    $newProcess = Start-Process -FilePath $TargetPath -ArgumentList '--skip-startup-update', '--skip-if-running' -WorkingDirectory $directory -PassThru -ErrorAction Stop
    # A user relaunch may win after the gate is released. The internal launch
    # then exits quietly; either winner must own a real registered tray icon.
    Wait-ReadyTray
    $startedNew = $true
    Write-UpdateLog "Updated to $tag"

    foreach ($entry in $rollbackEntries) {
        Remove-Item -LiteralPath $entry.Backup -Force -ErrorAction SilentlyContinue
    }
} catch {
    Write-UpdateLog ("Update failed: " + $_.Exception.Message)
    if ($rollbackEntries.Count -gt 0 -or $quitRequested) {
        try { Enter-UpdateGate }
        catch {
            Write-UpdateLog 'Rollback deferred: relaunch gate unavailable; backups preserved'
            exit 1
        }
    }
    if ($null -ne $newProcess -and -not $newProcess.HasExited) {
        # Stop only our own failed startup before restoring its executable.
        $newProcess.Kill()
        $null = $newProcess.WaitForExit(5000)
    }
    $userReplacement = @(Get-Process -Name brightwheel -ErrorAction SilentlyContinue | Where-Object {
        try { $_.Id -ne $ParentProcessId -and [string]::Equals($_.Path, $TargetPath, [StringComparison]::OrdinalIgnoreCase) }
        catch { $false }
    })
    if ($userReplacement.Count -gt 0) {
        Write-UpdateLog 'Rollback deferred: a user-started replacement is running; backups preserved'
        exit 1
    }
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
        if ($null -ne $parentProcess -and $parentProcess.HasExited) {
            try {
                Release-UpdateGate
                Start-Process -FilePath $TargetPath -ArgumentList '--skip-startup-update', '--skip-if-running' -ErrorAction Stop
            } catch {
                Write-UpdateLog ("Restart failed: " + $_.Exception.Message)
            }
        }
    }
    exit 1
} finally {
    Release-UpdateGate
    if ($null -ne $updateGate) { $updateGate.Dispose() }
    if ($null -ne $newProcess) { $newProcess.Dispose() }
    if ($null -ne $parentProcess) { $parentProcess.Dispose() }
    foreach ($entry in $stagedEntries) {
        Remove-Item -LiteralPath $entry.Candidate -Force -ErrorAction SilentlyContinue
    }
    Remove-Item -LiteralPath $workDir -Recurse -Force -ErrorAction SilentlyContinue
}
