# BrightWheel

BrightWheel is a small Windows tray app for controlling an external monitor's
brightness over DDC/CI. It uses the Windows monitor API directly, so it does not
require vendor software.

From the tray icon you can:

- scroll to change the primary monitor's brightness;
- double-click to toggle HDR;
- hold left Ctrl and click to turn off all monitors after five seconds;
- enable or disable startup with Windows, or exit the app.

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

## License

[MIT](LICENSE)
