//! `BrightWheel`'s Windows notification-area application.

#![windows_subsystem = "windows"]

mod brightness_icon;
mod gesture;
mod platform;
mod runtime;
mod tray_icon;
mod updater;

fn main() {
    let mut quit_for_update = false;
    let mut skip_startup_update = false;
    for argument in std::env::args_os().skip(1) {
        quit_for_update |= argument == "--quit-for-update";
        skip_startup_update |= argument == "--skip-startup-update";
    }
    let result = if quit_for_update {
        runtime::request_running_instance_exit()
    } else {
        runtime::run(skip_startup_update)
    };
    if let Err(error) = result {
        platform::show_error(&error.to_string());
    }
}
