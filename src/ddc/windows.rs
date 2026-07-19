use std::mem;
use std::ptr;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Devices::Display::{
    CapabilitiesRequestAndCapabilitiesReply, DestroyPhysicalMonitor, GetCapabilitiesStringLength,
    GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR,
    GetVCPFeatureAndVCPFeatureReply, MC_VCP_CODE_TYPE, PHYSICAL_MONITOR, SetVCPFeature,
};
use windows_sys::Win32::Foundation::{HANDLE, LPARAM, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows_sys::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

use super::{BRIGHTNESS_VCP_CODE, Brightness, percent_to_value};
use crate::wide::decode_null_terminated;
use crate::{DdcError, Result};

const DDC_ATTEMPTS: usize = 5;
const DDC_RETRY_DELAY: Duration = Duration::from_millis(75);

pub(super) struct PhysicalMonitor {
    handle: HANDLE,
    description: String,
    primary: bool,
}

impl PhysicalMonitor {
    pub(super) fn description(&self) -> &str {
        &self.description
    }

    pub(super) fn is_primary(&self) -> bool {
        self.primary
    }

    pub(super) fn read_brightness(&self) -> Result<Brightness> {
        let brightness = retry_ddc("GetVCPFeatureAndVCPFeatureReply(0x10) failed", || {
            let mut code_type: MC_VCP_CODE_TYPE = 0;
            let mut current = 0_u32;
            let mut maximum = 0_u32;

            // SAFETY: `handle` is an owned physical-monitor handle and all output
            // pointers remain valid for the duration of the call.
            let succeeded = unsafe {
                GetVCPFeatureAndVCPFeatureReply(
                    self.handle,
                    BRIGHTNESS_VCP_CODE,
                    &mut code_type,
                    &mut current,
                    &mut maximum,
                )
            };
            if succeeded == 0 {
                return None;
            }
            Some(Brightness { current, maximum })
        })?;

        if brightness.maximum == 0 {
            Err(DdcError::message(
                "monitor reported zero as the maximum brightness",
            ))
        } else {
            Ok(brightness)
        }
    }

    pub(super) fn set_brightness(&self, percent: u32) -> Result<Brightness> {
        let before = self.read_brightness()?;
        let value = percent_to_value(percent, before.maximum);

        retry_ddc("SetVCPFeature(0x10) failed", || {
            // SAFETY: `handle` is an owned physical-monitor handle and the VCP
            // value has already been converted to this monitor's valid range.
            (unsafe { SetVCPFeature(self.handle, BRIGHTNESS_VCP_CODE, value) } != 0).then_some(())
        })?;

        thread::sleep(DDC_RETRY_DELAY);
        self.read_brightness()
    }

    pub(super) fn capabilities(&self) -> Result<String> {
        let mut length = 0_u32;
        // SAFETY: `handle` is valid and `length` is a writable output parameter.
        if unsafe { GetCapabilitiesStringLength(self.handle, &mut length) } == 0 {
            return Err(DdcError::windows("GetCapabilitiesStringLength failed"));
        }
        if length == 0 {
            return Err(DdcError::message(
                "monitor returned an empty capabilities string",
            ));
        }

        let mut buffer = vec![0_u8; length as usize];
        // SAFETY: the buffer is writable for exactly `length` bytes and `handle`
        // remains valid for the call.
        if unsafe {
            CapabilitiesRequestAndCapabilitiesReply(self.handle, buffer.as_mut_ptr(), length)
        } == 0
        {
            return Err(DdcError::windows(
                "CapabilitiesRequestAndCapabilitiesReply failed",
            ));
        }

        let end = buffer
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(buffer.len());
        Ok(String::from_utf8_lossy(&buffer[..end]).into_owned())
    }
}

impl Drop for PhysicalMonitor {
    fn drop(&mut self) {
        // SAFETY: this instance exclusively owns the handle returned by
        // `GetPhysicalMonitorsFromHMONITOR`.
        unsafe {
            DestroyPhysicalMonitor(self.handle);
        }
    }
}

struct EnumContext {
    monitors: Vec<PhysicalMonitor>,
    error: Option<DdcError>,
}

pub(super) fn enumerate() -> Result<Vec<PhysicalMonitor>> {
    let mut context = EnumContext {
        monitors: Vec::new(),
        error: None,
    };

    // SAFETY: enumeration is synchronous, and the callback receives a pointer
    // to `context`, which remains alive and exclusively borrowed for the call.
    let succeeded = unsafe {
        EnumDisplayMonitors(
            ptr::null_mut(),
            ptr::null(),
            Some(enum_monitor),
            (&raw mut context).cast::<core::ffi::c_void>() as LPARAM,
        )
    };

    if let Some(error) = context.error {
        return Err(error);
    }
    if succeeded == 0 {
        return Err(DdcError::windows("EnumDisplayMonitors failed"));
    }
    if context.monitors.is_empty() {
        return Err(DdcError::message("no physical monitors were found"));
    }

    context.monitors.sort_by_key(|monitor| !monitor.primary);
    Ok(context.monitors)
}

unsafe extern "system" fn enum_monitor(
    logical_monitor: HMONITOR,
    _device_context: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> i32 {
    // SAFETY: `enumerate` supplies a live, exclusively borrowed `EnumContext`
    // and Windows invokes this callback synchronously.
    let context = unsafe { &mut *(data as *mut EnumContext) };
    match physical_monitors(logical_monitor) {
        Ok(monitors) => {
            context.monitors.extend(monitors);
            1
        }
        Err(error) => {
            context.error = Some(error);
            0
        }
    }
}

fn physical_monitors(logical_monitor: HMONITOR) -> Result<Vec<PhysicalMonitor>> {
    let mut monitor_info = MONITORINFO {
        cbSize: mem::size_of::<MONITORINFO>() as u32,
        ..MONITORINFO::default()
    };
    // SAFETY: `logical_monitor` is provided by the enumeration callback and
    // `monitor_info` has the required size and remains writable.
    if unsafe { GetMonitorInfoW(logical_monitor, &mut monitor_info) } == 0 {
        return Err(DdcError::windows("GetMonitorInfoW failed"));
    }
    let primary = monitor_info.dwFlags & MONITORINFOF_PRIMARY != 0;

    let mut count = 0_u32;
    // SAFETY: the logical monitor handle is valid during enumeration and
    // `count` is a writable output parameter.
    if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(logical_monitor, &mut count) } == 0 {
        return Err(DdcError::windows(
            "GetNumberOfPhysicalMonitorsFromHMONITOR failed",
        ));
    }
    if count == 0 {
        return Ok(Vec::new());
    }

    let mut raw_monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
    // SAFETY: `raw_monitors` contains space for exactly `count` records and the
    // logical monitor handle is valid during the callback.
    if unsafe { GetPhysicalMonitorsFromHMONITOR(logical_monitor, count, raw_monitors.as_mut_ptr()) }
        == 0
    {
        return Err(DdcError::windows("GetPhysicalMonitorsFromHMONITOR failed"));
    }

    Ok(raw_monitors
        .into_iter()
        .map(|raw| {
            // `PHYSICAL_MONITOR` is packed, so copy fields before borrowing them.
            let handle = raw.hPhysicalMonitor;
            let description = raw.szPhysicalMonitorDescription;
            PhysicalMonitor {
                handle,
                description: decode_null_terminated(&description),
                primary,
            }
        })
        .collect())
}

fn retry_ddc<T>(operation: &str, mut attempt: impl FnMut() -> Option<T>) -> Result<T> {
    let mut last_error = None;
    for attempt_index in 0..DDC_ATTEMPTS {
        if let Some(result) = attempt() {
            return Ok(result);
        }
        last_error = Some(DdcError::windows(operation));
        if attempt_index + 1 < DDC_ATTEMPTS {
            thread::sleep(DDC_RETRY_DELAY);
        }
    }
    Err(last_error.expect("at least one DDC operation was attempted"))
}
