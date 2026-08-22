//! Windows implementation of system-wide input simulation via `SendInput`.
//!
//! `SendInput` injects a batch of synthetic keyboard/mouse events into the
//! global input stream. It is the correct primitive for a "keyboard/mouse
//! driver" (as opposed to the low-level `WH_*_LL` hooks used for *capture* in
//! `input_win`): events go through the same path as real hardware, so target
//! apps receive them normally.

use crate::error::Result;
use crate::hotkeys::Key;
use crate::hotkeys_win::key_to_vk;
use crate::input::MouseButton;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN,
    MOUSEEVENTF_XUP, MOUSEINPUT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, XBUTTON1, XBUTTON2,
};

pub struct WinSim;

impl WinSim {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    pub fn press_key(&self, key: Key) -> Result<()> {
        let vk = key_to_vk(key);
        let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) } as u16;
        let mut inputs = [kb(vk as u16, scan, 0)];
        unsafe { send(&mut inputs) };
        Ok(())
    }

    pub fn release_key(&self, key: Key) -> Result<()> {
        let vk = key_to_vk(key);
        let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) } as u16;
        let mut inputs = [kb(vk as u16, scan, KEYEVENTF_KEYUP)];
        unsafe { send(&mut inputs) };
        Ok(())
    }

    pub fn tap_combo(&self, keys: &[Key]) -> Result<()> {
        let mut inputs: Vec<INPUT> = Vec::with_capacity(keys.len() * 2);
        for &k in keys {
            let vk = key_to_vk(k);
            let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) } as u16;
            inputs.push(kb(vk as u16, scan, 0));
        }
        for &k in keys.iter().rev() {
            let vk = key_to_vk(k);
            let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) } as u16;
            inputs.push(kb(vk as u16, scan, KEYEVENTF_KEYUP));
        }
        unsafe { send(&mut inputs) };
        Ok(())
    }

    pub fn type_text(&self, text: &str) -> Result<()> {
        let mut inputs: Vec<INPUT> = Vec::with_capacity(text.chars().count() * 2);
        for ch in text.chars() {
            let u = ch as u16;
            inputs.push(kb(0, u, KEYEVENTF_UNICODE));
            inputs.push(kb(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
        unsafe { send(&mut inputs) };
        Ok(())
    }

    pub fn move_mouse(&self, x: i32, y: i32, absolute: bool) -> Result<()> {
        let (dx, dy, flags) = if absolute {
            let w = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
            let h = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
            let nx = if w > 1 {
                (x as i64 * 65535 / (w as i64 - 1)) as i32
            } else {
                x
            };
            let ny = if h > 1 {
                (y as i64 * 65535 / (h as i64 - 1)) as i32
            } else {
                y
            };
            (nx, ny, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE)
        } else {
            (x, y, MOUSEEVENTF_MOVE)
        };
        let mut inputs = [mouse(dx, dy, 0, flags)];
        unsafe { send(&mut inputs) };
        Ok(())
    }

    pub fn press_mouse(&self, button: MouseButton) -> Result<()> {
        let (flags, data) = button_flags(button, true);
        let mut inputs = [mouse(0, 0, data, flags)];
        unsafe { send(&mut inputs) };
        Ok(())
    }

    pub fn release_mouse(&self, button: MouseButton) -> Result<()> {
        let (flags, data) = button_flags(button, false);
        let mut inputs = [mouse(0, 0, data, flags)];
        unsafe { send(&mut inputs) };
        Ok(())
    }

    pub fn scroll(&self, delta: i32) -> Result<()> {
        // Wheel delta lives in the high word of `mouseData` (signed).
        let data = ((delta as i16) as u32) << 16;
        let mut inputs = [mouse(0, 0, data, MOUSEEVENTF_WHEEL)];
        unsafe { send(&mut inputs) };
        Ok(())
    }
}

fn button_flags(button: MouseButton, down: bool) -> (u32, u32) {
    match button {
        MouseButton::Left => (
            if down {
                MOUSEEVENTF_LEFTDOWN
            } else {
                MOUSEEVENTF_LEFTUP
            },
            0,
        ),
        MouseButton::Right => (
            if down {
                MOUSEEVENTF_RIGHTDOWN
            } else {
                MOUSEEVENTF_RIGHTUP
            },
            0,
        ),
        MouseButton::Middle => (
            if down {
                MOUSEEVENTF_MIDDLEDOWN
            } else {
                MOUSEEVENTF_MIDDLEUP
            },
            0,
        ),
        MouseButton::X1 => (
            if down {
                MOUSEEVENTF_XDOWN
            } else {
                MOUSEEVENTF_XUP
            },
            XBUTTON1 as u32,
        ),
        MouseButton::X2 => (
            if down {
                MOUSEEVENTF_XDOWN
            } else {
                MOUSEEVENTF_XUP
            },
            XBUTTON2 as u32,
        ),
    }
}

/// Build a keyboard `INPUT` (VK-based when `vk != 0`, Unicode scan otherwise).
fn kb(vk: u16, scan: u16, flags: u32) -> INPUT {
    let mut input = unsafe { std::mem::zeroed::<INPUT>() };
    input.r#type = INPUT_KEYBOARD;
    input.Anonymous.ki = KEYBDINPUT {
        wVk: vk,
        wScan: scan,
        dwFlags: flags,
        time: 0,
        dwExtraInfo: 0,
    };
    input
}

/// Build a mouse `INPUT`.
fn mouse(dx: i32, dy: i32, mouse_data: u32, flags: u32) -> INPUT {
    let mut input = unsafe { std::mem::zeroed::<INPUT>() };
    input.r#type = INPUT_MOUSE;
    input.Anonymous.mi = MOUSEINPUT {
        dx,
        dy,
        mouseData: mouse_data,
        dwFlags: flags,
        time: 0,
        dwExtraInfo: 0,
    };
    input
}

/// Flush a batch of synthetic events into the OS input stream.
///
/// `SendInput` is declared with a `*const INPUT` parameter; we pass a const
/// pointer. We don't read `time` back, so the pointer direction is irrelevant.
unsafe fn send(inputs: &mut [INPUT]) {
    if !inputs.is_empty() {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}
