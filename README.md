# BrightWheel

Small Windows CLI for controlling external monitor brightness over DDC/CI.
It talks directly to the Windows monitor configuration API in `dxva2.dll` and
does not depend on ASUS DisplayWidget or its service.

The project also builds `brightwheel.exe`, a tray-only Windows application:

- hover the tray icon and turn the mouse wheel to change primary-monitor
  brightness; continuous scrolling accelerates dynamically from precise 2%
  steps up to 10% per notch;
- moving the pointer away from the tray icon cancels wheel input that has not
  reached the monitor yet, so a long burst does not keep running in the
  background;
- double-click the tray icon to toggle HDR on the primary monitor;
- right-click to toggle `Start with Windows` or exit;
- startup is enabled automatically on the first run and can then be disabled
  from that menu;
- only one instance can run at a time;
- the MSVC runtime is linked statically, so the release executable only uses
  DLLs included with Windows.

`brightwheel.exe` is self-contained and does not ship an ASUS library. The
brightness control works with external monitors that expose MCCS VCP `0x10`
through Windows DDC/CI; DDC/CI must be enabled in the monitor's on-screen menu.
HDR toggling additionally requires a Windows version and primary monitor that
support the advanced-color display configuration API.

## ASUS XG49V findings

The locally installed ASUS DisplayWidget `3.4.0.041` from Portrait Displays
imports these Windows APIs directly from `dxva2.dll`:

- `GetPhysicalMonitorsFromHMONITOR`
- `GetVCPFeatureAndVCPFeatureReply`
- `SetVCPFeature`
- `DestroyPhysicalMonitors`

Its UI binds brightness to `ddc.vcp16`. Decimal `16` is MCCS VCP code `0x10`
(luminance/brightness); the control range is `0..100`. Contrast is exposed as
`ddc.vcp18`, which maps to VCP `0x12`.

The monitor's own capabilities reply identifies it as `LCDXG49V`, declares
MCCS `2.2`, and includes VCP `10` and `12`. Windows uses the standard Microsoft
`monitor.inf`; there is no ASUS monitor driver in this path.

The app also has a LocalSystem service and named pipes for privileged helper
operations, but the brightness path does not require them. Direct VCP read and
write both work on the connected `ASUS XG49V` (`DISPLAY\AUS49A1`) while
DisplayWidget is running.

DDC/CI is a relatively slow serial protocol. The implementation waits briefly
after writes and retries transient failures, which also reduces collisions with
DisplayWidget's background polling.

## Build

Install directly from crates.io on Windows:

```powershell
cargo install brightwheel --locked
```

Build from a Windows shell:

```powershell
cargo build --release
```

Building the tray executable requires the Windows SDK resource compiler
(`rc.exe`) so Cargo can embed `assets\brightwheel.ico`. The resulting executable
loads the tray icon from its own resources; the image files are not needed at
runtime.

The repository can live on the WSL filesystem and be built by invoking
`cargo.exe` from WSL. Incremental compilation is disabled for development and
test profiles because Windows file locking is not supported for incremental
caches accessed through `\\wsl.localhost`.

The executables are `target\release\bright.exe` and
`target\release\brightwheel.exe`.

## Commands

```text
bright list
bright get [MONITOR]
bright set PERCENT [MONITOR]
bright change DELTA [MONITOR]
bright capabilities [MONITOR]
bright hdr
bright hdr-toggle
```

Examples:

```powershell
bright.exe get
bright.exe set 65
bright.exe change -5
```

`get`, `set`, and `change` print only the resulting percentage, which makes the
binary easy to call from a taskbar widget, AutoHotkey, PowerToys, or a tray UI.

## API references

- [GetVCPFeatureAndVCPFeatureReply](https://learn.microsoft.com/en-us/windows/win32/api/lowlevelmonitorconfigurationapi/nf-lowlevelmonitorconfigurationapi-getvcpfeatureandvcpfeaturereply)
- [SetVCPFeature](https://learn.microsoft.com/en-us/windows/win32/api/lowlevelmonitorconfigurationapi/nf-lowlevelmonitorconfigurationapi-setvcpfeature)
- [GetPhysicalMonitorsFromHMONITOR](https://learn.microsoft.com/en-us/windows/win32/api/physicalmonitorenumerationapi/nf-physicalmonitorenumerationapi-getphysicalmonitorsfromhmonitor)
