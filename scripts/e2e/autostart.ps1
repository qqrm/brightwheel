#requires -Version 5.1
# Exercise the real per-user Run entry and tray preference on a disposable
# Windows desktop. No logoff is needed: CreateProcess receives the exact saved
# command line, including its quoting, as it would at sign-in.
param(
    [string]$Executable = 'target/release/brightwheel.exe'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (Get-Process -Name brightwheel -ErrorAction SilentlyContinue) {
    throw 'Autostart E2E requires no existing BrightWheel process.'
}

# The native menu's InvokePattern exercises the production toggle rather than
# manufacturing a disabled preference directly in the registry.
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
public static class AutostartDesktop {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string className, string title);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessage(IntPtr window, uint message, UIntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, UIntPtr wParam,
        IntPtr lParam, uint flags, uint timeout, out UIntPtr result);
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct StartupInfo {
        public uint cb;
        public string reserved, desktop, title;
        public uint x, y, width, height, xChars, yChars, fill, flags;
        public ushort show, reservedSize;
        public IntPtr reservedData, input, output, error;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct ProcessInformation {
        public IntPtr process, thread;
        public uint processId, threadId;
    }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CreateProcess(string applicationName, StringBuilder commandLine,
        IntPtr processAttributes, IntPtr threadAttributes, bool inheritHandles, uint creationFlags,
        IntPtr environment, string currentDirectory, ref StartupInfo startup,
        out ProcessInformation process);
    [DllImport("kernel32.dll")]
    private static extern bool CloseHandle(IntPtr handle);
    public static int StartSavedCommand(string command, string directory) {
        var startup = new StartupInfo();
        startup.cb = (uint)Marshal.SizeOf(typeof(StartupInfo));
        ProcessInformation process;
        if (!CreateProcess(null, new StringBuilder(command), IntPtr.Zero, IntPtr.Zero,
            false, 0, IntPtr.Zero, directory, ref startup, out process)) {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
        CloseHandle(process.thread);
        CloseHandle(process.process);
        return (int)process.processId;
    }
}
'@

function Save-Value($key, [string]$name) {
    if ($key.GetValueNames() -contains $name) {
        return @{
            Value = $key.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            Kind = $key.GetValueKind($name)
        }
    }
    return $null
}

function Restore-Value($key, [string]$name, $saved) {
    if ($null -eq $saved) { $key.DeleteValue($name, $false) }
    else { $key.SetValue($name, $saved.Value, $saved.Kind) }
}

function Get-OwnedWindow([string]$className, [string]$title, [int]$processId) {
    # Also accept an ordinary hidden window, used for Explorer restart recovery.
    foreach ($parent in @([IntPtr]::Zero, [IntPtr](-3))) {
        $after = [IntPtr]::Zero
        while ($true) {
            $window = [AutostartDesktop]::FindWindowEx($parent, $after, $className, $title)
            if ($window -eq [IntPtr]::Zero) { break }
            [uint32]$owner = 0
            $null = [AutostartDesktop]::GetWindowThreadProcessId($window, [ref]$owner)
            if ($owner -eq $processId) { return $window }
            $after = $window
        }
    }
    return [IntPtr]::Zero
}

function Wait-Ready($process) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        if ($process.HasExited) { throw 'BrightWheel exited before its window was ready.' }
        $window = Get-OwnedWindow 'BrightWheel.HiddenWindow' 'BrightWheel' $process.Id
        [UIntPtr]$result = [UIntPtr]::Zero
        if ($window -ne [IntPtr]::Zero -and
            [AutostartDesktop]::SendMessageTimeout($window, 0, [UIntPtr]::Zero, [IntPtr]::Zero,
                2, 1000, [ref]$result) -ne [IntPtr]::Zero) {
            return $window
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'BrightWheel did not create a responsive window.'
}

function Stop-TestInstance($process) {
    if ($null -eq $process -or $process.HasExited) { return }
    $window = Get-OwnedWindow 'BrightWheel.HiddenWindow' 'BrightWheel' $process.Id
    if ($window -ne [IntPtr]::Zero) {
        # Cancel any popup left by a failed UI assertion before normal shutdown.
        $null = [AutostartDesktop]::PostMessage($window, 0x001F, [UIntPtr]::Zero, [IntPtr]::Zero)
        $null = [AutostartDesktop]::PostMessage($window, 0x8005, [UIntPtr]::Zero, [IntPtr]::Zero)
        if ($process.WaitForExit(10000)) { return }
    }
    $process.Kill()
    if (-not $process.WaitForExit(10000)) { throw 'Could not stop the test instance.' }
}

function Assert-RunEntry([bool]$enabled) {
    $actual = $run.GetValue('BrightWheel', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    if ($enabled) {
        if ($actual -cne ('"' + $target + '"') -or
            $run.GetValueKind('BrightWheel') -ne [Microsoft.Win32.RegistryValueKind]::String) {
            throw "Autostart did not store the exact quoted installed path: $actual"
        }
    } elseif ($null -ne $actual) {
        throw "Autostart remained enabled: $actual"
    }
    if ($config.GetValue('AutoStartConfigured') -ne 1) {
        throw 'Autostart preference was not persisted.'
    }
}

function Toggle-Autostart($process, [bool]$enabled) {
    $window = Wait-Ready $process
    if (-not [AutostartDesktop]::PostMessage($window, 0x8001, [UIntPtr]::Zero, [IntPtr]0x007B)) {
        throw 'Could not open the production tray menu.'
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    $popup = [IntPtr]::Zero
    do {
        $popup = Get-OwnedWindow '#32768' $null $process.Id
        if ($popup -ne [IntPtr]::Zero) { break }
        if ($process.HasExited) { throw 'BrightWheel exited while opening the tray menu.' }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($popup -eq [IntPtr]::Zero) { throw 'The production tray menu did not open.' }

    $menu = [System.Windows.Automation.AutomationElement]::FromHandle($popup)
    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::NameProperty, 'Start with Windows')
    $item = $menu.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
    if ($null -eq $item) { throw 'The tray menu has no Start with Windows item.' }
    $pattern = $null
    if ($item.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
        $pattern.Invoke()
    } elseif ($item.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$pattern)) {
        $pattern.Toggle()
    } else { throw 'The native autostart menu item cannot be invoked.' }

    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $registered = $null -ne $run.GetValue('BrightWheel')
        if ($registered -eq $enabled) { break }
        if ($process.HasExited) { throw 'BrightWheel exited while changing autostart.' }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    Assert-RunEntry $enabled
}

$temporaryRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$install = Join-Path $temporaryRoot "BrightWheel autostart $([guid]::NewGuid().ToString('N'))"
$target = Join-Path $install 'brightwheel.exe'
$configPath = 'Software\BrightWheel'
$config = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($configPath, $true)
$configExisted = $null -ne $config
if (-not $configExisted) { $config = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($configPath) }
$run = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Software\Microsoft\Windows\CurrentVersion\Run')
$savedRun = Save-Value $run 'BrightWheel'
$savedConfigured = Save-Value $config 'AutoStartConfigured'
$savedUpdates = Save-Value $config 'AutoUpdateEnabled'
$process = $null

try {
    New-Item -ItemType Directory -Path $install | Out-Null
    Copy-Item -LiteralPath $Executable -Destination $target
    $run.DeleteValue('BrightWheel', $false)
    $config.DeleteValue('AutoStartConfigured', $false)
    $config.SetValue('AutoUpdateEnabled', 0, [Microsoft.Win32.RegistryValueKind]::DWord)

    $process = Start-Process -FilePath $target -WorkingDirectory $install -PassThru
    $null = Wait-Ready $process
    Assert-RunEntry $true
    Write-Host 'PASS: first launch registers the installed executable, including spaces and quotes.'
    Stop-TestInstance $process

    $savedCommand = [string]$run.GetValue('BrightWheel')
    $startedId = [AutostartDesktop]::StartSavedCommand($savedCommand, $install)
    $process = Get-Process -Id $startedId
    $null = Wait-Ready $process
    Assert-RunEntry $true
    Write-Host 'PASS: the exact saved Run command launches a responsive installed application.'

    Toggle-Autostart $process $false
    Stop-TestInstance $process
    foreach ($attempt in 1..2) {
        $process = Start-Process -FilePath $target -WorkingDirectory $install -PassThru
        $null = Wait-Ready $process
        Assert-RunEntry $false
        if ($attempt -eq 1) { Stop-TestInstance $process }
    }
    Write-Host 'PASS: disabling autostart through the real tray menu survives subsequent launches.'

    Toggle-Autostart $process $true
    Stop-TestInstance $process
    $startedId = [AutostartDesktop]::StartSavedCommand([string]$run.GetValue('BrightWheel'), $install)
    $process = Get-Process -Id $startedId
    $null = Wait-Ready $process
    Assert-RunEntry $true
    if ($config.GetValue('AutoUpdateEnabled') -ne 0) { throw 'Autostart changed the automatic-update preference.' }
    Write-Host 'PASS: re-enabling autostart restores a working Run command without changing update preferences.'
} finally {
    try { Stop-TestInstance $process }
    finally {
        Restore-Value $run 'BrightWheel' $savedRun
        Restore-Value $config 'AutoStartConfigured' $savedConfigured
        Restore-Value $config 'AutoUpdateEnabled' $savedUpdates
        $run.Dispose()
        $removeConfig = -not $configExisted -and $config.ValueCount -eq 0 -and $config.SubKeyCount -eq 0
        $config.Dispose()
        if ($removeConfig) { [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKey($configPath, $false) }
        if (Test-Path -LiteralPath $install) { Remove-Item -LiteralPath $install -Recurse -Force }
    }
}
