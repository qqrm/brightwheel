# Exercise the production startup path without changing the user's settings or
# install. A successful network request is not needed to verify a check attempt.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

if (Get-Process -Name brightwheel -ErrorAction SilentlyContinue) {
    throw 'Startup E2E requires no existing BrightWheel process.'
}

$install = Join-Path $env:RUNNER_TEMP "startup-updates-$([guid]::NewGuid().ToString('N'))"
$target = Join-Path $install 'brightwheel.exe'
$originalLocalAppData = $env:LOCALAPPDATA
$configPath = 'Software\BrightWheel'
$config = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($configPath, $true)
$configExisted = $null -ne $config
if (-not $configExisted) {
    $config = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($configPath)
}
$savedValues = @{}
foreach ($name in @('AutoUpdateEnabled', 'AutoStartConfigured')) {
    $savedValues[$name] = if ($config.GetValueNames() -contains $name) {
        @{
            Value = $config.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            Kind = $config.GetValueKind($name)
        }
    } else { $null }
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class StartupUpdateWindow {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string className, string title);
}
'@

function Get-UpdaterProcesses {
    Get-CimInstance Win32_Process -Filter "Name = 'powershell.exe'" | Where-Object {
        $_.CommandLine -and $_.CommandLine.IndexOf($target, [StringComparison]::OrdinalIgnoreCase) -ge 0
    }
}

function Read-UpdateLog([string]$path) {
    if (Test-Path -LiteralPath $path) { return [string](Get-Content -LiteralPath $path -Raw) }
    return ''
}

function Assert-Startup([string]$name, [bool]$expectedCheck, [switch]$skip, [switch]$secondInstance) {
    $env:LOCALAPPDATA = Join-Path $install $name
    $logPath = Join-Path $env:LOCALAPPDATA 'BrightWheel\updater.log'
    $before = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash
    $arguments = @{ FilePath = $target; WorkingDirectory = $install; PassThru = $true }
    if ($skip) { $arguments.ArgumentList = '--skip-startup-update' }
    $process = Start-Process @arguments
    try {
        # Wait for the real tray window before observing a disabled check, so
        # slow app initialization cannot produce a false pass.
        $deadline = [DateTime]::UtcNow.AddSeconds(20)
        while ([StartupUpdateWindow]::FindWindowEx([IntPtr](-3), [IntPtr]::Zero, 'BrightWheel.HiddenWindow', 'BrightWheel') -eq [IntPtr]::Zero) {
            if ($process.HasExited -or [DateTime]::UtcNow -gt $deadline) {
                throw "$name did not start a live BrightWheel window"
            }
            Start-Sleep -Milliseconds 200
        }

        if ($expectedCheck) {
            $deadline = [DateTime]::UtcNow.AddSeconds(60)
            while ($true) {
                $log = Read-UpdateLog $logPath
                $checks = [regex]::Matches($log, 'Checking for updates \(').Count
                if ($checks -gt 0 -and @(Get-UpdaterProcesses).Count -eq 0) { break }
                if ($process.HasExited -or [DateTime]::UtcNow -gt $deadline) {
                    throw "$name did not complete a startup check: $log"
                }
                Start-Sleep -Milliseconds 250
            }
        } else {
            Start-Sleep -Seconds 4
        }

        if ($secondInstance) {
            $duplicate = Start-Process -FilePath $target -WorkingDirectory $install -PassThru
            try {
                if (-not $duplicate.WaitForExit(5000) -or $duplicate.ExitCode -ne 0) {
                    throw 'A duplicate launch did not exit successfully'
                }
            } finally {
                if (-not $duplicate.HasExited) { $duplicate.Kill(); $null = $duplicate.WaitForExit(10000) }
            }
            Start-Sleep -Seconds 3
        }

        $checks = [regex]::Matches((Read-UpdateLog $logPath), 'Checking for updates \(').Count
        if ($checks -ne [int]$expectedCheck -or @(Get-UpdaterProcesses).Count -ne 0) {
            throw "$name launched $checks update checks (expected $([int]$expectedCheck)), or left an updater running"
        }
        if ($process.HasExited -or (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -cne $before) {
            throw "$name stopped or changed the running application"
        }
        Write-Host "PASS: $name; startup checks=$checks, process and executable unchanged"
    } finally {
        # A failed assertion must not leave a detached updater able to replace
        # or restart the temporary application after this test finishes.
        foreach ($helper in @(Get-UpdaterProcesses)) {
            $child = Get-Process -Id $helper.ProcessId -ErrorAction SilentlyContinue
            if ($null -ne $child -and -not $child.HasExited) {
                $child.Kill()
                if (-not $child.WaitForExit(10000)) { throw 'Could not terminate the test updater' }
            }
        }
        if (-not $process.HasExited) { $process.Kill(); $null = $process.WaitForExit(10000) }
        if (Test-Path -LiteralPath $logPath) { Get-Content -LiteralPath $logPath | ForEach-Object { Write-Host $_ } }
    }
}

try {
    New-Item -ItemType Directory -Path $install | Out-Null
    Copy-Item -LiteralPath 'target/release/brightwheel.exe' -Destination $target
    # Suppress first-run autostart registration; the Run key remains untouched.
    $config.SetValue('AutoStartConfigured', 1, [Microsoft.Win32.RegistryValueKind]::DWord)

    $config.DeleteValue('AutoUpdateEnabled', $false)
    Assert-Startup 'default-enabled-and-single-instance' $true -secondInstance

    $config.SetValue('AutoUpdateEnabled', 0, [Microsoft.Win32.RegistryValueKind]::DWord)
    Assert-Startup 'disabled-first-launch' $false
    Assert-Startup 'disabled-next-launch' $false
    if ($config.GetValue('AutoUpdateEnabled') -ne 0) { throw 'Disabled choice was not preserved' }

    $config.SetValue('AutoUpdateEnabled', 1, [Microsoft.Win32.RegistryValueKind]::DWord)
    Assert-Startup 'enabled-explicitly' $true
    Assert-Startup 'updater-restart-skips-once' $false -skip
    if ($config.GetValue('AutoUpdateEnabled') -ne 1) { throw 'Skip flag changed the saved preference' }
    Assert-Startup 'normal-launch-after-updater-restart' $true
} finally {
    $env:LOCALAPPDATA = $originalLocalAppData
    foreach ($name in $savedValues.Keys) {
        $saved = $savedValues[$name]
        if ($null -eq $saved) {
            $config.DeleteValue($name, $false)
        } else {
            $config.SetValue($name, $saved.Value, $saved.Kind)
        }
    }
    $removeConfig = -not $configExisted -and $config.ValueCount -eq 0 -and $config.SubKeyCount -eq 0
    $config.Dispose()
    if ($removeConfig) { [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKey($configPath, $false) }
    if (Test-Path -LiteralPath $install) { Remove-Item -LiteralPath $install -Recurse -Force }
}
