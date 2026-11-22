use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::Path;
use std::ptr;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_PARAMETER, HANDLE, HWND, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, QueryFullProcessImageNameW, ReleaseMutex,
    TerminateProcess, WaitForSingleObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetWindowThreadProcessId, PostMessageW, WM_APP,
};

use super::diagnostics;
use super::platform::{SingleInstance, wide};

pub(crate) const WINDOW_CLASS: &str = "BrightWheel.HiddenWindow";
pub(crate) const WINDOW_TITLE: &str = "BrightWheel";
pub(crate) const EXIT_FOR_UPDATE: u32 = WM_APP + 5;
const INSTANCE_MUTEX: &str = "Local\\BrightWheel.Singleton";
const STARTUP_MUTEX: &str = "Local\\BrightWheel.Startup";

pub(crate) struct StartupGate(HANDLE);

impl StartupGate {
    fn acquire() -> io::Result<Self> {
        let name = wide(STARTUP_MUTEX);
        // SAFETY: the name is terminated and default security is sufficient.
        let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the handle is a live mutex; this wait is bounded.
        let result = unsafe { WaitForSingleObject(handle, 15_000) };
        if result == WAIT_OBJECT_0 || result == WAIT_ABANDONED {
            Ok(Self(handle))
        } else {
            let error = if result == WAIT_TIMEOUT {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "another BrightWheel launch is still starting",
                )
            } else {
                io::Error::last_os_error()
            };
            // SAFETY: acquisition failed, so only the unowned handle is closed.
            unsafe { CloseHandle(handle) };
            Err(error)
        }
    }
}

impl Drop for StartupGate {
    fn drop(&mut self) {
        // SAFETY: this guard owns the mutex on the calling startup thread.
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

pub(crate) fn begin(skip_if_running: bool) -> io::Result<Option<(SingleInstance, StartupGate)>> {
    let gate = StartupGate::acquire()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let instance = SingleInstance::acquire(INSTANCE_MUTEX)?;
        if !instance.already_running() {
            return Ok(Some((instance, gate)));
        }
        if skip_if_running {
            return Ok(None);
        }
        let window = find_running_window();
        if !window.is_null() && replace_existing(window)? {
            // Keep our lifetime handle: once the pinned old process has exited,
            // this launch is the only app instance holding it.
            return Ok(Some((instance, gate)));
        }
        // An older release can still be initializing before creating its HWND.
        // Close this probe so it cannot keep a departed instance's mutex alive.
        drop(instance);
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "BrightWheel is running, but its application window could not be located",
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(crate) fn find_running_window() -> HWND {
    let class = wide(WINDOW_CLASS);
    let title = wide(WINDOW_TITLE);
    // SAFETY: both strings are terminated. With both HWND arguments null,
    // FindWindowEx includes top-level AND legacy message-only windows.
    unsafe {
        FindWindowExW(
            ptr::null_mut(),
            ptr::null_mut(),
            class.as_ptr(),
            title.as_ptr(),
        )
    }
}

pub(crate) fn request_exit(expected_pid: Option<u32>) -> io::Result<()> {
    let window = find_running_window();
    if window.is_null() {
        return Ok(());
    }
    let mut pid = 0;
    // SAFETY: the HWND was found on this desktop and pid is writable.
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    if expected_pid.is_some_and(|expected| pid != expected) {
        // A stale updater must not close the user's replacement instance.
        return Ok(());
    }
    // SAFETY: the application message contains no pointers.
    if unsafe { PostMessageW(window, EXIT_FOR_UPDATE, 0, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct Process(HANDLE);

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: this guard exclusively owns a process handle.
        unsafe { CloseHandle(self.0) };
    }
}

fn replace_existing(window: HWND) -> io::Result<bool> {
    let mut pid = 0;
    // SAFETY: the output pointer is valid and this is a current-desktop HWND.
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    if pid == 0 {
        return Ok(false);
    }
    // SAFETY: GetCurrentProcessId takes no pointers.
    if pid == unsafe { GetCurrentProcessId() } {
        return Err(io::Error::other("invalid BrightWheel instance identity"));
    }
    // SAFETY: only the PID associated with our exact window class/title is opened.
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
            0,
            pid,
        )
    };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            // The peer finished between locating its HWND and opening its PID.
            Ok(false)
        } else {
            Err(error)
        };
    }
    let process = Process(handle);
    let mut path = vec![0_u16; 32768];
    let mut length = path.len() as u32;
    // SAFETY: the process permits queries and the writable buffer has length entries.
    if unsafe { QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut length) } == 0 {
        let error = io::Error::last_os_error();
        // SAFETY: an exited process remains waitable through the pinned handle.
        return if unsafe { WaitForSingleObject(process.0, 0) } == WAIT_OBJECT_0 {
            Ok(false)
        } else {
            Err(error)
        };
    }
    let path = OsString::from_wide(&path[..length as usize]);
    let executable = std::env::current_exe()?;
    let existing_name = Path::new(&path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let current_name = executable.file_name().unwrap_or_default().to_string_lossy();
    if !existing_name.eq_ignore_ascii_case(&current_name)
        && !existing_name.eq_ignore_ascii_case("brightwheel.exe")
    {
        return Err(io::Error::other(
            "the existing window does not belong to BrightWheel",
        ));
    }
    let mut current_pid = 0;
    // SAFETY: revalidate the HWND after pinning the process to reject HWND reuse.
    unsafe { GetWindowThreadProcessId(window, &mut current_pid) };
    if current_pid != pid {
        return Ok(false);
    }
    diagnostics::log(&format!("relaunch: requesting shutdown of pid={pid}"));
    // SAFETY: this is a private application message and no pointers are sent.
    unsafe { PostMessageW(window, EXIT_FOR_UPDATE, 0, 0) };
    // SAFETY: the pinned process handle remains live for both waits and termination.
    let status = unsafe { WaitForSingleObject(process.0, 1500) };
    if status == WAIT_OBJECT_0 {
        return Ok(true);
    }
    if status != WAIT_TIMEOUT {
        return Err(io::Error::last_os_error());
    }
    diagnostics::log(&format!("relaunch: terminating unresponsive pid={pid}"));
    // SAFETY: terminate only the verified, pinned previous app process.
    if unsafe { TerminateProcess(process.0, 0) } == 0 {
        // It may have finished during the timeout/termination race.
        // SAFETY: the handle remains valid even after the process exits.
        if unsafe { WaitForSingleObject(process.0, 0) } != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
    }
    // SAFETY: the wait is bounded and uses the same pinned process handle.
    if unsafe { WaitForSingleObject(process.0, 5000) } != WAIT_OBJECT_0 {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "BrightWheel did not stop",
        ));
    }
    Ok(true)
}
