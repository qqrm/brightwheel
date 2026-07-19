use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HHOOK, MB_ICONERROR, MB_OK, MessageBoxW, UnhookWindowsHookEx,
};

pub(crate) fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

pub(crate) fn show_error(message: &str) {
    let message = wide(message);
    let title = wide("BrightWheel error");
    // SAFETY: both strings are null-terminated and no owner window is needed.
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

pub(crate) struct MouseHook(HHOOK);

impl MouseHook {
    pub(crate) fn from_raw(handle: HHOOK) -> Self {
        Self(handle)
    }
}

impl Drop for MouseHook {
    fn drop(&mut self) {
        // SAFETY: this wrapper exclusively owns the installed hook handle.
        unsafe {
            UnhookWindowsHookEx(self.0);
        }
    }
}

pub(crate) struct SingleInstance {
    handle: HANDLE,
    already_running: bool,
}

impl SingleInstance {
    pub(crate) fn acquire(name: &str) -> io::Result<Self> {
        let name = wide(name);
        // SAFETY: the mutex name is null-terminated and default security is used.
        let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: this reads the calling thread's last-error value immediately
        // after `CreateMutexW`, as required to detect an existing mutex.
        let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        Ok(Self {
            handle,
            already_running,
        })
    }

    pub(crate) fn already_running(&self) -> bool {
        self.already_running
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // SAFETY: this wrapper exclusively owns the mutex handle.
        unsafe {
            CloseHandle(self.handle);
        }
    }
}
