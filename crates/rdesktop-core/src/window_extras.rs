//! Platform-specific window attributes for deep desktop scenarios.
//!
//! Provides [`apply_window_attributes`], called right after a tao window is
//! built. It realizes the `Wallpaper`/`Overlay` window kinds and click-through
//! requested in [`crate::config::WindowConfig`].
//!
//! - **Click-through**: the window ignores pointer input, which falls through
//!   to whatever is behind it. Required for wallpaper; optional for overlays.
//! - **Desktop layer**: the window is reparented beneath the desktop icons
//!   (Windows: `WorkerW`; macOS: `kCGDesktopWindowLevel`) so it behaves like
//!   a Wallpaper-Engine-style background.

use crate::config::{WindowConfig, WindowKind};
use tao::window::{Icon, Window};

/// Convert a framework icon into the platform window icon type.
pub fn window_icon(config: &WindowConfig) -> Option<Icon> {
    let icon = config.icon.as_ref()?;
    match Icon::from_rgba(icon.rgba.clone(), icon.width, icon.height) {
        Ok(icon) => Some(icon),
        Err(error) => {
            tracing::warn!(%error, "invalid rdesktop window icon; continuing without icon");
            None
        }
    }
}

/// Apply `config.kind` / `config.click_through` to a freshly built window.
pub fn apply_window_attributes(window: &Window, config: &WindowConfig) {
    let click_through = config.click_through || config.kind == WindowKind::Wallpaper;
    let is_wallpaper = config.kind == WindowKind::Wallpaper;

    #[cfg(target_os = "windows")]
    windows::apply(window, click_through, is_wallpaper);
    #[cfg(target_os = "macos")]
    macos::apply(window, click_through, is_wallpaper);
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (window, click_through, is_wallpaper);
        tracing::debug!("click-through / wallpaper layer not implemented on this platform");
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::ptr;

    use tao::platform::windows::WindowExtWindows;
    use tao::window::Window;
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, FindWindowExW, FindWindowW, GetWindowLongPtrW, SendMessageTimeoutW, SetParent,
        SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, HWND_BOTTOM, SMTO_ABORTIFHUNG,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WS_EX_LAYERED, WS_EX_TRANSPARENT,
    };

    pub(crate) fn apply(window: &Window, click_through: bool, is_wallpaper: bool) {
        let hwnd = window.hwnd() as HWND;
        if hwnd == 0 {
            return;
        }
        unsafe {
            if click_through {
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                let new_ex = ex | (WS_EX_LAYERED as isize) | (WS_EX_TRANSPARENT as isize);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_ex);
            }
            if is_wallpaper {
                if let Some(worker) = find_desktop_workerw() {
                    SetParent(hwnd, worker);
                }
                SetWindowPos(
                    hwnd,
                    HWND_BOTTOM,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }

    /// Locate the desktop `WorkerW` window (the host behind the desktop icons)
    /// by asking Explorer to (re)create it and then finding the sibling
    /// `WorkerW` that does not own `SHELLDLL_DefView`.
    unsafe fn find_desktop_workerw() -> Option<HWND> {
        let progman = FindWindowW(windows_sys::w!("Progman"), ptr::null::<u16>());
        if progman == 0 {
            return None;
        }
        // Prompt Explorer to create a dedicated wallpaper WorkerW host.
        SendMessageTimeoutW(
            progman,
            0x52C,
            0,
            0,
            SMTO_ABORTIFHUNG,
            1000,
            ptr::null_mut::<usize>(),
        );
        let mut worker: HWND = 0;
        EnumWindows(Some(enum_proc), &mut worker as *mut _ as LPARAM);
        if worker == 0 {
            None
        } else {
            Some(worker)
        }
    }

    extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let defview = FindWindowExW(
                hwnd,
                0,
                windows_sys::w!("SHELLDLL_DefView"),
                ptr::null::<u16>(),
            );
            if defview != 0 {
                let pworker = &mut *(lparam as *mut HWND);
                *pworker = FindWindowExW(0, hwnd, windows_sys::w!("WorkerW"), ptr::null::<u16>());
            }
            windows_sys::Win32::Foundation::TRUE
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use core_graphics::window::{CGWindowLevelForKey, CGWindowLevelKey};
    use objc::{msg_send, runtime::Object};
    use tao::platform::macos::WindowExtMacOS;
    use tao::window::Window;

    pub(crate) fn apply(window: &Window, click_through: bool, is_wallpaper: bool) {
        let ns_win = window.ns_window() as *mut Object;
        if ns_win.is_null() {
            return;
        }
        unsafe {
            let _: () = msg_send![ns_win, setIgnoresMouseEvents: click_through || is_wallpaper];
            if is_wallpaper {
                let level: i64 =
                    CGWindowLevelForKey(CGWindowLevelKey::kCGDesktopWindowLevelKey) as i64;
                let _: () = msg_send![ns_win, setLevel: level];
                let _: () = msg_send![ns_win, orderBack: ns_win];
            }
        }
    }
}
