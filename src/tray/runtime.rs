use std::error::Error;
use std::io;
use std::ptr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetMessageW, HC_ACTION, HICON, HWND_MESSAGE, IDI_APPLICATION, LoadCursorW, LoadIconW, MSG,
    MSLLHOOKSTRUCT, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW,
    SetWindowsHookExW, TranslateMessage, WH_MOUSE_LL, WM_APP, WM_CONTEXTMENU, WM_DESTROY,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONUP, WNDCLASSW,
};

use super::gesture::{
    WheelAccelerator, WheelEvent, is_double_click, needs_tray_hit_test, steps_from_mouse_data,
};
use super::platform::{MouseHook, SingleInstance, show_error, wide};
use super::tray_icon::{self, MenuCommand, Status};

const WINDOW_CLASS: &str = "BrightWheel.HiddenWindow";
const WINDOW_TITLE: &str = "BrightWheel";
const INSTANCE_MUTEX: &str = "Local\\BrightWheel.Singleton";
const ICON_RESOURCE_ID: usize = 1;
const BRIGHTNESS_UPDATED: u32 = WM_APP + 2;
const TOGGLE_HDR: u32 = WM_APP + 3;
const BATCH_WINDOW: Duration = Duration::from_millis(40);

static STATE: SharedState = SharedState::new();

struct SharedState {
    window: AtomicUsize,
    icon: AtomicUsize,
    taskbar_created: AtomicU32,
    brightness: AtomicI32,
    hdr: AtomicI32,
    wheel_sender: OnceLock<Sender<WheelEvent>>,
    interaction_active: AtomicBool,
    wheel_generation: AtomicU32,
    last_left_click: AtomicU32,
}

impl SharedState {
    const fn new() -> Self {
        Self {
            window: AtomicUsize::new(0),
            icon: AtomicUsize::new(0),
            taskbar_created: AtomicU32::new(0),
            brightness: AtomicI32::new(-1),
            hdr: AtomicI32::new(-1),
            wheel_sender: OnceLock::new(),
            interaction_active: AtomicBool::new(false),
            wheel_generation: AtomicU32::new(0),
            last_left_click: AtomicU32::new(0),
        }
    }

    fn window(&self) -> HWND {
        self.window.load(Ordering::Acquire) as HWND
    }

    fn icon(&self) -> HICON {
        self.icon.load(Ordering::Acquire) as HICON
    }

    fn status(&self) -> Status {
        Status {
            brightness: u32::try_from(self.brightness.load(Ordering::Relaxed)).ok(),
            hdr: match self.hdr.load(Ordering::Relaxed) {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            },
        }
    }

    fn cancel_wheel_interaction(&self) {
        self.interaction_active.store(false, Ordering::Relaxed);
        self.wheel_generation.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn run() -> Result<(), Box<dyn Error>> {
    let instance = SingleInstance::acquire(INSTANCE_MUTEX)?;
    if instance.already_running() {
        return Ok(());
    }

    brightwheel::autostart::initialize_default()?;
    initialize_status();

    // SAFETY: a null module name requests the current executable module.
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
        // SAFETY: the standard arrow cursor is a process-independent resource.
        hCursor: unsafe { LoadCursorW(ptr::null_mut(), 32512_u16 as *const u16) },
        lpszClassName: class_name.as_ptr(),
        ..WNDCLASSW::default()
    };
    // SAFETY: the class record and its string pointers remain valid for the call.
    if unsafe { RegisterClassW(&window_class) } == 0 {
        return Err(io::Error::last_os_error().into());
    }

    // SAFETY: resource ID 1 is linked into the current executable.
    let icon = unsafe { LoadIconW(module, ICON_RESOURCE_ID as *const u16) };
    let icon = if icon.is_null() {
        // SAFETY: the standard application icon is a process-independent resource.
        unsafe { LoadIconW(ptr::null_mut(), IDI_APPLICATION) }
    } else {
        icon
    };
    if icon.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    STATE.icon.store(icon as usize, Ordering::Release);

    // SAFETY: the registered class and all string pointers are valid; this
    // creates a message-only window with no application-owned creation data.
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
    STATE.window.store(window as usize, Ordering::Release);

    start_brightness_worker()?;

    let taskbar_created = wide("TaskbarCreated");
    // SAFETY: the message name is null-terminated and system-global.
    let taskbar_message = unsafe { RegisterWindowMessageW(taskbar_created.as_ptr()) };
    if taskbar_message == 0 {
        // SAFETY: `window` was created by this thread and has not been destroyed.
        unsafe {
            DestroyWindow(window);
        }
        return Err(io::Error::last_os_error().into());
    }
    STATE
        .taskbar_created
        .store(taskbar_message, Ordering::Release);

    if let Err(error) = tray_icon::add(window, icon, STATE.status()) {
        // SAFETY: `window` was created by this thread and has not been destroyed.
        unsafe {
            DestroyWindow(window);
        }
        return Err(error.into());
    }

    // SAFETY: the callback has the required ABI and `module` remains loaded for
    // the process lifetime. Thread ID zero installs the global low-level hook.
    let hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0) };
    if hook.is_null() {
        // SAFETY: `window` was created by this thread and is still live.
        unsafe {
            DestroyWindow(window);
        }
        return Err(io::Error::last_os_error().into());
    }
    let _hook = MouseHook::from_raw(hook);

    run_message_loop()?;
    drop(instance);
    Ok(())
}

fn initialize_status() {
    if let Ok(brightness) = brightwheel::get(0) {
        STATE
            .brightness
            .store(brightness.percent() as i32, Ordering::Relaxed);
    }
    if let Ok(state) = brightwheel::hdr::state() {
        STATE.hdr.store(i32::from(state.enabled), Ordering::Relaxed);
    }
}

