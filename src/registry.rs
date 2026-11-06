//! Shared access to the current user's Windows registry configuration.

use std::mem;
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, REG_SZ, RegCloseKey,
    RegCreateKeyW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};

use crate::wide::encode_null_terminated;
use crate::{DdcError, Result};

pub(crate) const CONFIG_KEY: &str = r"Software\BrightWheel";

pub(crate) fn read_value(subkey: &str, name: &str) -> Result<Option<(u32, Vec<u8>)>> {
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

pub(crate) fn write_string(subkey: &str, name: &str, value: &str) -> Result<()> {
    let words = encode_null_terminated(value);
    write_value(
        subkey,
        name,
        REG_SZ,
        words.as_ptr().cast(),
        (words.len() * mem::size_of::<u16>()) as u32,
    )
}

pub(crate) fn write_dword(subkey: &str, name: &str, value: u32) -> Result<()> {
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

pub(crate) fn delete_value(subkey: &str, name: &str) -> Result<()> {
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
