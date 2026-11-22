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

# Select the real native popup item. A Win32 popup can exist before UI
# Automation exposes its descendants, so inspect its HMENU and displayed item
# rectangle directly, then send an actual click through the Windows input queue.
if (-not [Environment]::UserInteractive -or (Get-Process -Id $PID).SessionId -eq 0) {
    throw 'Autostart E2E requires an interactive Windows desktop.'
}
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
    [StructLayout(LayoutKind.Sequential)]
    public struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }
    public sealed class MenuChoice {
        public IntPtr Popup, Menu;
        public uint Position, CommandId, State;
        public string Text;
    }
    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern int GetMenuItemCount(IntPtr menu);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern int GetMenuString(IntPtr menu, uint item, StringBuilder text, int capacity, uint flags);
    [DllImport("user32.dll")]
    private static extern uint GetMenuItemID(IntPtr menu, int position);
    [DllImport("user32.dll")]
    private static extern uint GetMenuState(IntPtr menu, uint item, uint flags);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool GetMenuItemRect(IntPtr window, IntPtr menu, uint position, out Rect rect);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")]
    private static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [StructLayout(LayoutKind.Sequential)]
    private struct MouseInput {
        public int X, Y;
        public uint Data, Flags, Time;
        public UIntPtr Extra;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct Input { public uint Type; public MouseInput Mouse; }
    [DllImport("user32.dll", SetLastError = true)]
    private static extern uint SendInput(uint count, Input[] inputs, int size);
    private static IntPtr PopupMenu(IntPtr window) {
        UIntPtr menu;
        if (SendMessageTimeout(window, 0x01E1, UIntPtr.Zero, IntPtr.Zero, 2, 1000, out menu) == IntPtr.Zero)
            return IntPtr.Zero;
        return new IntPtr(unchecked((long)menu.ToUInt64()));
    }
    public static MenuChoice FindMenuChoice(IntPtr popup, uint processId, string expectedText, out string diagnostics) {
        uint owner;
        GetWindowThreadProcessId(popup, out owner);
        diagnostics = String.Format("popup=0x{0:x}, owner={1}, visible={2}", popup.ToInt64(), owner, IsWindowVisible(popup));
        if (owner != processId || !IsWindowVisible(popup)) return null;
        var menu = PopupMenu(popup);
        if (menu == IntPtr.Zero) { diagnostics += "; no HMENU yet"; return null; }
        int count = GetMenuItemCount(menu);
        var details = new StringBuilder(diagnostics);
        details.AppendFormat("; HMENU=0x{0:x}, items={1}; ", menu.ToInt64(), count);
        if (count < 1 || count > 128) { diagnostics = details.ToString(); return null; }
        MenuChoice selected = null;
        for (int index = 0; index < count; ++index) {
            var text = new StringBuilder(512);
            GetMenuString(menu, (uint)index, text, text.Capacity, 0x0400);
            uint id = GetMenuItemID(menu, index);
            uint state = GetMenuState(menu, (uint)index, 0x0400);
            details.AppendFormat("[{0}] id={1}, state=0x{2:x}, '{3}'; ", index, id, state, text);
            if (String.Equals(text.ToString().Replace("&", ""), expectedText, StringComparison.Ordinal)) {
                if (selected != null) throw new InvalidOperationException("The popup has duplicate autostart items.");
                selected = new MenuChoice { Popup = popup, Menu = menu, Position = (uint)index,
                    CommandId = id, State = state, Text = text.ToString() };
            }
        }
        if (selected != null) {
            Rect rect;
            if (!GetMenuItemRect(IntPtr.Zero, menu, selected.Position, out rect) ||
                rect.Right <= rect.Left || rect.Bottom <= rect.Top) {
                details.Append("selected item rectangle is not ready; ");
                selected = null;
            } else {
                details.AppendFormat("selected rectangle=({0},{1},{2},{3}); ", rect.Left, rect.Top, rect.Right, rect.Bottom);
            }
        }
        diagnostics = details.ToString();
        return selected;
    }
    public static Point ReadCursor() {
        Point point;
        if (!GetCursorPos(out point)) throw new Win32Exception(Marshal.GetLastWin32Error());
        return point;
    }
    public static bool RestoreCursor(Point point) { return SetCursorPos(point.X, point.Y); }
    public static string ClickMenuChoice(MenuChoice item, uint processId, bool expectedChecked) {
        uint owner;
        GetWindowThreadProcessId(item.Popup, out owner);
        if (owner != processId || !IsWindowVisible(item.Popup) || PopupMenu(item.Popup) != item.Menu)
            throw new InvalidOperationException("The owned popup changed before selection.");
        uint state = GetMenuState(item.Menu, item.Position, 0x0400);
        if (state == UInt32.MaxValue || (state & 3) != 0 || ((state & 8) != 0) != expectedChecked)
            throw new InvalidOperationException("The autostart menu is disabled or its check mark disagrees with the saved preference.");
        var previousDpi = SetThreadDpiAwarenessContext(new IntPtr(-4));
        if (previousDpi == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
        try {
            Rect rect;
            if (!GetMenuItemRect(IntPtr.Zero, item.Menu, item.Position, out rect) ||
                rect.Right <= rect.Left || rect.Bottom <= rect.Top)
                throw new InvalidOperationException("The native menu item has no displayed rectangle.");
            var point = new Point { X = rect.Left + (rect.Right - rect.Left) / 2,
                Y = rect.Top + (rect.Bottom - rect.Top) / 2 };
            if (WindowFromPoint(point) != item.Popup)
                throw new InvalidOperationException("Another window covers the verified menu item.");
            if (!SetCursorPos(point.X, point.Y)) throw new Win32Exception(Marshal.GetLastWin32Error());
            var inputs = new Input[2];
            inputs[0].Mouse.Flags = 0x0002;
            inputs[1].Mouse.Flags = 0x0004;
            if (SendInput(2, inputs, Marshal.SizeOf(typeof(Input))) != 2) {
                int error = Marshal.GetLastWin32Error();
                // A partial insertion must not leave the mouse button held.
                SendInput(1, new Input[] { inputs[1] }, Marshal.SizeOf(typeof(Input)));
                throw new Win32Exception(error, "Windows did not accept the menu click.");
            }
            return String.Format("Clicked native menu item '{0}' id={1} at ({2},{3}), rectangle=({4},{5},{6},{7})",
                item.Text, item.CommandId, point.X, point.Y, rect.Left, rect.Top, rect.Right, rect.Bottom);
        } finally { SetThreadDpiAwarenessContext(previousDpi); }
    }
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
    $item = $null
    $diagnostics = 'No owned native popup was observed.'
    $lastReported = ''
    do {
        $popup = Get-OwnedWindow '#32768' $null $process.Id
        if ($popup -ne [IntPtr]::Zero) {
            $item = [AutostartDesktop]::FindMenuChoice($popup, $process.Id, 'Start with Windows', [ref]$diagnostics)
            if ($diagnostics -cne $lastReported) {
                Write-Host "Autostart native menu: $diagnostics"
                $lastReported = $diagnostics
            }
            if ($null -ne $item) { break }
        }
        if ($process.HasExited) { throw 'BrightWheel exited while opening the tray menu.' }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($null -eq $item) { throw "The production tray menu did not expose Start with Windows within 10 seconds. $diagnostics" }

    $cursor = [AutostartDesktop]::ReadCursor()
    try {
        Write-Host ([AutostartDesktop]::ClickMenuChoice($item, $process.Id, -not $enabled))
        $deadline = [DateTime]::UtcNow.AddSeconds(10)
        do {
            $registered = $null -ne $run.GetValue('BrightWheel')
            if ($registered -eq $enabled) { break }
            if ($process.HasExited) { throw 'BrightWheel exited while changing autostart.' }
            Start-Sleep -Milliseconds 100
        } while ([DateTime]::UtcNow -lt $deadline)
        Assert-RunEntry $enabled
    } finally {
        if (-not [AutostartDesktop]::RestoreCursor($cursor)) { Write-Host 'Could not restore the original test cursor position.' }
    }
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
