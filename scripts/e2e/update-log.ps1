# Production-process callback/menu E2E with a real associated reader EXE. Restore
# the disposable .log association in finally; never modify Windows UserChoice.
# Posted native shell callbacks are tested, not physical clicks on toast banners.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_OS -ne 'Windows' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'Update-log E2E changes a file association and requires a disposable GitHub-hosted Windows runner.'
}
if (-not [Environment]::UserInteractive) {
    throw 'Update-log E2E requires an interactive Windows desktop for real menu selection.'
}
if (Get-Process -Name brightwheel -ErrorAction SilentlyContinue) {
    throw 'Update-log E2E requires no existing BrightWheel process.'
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class UpdateLogNative {
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    public struct Identifier { public uint Size; public IntPtr Window; public uint Id; public Guid Guid; }
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindow(string className, string title);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessage(IntPtr window, uint message, UIntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")]
    public static extern IntPtr SendMessage(IntPtr window, uint message, UIntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, UIntPtr wparam, IntPtr lparam,
        uint flags, uint timeout, out UIntPtr result);
    [DllImport("user32.dll")]
    public static extern int GetMenuItemCount(IntPtr menu);
    [DllImport("user32.dll")]
    public static extern uint GetMenuItemID(IntPtr menu, int index);
    [DllImport("user32.dll")]
    public static extern uint GetMenuState(IntPtr menu, uint item, uint flags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetMenuString(IntPtr menu, uint item, StringBuilder text, int capacity, uint flags);
    [DllImport("user32.dll")]
    public static extern IntPtr GetWindow(IntPtr window, uint command);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetWindowText(IntPtr window, StringBuilder text, int capacity);
    [DllImport("shell32.dll")]
    private static extern int Shell_NotifyIconGetRect(ref Identifier identifier, out Rect rect);
    [DllImport("shell32.dll")]
    public static extern void SHChangeNotify(uint eventId, uint flags, IntPtr item1, IntPtr item2);
    [DllImport("shlwapi.dll", CharSet = CharSet.Unicode)]
    private static extern int AssocQueryString(uint flags, uint kind, string association, string extra,
        StringBuilder output, ref uint length);
    public static bool HasIcon(IntPtr window) {
        var id = new Identifier { Size = (uint)Marshal.SizeOf(typeof(Identifier)), Window = window, Id = 1 };
        Rect rect;
        return Shell_NotifyIconGetRect(ref id, out rect) == 0;
    }
    public static string Association(uint kind) {
        uint size = 32768;
        var output = new StringBuilder((int)size);
        int result = AssocQueryString(0, kind, ".log", null, output, ref size);
        return result == 0 ? output.ToString() : "<HRESULT 0x" + result.ToString("X8") + ">";
    }
    public static string AssociatedExecutable() { return Association(2); }
    public static string AssociatedProgId() { return Association(20); }
    public static string DialogText(IntPtr dialog) {
        var output = new StringBuilder();
        for (IntPtr child = GetWindow(dialog, 5); child != IntPtr.Zero; child = GetWindow(child, 2)) {
            var text = new StringBuilder(4096);
            GetWindowText(child, text, text.Capacity);
            output.AppendLine(text.ToString());
        }
        return output.ToString();
    }
}
'@

$temporaryRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$unique = [guid]::NewGuid().ToString('N')
# Spaces, Unicode and shell metacharacters must remain one literal document arg.
$install = Join-Path $temporaryRoot "update log & [Unicode-$([char]0x03bb)]-$unique"
$target = Join-Path $install 'brightwheel.exe'
$reader = Join-Path $install 'log-reader.exe'
$capture = Join-Path $install 'opened-paths.txt'
$fakeSystemRoot = Join-Path $install 'fixture-windows'
$helperDirectory = Join-Path $fakeSystemRoot 'System32\WindowsPowerShell\v1.0'
$helper = Join-Path $helperDirectory 'powershell.exe'
$helperResult = Join-Path $helperDirectory 'result-code.txt'
$data = Join-Path $install 'local data'
$logPath = Join-Path $data 'BrightWheel\updater.log'
$runtimeLog = Join-Path $data 'BrightWheel\runtime.log'
$originalLocalAppData = $env:LOCALAPPDATA
$originalSystemRoot = $env:SystemRoot
$process = $null
$config = $null
$extension = $null
$extensionExisted = $false
$associationChanged = $false
$progId = "BrightWheel.UpdateLog.E2E.$unique"
$progIdPath = "Software\Classes\$progId"
$extensionPath = 'Software\Classes\.log'
$configPath = 'Software\BrightWheel'
$savedConfig = @{}

function Wait-For([scriptblock]$condition, [string]$description) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        if (& $condition) { return }
        if ($null -ne $process -and $process.HasExited) { throw "BrightWheel exited while waiting for $description" }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for $description"
}

function Read-Text([string]$path) {
    if (Test-Path -LiteralPath $path) { return [IO.File]::ReadAllText($path) }
    return ''
}

function Send-TrayEvent([uint32]$event, [uint32]$iconId = 1) {
    $packed = [IntPtr](($iconId -shl 16) -bor $event)
    if (-not [UpdateLogNative]::PostMessage($script:window, 0x8001, [UIntPtr]::Zero, $packed)) {
        throw 'Could not post the native tray callback'
    }
}

function Invoke-TrayMenu([uint32]$commandId, [string]$label) {
    Send-TrayEvent 0x007b # WM_CONTEXTMENU under NOTIFYICON_VERSION_4
    Wait-For {
        $script:popup = [UpdateLogNative]::FindWindow('#32768', $null)
        [uint32]$ownerPid = 0
        $null = [UpdateLogNative]::GetWindowThreadProcessId($script:popup, [ref]$ownerPid)
        $script:popup -ne [IntPtr]::Zero -and $ownerPid -eq $process.Id
    } 'the production tray popup menu'
    # MN_GETHMENU reads the real TrackPopupMenu menu. Use native keyboard input
    # only after checking the command ID and visible label of the enabled item.
    $menu = [UpdateLogNative]::SendMessage($script:popup, 0x01e1, [UIntPtr]::Zero, [IntPtr]::Zero)
    $selectableBefore = 0
    $found = $false
    for ($index = 0; $index -lt [UpdateLogNative]::GetMenuItemCount($menu); $index++) {
        $state = [UpdateLogNative]::GetMenuState($menu, [uint32]$index, 0x0400)
        if (($state -band 0x0803) -ne 0) { continue } # separator/disabled
        if ([UpdateLogNative]::GetMenuItemID($menu, $index) -eq $commandId) {
            $text = [Text.StringBuilder]::new(256)
            $null = [UpdateLogNative]::GetMenuString($menu, [uint32]$index, $text, $text.Capacity, 0x0400)
            if ($text.ToString() -cne $label) { throw "Unexpected menu label '$text'" }
            $found = $true
            break
        }
        $selectableBefore++
    }
    if (-not $found) { throw "Tray menu did not expose enabled '$label'" }
    [Windows.Forms.SendKeys]::SendWait('{HOME}' + ('{DOWN}' * $selectableBefore) + '{ENTER}')
    Wait-For { [UpdateLogNative]::FindWindow('#32768', $null) -eq [IntPtr]::Zero } 'menu selection'
}

function Run-ManualCheck([int]$expectedCode) {
    $pattern = "manual update result: $expectedCode(?:\r?\n|$)"
    $before = [regex]::Matches((Read-Text $runtimeLog), $pattern).Count
    $action = if ($expectedCode -eq 10) { 'None' } else { 'OpenUpdateLog' }
    $noticePattern = "update notification submitted: $action(?:\r?\n|$)"
    $noticesBefore = [regex]::Matches((Read-Text $runtimeLog), $noticePattern).Count
    Invoke-TrayMenu 1003 'Check for updates'
    Wait-For {
        $log = Read-Text $runtimeLog
        [regex]::Matches($log, $pattern).Count -gt $before -and
            [regex]::Matches($log, $noticePattern).Count -gt $noticesBefore
    } "manual result $expectedCode and its successfully submitted notification"
}

function Assert-Opened([int]$expectedCount, [string]$contentsMarker) {
    Wait-For { @(Get-Content -LiteralPath $capture -ErrorAction SilentlyContinue).Count -ge $expectedCount } 'the associated log reader'
    $records = @(Get-Content -LiteralPath $capture)
    if ($records.Count -ne $expectedCount) { throw "Expected $expectedCount reader invocations, got $($records.Count)" }
    $fields = $records[-1] -split "`t"
    if ($fields.Count -ne 2) { throw "Reader failed: $($records[-1])" }
    $actualPath = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($fields[0]))
    $contents = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($fields[1]))
    if ($actualPath -cne $logPath -or -not $contents.Contains($contentsMarker)) {
        throw "Associated reader did not receive the exact readable updater log: '$actualPath'"
    }
}

