//! HDR state and toggling for the primary Windows display.

use std::mem;
use std::ptr;

use windows_sys::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
    DISPLAYCONFIG_DEVICE_INFO_SET_ADVANCED_COLOR_STATE, DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    DisplayConfigSetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS,
    QueryDisplayConfig,
};
use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, LUID, POINT};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFOEXW, MonitorFromPoint,
};

use crate::wide::decode_null_terminated;
use crate::{DdcError, Result};

const DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO_2: i32 = 15;
const DISPLAYCONFIG_DEVICE_INFO_SET_HDR_STATE: i32 = 16;

/// HDR state reported for the primary Windows display.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HdrState {
    /// Friendly display name reported by Windows.
    pub display_name: String,
    /// Whether HDR is currently enabled.
    pub enabled: bool,
}

/// Reads HDR state from the primary Windows display.
///
/// # Errors
///
/// Returns an error when the primary display is unavailable, does not support
/// HDR, or its advanced-color state cannot be queried.
pub fn state() -> Result<HdrState> {
    let target = select_target()?;
    Ok(HdrState {
        display_name: target.name,
        enabled: target.enabled,
    })
}

/// Toggles HDR on the primary Windows display and returns the resulting state.
///
/// # Errors
///
/// Returns an error when the primary display is unavailable, does not support
/// HDR, or Windows rejects the advanced-color state change.
pub fn toggle() -> Result<HdrState> {
    let target = select_target()?;
    let enabled = !target.enabled;
    let status = match target.api {
        HdrApi::Modern => {
            let request = DisplayConfigSetHdrState {
                header: header(
                    DISPLAYCONFIG_DEVICE_INFO_SET_HDR_STATE,
                    mem::size_of::<DisplayConfigSetHdrState>(),
                    target.adapter_id,
                    target.id,
                ),
                flags: u32::from(enabled),
            };
            // SAFETY: the packet is `repr(C)`, its header contains the exact
            // packet size, and it remains alive for the call.
            (unsafe { DisplayConfigSetDeviceInfo(&request.header) }) as u32
        }
        HdrApi::Legacy => {
            let mut request = DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE {
                header: header(
                    DISPLAYCONFIG_DEVICE_INFO_SET_ADVANCED_COLOR_STATE,
                    mem::size_of::<DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE>(),
                    target.adapter_id,
                    target.id,
                ),
                ..DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE::default()
            };
            request.Anonymous.value = u32::from(enabled);
            // SAFETY: the Windows SDK packet has been initialized with its exact
            // size and remains alive for the call.
            (unsafe { DisplayConfigSetDeviceInfo(&request.header) }) as u32
        }
    };
    check_status("DisplayConfigSetDeviceInfo(HDR) failed", status)?;

    Ok(HdrState {
        display_name: target.name,
        enabled,
    })
}

struct Target {
    adapter_id: LUID,
    id: u32,
    name: String,
    enabled: bool,
    api: HdrApi,
}

#[derive(Clone, Copy)]
enum HdrApi {
    Modern,
    Legacy,
}

fn select_target() -> Result<Target> {
    let paths = active_paths()?;
    let primary_device = primary_device_name()?;

    for path in paths {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: header(
                DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(),
                path.sourceInfo.adapterId,
                path.sourceInfo.id,
            ),
            ..DISPLAYCONFIG_SOURCE_DEVICE_NAME::default()
        };
        // SAFETY: `source` has the header and size required by this query and is
        // writable for the duration of the call.
        let status = unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } as u32;
        if status != ERROR_SUCCESS
            || !decode_null_terminated(&source.viewGdiDeviceName)
                .eq_ignore_ascii_case(&primary_device)
        {
            continue;
        }

        let adapter_id = path.targetInfo.adapterId;
        let id = path.targetInfo.id;

        let (supported, enabled, api) = query_hdr_state(adapter_id, id)?;
        if !supported {
            return Err(DdcError::message(
                "the primary display does not support HDR",
            ));
        }

        let mut name = DISPLAYCONFIG_TARGET_DEVICE_NAME {
            header: header(
                DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(),
                adapter_id,
                id,
            ),
            ..DISPLAYCONFIG_TARGET_DEVICE_NAME::default()
        };
        // SAFETY: `name` has the header and size required by this query and is
        // writable for the duration of the call.
        let status = unsafe { DisplayConfigGetDeviceInfo(&mut name.header) } as u32;
        let display_name = if status == ERROR_SUCCESS {
            decode_null_terminated(&name.monitorFriendlyDeviceName)
        } else {
            format!("Display {id}")
        };

        return Ok(Target {
            adapter_id,
            id,
            name: display_name,
            enabled,
            api,
        });
    }

    Err(DdcError::message(
        "the primary display is not active or does not support HDR",
    ))
}

