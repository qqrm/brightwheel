//! `BrightWheel`'s Windows notification-area application.

#![windows_subsystem = "windows"]

mod brightness_icon;
mod diagnostics;
mod gesture;
mod lifecycle;
mod platform;
mod runtime;
mod tray_icon;
mod updater;

fn main() {
    let mut quit_for_update = false;
    let mut expected_pid = None;
    let mut skip_startup_update = false;
    let mut skip_if_running = false;
    for argument in std::env::args_os().skip(1) {
        quit_for_update |= argument == "--quit-for-update";
        skip_startup_update |= argument == "--skip-startup-update";
        skip_if_running |= argument == "--skip-if-running";
        if let Some(pid) = argument
            .to_str()
            .and_then(|value| value.strip_prefix("--quit-for-update="))
        {
            quit_for_update = true;
            let Ok(pid) = pid.parse::<u32>() else { return };
            expected_pid = Some(pid);
        }
    }
    diagnostics::log(&format!("start version={}", env!("CARGO_PKG_VERSION")));
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |information| {
        diagnostics::log(&format!("panic: {information}"));
        previous_hook(information);
    }));
    let result = if quit_for_update {
        lifecycle::request_exit(expected_pid).map_err(Into::into)
    } else {
        runtime::run(skip_startup_update, skip_if_running)
    };
    if let Err(error) = result {
        diagnostics::log(&format!("exit with error: {error}"));
        platform::show_error(&error.to_string());
    } else {
        diagnostics::log("exit normally");
    }
}
