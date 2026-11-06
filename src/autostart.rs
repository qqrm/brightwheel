//! Per-user autostart configuration backed by the Windows registry.

use std::env;
use std::path::Path;

use windows_sys::Win32::System::Registry::REG_SZ;

use crate::registry::{CONFIG_KEY, delete_value, read_value, write_dword, write_string};
use crate::{DdcError, Result};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
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