fn query_hdr_state(adapter_id: LUID, id: u32) -> Result<(bool, bool, HdrApi)> {
    let mut modern = DisplayConfigGetAdvancedColorInfo2 {
        header: header(
            DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO_2,
            mem::size_of::<DisplayConfigGetAdvancedColorInfo2>(),
            adapter_id,
            id,
        ),
        ..DisplayConfigGetAdvancedColorInfo2::default()
    };
    // SAFETY: this locally defined packet matches the documented Windows ABI,
    // carries its exact size, and is writable for the call.
    let modern_status = unsafe { DisplayConfigGetDeviceInfo(&mut modern.header) } as u32;
    if modern_status == ERROR_SUCCESS {
        let (supported, enabled) = modern_hdr_flags(modern.flags);
        return Ok((supported, enabled, HdrApi::Modern));
    }

    let mut legacy = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
        header: header(
            DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
            mem::size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(),
            adapter_id,
            id,
        ),
        ..DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO::default()
    };
    // SAFETY: this Windows SDK packet carries its exact size and is writable
    // for the duration of the call.
    let legacy_status = unsafe { DisplayConfigGetDeviceInfo(&mut legacy.header) } as u32;
    check_status("DisplayConfigGetDeviceInfo(HDR) failed", legacy_status)?;
    // SAFETY: Windows initialized the active union field for this query type.
    let flags = unsafe { legacy.Anonymous.value };
    let (supported, enabled) = legacy_hdr_flags(flags);
    Ok((supported, enabled, HdrApi::Legacy))
}

fn modern_hdr_flags(flags: u32) -> (bool, bool) {
    (flags & (1 << 4) != 0, flags & (1 << 5) != 0)
}

fn legacy_hdr_flags(flags: u32) -> (bool, bool) {
    (flags & 1 != 0, flags & 2 != 0)
}

#[repr(C)]
#[derive(Default)]
struct DisplayConfigGetAdvancedColorInfo2 {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    flags: u32,
    color_encoding: i32,
    bits_per_color_channel: u32,
    active_color_mode: i32,
}

#[repr(C)]
struct DisplayConfigSetHdrState {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    flags: u32,
}

fn primary_device_name() -> Result<String> {
    // SAFETY: no pointer arguments are involved; Windows returns the primary
    // monitor handle for the supplied point.
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    if monitor.is_null() {
        return Err(DdcError::windows("MonitorFromPoint failed"));
    }

    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: `monitor` is valid and `info` has the required `cbSize` value.
    let succeeded = unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) };
    if succeeded == 0 {
        return Err(DdcError::windows("GetMonitorInfoW failed"));
    }
    Ok(decode_null_terminated(&info.szDevice))
}

fn active_paths() -> Result<Vec<DISPLAYCONFIG_PATH_INFO>> {
    for _ in 0..3 {
        let mut path_count = 0_u32;
        let mut mode_count = 0_u32;
        // SAFETY: both count pointers are writable and the query has no other
        // pointer arguments.
        let status = unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
        };
        check_status("GetDisplayConfigBufferSizes failed", status)?;

        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        // SAFETY: both vectors have the capacities reported immediately above;
        // their pointers and count fields remain valid for the call.
        let status = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                ptr::null_mut(),
            )
        };
        if status == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        check_status("QueryDisplayConfig failed", status)?;
        paths.truncate(path_count as usize);
        return Ok(paths);
    }

    Err(DdcError::message(
        "display configuration kept changing during enumeration",
    ))
}

fn header(kind: i32, size: usize, adapter_id: LUID, id: u32) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: kind,
        size: size as u32,
        adapterId: adapter_id,
        id,
    }
}

fn check_status(operation: &str, status: u32) -> Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(DdcError::windows_code(operation, status))
    }
}

#[cfg(test)]
mod tests {
    use super::{legacy_hdr_flags, modern_hdr_flags};

    #[test]
    fn decodes_modern_hdr_support_and_state_bits() {
        assert_eq!(modern_hdr_flags(0), (false, false));
        assert_eq!(modern_hdr_flags(1 << 4), (true, false));
        assert_eq!(modern_hdr_flags((1 << 4) | (1 << 5)), (true, true));
    }

    #[test]
    fn decodes_legacy_hdr_support_and_state_bits() {
        assert_eq!(legacy_hdr_flags(0), (false, false));
        assert_eq!(legacy_hdr_flags(1), (true, false));
        assert_eq!(legacy_hdr_flags(3), (true, true));
    }
}