function Assert-NoExtraOpen([int]$expectedCount) {
    Start-Sleep -Milliseconds 800
    if (@(Get-Content -LiteralPath $capture -ErrorAction SilentlyContinue).Count -ne $expectedCount) {
        throw 'An unrelated, invalid, or already consumed notification opened the log'
    }
}

try {
    New-Item -ItemType Directory -Path $helperDirectory -Force | Out-Null
    Copy-Item -LiteralPath 'target/release/brightwheel.exe' -Destination $target
    $compiler = Join-Path $originalSystemRoot 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
    $fixtureSource = Join-Path $PSScriptRoot 'update-log-fixture.cs'
    & $compiler /nologo /target:exe "/out:$helper" $fixtureSource
    if ($LASTEXITCODE -ne 0) { throw 'Could not compile the updater fixture' }
    & $compiler /nologo /target:winexe "/out:$reader" $fixtureSource
    if ($LASTEXITCODE -ne 0) { throw 'Could not compile the associated log reader' }
    [IO.File]::WriteAllText($helperResult, '23')

    $extension = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($extensionPath, $true)
    $extensionExisted = $null -ne $extension
    if (-not $extensionExisted) { $extension = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($extensionPath) }
    $hadDefault = $extension.GetValueNames() -contains ''
    if ($hadDefault) {
        $oldDefault = $extension.GetValue('', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $oldKind = $extension.GetValueKind('')
    }
    $originalExtension = if ($hadDefault) { "$oldDefault ($oldKind)" } else { '<absent>' }
    # Observe only the ProgId, not the protected hash; UserChoice is never changed.
    $userChoice = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.log\UserChoice')
    try { $userChoiceProgId = if ($null -ne $userChoice) { $userChoice.GetValue('ProgId', '<absent>') } else { '<absent>' } }
    finally { if ($null -ne $userChoice) { $userChoice.Dispose() } }
    Write-Host "Fixture association before setup: reader='$([UpdateLogNative]::AssociatedExecutable())'; resolved ProgId='$([UpdateLogNative]::AssociatedProgId())'; HKCU default='$originalExtension'; UserChoice ProgId='$userChoiceProgId'"
    $command = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey("$progIdPath\shell\open\command")
    try { $command.SetValue('', ('"{0}" "%1"' -f $reader), [Microsoft.Win32.RegistryValueKind]::String) }
    finally { $command.Dispose() }
    $associationChanged = $true
    $extension.SetValue('', $progId, [Microsoft.Win32.RegistryValueKind]::String)
    # SHCNF_IDLIST | SHCNF_FLUSH waits for change delivery; do not race shell
    # cache invalidation immediately after registering the disposable reader.
    [UpdateLogNative]::SHChangeNotify(0x08000000, 0x1000, [IntPtr]::Zero, [IntPtr]::Zero)
    try {
        Wait-For { [UpdateLogNative]::AssociatedExecutable() -ieq $reader } 'the exact effective .log reader'
    } catch {
        throw "The test .log association did not become effective: expected='$reader'; actual='$([UpdateLogNative]::AssociatedExecutable())'; resolved ProgId='$([UpdateLogNative]::AssociatedProgId())'; saved HKCU default='$originalExtension'; UserChoice ProgId='$userChoiceProgId'. $($_.Exception.Message)"
    }
    Write-Host "Fixture association ready: reader='$([UpdateLogNative]::AssociatedExecutable())'; resolved ProgId='$([UpdateLogNative]::AssociatedProgId())'"

    $config = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($configPath, $true)
    $configExisted = $null -ne $config
    if (-not $configExisted) { $config = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($configPath) }
    foreach ($name in @('AutoStartConfigured', 'AutoUpdateEnabled')) {
        $savedConfig[$name] = if ($config.GetValueNames() -contains $name) {
            @{ Value = $config.GetValue($name); Kind = $config.GetValueKind($name) }
        } else { $null }
    }
    $config.SetValue('AutoStartConfigured', 1, [Microsoft.Win32.RegistryValueKind]::DWord)
    $config.SetValue('AutoUpdateEnabled', 0, [Microsoft.Win32.RegistryValueKind]::DWord)
    $env:LOCALAPPDATA = $data
    $env:SystemRoot = $fakeSystemRoot
    try { $process = Start-Process -FilePath $target -ArgumentList '--skip-startup-update' -PassThru }
    finally { $env:LOCALAPPDATA = $originalLocalAppData; $env:SystemRoot = $originalSystemRoot }
    Wait-For {
        $script:window = [UpdateLogNative]::FindWindow('BrightWheel.HiddenWindow', 'BrightWheel')
        [uint32]$ownerPid = 0
        $null = [UpdateLogNative]::GetWindowThreadProcessId($script:window, [ref]$ownerPid)
        $ownerPid -eq $process.Id -and [UpdateLogNative]::HasIcon($script:window)
    } 'the production tray icon'

    Send-TrayEvent 0x0405 # NIN_BALLOONUSERCLICK, no action yet
    Assert-NoExtraOpen 0
    Run-ManualCheck 23
    $logged = Read-Text $logPath
    if (-not $logged.Contains('E2E updater stderr sentinel') -or $logged.Contains('END-SHOULD-NOT-BE-CAPTURED') -or $logged.Length -gt 17000) {
        throw 'Early helper stderr was missing, unbounded, or not captured'
    }
    Send-TrayEvent 0x0405 2 # wrong icon ID must not consume the action
    Assert-NoExtraOpen 0
    Send-TrayEvent 0x0404 # timeout/hide can leave a clickable notification in history
    Send-TrayEvent 0x0403
    Send-TrayEvent 0x0405
    Assert-Opened 1 'E2E updater stderr sentinel'
    Send-TrayEvent 0x0405
    Assert-NoExtraOpen 1
    Write-Host 'PASS: early updater failure is logged; v4 failure click opens the exact log in the associated reader, even after banner timeout'

    Run-ManualCheck 23 # arm a fresh failure, then replace it with information
    [IO.File]::WriteAllText($helperResult, '10')
    Run-ManualCheck 10
    Send-TrayEvent 0x0405
    Assert-NoExtraOpen 1
    Write-Host 'PASS: informational notification clears the failure action; invalid/repeated clicks do not open an old log'

    Invoke-TrayMenu 1005 'Open update log'
    Assert-Opened 2 'E2E updater stderr sentinel'
    Write-Host 'PASS: the real Open update log tray menu opens the configured reader with one literal Unicode path argument'

    Remove-Item -LiteralPath $logPath
    Invoke-TrayMenu 1005 'Open update log'
    Wait-For {
        $script:dialog = [UpdateLogNative]::FindWindow('#32770', 'BrightWheel error')
        [uint32]$ownerPid = 0
        $null = [UpdateLogNative]::GetWindowThreadProcessId($script:dialog, [ref]$ownerPid)
        $script:dialog -ne [IntPtr]::Zero -and $ownerPid -eq $process.Id
    } 'a visible missing-log error'
    $dialogText = [UpdateLogNative]::DialogText($script:dialog)
    if (-not $dialogText.Contains('Could not open the update log') -or -not $dialogText.Contains($logPath)) {
        throw "Missing-log error did not identify the failed path: $dialogText"
    }
    [UIntPtr]$response = [UIntPtr]::Zero
    if ([UpdateLogNative]::SendMessageTimeout($script:window, 0, [UIntPtr]::Zero, [IntPtr]::Zero, 2, 2000, [ref]$response) -eq [IntPtr]::Zero) {
        throw 'Opening the log blocked the tray UI'
    }
    $null = [UpdateLogNative]::PostMessage($script:dialog, 0x0010, [UIntPtr]::Zero, [IntPtr]::Zero)
    Wait-For { [UpdateLogNative]::FindWindow('#32770', 'BrightWheel error') -eq [IntPtr]::Zero } 'error dialog dismissal'
    Assert-NoExtraOpen 2
    Write-Host 'PASS: a missing log produces a visible path-specific error while the tray remains responsive'

    Remove-Item -LiteralPath $helper
    Run-ManualCheck 1
    Send-TrayEvent 0x0405
    Assert-Opened 3 'Update check failed:'
    Write-Host 'PASS: helper process creation failure uses the same actionable notification and diagnostic log'
} finally {
    $env:LOCALAPPDATA = $originalLocalAppData
    $env:SystemRoot = $originalSystemRoot
    if ($null -ne $process) {
        if (-not $process.HasExited) { $process.Kill(); $null = $process.WaitForExit(10000) }
        $process.Dispose()
    }
    if ($null -ne $config) {
        foreach ($name in $savedConfig.Keys) {
            $saved = $savedConfig[$name]
            if ($null -eq $saved) { $config.DeleteValue($name, $false) }
            else { $config.SetValue($name, $saved.Value, $saved.Kind) }
        }
        $removeConfig = -not $configExisted -and $config.ValueCount -eq 0 -and $config.SubKeyCount -eq 0
        $config.Dispose()
        if ($removeConfig) { [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKey($configPath, $false) }
    }
    if ($associationChanged) {
        if ($hadDefault) { $extension.SetValue('', $oldDefault, $oldKind) }
        else { $extension.DeleteValue('', $false) }
    }
    if ($null -ne $extension) {
        $removeExtension = -not $extensionExisted -and $extension.ValueCount -eq 0 -and $extension.SubKeyCount -eq 0
        $extension.Dispose()
        if ($removeExtension) { [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKey($extensionPath, $false) }
    }
    [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($progIdPath, $false)
    [UpdateLogNative]::SHChangeNotify(0x08000000, 0x1000, [IntPtr]::Zero, [IntPtr]::Zero)
    if (Test-Path -LiteralPath $runtimeLog) { Get-Content -LiteralPath $runtimeLog -Tail 12 | ForEach-Object { Write-Host $_.Substring(0, [Math]::Min($_.Length, 250)) } }
    if (Test-Path -LiteralPath $install) { Remove-Item -LiteralPath $install -Recurse -Force }
}
