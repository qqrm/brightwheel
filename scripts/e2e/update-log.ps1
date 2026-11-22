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

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
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
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr window, StringBuilder text, int capacity);
    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr window);
    private delegate bool EnumWindowsCallback(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsCallback callback, IntPtr parameter);
    [DllImport("shell32.dll")]
    private static extern int Shell_NotifyIconGetRect(ref Identifier identifier, out Rect rect);
    [DllImport("shell32.dll")]
    public static extern void SHChangeNotify(uint eventId, uint flags, IntPtr item1, IntPtr item2);
    [DllImport("shell32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CommandLineToArgvW(string commandLine, out int count);
    [DllImport("kernel32.dll")]
    private static extern IntPtr LocalFree(IntPtr memory);
    [DllImport("shlwapi.dll", CharSet = CharSet.Unicode)]
    private static extern int AssocQueryString(uint flags, uint kind, string association, string extra,
        StringBuilder output, ref uint length);
    public static bool HasIcon(IntPtr window) {
        var id = new Identifier { Size = (uint)Marshal.SizeOf(typeof(Identifier)), Window = window, Id = 1 };
        Rect rect;
        return Shell_NotifyIconGetRect(ref id, out rect) == 0;
    }
    public static string Association(uint kind, string association, uint flags) {
        uint size = 32768;
        var output = new StringBuilder((int)size);
        int result = AssocQueryString(flags, kind, association, null, output, ref size);
        return result == 0 ? output.ToString() : "<HRESULT 0x" + result.ToString("X8") + ">";
    }
    public static string AssociatedExecutable() { return Association(2, ".log", 0); }
    public static string AssociatedProgId() { return Association(20, ".log", 0); }
    public static string RegisteredExecutable(string progId) { return Association(2, progId, 0x800); }
    public static int LogAssociationResult() {
        uint size = 256;
        return AssocQueryString(0x400, 20, ".log", null, new StringBuilder((int)size), ref size);
    }
    public static string[] Arguments(string commandLine) {
        int count;
        IntPtr values = CommandLineToArgvW(commandLine, out count);
        if (values == IntPtr.Zero) throw new System.ComponentModel.Win32Exception();
        try {
            var result = new string[count];
            for (int i = 0; i < count; i++) result[i] = Marshal.PtrToStringUni(Marshal.ReadIntPtr(values, i * IntPtr.Size));
            return result;
        } finally { LocalFree(values); }
    }
    public static string DialogText(IntPtr dialog) {
        var output = new StringBuilder();
        for (IntPtr child = GetWindow(dialog, 5); child != IntPtr.Zero; child = GetWindow(child, 2)) {
            var text = new StringBuilder(4096);
            GetWindowText(child, text, text.Capacity);
            output.AppendLine(text.ToString());
        }
        return output.ToString();
    }
    public static string[] OwnedWindows(uint processId) {
        var output = new System.Collections.Generic.List<string>();
        EnumWindows(delegate(IntPtr window, IntPtr parameter) {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner != processId) return true;
            var name = new StringBuilder(256);
            var title = new StringBuilder(256);
            GetClassName(window, name, name.Capacity);
            GetWindowText(window, title, title.Capacity);
            output.Add("hwnd=0x" + window.ToString("X") + "; class=" + name +
                "; visible=" + IsWindowVisible(window) + "; title=" + title);
            return output.Count < 8;
        }, IntPtr.Zero);
        return output.ToArray();
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
$helperInvocations = Join-Path $helperDirectory 'helper-invocations.txt'
$data = Join-Path $install 'local data'
$logPath = Join-Path $data 'BrightWheel\updater.log'
$runtimeLog = Join-Path $data 'BrightWheel\runtime.log'
$originalSystemRoot = $env:SystemRoot
$process = $null
$notepad = $null
$config = $null
$extension = $null
$extensionExisted = $false
$commandKey = $null
$commandChanged = $false
$createdCommandKeys = [Collections.Generic.List[string]]::new()
$associationChanged = $false
# ProgIDs must fit 39 characters; retain the complete random GUID for uniqueness.
$fallbackProgId = "BWLog.$unique"
$fallbackClassPath = "Software\Classes\$fallbackProgId"
$fallbackClassCreated = $false
$extensionPath = 'Software\Classes\.log'
$commandPath = 'Software\Classes\txtfile\shell\open\command'
$configPath = 'Software\BrightWheel'
$savedConfig = @{}

function Wait-For([scriptblock]$condition, [string]$description) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        if (& $condition) { return }
        if ($null -ne $process -and $process.HasExited) { throw "BrightWheel exited while waiting for $description" }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    Write-TimeoutDiagnostics $description
    throw "Timed out waiting for $description"
}

function Read-Text([string]$path) {
    if (Test-Path -LiteralPath $path) { return [IO.File]::ReadAllText($path) }
    return ''
}

function Write-TimeoutDiagnostics([string]$description) {
    # Collect only this fixture's process tree and short log tails, before its
    # finally block destroys them. A diagnostic failure must preserve the test error.
    Write-Host "Fixture timeout: $description"
    try {
        Write-Host "Effective .log association: ProgId='$([UpdateLogNative]::AssociatedProgId())'; reader='$([UpdateLogNative]::AssociatedExecutable())'; IGNOREUNKNOWN result=$([UpdateLogNative]::LogAssociationResult())"
        if ($null -ne $process) {
            $owned = @(Get-CimInstance Win32_Process -OperationTimeoutSec 5 -Filter "ProcessId=$($process.Id) OR ParentProcessId=$($process.Id)" | Select-Object -First 8)
            foreach ($item in $owned) {
                Write-Host "Owned process: pid=$($item.ProcessId); parent=$($item.ParentProcessId); executable=$($item.ExecutablePath)"
                foreach ($window in [UpdateLogNative]::OwnedWindows([uint32]$item.ProcessId)) { Write-Host $window }
            }
        }
        foreach ($path in @($helperInvocations, $runtimeLog, $logPath)) {
            Write-Host "Fixture diagnostic tail: $path"
            if (Test-Path -LiteralPath $path) {
                Get-Content -LiteralPath $path -Tail 8 | ForEach-Object { Write-Host $_.Substring(0, [Math]::Min($_.Length, 400)) }
            } else { Write-Host '<absent>' }
        }
    } catch { Write-Host "Could not collect fixture diagnostics: $($_.Exception.Message)" }
}

function Read-RegistryDefault([Microsoft.Win32.RegistryKey]$root, [string]$path) {
    $key = $root.OpenSubKey($path)
    try {
        if ($null -eq $key) { return '<absent>' }
        return $key.GetValue('', '<absent>', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    } finally { if ($null -ne $key) { $key.Dispose() } }
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
    # MN_GETHMENU reads the real TrackPopupMenu menu. Invoke its actual UI
    # Automation item only after checking the command ID and enabled label.
    $menu = [UpdateLogNative]::SendMessage($script:popup, 0x01e1, [UIntPtr]::Zero, [IntPtr]::Zero)
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
    }
    if (-not $found) { throw "Tray menu did not expose enabled '$label'" }
    $popupElement = [System.Windows.Automation.AutomationElement]::FromHandle($script:popup)
    $nameCondition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty, $label)
    Wait-For {
        $script:menuItem = $popupElement.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $nameCondition)
        $null -ne $script:menuItem -and $script:menuItem.Current.ProcessId -eq $process.Id -and
            $script:menuItem.Current.IsEnabled -and
            $script:menuItem.Current.ControlType -eq [System.Windows.Automation.ControlType]::MenuItem
    } "the owned UI Automation menu item '$label'"
    $invoke = [System.Windows.Automation.InvokePattern]$script:menuItem.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    Write-Host "Invoking owned tray menu: pid=$($process.Id); command=$commandId; label='$label'"
    $invoke.Invoke()
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

    $extension = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($extensionPath)
    $extensionExisted = $null -ne $extension
    $hadDefault = $extensionExisted -and $extension.GetValueNames() -contains ''
    if ($hadDefault) {
        $oldDefault = $extension.GetValue('', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $oldKind = $extension.GetValueKind('')
    }
    if ($null -ne $extension) { $extension.Dispose(); $extension = $null }
    $originalExtension = if ($hadDefault) { "$oldDefault ($oldKind)" } else { '<absent>' }
    # Observe only the ProgId, not the protected hash; UserChoice is never changed.
    $userChoice = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.log\UserChoice')
    try { $userChoiceProgId = if ($null -ne $userChoice) { $userChoice.GetValue('ProgId', '<absent>') } else { '<absent>' } }
    finally { if ($null -ne $userChoice) { $userChoice.Dispose() } }
    $baselineReader = [UpdateLogNative]::AssociatedExecutable()
    $baselineProgId = [UpdateLogNative]::AssociatedProgId()
    $systemNotepad = Join-Path ([Environment]::GetFolderPath('System')) 'notepad.exe'
    Write-Host "Fixture association before setup: reader='$baselineReader'; resolved ProgId='$baselineProgId'; HKCU default='$originalExtension'; UserChoice ProgId='$userChoiceProgId'"
    if ($baselineProgId -ine 'txtfile' -or $baselineReader -ine $systemNotepad) {
        throw 'The disposable runner must start with the txtfile/system Notepad .log association'
    }
    $mergedCommandKey = [Microsoft.Win32.Registry]::ClassesRoot.OpenSubKey('txtfile\shell\open\command')
    try {
        if ($null -eq $mergedCommandKey -or $mergedCommandKey.GetValueNames() -contains 'DelegateExecute') {
            throw 'The fixture requires the existing static Notepad open command; DelegateExecute is never changed'
        }
    } finally { if ($null -ne $mergedCommandKey) { $mergedCommandKey.Dispose() } }
    # Keep the effective .log ProgID intact. Only replace its existing static
    # handler command for this disposable user, recording every created parent.
    foreach ($path in @('Software\Classes', 'Software\Classes\txtfile', 'Software\Classes\txtfile\shell',
            'Software\Classes\txtfile\shell\open', $commandPath)) {
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($path, $true)
        if ($null -eq $key) {
            $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($path)
            $createdCommandKeys.Add($path)
        }
        $key.Dispose()
    }
    $commandKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($commandPath, $true)
    $hadCommandDefault = $commandKey.GetValueNames() -contains ''
    if ($hadCommandDefault) {
        $oldCommandDefault = $commandKey.GetValue('', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $oldCommandKind = $commandKey.GetValueKind('')
    }
    $commandChanged = $true
    $commandKey.SetValue('', ('"{0}" "%1"' -f $reader), [Microsoft.Win32.RegistryValueKind]::String)
    $registeredCommand = $commandKey.GetValue('')
    $mergedExtension = Read-RegistryDefault ([Microsoft.Win32.Registry]::ClassesRoot) '.log'
    $mergedCommand = Read-RegistryDefault ([Microsoft.Win32.Registry]::ClassesRoot) 'txtfile\shell\open\command'
    Write-Host "Fixture merged HKCR: .log default='$mergedExtension'; txtfile open command='$mergedCommand'; direct handler='$([UpdateLogNative]::RegisteredExecutable('txtfile'))'"
    # SHCNF_IDLIST | SHCNF_FLUSH waits for change delivery to all consumers.
    [UpdateLogNative]::SHChangeNotify(0x08000000, 0x1000, [IntPtr]::Zero, [IntPtr]::Zero)
    try {
        Wait-For { [UpdateLogNative]::AssociatedExecutable() -ieq $reader } 'the exact effective .log reader'
    } catch {
        throw "The test .log reader did not become effective: expected='$reader'; actual='$([UpdateLogNative]::AssociatedExecutable())'; resolved ProgId='$([UpdateLogNative]::AssociatedProgId())'; direct txtfile reader='$([UpdateLogNative]::RegisteredExecutable('txtfile'))'; registered command='$registeredCommand'; saved HKCU default='$originalExtension'; UserChoice ProgId='$userChoiceProgId'. $($_.Exception.Message)"
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
    # Override only the application's environment. The test host must keep its
    # real SystemRoot so Windows can load the modules used to launch processes.
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $target
    $startInfo.Arguments = '--skip-startup-update'
    $startInfo.UseShellExecute = $false
    $startInfo.EnvironmentVariables['LOCALAPPDATA'] = $data
    $startInfo.EnvironmentVariables['SystemRoot'] = $fakeSystemRoot
    $process = [Diagnostics.Process]::Start($startInfo)
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

    # The real default-reader case must pass first. Recreate the valid owned
    # registration that this hosted shell leaves without an effective default.
    # A nonexistent class alone can silently fall back to txtfile; only the
    # precise native no-association result below establishes this test state.
    $existingClass = [Microsoft.Win32.Registry]::ClassesRoot.OpenSubKey($fallbackProgId)
    if ($null -ne $existingClass) { $existingClass.Dispose(); throw 'The unique fallback fixture ProgID unexpectedly exists' }
    $registration = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($fallbackClassPath)
    $fallbackClassCreated = $true
    try { $registration.SetValue('', 'BrightWheel E2E fallback association', [Microsoft.Win32.RegistryValueKind]::String) }
    finally { $registration.Dispose() }
    $shell = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey("$fallbackClassPath\shell")
    try { $shell.SetValue('', 'open', [Microsoft.Win32.RegistryValueKind]::String) }
    finally { $shell.Dispose() }
    $command = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey("$fallbackClassPath\shell\open\command")
    try { $command.SetValue('', ('"{0}" "%1"' -f $reader), [Microsoft.Win32.RegistryValueKind]::String) }
    finally { $command.Dispose() }
    if ($null -eq $extension) { $extension = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($extensionPath) }
    $associationChanged = $true
    $extension.SetValue('', $fallbackProgId, [Microsoft.Win32.RegistryValueKind]::String)
    [UpdateLogNative]::SHChangeNotify(0x08000000, 0x1000, [IntPtr]::Zero, [IntPtr]::Zero)
    Write-Host "Fallback fixture: owned ProgId='$fallbackProgId'; direct registered reader='$([UpdateLogNative]::RegisteredExecutable($fallbackProgId))'; effective ProgId='$([UpdateLogNative]::AssociatedProgId())'"
    # HRESULT_FROM_WIN32(ERROR_NO_ASSOCIATION), not a null executable query:
    # valid packaged/COM associations need not expose ASSOCSTR_EXECUTABLE.
    Wait-For { [UpdateLogNative]::LogAssociationResult() -eq -2147023741 } 'explicit HRESULT 0x80070483 for no .log association'
    Invoke-TrayMenu 1005 'Open update log'
    Wait-For {
        $script:fallbacks = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($process.Id)" | Where-Object {
            $_.ExecutablePath -ieq $systemNotepad -and $_.CommandLine
        })
        $script:fallbacks.Count -eq 1
    } 'the actual Windows system Notepad child'
    $fallback = $script:fallbacks[0]
    $notepad = Get-Process -Id $fallback.ProcessId
    $arguments = [UpdateLogNative]::Arguments($fallback.CommandLine)
    if ($arguments.Length -ne 2 -or $arguments[1] -cne $logPath) {
        throw 'The installed Notepad did not receive exactly the updater log path'
    }
    Wait-For { $notepad.Refresh(); $notepad.MainWindowHandle -ne [IntPtr]::Zero } 'the actual Notepad window'
    $null = [UpdateLogNative]::PostMessage($notepad.MainWindowHandle, 0x0010, [UIntPtr]::Zero, [IntPtr]::Zero)
    if (-not $notepad.WaitForExit(5000)) { throw 'The owned Notepad window did not close' }
    Assert-NoExtraOpen 3
    Write-Host 'PASS: an explicitly absent .log association opens the installed system Notepad with the exact document argument'
} finally {
    $cleanupErrors = [Collections.Generic.List[string]]::new()
    function Invoke-UpdateLogCleanup([scriptblock]$action) {
        try { & $action } catch { $cleanupErrors.Add($_.Exception.Message) }
    }
    # An already-exiting process or a settings cleanup error must never prevent
    # restoration of the file association. Report all cleanup failures afterwards.
    foreach ($owned in @($notepad, $process)) {
        if ($null -ne $owned) {
            Invoke-UpdateLogCleanup {
                try {
                    if (-not $owned.HasExited) {
                        $owned.Kill()
                        if (-not $owned.WaitForExit(10000)) { throw "Owned process $($owned.Id) did not exit" }
                    }
                } finally { $owned.Dispose() }
            }
        }
    }
    if ($null -ne $config) {
        foreach ($name in $savedConfig.Keys) {
            $saved = $savedConfig[$name]
            Invoke-UpdateLogCleanup {
                if ($null -eq $saved) { $config.DeleteValue($name, $false) }
                else { $config.SetValue($name, $saved.Value, $saved.Kind) }
            }
        }
        Invoke-UpdateLogCleanup {
            $removeConfig = -not $configExisted -and $config.ValueCount -eq 0 -and $config.SubKeyCount -eq 0
            $config.Dispose()
            if ($removeConfig) { [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKey($configPath, $false) }
        }
    }
    Invoke-UpdateLogCleanup {
        if ($commandChanged) {
            if ($hadCommandDefault) { $commandKey.SetValue('', $oldCommandDefault, $oldCommandKind) }
            else { $commandKey.DeleteValue('', $false) }
        }
    }
    Invoke-UpdateLogCleanup { if ($null -ne $commandKey) { $commandKey.Dispose() } }
    Invoke-UpdateLogCleanup {
        if ($associationChanged) {
            if ($hadDefault) { $extension.SetValue('', $oldDefault, $oldKind) }
            else { $extension.DeleteValue('', $false) }
        }
    }
    Invoke-UpdateLogCleanup {
        if ($null -ne $extension) {
            $removeExtension = -not $extensionExisted -and $extension.ValueCount -eq 0 -and $extension.SubKeyCount -eq 0
            $extension.Dispose()
            if ($removeExtension) { [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKey($extensionPath, $false) }
        }
    }
    Invoke-UpdateLogCleanup {
        if ($fallbackClassCreated) { [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($fallbackClassPath, $false) }
    }
    for ($index = $createdCommandKeys.Count - 1; $index -ge 0; $index--) {
        $path = $createdCommandKeys[$index]
        Invoke-UpdateLogCleanup {
            $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($path)
            if ($null -ne $key) {
                try { $empty = $key.ValueCount -eq 0 -and $key.SubKeyCount -eq 0 }
                finally { $key.Dispose() }
                if ($empty) { [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKey($path, $false) }
            }
        }
    }
    Invoke-UpdateLogCleanup { [UpdateLogNative]::SHChangeNotify(0x08000000, 0x1000, [IntPtr]::Zero, [IntPtr]::Zero) }
    Invoke-UpdateLogCleanup {
        if (Test-Path -LiteralPath $runtimeLog) { Get-Content -LiteralPath $runtimeLog -Tail 12 | ForEach-Object { Write-Host $_.Substring(0, [Math]::Min($_.Length, 250)) } }
    }
    Invoke-UpdateLogCleanup { if (Test-Path -LiteralPath $install) { Remove-Item -LiteralPath $install -Recurse -Force } }
    if ($cleanupErrors.Count -gt 0) { throw "Update-log fixture cleanup failed: $($cleanupErrors -join '; ')" }
}
