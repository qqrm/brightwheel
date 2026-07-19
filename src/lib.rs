#![cfg(windows)]

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;
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

pub mod autostart;
pub mod hdr;

pub const BRIGHTNESS_VCP_CODE: u8 = 0x10;
const DDC_ATTEMPTS: usize = 5;
const DDC_RETRY_DELAY: Duration = Duration::from_millis(75);

#[derive(Debug)]
pub struct DdcError {
    message: String,
    source: Option<io::Error>,
}

impl DdcError {
    fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    fn windows(operation: &str) -> Self {
        Self {
            message: operation.to_owned(),
            source: Some(io::Error::last_os_error()),
        }
    }

    fn windows_code(operation: &str, code: u32) -> Self {
        Self {
            message: operation.to_owned(),
            source: Some(io::Error::from_raw_os_error(code as i32)),
        }
    }
}

impl Display for DdcError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match &self.source {
            Some(source) => write!(formatter, "{}: {}", self.message, source),
            None => formatter.write_str(&self.message),
        }
    }
}

impl Error for DdcError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

pub type Result<T> = std::result::Result<T, DdcError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Brightness {
    pub current: u32,
    pub maximum: u32,
}

impl Brightness {
    pub fn percent(self) -> u32 {
        value_to_percent(self.current, self.maximum)
    }
}

#[derive(Debug)]
pub struct MonitorInfo {
    pub index: usize,
    pub description: String,
    pub primary: bool,
    pub brightness: Result<Brightness>,
}

struct PhysicalMonitor {
    handle: HANDLE,
    description: String,
    primary: bool,
}

impl Drop for PhysicalMonitor {
    fn drop(&mut self) {
        // The handle is owned by GetPhysicalMonitorsFromHMONITOR.
        unsafe {
            DestroyPhysicalMonitor(self.handle);
        }
    }
}

struct EnumContext {
    monitors: Vec<PhysicalMonitor>,
    error: Option<DdcError>,
}

pub fn list() -> Result<Vec<MonitorInfo>> {
    let monitors = enumerate()?;
    Ok(monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| MonitorInfo {
            index,
            description: monitor.description.clone(),
            primary: monitor.primary,
            brightness: read_monitor(monitor),
        })
        .collect())
}

pub fn get(index: usize) -> Result<Brightness> {
    let monitors = enumerate()?;
    let monitor = select(&monitors, index)?;
    read_monitor(monitor)
}

pub fn set(index: usize, percent: u32) -> Result<Brightness> {
    if percent > 100 {
        return Err(DdcError::message(format!(
            "brightness must be between 0 and 100, got {percent}"
        )));
    }

    let monitors = enumerate()?;
    let monitor = select(&monitors, index)?;
    set_monitor(monitor, percent)
}

pub fn change(index: usize, delta: i32) -> Result<Brightness> {
    let monitors = enumerate()?;
    let monitor = select(&monitors, index)?;
    let current = read_monitor(monitor)?.percent() as i32;
    let target = (current + delta).clamp(0, 100) as u32;
    set_monitor(monitor, target)
}

