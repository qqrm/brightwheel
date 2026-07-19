//! Per-user autostart configuration backed by the Windows registry.

use std::env;
use std::mem;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, REG_SZ, RegCloseKey,
    RegCreateKeyW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};

use crate::wide::encode_null_terminated;
use crate::{DdcError, Result};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const CONFIG_KEY: &str = r"Software\BrightWheel";
const VALUE_NAME: &str = "BrightWheel";
const CONFIGURED_NAME: &str = "AutoStartConfigured";

/// Enables autostart on first launch without overwriting an explicit choice.
///
/// # Errors
///
/// Returns an error when the current executable cannot be resolved or the
/// current user's registry configuration cannot be read or written.
pub fn initialize_default() -> Result<()> {
    if read_value(CONFIG_KEY, CONFIGURED_NAME)?.is_none() {
        set_enabled(true)?;
    }
    Ok(())
}

/// Returns whether the current executable is registered to start with Windows.
///
/// # Errors
///
/// Returns an error when the current executable cannot be resolved or the
/// current user's registry configuration cannot be read.
pub fn is_enabled() -> Result<bool> {
    let Some((value_type, bytes)) = read_value(RUN_KEY, VALUE_NAME)? else {
        return Ok(false);
    };
    let Some(registered) = decode_registry_string(value_type, &bytes) else {
        return Ok(false);
    };

    Ok(registered.eq_ignore_ascii_case(&current_command()?))
}

/// Enables or disables autostart for the current executable.
///
/// # Errors
///
/// Returns an error when the current executable cannot be resolved or the
/// current user's registry configuration cannot be written.
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
    Ok(command_for_executable(&executable))
}

fn command_for_executable(executable: &Path) -> String {
    format!("\"{}\"", executable.display())
}

fn decode_registry_string(value_type: u32, bytes: &[u8]) -> Option<String> {
    if value_type != REG_SZ || bytes.len() % 2 != 0 {
        return None;
    }
    let words = bytes
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .take_while(|character| *character != 0)
        .collect::<Vec<_>>();
    Some(String::from_utf16_lossy(&words))
}

fn read_value(subkey: &str, name: &str) -> Result<Option<(u32, Vec<u8>)>> {
    let subkey = encode_null_terminated(subkey);
    let name = encode_null_terminated(name);
    let mut key: HKEY = ptr::null_mut();
    // SAFETY: the key and name buffers are null-terminated, and `key` is a
    // writable output pointer.
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
    // SAFETY: `key` is open for querying; all output pointers remain valid.
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
    // SAFETY: `bytes` has the size reported by the first query and all output
    // pointers remain valid for the call.
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
    let words = encode_null_terminated(value);
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
    let subkey = encode_null_terminated(subkey);
    let name = encode_null_terminated(name);
    let mut key: HKEY = ptr::null_mut();
    // SAFETY: `subkey` is null-terminated and `key` is a writable output pointer.
    let status = unsafe { RegCreateKeyW(HKEY_CURRENT_USER, subkey.as_ptr(), &mut key) };
    check_status("RegCreateKeyW failed", status)?;
    let key = RegistryKey(key);

    // SAFETY: `key` is open for writing, `name` is null-terminated, and the
    // caller provides a readable data buffer of exactly `size` bytes.
    let status = unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, value_type, data, size) };
    check_status("RegSetValueExW failed", status)
}

fn delete_value(subkey: &str, name: &str) -> Result<()> {
    let subkey = encode_null_terminated(subkey);
    let name = encode_null_terminated(name);
    let mut key: HKEY = ptr::null_mut();
    // SAFETY: the key and name buffers are null-terminated, and `key` is a
    // writable output pointer.
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

    // SAFETY: `key` is open for writes and `name` is null-terminated.
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

struct RegistryKey(HKEY);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        // SAFETY: this wrapper exclusively owns the open registry key.
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{command_for_executable, decode_registry_string};
    use std::path::Path;
    use windows_sys::Win32::System::Registry::{REG_DWORD, REG_SZ};

    #[test]
    fn quotes_the_executable_path() {
        assert_eq!(
            command_for_executable(Path::new(r"C:\Program Files\BrightWheel\brightwheel.exe")),
            r#""C:\Program Files\BrightWheel\brightwheel.exe""#
        );
    }

    #[test]
    fn decodes_a_null_terminated_registry_string() {
        let bytes = "BrightWheel\0ignored"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();

        assert_eq!(
            decode_registry_string(REG_SZ, &bytes).as_deref(),
            Some("BrightWheel")
        );
    }

    #[test]
    fn rejects_non_string_and_misaligned_registry_values() {
        assert_eq!(decode_registry_string(REG_DWORD, &[1, 0, 0, 0]), None);
        assert_eq!(decode_registry_string(REG_SZ, &[1]), None);
    }
}
