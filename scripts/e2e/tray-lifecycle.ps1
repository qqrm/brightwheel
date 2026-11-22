# Production-process regression checks. The script uses a disposable install,
# preserves settings, and requires an interactive Windows shell (no silent skip).
[CmdletBinding()]
param([switch]$RestartExplorer)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (Get-Process -Name brightwheel -ErrorAction SilentlyContinue) {
    throw 'Tray lifecycle E2E requires no existing BrightWheel process.'
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class TrayLifecycle {
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    public struct Identifier { public uint Size; public IntPtr Window; public uint Id; public Guid Guid; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct IconData {
        public uint Size; public IntPtr Window; public uint Id, Flags, Callback; public IntPtr Icon;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string Tip;
        public uint State, StateMask;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string Info;
        public uint Version;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 64)] public string Title;
        public uint InfoFlags; public Guid Guid; public IntPtr BalloonIcon;
    }
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindow(string className, string title);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessage(IntPtr window, uint message, UIntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern uint RegisterWindowMessage(string name);
    [DllImport("shell32.dll")]
    private static extern int Shell_NotifyIconGetRect(ref Identifier identifier, out Rect rect);
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    private static extern bool Shell_NotifyIcon(uint operation, ref IconData data);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern IntPtr OpenThread(uint access, bool inherit, uint threadId);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern uint SuspendThread(IntPtr thread);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll")]
    public static extern bool CloseHandle(IntPtr handle);
    public static bool HasIcon(IntPtr window) {
        var id = new Identifier { Size = (uint)Marshal.SizeOf(typeof(Identifier)), Window = window, Id = 1 };
        Rect rect;
        return Shell_NotifyIconGetRect(ref id, out rect) == 0 && rect.Right > rect.Left && rect.Bottom > rect.Top;
    }
    public static bool DeleteIcon(IntPtr window) {
        var data = new IconData { Size = (uint)Marshal.SizeOf(typeof(IconData)), Window = window, Id = 1 };
        return Shell_NotifyIcon(2, ref data);
    }
}
'@

$temporaryRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$install = Join-Path $temporaryRoot "tray-lifecycle-$([guid]::NewGuid().ToString('N'))"
$target = Join-Path $install 'brightwheel.exe'
$originalLocalAppData = $env:LOCALAPPDATA
$processes = [Collections.Generic.List[Diagnostics.Process]]::new()
$configPath = 'Software\BrightWheel'
$config = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($configPath, $true)
$configExisted = $null -ne $config
if (-not $configExisted) { $config = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($configPath) }
$savedValues = @{}
foreach ($name in @('AutoStartConfigured', 'AutoUpdateEnabled')) {
    $savedValues[$name] = if ($config.GetValueNames() -contains $name) {
        @{ Value = $config.GetValue($name); Kind = $config.GetValueKind($name) }
    } else { $null }
}

function Start-Tray {
    $started = Start-Process -FilePath $target -ArgumentList '--skip-startup-update' -PassThru
    $processes.Add($started)
    return $started
}

function Wait-Tray([Diagnostics.Process]$process) {
    $deadline = [DateTime]::UtcNow.AddSeconds(25)
    do {
        if ($process.HasExited) { throw "BrightWheel pid=$($process.Id) exited before its tray was ready" }
        # FindWindow only sees top-level windows: HWND_MESSAGE is a regression.
        $window = [TrayLifecycle]::FindWindow('BrightWheel.HiddenWindow', 'BrightWheel')
        [uint32]$ownerPid = 0
        if ($window -ne [IntPtr]::Zero) {
            $null = [TrayLifecycle]::GetWindowThreadProcessId($window, [ref]$ownerPid)
            if ($ownerPid -eq $process.Id -and [TrayLifecycle]::HasIcon($window)) { return $window }
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "No registered tray icon for pid=$($process.Id); an interactive shell is required"
}

function Assert-Recovery([Diagnostics.Process]$process, [uint32]$message, [uint32]$parameter, [string]$description) {
    $window = Wait-Tray $process
    if (-not [TrayLifecycle]::DeleteIcon($window) -or [TrayLifecycle]::HasIcon($window)) {
        throw "$description could not remove the existing tray icon"
    }
    # Real system broadcast delivery is essential: sending directly to the app
    # HWND would allow the old message-only-window defect to pass this check.
    if (-not [TrayLifecycle]::PostMessage([IntPtr]0xffff, $message, [UIntPtr]$parameter, [IntPtr]::Zero)) {
        throw "$description broadcast failed"
    }
    $restored = Wait-Tray $process
    if ($restored -ne $window) { throw "$description recreated the application instead of its icon" }
    Write-Host "PASS: $description restored the registered tray icon with the same pid=$($process.Id) and HWND"
}

try {
    New-Item -ItemType Directory -Path $install | Out-Null
    Copy-Item -LiteralPath 'target/release/brightwheel.exe' -Destination $target
    $env:LOCALAPPDATA = Join-Path $install 'data'
    $config.SetValue('AutoStartConfigured', 1, [Microsoft.Win32.RegistryValueKind]::DWord)
    $config.SetValue('AutoUpdateEnabled', 0, [Microsoft.Win32.RegistryValueKind]::DWord)

    $first = Start-Tray
    $null = Wait-Tray $first
    $taskbarCreated = [TrayLifecycle]::RegisterWindowMessage('TaskbarCreated')
    if ($taskbarCreated -eq 0) { throw 'RegisterWindowMessage failed' }
    Assert-Recovery $first $taskbarCreated 0 'TaskbarCreated'
    Assert-Recovery $first $taskbarCreated 0 'repeated TaskbarCreated'
    Assert-Recovery $first 0x007e 32 'display change'
    Assert-Recovery $first 0x0218 18 'automatic resume notification'

    $replacement = Start-Tray
    if (-not $first.WaitForExit(10000)) { throw 'Normal relaunch did not terminate the original process' }
    $window = Wait-Tray $replacement
    if ($replacement.Id -eq $first.Id) { throw 'Relaunch kept the same process' }
    Write-Host 'PASS: user relaunch replaces the running process and creates a usable tray'

    $staleQuit = Start-Process -FilePath $target -ArgumentList "--quit-for-update=$($first.Id)" -PassThru
    $processes.Add($staleQuit)
    if (-not $staleQuit.WaitForExit(5000)) { throw 'Targeted stale updater command did not exit' }
    $null = Wait-Tray $replacement
    $internalRestart = Start-Process -FilePath $target -ArgumentList '--skip-startup-update', '--skip-if-running' -PassThru
    $processes.Add($internalRestart)
    if (-not $internalRestart.WaitForExit(5000)) { throw 'Internal updater restart replaced the user instance' }
    $null = Wait-Tray $replacement
    Write-Host 'PASS: stale updater shutdown/restart cannot replace the user instance'

    [uint32]$ownerPid = 0
    $threadId = [TrayLifecycle]::GetWindowThreadProcessId($window, [ref]$ownerPid)
    $threadHandle = [TrayLifecycle]::OpenThread(2, $false, $threadId)
    if ($threadHandle -eq [IntPtr]::Zero) { throw 'Could not open the application UI thread' }
    try {
        if ([TrayLifecycle]::SuspendThread($threadHandle) -eq [uint32]::MaxValue) { throw 'Could not suspend the application UI thread' }
        $current = Start-Tray
        if (-not $replacement.WaitForExit(12000)) { throw 'Relaunch did not terminate the unresponsive old instance' }
        $null = Wait-Tray $current
        Write-Host 'PASS: relaunch replaces a genuinely suspended/unresponsive UI thread'
    } finally {
        if (-not $replacement.HasExited) { $null = [TrayLifecycle]::ResumeThread($threadHandle) }
        $null = [TrayLifecycle]::CloseHandle($threadHandle)
    }

    $null = Start-Tray
    $null = Start-Tray
    $null = Start-Tray
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        $live = @($processes | Where-Object { -not $_.HasExited })
        if ($live.Count -eq 1) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($live.Count -ne 1) { throw "Concurrent relaunches left $($live.Count) instances" }
    $current = $live[0]
    $null = Wait-Tray $current
    Write-Host 'PASS: overlapping relaunches converge on exactly one usable tray instance'

    if ($RestartExplorer) {
        $session = (Get-Process -Id $PID).SessionId
        $explorers = @(Get-Process -Name explorer -ErrorAction SilentlyContinue | Where-Object SessionId -eq $session)
        if ($explorers.Count -eq 0) { throw 'Explorer restart check requires an existing shell in this session' }
        foreach ($explorer in $explorers) {
            Stop-Process -Id $explorer.Id -Force
            if (-not $explorer.WaitForExit(10000)) { throw 'Explorer did not stop' }
        }
        Start-Process -FilePath (Join-Path $env:SystemRoot 'explorer.exe')
        $null = Wait-Tray $current
        Write-Host 'PASS: actual Explorer restart restored the icon without replacing BrightWheel'
    }
} finally {
    foreach ($process in $processes) {
        if (-not $process.HasExited) { $process.Kill(); $null = $process.WaitForExit(10000) }
        $process.Dispose()
    }
    $log = Join-Path $env:LOCALAPPDATA 'BrightWheel\runtime.log'
    if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log -Tail 80 | ForEach-Object { Write-Host $_ } }
    $env:LOCALAPPDATA = $originalLocalAppData
    foreach ($name in $savedValues.Keys) {
        $saved = $savedValues[$name]
        if ($null -eq $saved) { $config.DeleteValue($name, $false) }
        else { $config.SetValue($name, $saved.Value, $saved.Kind) }
    }
    $removeConfig = -not $configExisted -and $config.ValueCount -eq 0 -and $config.SubKeyCount -eq 0
    $config.Dispose()
    if ($removeConfig) { [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKey($configPath, $false) }
    if (Test-Path -LiteralPath $install) { Remove-Item -LiteralPath $install -Recurse -Force }
}
