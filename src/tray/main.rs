//! `BrightWheel`'s Windows notification-area application.

#![windows_subsystem = "windows"]

mod brightness_icon;
mod gesture;
mod platform;
mod runtime;
mod tray_icon;
mod updater;

fn main() {
    let quit_for_update = std::env::args()
        .skip(1)
        .any(|argument| argument == "--quit-for-update");
    let result = if quit_for_update {
        runtime::request_running_instance_exit()
    } else {
        runtime::run()
    };
    if let Err(error) = result {
        platform::show_error(&error.to_string());
    }
}
