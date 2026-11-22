# BrightWheel

BrightWheel is a small Windows tray app for controlling an external monitor's
brightness over DDC/CI. It uses the Windows monitor API directly, so it does not
require vendor software.

From the tray icon you can:

- see the current brightness as an icon color from gray (0%) to yellow (100%);
- scroll to change the primary monitor's brightness;
- double-click to turn off all monitors immediately;
- middle-click to toggle HDR;
- enable or disable startup with Windows;
- enable or disable automatic update checks at startup;
- check for a new release, update in place, and automatically restart;
- open the update log in the Windows default reader for `.log` files;
- exit the app.

Scrolling accelerates from precise 2% steps to 10% steps during a continuous
gesture. Startup with Windows is enabled on first launch and can be disabled
from the tray menu.

Launching BrightWheel again restarts the running instance. The new launch asks
the previous instance to exit, then terminates it if it does not respond within
1.5 seconds. This also recovers an instance whose tray icon is unavailable.
Overlapping launches are serialized, so only one instance owns the tray.

The tray icon is restored after Explorer recreates the taskbar, and its
registration is checked again after display changes and sleep/resume. A
temporarily unavailable shell is retried without blocking the UI.

## Requirements

- 64-bit Windows 10 or 11 for the prebuilt release;
- an external monitor with DDC/CI enabled in its on-screen settings;
- Windows and monitor support for HDR toggling.

## Install

Download `brightwheel.exe` from the
[latest release](https://github.com/qqrm/brightwheel/releases/latest) and run it.
The executable is self-contained, but unsigned releases may trigger a Windows
SmartScreen warning on first launch.

### Updating without manually exiting

BrightWheel checks for updates in the background on every normal launch by
default. The **Check for updates automatically** checkbox in the tray menu
saves your preference; disabling it stops checks on subsequent launches.
**Check for updates** remains available for a manual check at any time.
While a check is running, that menu item becomes **Checking for updates...**
and is disabled. Manual checks show a notification only for the final result;
there is no separate progress notification left in Windows notification history.
Notifications use the **BrightWheel** application name and the fixed full-brightness
icon with five dots. The tray icon continues to reflect the current brightness.

BrightWheel checks the latest stable GitHub Release and, if it is newer,
downloads and verifies its executable assets against GitHub's SHA-256 digests
and byte sizes.
A detached PowerShell helper then asks the running tray process to exit,
replaces `brightwheel.exe` (and the optional co-located `bright.exe` CLI),
and restarts the tray. The previous executables are preserved for rollback
if replacement or restart fails.

The updater requires Windows PowerShell 5.1, HTTPS access to GitHub, and
write access to the installation directory. Automatic startup checks do not show
notifications for checking, no update, or network errors, and never block the
tray UI. There is no periodic polling. After an update or rollback, the helper
skips the startup check for that restart to avoid repeated update attempts; the
next normal launch checks again. Changing the checkbox does not cancel a check
already in progress. The local update log is at
`%LOCALAPPDATA%\BrightWheel\updater.log`. No monitor data is uploaded.

Click an **Update failed** notification to open that log, or choose **Open update
log** from the tray menu at any time. Windows uses your default application for
`.log` files; if no association exists, BrightWheel falls back to Notepad. A
missing or unreadable log produces an error with its full path. Opening the
reader does not block tray controls. Notification clicks refer to the latest
notification in the running instance; use the menu after restarting BrightWheel
or when an older entry in Windows notification history is no longer actionable.
Early helper launch failures and a bounded excerpt of PowerShell errors are
also recorded, including failures before the helper can initialize its own log.

Runtime lifecycle events and errors are recorded separately in
`%LOCALAPPDATA%\BrightWheel\runtime.log`: version, launch, relaunch, shell/display
and resume notifications, failed tray recovery, and shutdown. This local log is
rotated at 256 KiB, retaining one previous file. Logging is best-effort and does
not record typed text or mouse movements.

When installing with `cargo install brightwheel --force` instead, a running
executable still needs to be closed before Cargo can overwrite it.

Alternatively, install both the tray app and CLI with Rust:

```powershell
cargo install brightwheel --locked
```

## Command line

The optional `bright.exe` companion is intended for diagnostics and scripts:

```text
bright list
bright get [MONITOR]
bright set PERCENT [MONITOR]
bright change DELTA [MONITOR]
bright capabilities [MONITOR]
bright hdr
bright hdr-toggle
```

`PERCENT` is a brightness value from 0 to 100. `[MONITOR]` is an optional
numeric index printed by `bright list`; do not type the square brackets. If the
index is omitted, monitor `0` is used.

For example:

```powershell
bright get          # read brightness from monitor 0
bright get 1        # read brightness from monitor 1
bright set 65 1     # set monitor 1 to 65%
bright change -5    # lower monitor 0 by 5%
```

## Build

Build from a Windows shell with Rust 1.85 or newer and the Windows SDK installed
(`rc.exe` is required for the tray icon):

```powershell
cargo build --release --locked
```

The binaries are written to `target\release\brightwheel.exe` and
`target\release\bright.exe`.

Run the project checks with:

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
```

Hardware-independent behavior is covered by automated tests. DDC/CI and HDR
still require manual testing with a compatible monitor.

The Windows CI also runs `scripts/e2e/tray-lifecycle.ps1 -RestartExplorer`
against the production executable. It removes the registered icon and verifies
recovery through actual broadcast delivery, repeats display/resume notifications,
restarts Explorer, and checks normal, unresponsive, and overlapping process
replacement. These notification tests do not claim a physical suspend/resume
test; that still needs a Windows machine and its actual display driver.

On the disposable Windows CI runner, `scripts/e2e/update-log.ps1` checks the
production failure-to-notification path, posted version-4 shell callbacks and
actual tray menu selection. A temporary
`.log` association launches a real reader and verifies the exact path and log
contents. The test restores the original association and settings afterwards;
it does not simulate a physical click on a Windows notification banner.

## License

[MIT](LICENSE)
