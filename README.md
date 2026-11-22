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
The published `0.1.9` catalog entry distributes a portable executable and
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

### Windows installation acceptance tests

The Windows CI job and release gate exercise the shipped executable and
installer:

- first launch creates the real per-user Run entry; its exact quoted command
  starts the app from a path containing spaces;
- the actual tray menu disables and re-enables autostart, and the choice
  survives subsequent launches;
- the real WinGet client installs the published `0.1.9` portable package, then
  upgrades the running application through a temporary HTTPS WinGet REST source
  containing the generated candidate manifests;
- subsequent installer upgrades preserve running/stopped state, the autostart
  choice, the installed path, and user settings;
- installed payload hashes, process replacement, tray readiness, and the
  installed version are checked together;
- a failed or unverified update must preserve or restore the working version;
- the built-in updater is exercised with an actual local installer and a
  fixture for GitHub transport, including rejection of a mismatching SHA-256.

Installer tests use explicitly synthetic package versions such as `999.0.1` to
exercise multiple upgrades before a public release exists. They run only in an
elevated session on disposable GitHub-hosted Windows runners. They do not
replace a real sign-in test or hardware-dependent DDC/CI, HDR, and suspend/resume
checks.

The candidate source is registered as a trusted test source on that disposable
VM. It serves the production-generated package metadata and installer hashes;
the initial portable package still comes from the public `winget` source. The
harness also hydrates any declared dependencies from official WinGet exports,
preserving their public installer URLs and metadata, and verifies their exact
installed product-code/version correlation before the application upgrade. The
harness temporarily adds its generated test CA to that VM's machine root
certificate store. It removes that exact certificate, the temporary source,
and the server when it finishes; private TLS material stays outside uploaded
diagnostics. This exercises source-backed package selection and installation;
it does not publish the candidate to the public WinGet catalog.

Both Windows acceptance jobs install the source fixture's pinned dependency,
`PyYAML==6.0.3`, and run its focused tests before invoking the WinGet harness.
The fixture tests can also be run with Python 3.10 or newer and OpenSSL on PATH
for the TLS cases; CI requires those cases to run:

```powershell
python -m pip install -r scripts/e2e/winget-test-source-requirements.txt
python -m unittest discover -s scripts/e2e -p test_winget_test_source.py -v
```

Before invoking WinGet, both jobs also compile and run
`scripts/e2e/inno-array-probe.ps1`. Its required controls check native process
queries and ownership of a live process handle in a fixed buffer. Dynamic-array
handoffs are recorded as diagnostic observations, including any mismatches.
The probe's `result.json` and `setup.log` are preserved with the Windows package
test artifacts; a required control failure blocks the WinGet scenarios.

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
