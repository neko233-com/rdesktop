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
    let fit_to_work_area = config.kind == WindowKind::Normal;

    #[cfg(target_os = "windows")]
    windows::apply(window, click_through, is_wallpaper, fit_to_work_area);
    #[cfg(target_os = "macos")]
    macos::apply(window, click_through, is_wallpaper);
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (window, click_through, is_wallpaper, fit_to_work_area);
        tracing::debug!("click-through / wallpaper layer not implemented on this platform");
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::{mem::size_of, ptr};

    use tao::platform::windows::WindowExtWindows;
    use tao::window::Window;
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, FindWindowExW, FindWindowW, GetWindowLongPtrW, GetWindowRect,
        SendMessageTimeoutW, SetParent, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, HWND_BOTTOM,
        SMTO_ABORTIFHUNG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
        WS_EX_LAYERED, WS_EX_TRANSPARENT,
    };

    pub(crate) fn apply(
        window: &Window,
        click_through: bool,
        is_wallpaper: bool,
        fit_to_work_area: bool,
    ) {
        let hwnd = window.hwnd() as HWND;
        if hwnd == 0 {
            return;
        }
        unsafe {
            if fit_to_work_area {
                center_and_fit_to_work_area(hwnd);
            }
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

    fn fitted_window_rect(window: RECT, work: RECT) -> RECT {
        let work_width = (work.right - work.left).max(1);
        let work_height = (work.bottom - work.top).max(1);
        let width = (window.right - window.left).clamp(1, work_width);
        let height = (window.bottom - window.top).clamp(1, work_height);
        let left = work.left + (work_width - width) / 2;
        let top = work.top + (work_height - height) / 2;
        RECT {
            left,
            top,
            right: left + width,
            bottom: top + height,
        }
    }

    unsafe fn center_and_fit_to_work_area(hwnd: HWND) {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if monitor == 0 {
            return;
        }
        let mut monitor_info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            rcMonitor: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            rcWork: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            dwFlags: 0,
        };
        let mut window_rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetMonitorInfoW(monitor, &mut monitor_info) == 0
            || GetWindowRect(hwnd, &mut window_rect) == 0
        {
            return;
        }
        let fitted = fitted_window_rect(window_rect, monitor_info.rcWork);
        SetWindowPos(
            hwnd,
            0,
            fitted.left,
            fitted.top,
            fitted.right - fitted.left,
            fitted.bottom - fitted.top,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER,
        );
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn centers_window_and_clamps_oversized_height_to_work_area() {
            let fitted = fitted_window_rect(
                RECT {
                    left: 120,
                    top: 120,
                    right: 2280,
                    bottom: 1560,
                },
                RECT {
                    left: 0,
                    top: 0,
                    right: 2560,
                    bottom: 1400,
                },
            );

            assert_eq!(
                (fitted.left, fitted.top, fitted.right, fitted.bottom),
                (200, 0, 2360, 1400)
            );
        }

        #[test]
        fn centers_smaller_window_without_resizing_it() {
            let fitted = fitted_window_rect(
                RECT {
                    left: 0,
                    top: 0,
                    right: 1455,
                    bottom: 957,
                },
                RECT {
                    left: 0,
                    top: 0,
                    right: 2560,
                    bottom: 1400,
                },
            );

            assert_eq!(
                (fitted.left, fitted.top, fitted.right, fitted.bottom),
                (552, 221, 2007, 1178)
            );
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
