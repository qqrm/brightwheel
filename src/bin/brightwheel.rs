#![windows_subsystem = "windows"]

use std::error::Error;
use std::ffi::OsStr;
use std::io;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect,
    Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CS_DBLCLKS, CallNextHookEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, HC_ACTION, HHOOK,
    HICON, HWND_MESSAGE, IDI_APPLICATION, LoadCursorW, LoadIconW, MB_ICONERROR, MB_OK, MF_CHECKED,
    MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, MessageBoxW, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, SetWindowsHookExW, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, UnhookWindowsHookEx,
    WH_MOUSE_LL, WHEEL_DELTA, WM_APP, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
};

const WINDOW_CLASS: &str = "BrightWheel.HiddenWindow";
const WINDOW_TITLE: &str = "BrightWheel";
const INSTANCE_MUTEX: &str = "Local\\BrightWheel.Singleton";
const ICON_RESOURCE_ID: usize = 1;
const ICON_ID: u32 = 1;
const TRAY_CALLBACK: u32 = WM_APP + 1;
const BRIGHTNESS_UPDATED: u32 = WM_APP + 2;
const TOGGLE_HDR: u32 = WM_APP + 3;
const MENU_AUTOSTART: usize = 1001;
const MENU_EXIT: usize = 1002;
const BATCH_WINDOW: Duration = Duration::from_millis(40);

static WINDOW_HANDLE: AtomicUsize = AtomicUsize::new(0);
static ICON_HANDLE: AtomicUsize = AtomicUsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static CURRENT_BRIGHTNESS: AtomicI32 = AtomicI32::new(-1);
static CURRENT_HDR: AtomicI32 = AtomicI32::new(-1);
static WHEEL_SENDER: OnceLock<Sender<WheelEvent>> = OnceLock::new();
static INTERACTION_ACTIVE: AtomicBool = AtomicBool::new(false);
static WHEEL_GENERATION: AtomicU32 = AtomicU32::new(0);
static LAST_LEFT_CLICK: AtomicU32 = AtomicU32::new(0);