pub fn capabilities(index: usize) -> Result<String> {
    let monitors = enumerate()?;
    let monitor = select(&monitors, index)?;
    let mut length = 0_u32;

    let succeeded = unsafe { GetCapabilitiesStringLength(monitor.handle, &mut length) };
    if succeeded == 0 {
        return Err(DdcError::windows("GetCapabilitiesStringLength failed"));
    }
    if length == 0 {
        return Err(DdcError::message(
            "monitor returned an empty capabilities string",
        ));
    }

    let mut buffer = vec![0_u8; length as usize];
    let succeeded = unsafe {
        CapabilitiesRequestAndCapabilitiesReply(monitor.handle, buffer.as_mut_ptr(), length)
    };
    if succeeded == 0 {
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

fn read_monitor(monitor: &PhysicalMonitor) -> Result<Brightness> {
    let mut read_error = None;

    for attempt in 0..DDC_ATTEMPTS {
        let mut code_type: MC_VCP_CODE_TYPE = 0;
        let mut current = 0_u32;
        let mut maximum = 0_u32;

        let succeeded = unsafe {
            GetVCPFeatureAndVCPFeatureReply(
                monitor.handle,
                BRIGHTNESS_VCP_CODE,
                &mut code_type,
                &mut current,
                &mut maximum,
            )
        };
        if succeeded != 0 {
            if maximum == 0 {
                return Err(DdcError::message(
                    "monitor reported zero as the maximum brightness",
                ));
            }
            return Ok(Brightness { current, maximum });
        }

        read_error = Some(DdcError::windows(
            "GetVCPFeatureAndVCPFeatureReply(0x10) failed",
        ));
        if attempt + 1 < DDC_ATTEMPTS {
            thread::sleep(DDC_RETRY_DELAY);
        }
    }

    Err(read_error.expect("at least one DDC read was attempted"))
}

fn set_monitor(monitor: &PhysicalMonitor, percent: u32) -> Result<Brightness> {
    let before = read_monitor(monitor)?;
    let value = percent_to_value(percent, before.maximum);
    write_monitor(monitor, value)
}

fn write_monitor(monitor: &PhysicalMonitor, value: u32) -> Result<Brightness> {
    let mut write_error = None;
    for attempt in 0..DDC_ATTEMPTS {
        let succeeded = unsafe { SetVCPFeature(monitor.handle, BRIGHTNESS_VCP_CODE, value) };
        if succeeded != 0 {
            thread::sleep(DDC_RETRY_DELAY);
            return read_monitor(monitor);
        }

        write_error = Some(DdcError::windows("SetVCPFeature(0x10) failed"));
        if attempt + 1 < DDC_ATTEMPTS {
            thread::sleep(DDC_RETRY_DELAY);
        }
    }

    Err(write_error.expect("at least one DDC write was attempted"))
}

fn select(monitors: &[PhysicalMonitor], index: usize) -> Result<&PhysicalMonitor> {
    monitors.get(index).ok_or_else(|| {
        DdcError::message(format!(
            "monitor index {index} is out of range; found {} physical monitor(s)",
            monitors.len()
        ))
    })
}

fn enumerate() -> Result<Vec<PhysicalMonitor>> {
    let mut context = EnumContext {
        monitors: Vec::new(),
        error: None,
    };

    let succeeded = unsafe {
        EnumDisplayMonitors(
            ptr::null_mut(),
            ptr::null(),
            Some(enum_monitor),
            &mut context as *mut EnumContext as LPARAM,
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
    // SAFETY: EnumDisplayMonitors invokes this synchronously while context is alive.
    let context = unsafe { &mut *(data as *mut EnumContext) };
    let mut count = 0_u32;
    let mut monitor_info = MONITORINFO {
        cbSize: mem::size_of::<MONITORINFO>() as u32,
        ..MONITORINFO::default()
    };

    let succeeded = unsafe { GetMonitorInfoW(logical_monitor, &mut monitor_info) };
    if succeeded == 0 {
        context.error = Some(DdcError::windows("GetMonitorInfoW failed"));
        return 0;
    }
    let primary = monitor_info.dwFlags & MONITORINFOF_PRIMARY != 0;

    let succeeded = unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(logical_monitor, &mut count) };
    if succeeded == 0 {
        context.error = Some(DdcError::windows(
            "GetNumberOfPhysicalMonitorsFromHMONITOR failed",
        ));
        return 0;
    }
    if count == 0 {
        return 1;
    }

    let mut raw_monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
    let succeeded = unsafe {
        GetPhysicalMonitorsFromHMONITOR(logical_monitor, count, raw_monitors.as_mut_ptr())
    };
    if succeeded == 0 {
        context.error = Some(DdcError::windows("GetPhysicalMonitorsFromHMONITOR failed"));
        return 0;
    }

    context.monitors.extend(raw_monitors.into_iter().map(|raw| {
        let handle = raw.hPhysicalMonitor;
        let description_utf16 = raw.szPhysicalMonitorDescription;
        let end = description_utf16
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(description_utf16.len());

        PhysicalMonitor {
            handle,
            description: String::from_utf16_lossy(&description_utf16[..end]),
            primary,
        }
    }));

    1
}

fn value_to_percent(value: u32, maximum: u32) -> u32 {
    if maximum == 0 {
        return 0;
    }
    ((u64::from(value) * 100 + u64::from(maximum) / 2) / u64::from(maximum)) as u32
}

fn percent_to_value(percent: u32, maximum: u32) -> u32 {
    ((u64::from(percent) * u64::from(maximum) + 50) / 100) as u32
}

#[cfg(test)]
mod tests {
    use super::{percent_to_value, value_to_percent};

    #[test]
    fn converts_non_hundred_ranges() {
        assert_eq!(value_to_percent(127, 255), 50);
        assert_eq!(percent_to_value(50, 255), 128);
    }

    #[test]
    fn preserves_endpoints() {
        assert_eq!(value_to_percent(0, 100), 0);
        assert_eq!(value_to_percent(100, 100), 100);
        assert_eq!(percent_to_value(0, 100), 0);
        assert_eq!(percent_to_value(100, 100), 100);
    }
}
