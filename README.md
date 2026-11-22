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
- exit the app.

Scrolling accelerates from precise 2% steps to 10% steps during a continuous
gesture. Startup with Windows is enabled on first launch and can be disabled
from the tray menu.

## Requirements

- 64-bit Windows 10 or 11 for the prebuilt release;
- an external monitor with DDC/CI enabled in its on-screen settings;
- Windows and monitor support for HDR toggling.

## Install

Download `brightwheel.exe` from the
[latest release](https://github.com/qqrm/brightwheel/releases/latest) and run it.
The executable is self-contained, but unsigned releases may trigger a Windows
SmartScreen warning on first launch.

### WinGet and UniGetUI installer support

The next installer release adds `brightwheel-setup.exe`, a per-user installer
for Windows 10/11. It updates a running copy, waits for the old process to exit,
and starts the replacement only when the application was running before the
upgrade. A stopped application stays stopped during a silent installation.
Autostart and automatic-update preferences are preserved.

For existing `QQRM.BrightWheel` WinGet portable installations, the installer
validates WinGet's per-user registration and upgrades the executable in the same
directory. This keeps the existing command alias and autostart path valid. The
portable registration is retired only after installation and any required
restart succeed. Fresh installations use `%LOCALAPPDATA%\Programs\BrightWheel`
and a Start menu shortcut. Machine-wide portable installations are outside this
per-user migration path.

This support reaches UniGetUI when a release containing the installer is
published **and its installer manifest is accepted in
[microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs/tree/master/manifests/q/QQRM/BrightWheel)**.
The current `0.1.9` catalog entry still distributes a portable executable and
does not gain these behaviors from a source change alone.

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

For copies owned by the new installer, the same helper instead verifies and
runs `brightwheel-setup.exe`. This also updates the installed version shown to
WinGet and UniGetUI. Installer failures are reported without falling back to
raw executable replacement. Standalone and Cargo installations retain the
portable update path described above.

The updater requires Windows PowerShell 5.1, HTTPS access to GitHub, and
write access to the installation directory. Automatic startup checks do not show
notifications for checking, no update, or network errors, and never block the
tray UI. There is no periodic polling. After an update or rollback, the helper
skips the startup check for that restart to avoid repeated update attempts; the
next normal launch checks again. Changing the checkbox does not cancel a check
already in progress. The local update log is at
`%LOCALAPPDATA%\BrightWheel\updater.log`. No monitor data is uploaded.

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

### Windows installation acceptance tests

The Windows CI job and release gate exercise the shipped executable and
installer:

- first launch creates the real per-user Run entry; its exact quoted command
  starts the app from a path containing spaces;
- the actual tray menu disables and re-enables autostart, and the choice
  survives subsequent launches;
- the real WinGet client installs the published `0.1.9` portable package, then
  upgrades the running application using a generated installer manifest;
- subsequent installer upgrades preserve running/stopped state, the autostart
  choice, the installed path, and user settings;
- installed payload hashes, process replacement, tray readiness, and the
  installed version are checked together;
- a failed or unverified update must preserve or restore the working version;
- the built-in updater is exercised with an actual local installer and a
  fixture for GitHub transport, including rejection of a mismatching SHA-256.

Installer tests use explicitly synthetic package versions such as `999.0.1` to
exercise multiple upgrades before a public release exists. They run only on
disposable GitHub-hosted Windows runners. They do not replace a real sign-in
test or hardware-dependent DDC/CI, HDR, and suspend/resume checks.

### Packaging and WinGet publication

In a Windows shell with Inno Setup 6 installed:

```powershell
cargo build --release --locked
./scripts/package-windows.ps1
winget validate --manifest target/dist/winget
```

The package script emits the installer, portable executables and ZIP,
`SHA256SUMS.txt`, and three WinGet manifests under `target/dist/winget`. The
release workflow publishes these manifests as
`brightwheel-winget-manifests.zip`, with the installer URL and SHA-256 taken
from the actual release build. Prerelease tags retain the portable-only release
format and do not produce a stable installer or WinGet manifests.

After publishing a new version, submit the three generated YAML files under
`manifests/q/QQRM/BrightWheel/<version>/` in `microsoft/winget-pkgs`. Keep the
portable `AppsAndFeaturesEntries` identity in the installer manifest: WinGet
needs it to select the new installer for users upgrading the existing portable
package. Uploading the release alone does not update the WinGet catalog.

## License

[MIT](LICENSE)
