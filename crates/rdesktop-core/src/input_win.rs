//! Windows implementation of global input hooks.
//!
//! Installs `WH_KEYBOARD_LL` and `WH_MOUSE_LL` on a dedicated thread that runs
//! its own `GetMessageW` pump. Low-level hooks are delivered on the installing
//! thread and require that thread to pump messages, which is why a dedicated
//! thread is used rather than piggybacking on tao's loop.

use super::input::{GlobalInputEvent, GlobalInputHandler, KeyState, MouseButton};
use crate::error::{RdesktopError, Result};
use crate::hotkeys::{Key, Modifiers};
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::thread;
use windows_sys::Win32::Foundation::{LPARAM, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, PostThreadMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, MSG, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

// Virtual-key codes (integer literals; windows-sys 0.52 types VK_* as the
// `VIRTUAL_KEY` newtype, which we avoid here).
const VK_CONTROL: u32 = 0x11;
const VK_MENU: u32 = 0x12; // Alt
const VK_SHIFT: u32 = 0x10;
const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;

struct InputCtx {
    handler: Arc<dyn GlobalInputHandler>,
    include_mouse_move: bool,
}

// Single global context shared with the hook callbacks (one input manager per
// process is the expected usage; re-set is supported via the Mutex).
static INPUT_CTX: Mutex<Option<Arc<InputCtx>>> = Mutex::new(None);

pub struct WinInput {
    keyboard_hook: Option<HHOOK>,
    mouse_hook: Option<HHOOK>,
    thread: Option<thread::JoinHandle<()>>,
    thread_id: u32,
}

fn vk_to_key(vk: u32) -> Option<Key> {
    match vk {
        0x41..=0x5A => Some(Key::Letter((vk as u8 - 0x41 + b'a') as char)),
        0x30..=0x39 => Some(Key::Digit((vk as u8 - 0x30) as u8)),
        0x70..=0x87 => Some(Key::F((vk as u8 - 0x70 + 1) as u8)),
        0x20 => Some(Key::Space),
        0x0D => Some(Key::Enter),
        0x1B => Some(Key::Escape),
        0x09 => Some(Key::Tab),
        0x08 => Some(Key::Backspace),
        0x2E => Some(Key::Delete),
        0x26 => Some(Key::ArrowUp),
        0x28 => Some(Key::ArrowDown),
        0x25 => Some(Key::ArrowLeft),
        0x27 => Some(Key::ArrowRight),
        0x24 => Some(Key::Home),
        0x23 => Some(Key::End),
        0x21 => Some(Key::PageUp),
        0x22 => Some(Key::PageDown),
        0x2D => Some(Key::Insert),
        _ => None,
    }
}

fn current_modifiers() -> Modifiers {
    let down = |vk: u32| unsafe { GetAsyncKeyState(vk as i32) } < 0;
    Modifiers {
        ctrl: down(VK_CONTROL),
        alt: down(VK_MENU),
        shift: down(VK_SHIFT),
        meta: down(VK_LWIN) || down(VK_RWIN),
    }
}

unsafe extern "system" fn keyboard_proc(n_code: i32, w_param: WPARAM, l_param: LPARAM) -> LPARAM {
    if n_code >= 0 {
        let kb = *(l_param as *const KBDLLHOOKSTRUCT);
        let up = kb.flags & 0x80 != 0;
        if let Some(key) = vk_to_key(kb.vkCode) {
            if let Some(ctx) = INPUT_CTX.lock().unwrap().clone() {
                ctx.handler.on_event(GlobalInputEvent::Keyboard {
                    key,
                    state: if up { KeyState::Released } else { KeyState::Pressed },
                    modifiers: current_modifiers(),
                });
            }
        }
    }
    CallNextHookEx(0, n_code, w_param, l_param)
}

unsafe extern "system" fn mouse_proc(n_code: i32, w_param: WPARAM, l_param: LPARAM) -> LPARAM {
    if n_code >= 0 {
        let ms = *(l_param as *const MSLLHOOKSTRUCT);
        if let Some(ctx) = INPUT_CTX.lock().unwrap().clone() {
            let wp = w_param as u32;
            let pressed = |down_code: u32, up_code: u32, btn: MouseButton| {
                if wp == down_code || wp == up_code {
                    let state = if wp == down_code {
                        KeyState::Pressed
                    } else {
                        KeyState::Released
                    };
                    Some((btn, state))
                } else {
                    None
                }
            };
            let button = pressed(WM_LBUTTONDOWN, WM_LBUTTONUP, MouseButton::Left)
                .or_else(|| pressed(WM_RBUTTONDOWN, WM_RBUTTONUP, MouseButton::Right))
                .or_else(|| pressed(WM_MBUTTONDOWN, WM_MBUTTONUP, MouseButton::Middle))
                .or_else(|| {
                    if wp == WM_XBUTTONDOWN || wp == WM_XBUTTONUP {
                        let x = (ms.mouseData >> 16) & 0xFFFF;
                        let btn = if x == 0x0001 {
                            MouseButton::X1
                        } else {
                            MouseButton::X2
                        };
                        let state = if wp == WM_XBUTTONDOWN {
                            KeyState::Pressed
                        } else {
                            KeyState::Released
                        };
                        Some((btn, state))
                    } else {
                        None
                    }
                });
            if let Some((button, state)) = button {
                ctx.handler.on_event(GlobalInputEvent::Mouse {
                    button,
                    state,
                    x: ms.pt.x,
                    y: ms.pt.y,
                });
            } else if wp == WM_MOUSEMOVE && ctx.include_mouse_move {
                ctx.handler.on_event(GlobalInputEvent::MouseMove {
                    x: ms.pt.x,
                    y: ms.pt.y,
                });
            } else if wp == WM_MOUSEWHEEL {
                let _delta = (ms.mouseData >> 16) as i16;
                // Wheel is captured but not surfaced as a first-class event yet.
            }
        }
    }
    CallNextHookEx(0, n_code, w_param, l_param)
}

impl WinInput {
    pub fn start(handler: Arc<dyn GlobalInputHandler>, include_mouse_move: bool) -> Result<Self> {
        *INPUT_CTX.lock().unwrap() = Some(Arc::new(InputCtx {
            handler,
            include_mouse_move,
        }));

        let keyboard_hook =
            unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), 0, 0) };
        let mouse_hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), 0, 0) };

        if keyboard_hook == 0 && mouse_hook == 0 {
            return Err(RdesktopError::GlobalInput(
                "failed to install global input hooks (SetWindowsHookExW)".into(),
            ));
        }

        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (id_tx, id_rx) = mpsc::channel::<u32>();
        let thread = thread::spawn(move || {
            let tid = unsafe { GetCurrentThreadId() };
            let _ = id_tx.send(tid);
            let mut msg = unsafe { std::mem::zeroed::<MSG>() };
            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                let res = unsafe { GetMessageW(&mut msg, 0, 0, 0) };
                if res == 0 || res == -1 {
                    break; // WM_QUIT / error
                }
                // Low-level hooks are dispatched inside the proc via
                // CallNextHookEx; nothing else to do here.
            }
            let _ = stop_tx;
        });
        let thread_id = id_rx.recv().unwrap_or(0);

        Ok(Self {
            keyboard_hook: if keyboard_hook != 0 { Some(keyboard_hook) } else { None },
            mouse_hook: if mouse_hook != 0 { Some(mouse_hook) } else { None },
            thread: Some(thread),
            thread_id,
        })
    }

    pub fn stop(mut self) {
        if let Some(h) = self.keyboard_hook.take() {
            unsafe {
                UnhookWindowsHookEx(h);
            }
        }
        if let Some(h) = self.mouse_hook.take() {
            unsafe {
                UnhookWindowsHookEx(h);
            }
        }
        if self.thread_id != 0 {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
            }
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        *INPUT_CTX.lock().unwrap() = None;
    }
}
