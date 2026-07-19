//! `BrightWheel`'s Windows notification-area application.

#![windows_subsystem = "windows"]

mod gesture;
mod platform;
mod runtime;
mod tray_icon;

fn main() {
    if let Err(error) = runtime::run() {
        platform::show_error(&error.to_string());
    }
}
