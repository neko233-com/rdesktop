//! Synchronous native resize hit testing above the WebView child window.
//!
//! WebView pointer events reach Rust asynchronously. For a quick drag, the
//! cursor may already be at its destination when a JavaScript resize request
//! starts. This hollow child window lets Windows handle the original press.

use std::{cell::Cell, io, ptr, sync::OnceLock};
use tao::{platform::windows::WindowExtWindows, window::Window};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CombineRgn, CreateRectRgn, DeleteObject, EndPaint, SetWindowRgn, PAINTSTRUCT,
        RGN_DIFF,
    },
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

const CLASS_NAME: windows_sys::core::PCWSTR = windows_sys::core::w!("RdesktopResizeBorder");

fn edge_at(x: i32, y: i32, width: i32, height: i32, border: i32, corner: i32) -> u32 {
    if x < 0 || y < 0 || x >= width || y >= height {
        return HTNOWHERE;
    }
    if y < border {
        if x < corner {
            HTTOPLEFT
        } else if x >= width - corner {
            HTTOPRIGHT
        } else {
            HTTOP
        }
    } else if y >= height - border {
        if x < corner {
            HTBOTTOMLEFT
        } else if x >= width - corner {
            HTBOTTOMRIGHT
        } else {
            HTBOTTOM
        }
    } else if x < border {
        if y < corner {
            HTTOPLEFT
        } else if y >= height - corner {
            HTBOTTOMLEFT
        } else {
            HTLEFT
        }
    } else if x >= width - border {
        if y < corner {
            HTTOPRIGHT
        } else if y >= height - corner {
            HTBOTTOMRIGHT
        } else {
            HTRIGHT
        }
    } else {
        HTNOWHERE
    }
}

unsafe extern "system" fn border_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => {
            let mut rect = RECT::default();
            if unsafe { GetWindowRect(hwnd, &mut rect) } != 0 {
                let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
                let x = (lparam & 0xffff) as u16 as i16 as i32 - rect.left;
                let y = ((lparam >> 16) & 0xffff) as u16 as i16 as i32 - rect.top;
                return edge_at(
                    x,
                    y,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    (4 * dpi / 96) as i32,
                    (8 * dpi / 96) as i32,
                ) as LRESULT;
            }
        }
        WM_NCLBUTTONDOWN => {
            let parent = unsafe { GetParent(hwnd) };
            if !parent.is_null() {
                // Forward the real event before later mouse moves are dispatched.
                // lParam contains signed screen coordinates, not a POINT pointer.
                return unsafe { SendMessageW(parent, msg, wparam, lparam) };
            }
        }
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            unsafe {
                BeginPaint(hwnd, &mut paint);
                EndPaint(hwnd, &paint);
            }
            return 0;
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

pub(crate) struct NativeResizeBorder {
    hwnd: HWND,
    layout: Cell<Option<(u32, u32, u32, bool)>>,
}

fn client_insets(hwnd: HWND) -> io::Result<(i32, i32)> {
    let mut outer = RECT::default();
    let mut inner = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut outer) } == 0
        || unsafe { GetClientRect(hwnd, &mut inner) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok((
        (outer.right - outer.left - inner.right + inner.left).max(0),
        (outer.bottom - outer.top - inner.bottom + inner.top).max(0),
    ))
}

