//! Persistent preferences for the current Windows user.

use windows_sys::Win32::System::Registry::REG_DWORD;

use crate::registry::{CONFIG_KEY, read_value, write_dword};
use crate::{DdcError, Result};

const AUTO_UPDATE_NAME: &str = "AutoUpdateEnabled";

/// Returns whether `BrightWheel` should check for updates at startup.
///
/// Automatic checks are enabled until the user saves a different preference.
///
/// # Errors
///
/// Returns an error when the preference cannot be read or is not a DWORD
/// containing zero (disabled) or one (enabled).
pub fn automatic_updates_enabled() -> Result<bool> {
    let value = read_value(CONFIG_KEY, AUTO_UPDATE_NAME)?;
    decode_automatic_updates(
        value
            .as_ref()
            .map(|(value_type, bytes)| (*value_type, bytes.as_slice())),
    )
}

/// Saves whether `BrightWheel` should check for updates at startup.
///
/// # Errors
///
/// Returns an error when the current user's registry preference cannot be
/// written. Disabling checks stores an explicit choice that survives restarts.
pub fn set_automatic_updates_enabled(enabled: bool) -> Result<()> {
    write_dword(CONFIG_KEY, AUTO_UPDATE_NAME, u32::from(enabled))
}

fn decode_automatic_updates(value: Option<(u32, &[u8])>) -> Result<bool> {
    match value {
        None => Ok(true),
        Some((REG_DWORD, [0, 0, 0, 0])) => Ok(false),
        Some((REG_DWORD, [1, 0, 0, 0])) => Ok(true),
        Some(_) => Err(DdcError::message(
            "invalid automatic update preference: expected a DWORD containing 0 or 1",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::decode_automatic_updates;
    use windows_sys::Win32::System::Registry::{REG_BINARY, REG_DWORD, REG_SZ};

    #[test]
    fn enables_automatic_updates_without_a_saved_preference() {
        assert!(decode_automatic_updates(None).unwrap());
    }

    #[test]
    fn respects_a_saved_disabled_preference() {
        assert!(!decode_automatic_updates(Some((REG_DWORD, &[0, 0, 0, 0]))).unwrap());
    }

    #[test]
    fn respects_a_saved_enabled_preference() {
        assert!(decode_automatic_updates(Some((REG_DWORD, &[1, 0, 0, 0]))).unwrap());
    }

    #[test]
    fn rejects_malformed_preferences_instead_of_enabling_updates() {
        for value_type in [REG_BINARY, REG_SZ] {
            assert!(decode_automatic_updates(Some((value_type, &[1, 0, 0, 0]))).is_err());
        }
        for bytes in [
            &[][..],
            &[1][..],
            &[1, 0, 0][..],
            &[1, 0, 0, 0, 0][..],
            &[2, 0, 0, 0][..],
            &[0, 0, 0, 1][..],
            &[255, 255, 255, 255][..],
        ] {
            assert!(decode_automatic_updates(Some((REG_DWORD, bytes))).is_err());
        }
    }
}
