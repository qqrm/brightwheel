//! Detached Windows self-update launcher.
//!
//! A PowerShell helper validates an immutable release asset before requesting
//! shutdown, so the running GUI never needs to overwrite its own executable.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    mem,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    ptr,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use windows_sys::Win32::Foundation::{ERROR_NO_ASSOCIATION, HWND};
use windows_sys::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows_sys::Win32::UI::Shell::{
    SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, SW_SHOWNORMAL};

use super::{diagnostics, platform, runtime::UPDATE_RESULT, tray_icon};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_STDERR_BYTES: u64 = 16 * 1024;
static UPDATE_RUNNING: AtomicBool = AtomicBool::new(false);
static LOG_OPEN_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum CheckMode {
    Automatic,
    Manual,
}

pub(super) fn is_running() -> bool {
    UPDATE_RUNNING.load(Ordering::Acquire)
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
        return;
    }

    let window_id = window as usize;
    let worker = thread::Builder::new()
        .name("brightwheel-update-monitor".into())
        .spawn(move || {
            // Start the worker before the helper, so thread-creation failure
            // cannot leave an unmonitored updater running with the guard clear.
            // File I/O and process creation also stay off the UI thread.
            let code = match spawn_updater().and_then(wait_for_updater) {
                Ok(code) => code,
                Err(error) => {
                    log_launch_error(&error);
                    1
                }
            };
            UPDATE_RUNNING.store(false, Ordering::Release);
            if notify {
                post_result(window_id as HWND, code);
            }
        });
    if let Err(error) = worker {
        UPDATE_RUNNING.store(false, Ordering::Release);
        log_launch_error(&error);
        if notify {
            post_result(window, 1);
        }
    }
}

fn post_result(window: HWND, code: i32) {
    // SAFETY: only a scalar exit code is posted to the supplied UI window. A
    // destroyed window makes the post fail harmlessly; no allocation escapes.
    unsafe {
        PostMessageW(window, UPDATE_RESULT, code as usize, 0);
    }
}

fn wait_for_updater(mut child: Child) -> io::Result<i32> {
    // Drain stderr before waiting so a verbose failure cannot fill the pipe and
    // deadlock the helper. Keep only a bounded prefix; stdout is discarded.
    // In particular this captures PowerShell/Add-Type errors before its own
    // try/catch and logging have started, without holding its log file open.
    let stderr = child.stderr.take().map(read_stderr).transpose();
    let code = child.wait()?.code().unwrap_or(1);
    match stderr {
        Ok(Some(stderr)) if code != 0 && code != 10 => {
            log_update_error(&format!(
                "Updater exited with code {code}. stderr (first {MAX_STDERR_BYTES} bytes):\n{}",
                String::from_utf8_lossy(&stderr)
            ));
        }
        Err(error) => log_update_error(&format!("Could not read updater stderr: {error}")),
        _ => {}
    }
    Ok(code)
}

fn read_stderr(mut input: impl Read) -> io::Result<Vec<u8>> {
    let mut captured = Vec::new();
    input
        .by_ref()
        .take(MAX_STDERR_BYTES)
        .read_to_end(&mut captured)?;
    io::copy(&mut input, &mut io::sink())?;
    Ok(captured)
}

fn update_log_path() -> io::Result<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is unavailable"))?;
    Ok(local_app_data.join("BrightWheel").join("updater.log"))
}

fn log_launch_error(error: &io::Error) {
    log_update_error(&format!("Update check failed: {error}"));
}

fn log_update_error(message: &str) {
    diagnostics::log(message);
    let _ = (|| -> io::Result<()> {
        let log_path = update_log_path()?;
        if let Some(directory) = log_path.parent() {
            fs::create_dir_all(directory)?;
        }
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        writeln!(log, "[launcher] {message}")
    })();
}

pub(super) fn open_log(window: HWND) {
    if LOG_OPEN_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let worker = thread::Builder::new()
        .name("brightwheel-open-update-log".into())
        .spawn(|| {
            let result = update_log_path().and_then(|path| {
                open_log_file(&path)
                    .map_err(|error| io::Error::other(format!("{}\n\n{error}", path.display())))
            });
            if let Err(error) = result {
                let message = format!("Could not open the update log.\n\n{error}");
                diagnostics::log(&message);
                // The error dialog belongs to this worker, leaving the tray
                // message loop and relaunch/recovery handling responsive.
                platform::show_error(&message);
            }
            LOG_OPEN_RUNNING.store(false, Ordering::Release);
        });
    if let Err(error) = worker {
        LOG_OPEN_RUNNING.store(false, Ordering::Release);
        diagnostics::log(&format!("Could not start update-log opener: {error}"));
        tray_icon::notify(window, "Could not open the update log", &error.to_string());
    }
}

fn open_log_file(path: &Path) -> io::Result<()> {
    // Do not let an editor silently create a new empty file when the log is
    // missing. Verify readability before handing the document to the shell.
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("The update log is not a regular file"));
    }
    drop(file);

    let _apartment = ShellApartment::initialize()?;
    let document = platform::wide(path);
    let mut execute = SHELLEXECUTEINFOW {
        cbSize: mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpFile: document.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..SHELLEXECUTEINFOW::default()
    };
    // SAFETY: the structure and absolute document path live through the call.
    // A null verb selects the user's default association; no shell command or
    // parameter string is constructed. NOASYNC completes activation before
    // this short-lived STA worker exits, including DDE-based associations.
    if unsafe { ShellExecuteExW(&mut execute) } != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_NO_ASSOCIATION as i32) {
        return Err(error);
    }

    // Only the absence of an association selects Notepad. Broken or denied
    // user-configured editors remain visible errors instead of being bypassed.
    let notepad = system_root()?.join(r"System32\notepad.exe");
    Command::new(notepad).arg(path).spawn()?;
    Ok(())
}

struct ShellApartment;

impl ShellApartment {
    fn initialize() -> io::Result<Self> {
        // SAFETY: this new worker has no prior COM apartment. The matching
        // CoUninitialize runs on the same thread after document activation.
        let result = unsafe {
            CoInitializeEx(
                ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        };
        if result < 0 {
            Err(io::Error::other(format!(
                "Could not initialize Windows file associations (HRESULT {result:#010x})"
            )))
        } else {
            Ok(Self)
        }
    }
}

impl Drop for ShellApartment {
    fn drop(&mut self) {
        // SAFETY: this thread successfully initialized COM once above.
        unsafe { CoUninitialize() };
    }
}

fn system_root() -> io::Result<PathBuf> {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "SystemRoot is unavailable"))
}

fn spawn_updater() -> io::Result<Child> {
    let target = std::env::current_exe()?;
    let log_path = update_log_path()?;
    let powershell = system_root()?.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");

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
        .stderr(Stdio::piped())
        .spawn();

    if child.is_err() {
        let _ = fs::remove_dir_all(&directory);
    }
    child
}

#[cfg(test)]
mod tests {
    use super::{MAX_STDERR_BYTES, read_stderr};
    use std::io::{Cursor, Seek};

    #[test]
    fn retains_bounded_diagnostics_but_drains_verbose_child_stderr() {
        let mut input = Cursor::new(vec![b'x'; MAX_STDERR_BYTES as usize * 3]);
        let retained = read_stderr(&mut input).unwrap();
        assert_eq!(retained, vec![b'x'; MAX_STDERR_BYTES as usize]);
        assert_eq!(input.stream_position().unwrap(), MAX_STDERR_BYTES * 3);
    }
}
