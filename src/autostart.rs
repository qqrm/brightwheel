use std::env;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, REG_SZ, RegCloseKey,
    RegCreateKeyW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};

use crate::{DdcError, Result};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const CONFIG_KEY: &str = r"Software\BrightWheel";
const VALUE_NAME: &str = "BrightWheel";
const CONFIGURED_NAME: &str = "AutoStartConfigured";

pub fn initialize_default() -> Result<()> {
    if read_value(CONFIG_KEY, CONFIGURED_NAME)?.is_none() {
        set_enabled(true)?;
    }
    Ok(())
}

pub fn is_enabled() -> Result<bool> {
    let Some((value_type, bytes)) = read_value(RUN_KEY, VALUE_NAME)? else {
        return Ok(false);
    };
    if value_type != REG_SZ || bytes.len() % 2 != 0 {
        return Ok(false);
    }

    let words: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .take_while(|character| *character != 0)
        .collect();
    let registered = String::from_utf16_lossy(&words);

    Ok(registered.eq_ignore_ascii_case(&current_command()?))
}

pub fn set_enabled(enabled: bool) -> Result<()> {
    if enabled {
        write_string(RUN_KEY, VALUE_NAME, &current_command()?)?;
    } else {
        delete_value(RUN_KEY, VALUE_NAME)?;
    }
    write_dword(CONFIG_KEY, CONFIGURED_NAME, 1)
}

fn current_command() -> Result<String> {
    let executable = env::current_exe().map_err(|error| {
        DdcError::message(format!("cannot resolve current executable: {error}"))
    })?;
    Ok(format!("\"{}\"", executable.display()))
}

fn read_value(subkey: &str, name: &str) -> Result<Option<(u32, Vec<u8>)>> {
    let subkey = wide(subkey);
    let name = wide(name);
    let mut key: HKEY = ptr::null_mut();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check_status("RegOpenKeyExW failed", status)?;
    let key = RegistryKey(key);

    let mut value_type = 0_u32;
    let mut size = 0_u32;
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            ptr::null(),
            &mut value_type,
            ptr::null_mut(),
            &mut size,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check_status("RegQueryValueExW failed", status)?;

    let mut bytes = vec![0_u8; size as usize];
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            ptr::null(),
            &mut value_type,
            bytes.as_mut_ptr(),
            &mut size,
        )
    };
    check_status("RegQueryValueExW failed", status)?;
    bytes.truncate(size as usize);
    Ok(Some((value_type, bytes)))
}

fn write_string(subkey: &str, name: &str, value: &str) -> Result<()> {
    let words = wide(value);
    write_value(
        subkey,
        name,
        REG_SZ,
        words.as_ptr().cast(),
        (words.len() * mem::size_of::<u16>()) as u32,
    )
}

fn write_dword(subkey: &str, name: &str, value: u32) -> Result<()> {
    write_value(
        subkey,
        name,
        REG_DWORD,
        (&value as *const u32).cast(),
        mem::size_of::<u32>() as u32,
    )
}

fn write_value(
    subkey: &str,
    name: &str,
    value_type: u32,
    data: *const u8,
    size: u32,
) -> Result<()> {
    let subkey = wide(subkey);
    let name = wide(name);
    let mut key: HKEY = ptr::null_mut();
    let status = unsafe { RegCreateKeyW(HKEY_CURRENT_USER, subkey.as_ptr(), &mut key) };
    check_status("RegCreateKeyW failed", status)?;
    let key = RegistryKey(key);

    let status = unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, value_type, data, size) };
    check_status("RegSetValueExW failed", status)
}

fn delete_value(subkey: &str, name: &str) -> Result<()> {
    let subkey = wide(subkey);
    let name = wide(name);
    let mut key: HKEY = ptr::null_mut();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check_status("RegOpenKeyExW failed", status)?;
    let key = RegistryKey(key);

    let status = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check_status("RegDeleteValueW failed", status)
}

fn check_status(operation: &str, status: u32) -> Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(DdcError::windows_code(operation, status))
    }
}

fn wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(Some(0))
        .collect()
}

struct RegistryKey(HKEY);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}
