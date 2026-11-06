//! Detached Windows self-update launcher.
//!
//! A PowerShell helper validates an immutable release asset before requesting
//! shutdown, so the running GUI never needs to overwrite its own executable.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

use super::{runtime::UPDATE_RESULT, tray_icon};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
static UPDATE_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum CheckMode {
    Automatic,
    Manual,
}

pub(super) fn start(window: HWND, mode: CheckMode) {
    let notify = mode == CheckMode::Manual;
    if cfg!(debug_assertions) {
        if notify {
            tray_icon::notify(
                window,
                "BrightWheel",
                "Use a release build to check for updates",
            );
        }
        return;
    }
    if UPDATE_RUNNING.swap(true, Ordering::AcqRel) {
        if notify {
            tray_icon::notify(window, "BrightWheel", "An update check is already running");
        }
        return;
    }

    let window_id = window as usize;
    let worker = thread::Builder::new()
        .name("brightwheel-update-monitor".into())
        .spawn(move || {
            // Start the worker before the helper, so thread-creation failure
            // cannot leave an unmonitored updater running with the guard clear.
            // File I/O and process creation also stay off the UI thread.
            let code = match spawn_updater().and_then(|mut child| child.wait()) {
                Ok(status) => status.code().unwrap_or(1),
                Err(error) => {
                    log_launch_error(&error);
                    1
                }
            };
            UPDATE_RUNNING.store(false, Ordering::Release);
            if notify {
                // SAFETY: the window identifier is restored to the handle
                // supplied by the UI thread. A destroyed window makes the
                // post fail harmlessly.
                unsafe {
                    PostMessageW(window_id as HWND, UPDATE_RESULT, code as usize, 0);
                }
            }
        });
    if let Err(error) = worker {
        UPDATE_RUNNING.store(false, Ordering::Release);
        log_launch_error(&error);
        if notify {
            tray_icon::notify(window, "BrightWheel update failed", &error.to_string());
        }
    } else if notify {
        tray_icon::notify(window, "BrightWheel", "Checking for updates");
    }
}

fn update_log_path() -> io::Result<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is unavailable"))?;
    Ok(local_app_data.join("BrightWheel").join("updater.log"))
}

fn log_launch_error(error: &io::Error) {
    let _ = (|| -> io::Result<()> {
        let log_path = update_log_path()?;
        if let Some(directory) = log_path.parent() {
            fs::create_dir_all(directory)?;
        }
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        writeln!(log, "[launcher] Update check failed: {error}")
    })();
}

fn spawn_updater() -> io::Result<Child> {
    let target = std::env::current_exe()?;
    let log_path = update_log_path()?;
    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "SystemRoot is unavailable"))?;
    let powershell = system_root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("brightwheel-update-{}-{nonce}", std::process::id()));
    fs::create_dir(&directory)?;
    let script = directory.join("updater.ps1");
    let write_result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&script)?;
        output.write_all(include_str!("../../scripts/windows-updater.ps1").as_bytes())?;
        output.sync_all()
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_dir_all(&directory);
        return Err(error);
    }

    let child = Command::new(powershell)
        .arg("-NoLogo")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(&script)
        .arg("-TargetPath")
        .arg(&target)
        .arg("-CurrentVersion")
        .arg(env!("CARGO_PKG_VERSION"))
        .arg("-ParentProcessId")
        .arg(std::process::id().to_string())
        .arg("-LogPath")
        .arg(&log_path)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    if child.is_err() {
        let _ = fs::remove_dir_all(&directory);
    }
    child
}