fn start_brightness_worker() -> Result<(), Box<dyn Error>> {
    let (sender, receiver) = mpsc::channel();
    STATE
        .wheel_sender
        .set(sender)
        .map_err(|_| "wheel worker was already initialized")?;
    thread::Builder::new()
        .name("brightness-worker".to_owned())
        .spawn(move || brightness_worker(receiver))?;
    Ok(())
}

fn run_message_loop() -> io::Result<()> {
    let mut message = MSG::default();
    loop {
        // SAFETY: `message` is writable and this thread owns the window queue.
        let result = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
        if result == -1 {
            return Err(io::Error::last_os_error());
        }
        if result == 0 {
            return Ok(());
        }
        // SAFETY: the message was initialized by `GetMessageW`.
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == STATE.taskbar_created.load(Ordering::Acquire) {
        let icon = STATE.icon();
        if !icon.is_null() {
            let _ = tray_icon::add(window, icon, STATE.status());
        }
        return 0;
    }

    match message {
        tray_icon::CALLBACK_MESSAGE => {
            let event = lparam as u32 & 0xffff;
            if event == WM_CONTEXTMENU || event == WM_RBUTTONUP {
                show_context_menu(window);
            }
            0
        }
        TOGGLE_HDR => {
            match brightwheel::hdr::toggle() {
                Ok(state) => {
                    STATE.hdr.store(i32::from(state.enabled), Ordering::Relaxed);
                    tray_icon::update(window, STATE.status());
                }
                Err(error) => show_error(&error.to_string()),
            }
            0
        }
        BRIGHTNESS_UPDATED => {
            STATE.brightness.store(wparam as i32, Ordering::Relaxed);
            tray_icon::update(window, STATE.status());
            0
        }
        WM_DESTROY => {
            tray_icon::remove(window);
            STATE.window.store(0, Ordering::Release);
            // SAFETY: this callback runs on the message-loop thread.
            unsafe {
                PostQuitMessage(0);
            }
            0
        }
        // SAFETY: unhandled messages must be forwarded to the default window
        // procedure with their original parameters.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let window = STATE.window();
        if !window.is_null() {
            // SAFETY: for `HC_ACTION`, Windows supplies a valid `MSLLHOOKSTRUCT`
            // pointer for the duration of this callback.
            let event = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
            let message = wparam as u32;
            let needs_hit_test = needs_tray_hit_test(
                message,
                STATE.interaction_active.load(Ordering::Relaxed),
                STATE.last_left_click.load(Ordering::Relaxed) != 0,
            );
            if needs_hit_test {
                let over_icon = tray_icon::contains(window, event.pt);
                handle_mouse_event(window, message, event, over_icon);
                if message == WM_MOUSEWHEEL && over_icon {
                    return 1;
                }
            }
        }
    }

    // SAFETY: events not consumed above must continue through the hook chain.
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

fn handle_mouse_event(window: HWND, message: u32, event: &MSLLHOOKSTRUCT, over_icon: bool) {
    match message {
        WM_MOUSEMOVE => {
            if STATE.interaction_active.load(Ordering::Relaxed) && !over_icon {
                STATE.cancel_wheel_interaction();
            } else if STATE.last_left_click.load(Ordering::Relaxed) != 0 && !over_icon {
                STATE.last_left_click.store(0, Ordering::Relaxed);
            }
        }
        WM_MOUSEWHEEL if over_icon => {
            STATE.interaction_active.store(true, Ordering::Relaxed);
            if let Some(sender) = STATE.wheel_sender.get() {
                let _ = sender.send(WheelEvent {
                    steps: steps_from_mouse_data(event.mouseData),
                    timestamp: Instant::now(),
                    generation: STATE.wheel_generation.load(Ordering::Relaxed),
                });
            }
        }
        WM_LBUTTONUP if over_icon => {
            let previous = STATE.last_left_click.swap(event.time, Ordering::Relaxed);
            // SAFETY: `GetDoubleClickTime` takes no pointer arguments.
            let maximum_interval = unsafe { GetDoubleClickTime() };
            if is_double_click(previous, event.time, maximum_interval) {
                STATE.last_left_click.store(0, Ordering::Relaxed);
                // SAFETY: `window` is the current live message window.
                unsafe {
                    PostMessageW(window, TOGGLE_HDR, 0, 0);
                }
            }
        }
        WM_LBUTTONUP => STATE.last_left_click.store(0, Ordering::Relaxed),
        _ => {}
    }
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

        if generation != STATE.wheel_generation.load(Ordering::Relaxed)
            || !STATE.interaction_active.load(Ordering::Relaxed)
        {
            accelerator.reset();
            continue;
        }

        let brightness = brightwheel::change(0, adjustment)
            .map(|brightness| brightness.percent() as i32)
            .unwrap_or(-1);
        let window = STATE.window();
        if !window.is_null() {
            // SAFETY: `window` is the current live message window and the
            // brightness value is transported directly in `WPARAM`.
            unsafe {
                PostMessageW(window, BRIGHTNESS_UPDATED, brightness as WPARAM, 0);
            }
        }
    }
}

fn show_context_menu(window: HWND) {
    let autostart_enabled = brightwheel::autostart::is_enabled().unwrap_or(false);
    match tray_icon::context_menu(window, autostart_enabled) {
        Ok(Some(MenuCommand::ToggleAutostart)) => {
            if let Err(error) = brightwheel::autostart::set_enabled(!autostart_enabled) {
                show_error(&error.to_string());
            }
        }
        Ok(Some(MenuCommand::Exit)) => {
            // SAFETY: `window` is the current live message window.
            unsafe {
                DestroyWindow(window);
            }
        }
        Ok(None) => {}
        Err(error) => show_error(&error.to_string()),
    }
}