pub(crate) fn preserve_client_size(window: &Window, width: u32, height: u32) -> io::Result<()> {
    let hwnd = window.hwnd() as HWND;
    let (horizontal, vertical) = client_insets(hwnd)?;
    let width = i32::try_from(width)
        .ok()
        .and_then(|v| v.checked_add(horizontal));
    let height = i32::try_from(height)
        .ok()
        .and_then(|v| v.checked_add(vertical));
    let (Some(width), Some(height)) = (width, height) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Window dimensions exceed native bounds",
        ));
    };
    if unsafe {
        SetWindowPos(
            hwnd,
            ptr::null_mut(),
            0,
            0,
            width,
            height,
            SWP_NOMOVE | SWP_NOACTIVATE | SWP_NOZORDER,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

unsafe extern "system" fn frame_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    child: usize,
) -> LRESULT {
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    if msg == WM_NCDESTROY {
        unsafe {
            RemoveWindowSubclass(hwnd, Some(frame_proc), id);
        }
    }
    let result = unsafe { DefSubclassProc(hwnd, msg, wp, lp) };
    if msg == WM_GETMINMAXINFO && lp != 0 && unsafe { IsWindowVisible(child as HWND) } != 0 {
        // tao 0.36 treats undecorated constraints as outer sizes, but Windows
        // still reserves shadow/frame insets. Keep the requested CLIENT size.
        if let Ok((horizontal, vertical)) = client_insets(hwnd) {
            let flags = unsafe { GetWindowLongPtrW(child as HWND, GWLP_USERDATA) };
            let limits = unsafe { &mut *(lp as *mut MINMAXINFO) };
            if flags & 1 != 0 {
                limits.ptMinTrackSize.x = limits.ptMinTrackSize.x.saturating_add(horizontal);
                limits.ptMinTrackSize.y = limits.ptMinTrackSize.y.saturating_add(vertical);
            }
            if flags & 2 != 0 {
                limits.ptMaxTrackSize.x = limits.ptMaxTrackSize.x.saturating_add(horizontal);
                limits.ptMaxTrackSize.y = limits.ptMaxTrackSize.y.saturating_add(vertical);
            }
        }
    }
    result
}

impl NativeResizeBorder {
    pub(crate) fn new(window: &Window, has_min: bool, has_max: bool) -> io::Result<Self> {
        static REGISTERED: OnceLock<Result<(), i32>> = OnceLock::new();
        let registered = REGISTERED.get_or_init(|| {
            let class = WNDCLASSW {
                lpfnWndProc: Some(border_proc),
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            if unsafe { RegisterClassW(&class) } == 0 {
                Err(io::Error::last_os_error().raw_os_error().unwrap_or(0))
            } else {
                Ok(())
            }
        });
        if let Err(code) = registered {
            return Err(io::Error::from_raw_os_error(*code));
        }
        // The parent owns this child and destroys it with the native window.
        // No borrowed Rust data is stored in the window procedure.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                CLASS_NAME,
                ptr::null(),
                WS_CHILD,
                0,
                0,
                0,
                0,
                window.hwnd() as HWND,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(io::Error::last_os_error());
        }
        unsafe {
            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                // windows-sys aliases this API to SetWindowLongW on x86.
                // The flags fit both its i32 value and the x64 isize value.
                (u8::from(has_min) | (u8::from(has_max) << 1)) as _,
            );
            if windows_sys::Win32::UI::Shell::SetWindowSubclass(
                window.hwnd() as HWND,
                Some(frame_proc),
                233,
                hwnd as usize,
            ) == 0
            {
                let error = io::Error::last_os_error();
                DestroyWindow(hwnd);
                return Err(error);
            }
        }
        Ok(Self {
            hwnd,
            layout: Cell::new(None),
        })
    }

    pub(crate) fn update(&self, window: &Window) -> io::Result<()> {
        let size = window.inner_size();
        let dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96);
        let enabled = !window.is_decorated()
            && window.is_resizable()
            && !window.is_minimized()
            && !window.is_maximized()
            && window.fullscreen().is_none();
        let next = (size.width, size.height, dpi, enabled);
        if self.layout.get() == Some(next) {
            return Ok(());
        }
        if !enabled {
            unsafe {
                ShowWindow(self.hwnd, SW_HIDE);
            }
            self.layout.set(Some(next));
            return Ok(());
        }
        let (width, height, border) =
            (size.width as i32, size.height as i32, (4 * dpi / 96) as i32);
        unsafe {
            let outer = CreateRectRgn(0, 0, width, height);
            let inner = CreateRectRgn(border, border, width - border, height - border);
            if outer.is_null() || inner.is_null() {
                let error = io::Error::last_os_error();
                if !outer.is_null() {
                    DeleteObject(outer);
                }
                if !inner.is_null() {
                    DeleteObject(inner);
                }
                return Err(error);
            }
            let combined = CombineRgn(outer, outer, inner, RGN_DIFF);
            DeleteObject(inner);
            if combined == 0 || SetWindowRgn(self.hwnd, outer, 0) == 0 {
                let error = io::Error::last_os_error();
                DeleteObject(outer);
                return Err(error);
            }
            // Windows owns outer after SetWindowRgn succeeds.
            if SetWindowPos(
                self.hwnd,
                HWND_TOP,
                0,
                0,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        self.layout.set(Some(next));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_and_corners_exclude_content_and_outside_points() {
        let hit = |x, y| edge_at(x, y, 1280, 720, 4, 8);
        assert_eq!(
            [
                hit(3, 3),
                hit(640, 3),
                hit(1277, 3),
                hit(3, 360),
                hit(1277, 360),
                hit(3, 717),
                hit(640, 717),
                hit(1277, 717)
            ],
            [
                HTTOPLEFT,
                HTTOP,
                HTTOPRIGHT,
                HTLEFT,
                HTRIGHT,
                HTBOTTOMLEFT,
                HTBOTTOM,
                HTBOTTOMRIGHT
            ]
        );
        for point in [(-1, 0), (1280, 0), (0, 720), (4, 4), (640, 360)] {
            assert_eq!(hit(point.0, point.1), HTNOWHERE);
        }
        assert_eq!(hit(2, 7), HTTOPLEFT);
        assert_eq!(hit(2, 8), HTLEFT);
    }
}