fn main() {
    if let Err(error) = run() {
        show_error(&error.to_string());
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let instance = SingleInstance::acquire()?;
    if instance.already_running {
        return Ok(());
    }

    bright::autostart::initialize_default()?;

    if let Ok(brightness) = bright::get(0) {
        CURRENT_BRIGHTNESS.store(brightness.percent() as i32, Ordering::Relaxed);
    }
    if let Ok(state) = bright::hdr::state() {
        CURRENT_HDR.store(i32::from(state.enabled), Ordering::Relaxed);
    }

    let module = unsafe { GetModuleHandleW(ptr::null()) };
    if module.is_null() {
        return Err(io::Error::last_os_error().into());
    }

    let class_name = wide(WINDOW_CLASS);
    let window_title = wide(WINDOW_TITLE);
    let window_class = WNDCLASSW {
        style: CS_DBLCLKS,
        lpfnWndProc: Some(window_proc),
        hInstance: module,
        hCursor: unsafe { LoadCursorW(ptr::null_mut(), 32512_u16 as *const u16) },
        lpszClassName: class_name.as_ptr(),
        ..WNDCLASSW::default()
    };
    if unsafe { RegisterClassW(&window_class) } == 0 {
        return Err(io::Error::last_os_error().into());
    }

    let icon = unsafe { LoadIconW(module, ICON_RESOURCE_ID as *const u16) };
    let icon = if icon.is_null() {
        unsafe { LoadIconW(ptr::null_mut(), IDI_APPLICATION) }
    } else {
        icon
    };
    if icon.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    ICON_HANDLE.store(icon as usize, Ordering::Release);

    let window = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_title.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            ptr::null_mut(),
            module,
            ptr::null(),
        )
    };
    if window.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    WINDOW_HANDLE.store(window as usize, Ordering::Release);

    let (wheel_sender, wheel_receiver) = mpsc::channel();
    WHEEL_SENDER
        .set(wheel_sender)
        .map_err(|_| "wheel worker was already initialized")?;
    thread::Builder::new()
        .name("brightness-worker".to_owned())
        .spawn(move || brightness_worker(wheel_receiver))?;

    let taskbar_created = wide("TaskbarCreated");
    let taskbar_message = unsafe { RegisterWindowMessageW(taskbar_created.as_ptr()) };
    TASKBAR_CREATED.store(taskbar_message, Ordering::Release);

    add_tray_icon(window, icon)?;

    let hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0) };
    if hook.is_null() {
        unsafe {
            DestroyWindow(window);
        }
        return Err(io::Error::last_os_error().into());
    }
    let hook = MouseHook(hook);

    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
        if result == -1 {
            return Err(io::Error::last_os_error().into());
        }
        if result == 0 {
            break;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    drop(hook);
    drop(instance);
    Ok(())
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == TASKBAR_CREATED.load(Ordering::Acquire) {
        let icon = ICON_HANDLE.load(Ordering::Acquire) as HICON;
        if !icon.is_null() {
            let _ = add_tray_icon(window, icon);
        }
        return 0;
    }

    match message {
        TRAY_CALLBACK => {
            let event = lparam as u32 & 0xffff;
            if event == WM_CONTEXTMENU || event == WM_RBUTTONUP {
                show_context_menu(window);
            }
            0
        }
        TOGGLE_HDR => {
            match bright::hdr::toggle() {
                Ok(state) => {
                    CURRENT_HDR.store(i32::from(state.enabled), Ordering::Relaxed);
                    update_tooltip(window);
                }
                Err(error) => show_error(&error.to_string()),
            }
            0
        }
        BRIGHTNESS_UPDATED => {
            CURRENT_BRIGHTNESS.store(wparam as i32, Ordering::Relaxed);
            update_tooltip(window);
            0
        }
        WM_DESTROY => {
            delete_tray_icon(window);
            WINDOW_HANDLE.store(0, Ordering::Release);
            unsafe {
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let window = WINDOW_HANDLE.load(Ordering::Acquire) as HWND;
        if !window.is_null() {
            let event = unsafe {
                &*(lparam as *const windows_sys::Win32::UI::WindowsAndMessaging::MSLLHOOKSTRUCT)
            };
            if wparam as u32 == WM_MOUSEMOVE
                && INTERACTION_ACTIVE.load(Ordering::Relaxed)
                && !point_over_tray_icon(window, event.pt)
            {
                cancel_wheel_interaction();
            } else if wparam as u32 == WM_MOUSEMOVE
                && LAST_LEFT_CLICK.load(Ordering::Relaxed) != 0
                && !point_over_tray_icon(window, event.pt)
            {
                LAST_LEFT_CLICK.store(0, Ordering::Relaxed);
            } else if wparam as u32 == WM_MOUSEWHEEL && point_over_tray_icon(window, event.pt) {
                INTERACTION_ACTIVE.store(true, Ordering::Relaxed);
                let delta = ((event.mouseData >> 16) as u16) as i16 as i32;
                let steps = if delta.abs() >= WHEEL_DELTA as i32 {
                    delta / WHEEL_DELTA as i32
                } else {
                    delta.signum()
                };
                if let Some(sender) = WHEEL_SENDER.get() {
                    let _ = sender.send(WheelEvent {
                        steps,
                        timestamp: Instant::now(),
                        generation: WHEEL_GENERATION.load(Ordering::Relaxed),
                    });
                }
                return 1;
            } else if wparam as u32 == WM_LBUTTONUP {
                if point_over_tray_icon(window, event.pt) {
                    let previous = LAST_LEFT_CLICK.swap(event.time, Ordering::Relaxed);
                    let interval = event.time.wrapping_sub(previous);
                    if previous != 0 && interval <= unsafe { GetDoubleClickTime() } {
                        LAST_LEFT_CLICK.store(0, Ordering::Relaxed);
                        unsafe {
                            PostMessageW(window, TOGGLE_HDR, 0, 0);
                        }
                    }
                } else {
                    LAST_LEFT_CLICK.store(0, Ordering::Relaxed);
                }
            }
        }
    }

    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

fn brightness_worker(receiver: Receiver<WheelEvent>) {
    let mut accelerator = WheelAccelerator::default();

    while let Ok(first) = receiver.recv() {
        let mut generation = first.generation;
        let mut adjustment = accelerator.adjust(first);
        let deadline = Instant::now() + BATCH_WINDOW;

        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match receiver.recv_timeout(deadline - now) {
                Ok(event) => {
                    if event.generation != generation {
                        generation = event.generation;
                        adjustment = 0;
                        accelerator.reset();
                    }
                    adjustment = adjustment.saturating_add(accelerator.adjust(event));
                }
                Err(_) => break,
            }
        }

        if generation != WHEEL_GENERATION.load(Ordering::Relaxed)
            || !INTERACTION_ACTIVE.load(Ordering::Relaxed)
        {
            accelerator.reset();
            continue;
        }

        let brightness = match bright::change(0, adjustment) {
            Ok(brightness) => brightness.percent() as i32,
            Err(_) => -1,
        };
        let window = WINDOW_HANDLE.load(Ordering::Acquire) as HWND;
        if !window.is_null() {
            unsafe {
                PostMessageW(window, BRIGHTNESS_UPDATED, brightness as WPARAM, 0);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct WheelEvent {
    steps: i32,
    timestamp: Instant,
    generation: u32,
}

#[derive(Default)]
struct WheelAccelerator {
    direction: i32,
    streak: u32,
    last_event: Option<Instant>,
}

impl WheelAccelerator {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn adjust(&mut self, event: WheelEvent) -> i32 {
        let direction = event.steps.signum();
        let reset = direction != self.direction
            || self.last_event.is_none_or(|last| {
                event.timestamp.saturating_duration_since(last) > Duration::from_millis(350)
            });
        if reset {
            self.streak = 0;
            self.direction = direction;
        }
        self.last_event = Some(event.timestamp);

        let mut adjustment: i32 = 0;
        for _ in 0..event.steps.unsigned_abs() {
            self.streak = self.streak.saturating_add(1);
            let step = match self.streak {
                1..=2 => 2,
                3..=5 => 4,
                6..=9 => 7,
                _ => 10,
            };
            adjustment = adjustment.saturating_add(direction.saturating_mul(step));
        }
        adjustment
    }
}

fn cancel_wheel_interaction() {
    INTERACTION_ACTIVE.store(false, Ordering::Relaxed);
    WHEEL_GENERATION.fetch_add(1, Ordering::Relaxed);
}

fn point_over_tray_icon(window: HWND, point: POINT) -> bool {
    let identifier = NOTIFYICONIDENTIFIER {
        cbSize: mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: window,
        uID: ICON_ID,
        ..NOTIFYICONIDENTIFIER::default()
    };
    let mut rectangle = RECT::default();
    let result = unsafe { Shell_NotifyIconGetRect(&identifier, &mut rectangle) };
    result == 0
        && point.x >= rectangle.left
        && point.x < rectangle.right
        && point.y >= rectangle.top
        && point.y < rectangle.bottom
}

fn add_tray_icon(window: HWND, icon: HICON) -> io::Result<()> {
    let mut data = tray_data(window);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = TRAY_CALLBACK;
    data.hIcon = icon;
    set_tooltip(&mut data.szTip);

    if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
        return Err(io::Error::last_os_error());
    }

    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    if unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) } == 0 {
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &data);
        }
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn update_tooltip(window: HWND) {
    let mut data = tray_data(window);
    data.uFlags = NIF_TIP | NIF_SHOWTIP;
    set_tooltip(&mut data.szTip);
    unsafe {
        Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

fn delete_tray_icon(window: HWND) {
    let data = tray_data(window);
    unsafe {
        Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

fn tray_data(window: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: ICON_ID,
        ..NOTIFYICONDATAW::default()
    }
}

fn set_tooltip(destination: &mut [u16; 128]) {
    let brightness = CURRENT_BRIGHTNESS.load(Ordering::Relaxed);
    let hdr = CURRENT_HDR.load(Ordering::Relaxed);
    let brightness = if brightness >= 0 {
        format!("Brightness {brightness}%")
    } else {
        "Brightness unavailable".to_owned()
    };
    let hdr = match hdr {
        0 => "HDR Off",
        1 => "HDR On",
        _ => "HDR unavailable",
    };
    copy_wide(destination, &format!("BrightWheel | {brightness} | {hdr}"));
}

fn show_context_menu(window: HWND) {
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        show_error(&io::Error::last_os_error().to_string());
        return;
    }

    let autostart_enabled = bright::autostart::is_enabled().unwrap_or(false);
    let autostart_label = wide("Start with Windows");
    let exit_label = wide("Exit");
    let check_flag = if autostart_enabled {
        MF_CHECKED
    } else {
        MF_UNCHECKED
    };

    unsafe {
        AppendMenuW(
            menu,
            MF_STRING | check_flag,
            MENU_AUTOSTART,
            autostart_label.as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(menu, MF_STRING, MENU_EXIT, exit_label.as_ptr());
        SetForegroundWindow(window);
    }

    let mut point = POINT::default();
    unsafe {
        GetCursorPos(&mut point);
    }
    let command = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            point.x,
            point.y,
            0,
            window,
            ptr::null(),
        )
    } as usize;

    match command {
        MENU_AUTOSTART => {
            if let Err(error) = bright::autostart::set_enabled(!autostart_enabled) {
                show_error(&error.to_string());
            }
        }
        MENU_EXIT => unsafe {
            DestroyWindow(window);
        },
        _ => {}
    }

    unsafe {
        DestroyMenu(menu);
        PostMessageW(window, WM_NULL, 0, 0);
    }
}

fn copy_wide<const N: usize>(destination: &mut [u16; N], value: &str) {
    destination.fill(0);
    for (slot, character) in destination
        .iter_mut()
        .take(N.saturating_sub(1))
        .zip(OsStr::new(value).encode_wide())
    {
        *slot = character;
    }
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

fn show_error(message: &str) {
    let message = wide(message);
    let title = wide("BrightWheel error");
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

struct MouseHook(HHOOK);

impl Drop for MouseHook {
    fn drop(&mut self) {
        unsafe {
            UnhookWindowsHookEx(self.0);
        }
    }
}

struct SingleInstance {
    handle: *mut core::ffi::c_void,
    already_running: bool,
}

impl SingleInstance {
    fn acquire() -> io::Result<Self> {
        let name = wide(INSTANCE_MUTEX);
        let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        Ok(Self {
            handle,
            already_running,
        })
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WheelAccelerator, WheelEvent};
    use std::time::{Duration, Instant};

    #[test]
    fn accelerates_a_continuous_burst() {
        let start = Instant::now();
        let mut accelerator = WheelAccelerator::default();
        let adjustments: Vec<i32> = (0..10)
            .map(|index| {
                accelerator.adjust(WheelEvent {
                    steps: 1,
                    timestamp: start + Duration::from_millis(index * 20),
                    generation: 0,
                })
            })
            .collect();

        assert_eq!(adjustments, vec![2, 2, 4, 4, 4, 7, 7, 7, 7, 10]);
    }

    #[test]
    fn resets_after_pause_or_direction_change() {
        let start = Instant::now();
        let mut accelerator = WheelAccelerator::default();
        assert_eq!(
            accelerator.adjust(WheelEvent {
                steps: 3,
                timestamp: start,
                generation: 0,
            }),
            8
        );
        assert_eq!(
            accelerator.adjust(WheelEvent {
                steps: 1,
                timestamp: start + Duration::from_millis(500),
                generation: 0,
            }),
            2
        );
        assert_eq!(
            accelerator.adjust(WheelEvent {
                steps: -1,
                timestamp: start + Duration::from_millis(520),
                generation: 0,
            }),
            -2
        );
    }
}
