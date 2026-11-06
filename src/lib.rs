//! Windows monitor controls used by the `BrightWheel` tray and CLI applications.
//!
//! Brightness is controlled through DDC/CI. HDR and autostart use native
//! Windows APIs and always target the primary display or the current user.

#![cfg(windows)]

mod ddc;
mod error;
mod registry;
mod wide;

/// Per-user Windows autostart configuration.
pub mod autostart;
/// HDR state for the primary Windows display.
pub mod hdr;
/// Persistent preferences for the current Windows user.
pub mod settings;

pub use ddc::{BRIGHTNESS_VCP_CODE, Brightness, MonitorInfo, capabilities, change, get, list, set};
pub use error::{DdcError, Result};
