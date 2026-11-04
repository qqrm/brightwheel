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

pub(super) fn start(window: HWND) {
    if cfg!(debug_assertions) {
        tray_icon::notify(
            window,
            "BrightWheel",
            "Use a release build to check for updates",
        );
        return;
    }
    if UPDATE_RUNNING.swap(true, Ordering::AcqRel) {
        tray_icon::notify(window, "BrightWheel", "An update check is already running");
        return;
    }

    match spawn_updater() {
        Ok(mut child) => {
            tray_icon::notify(window, "BrightWheel", "Checking for updates");
            let window_id = window as usize;
            if thread::Builder::new()
                .name("brightwheel-update-monitor".into())
                .spawn(move || {
                    let code = child
                        .wait()
                        .ok()
                        .and_then(|status| status.code())
                        .unwrap_or(1);
                    UPDATE_RUNNING.store(false, Ordering::Release);
                    // SAFETY: the window identifier is restored to the handle
                    // supplied by the UI thread. A destroyed window makes the
                    // post fail harmlessly.
                    unsafe {
                        PostMessageW(window_id as HWND, UPDATE_RESULT, code as usize, 0);
                    }
                })
                .is_err()
            {
                UPDATE_RUNNING.store(false, Ordering::Release);
            }
        }
        Err(error) => {
            UPDATE_RUNNING.store(false, Ordering::Release);
            tray_icon::notify(window, "BrightWheel update failed", &error.to_string());
        }
    }
}

fn spawn_updater() -> io::Result<Child> {
    let target = std::env::current_exe()?;
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is unavailable"))?;
    let log_path = local_app_data.join("BrightWheel").join("updater.log");

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

    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "SystemRoot is unavailable"))?;
    let powershell = system_root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
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
