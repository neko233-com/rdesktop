//! Native move/size requests for custom WebView title bars.

use std::io;
use tao::platform::windows::WindowExtWindows;
use tao::window::{ResizeDirection, Window};
use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetCapture, ReleaseCapture};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, PostMessageW, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTLEFT, HTRIGHT,
    HTTOP, HTTOPLEFT, HTTOPRIGHT, WM_NCLBUTTONDOWN,
};

fn hit_test(edge: Option<ResizeDirection>) -> u32 {
    match edge {
        None => HTCAPTION,
        Some(ResizeDirection::North) => HTTOP,
        Some(ResizeDirection::South) => HTBOTTOM,
        Some(ResizeDirection::West) => HTLEFT,
        Some(ResizeDirection::East) => HTRIGHT,
        Some(ResizeDirection::NorthWest) => HTTOPLEFT,
        Some(ResizeDirection::NorthEast) => HTTOPRIGHT,
        Some(ResizeDirection::SouthWest) => HTBOTTOMLEFT,
        Some(ResizeDirection::SouthEast) => HTBOTTOMRIGHT,
    }
}

fn cursor_lparam(x: i32, y: i32) -> isize {
    // WM_NCLBUTTONDOWN carries two signed 16-bit screen coordinates by value.
    // A POINTS pointer (as used by tao 0.36) is neither coordinates nor valid
    // storage after an asynchronous message returns. Preserve negative monitor positions.
    ((x as u16 as u32) | ((y as u16 as u32) << 16)) as usize as isize
}

pub(crate) fn start(window: &Window, edge: Option<ResizeDirection>) -> io::Result<()> {
    let hwnd = window.hwnd() as HWND;
    let mut cursor = POINT::default();
    // Called on the owning event-loop thread while the Window is alive.
    unsafe {
        if GetCursorPos(&mut cursor) == 0 {
            return Err(io::Error::last_os_error());
        }
        // No capture is already a valid state; do not turn it into a failure.
        if !GetCapture().is_null() && ReleaseCapture() == 0 {
            return Err(io::Error::last_os_error());
        }
        if PostMessageW(
            hwnd,
            WM_NCLBUTTONDOWN,
            hit_test(edge) as usize,
            cursor_lparam(cursor.x, cursor.y),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_round_trip_for_primary_and_negative_monitors() {
        for (x, y) in [(0, 0), (1919, 1079), (-1920, -1080), (-32768, 32767)] {
            let packed = cursor_lparam(x, y);
            assert_eq!((packed & 0xffff) as u16 as i16 as i32, x);
            assert_eq!(((packed >> 16) & 0xffff) as u16 as i16 as i32, y);
        }
    }

    #[test]
    fn all_edges_and_title_bar_have_distinct_native_hit_tests() {
        let edges = [
            None,
            Some(ResizeDirection::North),
            Some(ResizeDirection::South),
            Some(ResizeDirection::West),
            Some(ResizeDirection::East),
            Some(ResizeDirection::NorthWest),
            Some(ResizeDirection::NorthEast),
            Some(ResizeDirection::SouthWest),
            Some(ResizeDirection::SouthEast),
        ];
        let actual = edges.map(hit_test);
        assert_eq!(actual, [2, 12, 15, 10, 11, 13, 14, 16, 17]);
    }
}
