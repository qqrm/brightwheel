use std::ffi::OsStr;
use std::io;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIIF_LARGE_ICON, NIIF_USER,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, HICON, HMENU, LoadIconW, MF_CHECKED,
    MF_GRAYED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, PostMessageW, SetForegroundWindow,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_NULL,
};

use super::brightness_icon::BrightnessIcon;
use super::platform::wide;

pub(crate) const CALLBACK_MESSAGE: u32 = WM_APP + 1;

const ICON_ID: u32 = 1;
const APPLICATION_ICON_RESOURCE_ID: usize = 1;
const MENU_AUTOSTART: usize = 1001;
const MENU_EXIT: usize = 1002;
const MENU_UPDATE: usize = 1003;
const MENU_AUTO_UPDATE: usize = 1004;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MenuCommand {
    ToggleAutostart,
    ToggleAutomaticUpdates,
    CheckUpdates,
    Exit,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Status {
    pub(crate) brightness: Option<u32>,
    pub(crate) hdr: Option<bool>,
}

pub(crate) fn application_icon() -> HICON {
    // SAFETY: a null module name selects this executable, which embeds the
    // fixed five-dot application icon as resource 1. LoadIconW returns a shared
    // resource handle that remains valid for queued notifications.
    unsafe {
        LoadIconW(
            GetModuleHandleW(ptr::null()),
            APPLICATION_ICON_RESOURCE_ID as *const u16,
        )
    }
}

pub(crate) fn add(window: HWND, icon: HICON, status: Status) -> io::Result<()> {
    let dynamic_icon = BrightnessIcon::create(icon, status.brightness).ok();
    let mut data = data(window);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = CALLBACK_MESSAGE;
    data.hIcon = dynamic_icon.as_ref().map_or(icon, BrightnessIcon::handle);
    set_tooltip(&mut data.szTip, status);

    // SAFETY: `data` is initialized with its exact ABI size and valid handles.
    if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
        return Err(io::Error::last_os_error());
    }

    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    // SAFETY: the icon was added successfully and `data` identifies it.
    if unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) } == 0 {
        // SAFETY: best-effort cleanup of the icon identified by `data`.
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &data);
        }
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn update(window: HWND, icon: HICON, status: Status) {
    let dynamic_icon = BrightnessIcon::create(icon, status.brightness).ok();
    let mut data = data(window);
    data.uFlags = NIF_TIP | NIF_SHOWTIP;
    if let Some(icon) = &dynamic_icon {
        data.uFlags |= NIF_ICON;
        data.hIcon = icon.handle();
    }
    set_tooltip(&mut data.szTip, status);
    // SAFETY: `data` identifies the existing icon and contains a valid tooltip.
    unsafe {
        Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

pub(crate) fn notify(window: HWND, title: &str, message: &str) {
    let mut data = data(window);
    data.uFlags = NIF_INFO;
    data.hBalloonIcon = application_icon();
    data.dwInfoFlags = if data.hBalloonIcon.is_null() {
        NIIF_INFO
    } else {
        NIIF_USER | NIIF_LARGE_ICON
    };
    copy_wide(&mut data.szInfoTitle, title);
    copy_wide(&mut data.szInfo, message);
    // SAFETY: data contains valid fixed-length UTF-16 strings and identifies
    // the existing notification-area icon.
    unsafe {
        Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

pub(crate) fn remove(window: HWND) {
    let data = data(window);
    // SAFETY: `data` identifies the icon associated with this window.
    unsafe {
        Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

pub(crate) fn contains(window: HWND, point: POINT) -> bool {
    let identifier = NOTIFYICONIDENTIFIER {
        cbSize: mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: window,
        uID: ICON_ID,
        ..NOTIFYICONIDENTIFIER::default()
    };
    let mut rectangle = RECT::default();
    // SAFETY: the identifier and output rectangle are valid for the call.
    let result = unsafe { Shell_NotifyIconGetRect(&identifier, &mut rectangle) };
    result == 0
        && point.x >= rectangle.left
        && point.x < rectangle.right
        && point.y >= rectangle.top
        && point.y < rectangle.bottom
}

pub(crate) fn context_menu(
    window: HWND,
    autostart_enabled: bool,
    automatic_updates_enabled: bool,
    update_running: bool,
) -> io::Result<Option<MenuCommand>> {
    let menu = Menu::create()?;
    let check_flag = if autostart_enabled {
        MF_CHECKED
    } else {
        MF_UNCHECKED
    };
    menu.append(MF_STRING | check_flag, MENU_AUTOSTART, "Start with Windows")?;
    menu.append_separator()?;
    let check_flag = if automatic_updates_enabled {
        MF_CHECKED
    } else {
        MF_UNCHECKED
    };
    menu.append(
        MF_STRING | check_flag,
        MENU_AUTO_UPDATE,
        "Check for updates automatically",
    )?;
    if update_running {
        menu.append(
            MF_STRING | MF_GRAYED,
            MENU_UPDATE,
            "Checking for updates...",
        )?;
    } else {
        menu.append(MF_STRING, MENU_UPDATE, "Check for updates")?;
    }
    menu.append_separator()?;
    menu.append(MF_STRING, MENU_EXIT, "Exit")?;

    // SAFETY: `window` is the live message window that owns this menu.
    unsafe {
        SetForegroundWindow(window);
    }
    let mut point = POINT::default();
    // SAFETY: `point` is a writable output structure.
    if unsafe { GetCursorPos(&mut point) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the menu and owner window remain valid for this synchronous call.
    let command = unsafe {
        TrackPopupMenu(
            menu.0,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            point.x,
            point.y,
            0,
            window,
            ptr::null(),
        )
    } as usize;

    drop(menu);
    // SAFETY: posting `WM_NULL` is the documented way to finish tray menu
    // dismissal after calling `SetForegroundWindow`.
    unsafe {
        PostMessageW(window, WM_NULL, 0, 0);
    }

    Ok(match command {
        MENU_AUTOSTART => Some(MenuCommand::ToggleAutostart),
        MENU_AUTO_UPDATE => Some(MenuCommand::ToggleAutomaticUpdates),
        MENU_UPDATE => Some(MenuCommand::CheckUpdates),
        MENU_EXIT => Some(MenuCommand::Exit),
        _ => None,
    })
}

fn data(window: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: ICON_ID,
        ..NOTIFYICONDATAW::default()
    }
}

fn set_tooltip(destination: &mut [u16; 128], status: Status) {
    copy_wide(destination, &tooltip(status));
}

fn tooltip(status: Status) -> String {
    let brightness = status.brightness.map_or_else(
        || "Brightness unavailable".to_owned(),
        |brightness| format!("Brightness {brightness}%"),
    );
    let hdr = match status.hdr {
        Some(false) => "HDR Off",
        Some(true) => "HDR On",
        None => "HDR unavailable",
    };
    format!("{brightness} | {hdr} | Double-click: off | Middle-click: HDR")
}

fn copy_wide(destination: &mut [u16], value: &str) {
    destination.fill(0);
    let capacity = destination.len().saturating_sub(1);
    for (slot, character) in destination
        .iter_mut()
        .take(capacity)
        .zip(OsStr::new(value).encode_wide())
    {
        *slot = character;
    }
}

struct Menu(HMENU);

impl Menu {
    fn create() -> io::Result<Self> {
        // SAFETY: `CreatePopupMenu` takes no pointer arguments.
        let handle = unsafe { CreatePopupMenu() };
        if handle.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(handle))
        }
    }

    fn append(&self, flags: u32, id: usize, label: &str) -> io::Result<()> {
        let label = wide(label);
        // SAFETY: the menu is owned by this wrapper and `label` is
        // null-terminated for the duration of the call.
        if unsafe { AppendMenuW(self.0, flags, id, label.as_ptr()) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn append_separator(&self) -> io::Result<()> {
        // SAFETY: a separator ignores the identifier and label pointer.
        if unsafe { AppendMenuW(self.0, MF_SEPARATOR, 0, ptr::null()) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for Menu {
    fn drop(&mut self) {
        // SAFETY: this wrapper exclusively owns the menu handle.
        unsafe {
            DestroyMenu(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Status, copy_wide, tooltip};

    #[test]
    fn formats_complete_and_unavailable_statuses() {
        assert_eq!(
            tooltip(Status {
                brightness: Some(77),
                hdr: Some(true)
            }),
            "Brightness 77% | HDR On | Double-click: off | Middle-click: HDR"
        );
        assert_eq!(
            tooltip(Status::default()),
            "Brightness unavailable | HDR unavailable | Double-click: off | Middle-click: HDR"
        );
    }

    #[test]
    fn copies_utf16_with_room_for_a_terminal_null() {
        let mut destination = [99_u16; 5];
        copy_wide(&mut destination, "Bright");
        assert_eq!(destination, [66, 114, 105, 103, 0]);
    }
}
