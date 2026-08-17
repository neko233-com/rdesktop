//! Windows implementation of the global hotkey manager.
//!
//! Uses `RegisterHotKey` (the correct OS primitive for app-level global
//! shortcuts, distinct from low-level keyboard hooks) installed on a dedicated
//! thread that runs its own `GetMessageW` pump. The thread owns registration
//! so that `WM_HOTKEY` is delivered to it directly.

use super::hotkeys::{Hotkey, HotkeyHandler, Key, Modifiers};
use crate::error::{RdesktopError, Result};
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostQuitMessage, TranslateMessage, WM_HOTKEY,
};

// Virtual-key codes (integer literals; windows-sys 0.52 types VK_* as the
// `VIRTUAL_KEY` newtype, which we avoid here).
const VK_SPACE: u32 = 0x20;
const VK_RETURN: u32 = 0x0D;
const VK_ESCAPE: u32 = 0x1B;
const VK_TAB: u32 = 0x09;
const VK_BACK: u32 = 0x08;
const VK_DELETE: u32 = 0x2E;
const VK_UP: u32 = 0x26;
const VK_DOWN: u32 = 0x28;
const VK_LEFT: u32 = 0x25;
const VK_RIGHT: u32 = 0x27;
const VK_HOME: u32 = 0x24;
const VK_END: u32 = 0x23;
const VK_PRIOR: u32 = 0x21; // PageUp
const VK_NEXT: u32 = 0x22; // PageDown
const VK_INSERT: u32 = 0x2D;

// Hot-key modifier bit flags (windows-sys `HOT_KEY_MODIFIERS` is a u32 newtype).
const MOD_ALT: u32 = 0x0001;
const MOD_CONTROL: u32 = 0x0002;
const MOD_SHIFT: u32 = 0x0004;
const MOD_WIN: u32 = 0x0008;

pub struct WinHotkey {
    tx: mpsc::Sender<Cmd>,
    thread: Option<thread::JoinHandle<()>>,
}

enum Cmd {
    Register(u32, Hotkey),
    Unregister(u32),
    Stop,
}

/// Map a platform-independent [`Key`] to a Windows virtual-key code.
pub fn key_to_vk(key: Key) -> u32 {
    match key {
        Key::Letter(c) => c.to_ascii_uppercase() as u32, // VK_A..=VK_Z
        Key::Digit(d) => 0x30 + d as u32,                // VK_0..=VK_9
        Key::F(n) => 0x70 + (n as u32 - 1),              // VK_F1..=VK_F24
        Key::Space => VK_SPACE,
        Key::Enter => VK_RETURN,
        Key::Escape => VK_ESCAPE,
        Key::Tab => VK_TAB,
        Key::Backspace => VK_BACK,
        Key::Delete => VK_DELETE,
        Key::ArrowUp => VK_UP,
        Key::ArrowDown => VK_DOWN,
        Key::ArrowLeft => VK_LEFT,
        Key::ArrowRight => VK_RIGHT,
        Key::Home => VK_HOME,
        Key::End => VK_END,
        Key::PageUp => VK_PRIOR,
        Key::PageDown => VK_NEXT,
        Key::Insert => VK_INSERT,
    }
}

pub fn mods_to_raw(m: Modifiers) -> u32 {
    let mut r = 0u32;
    if m.ctrl {
        r |= MOD_CONTROL;
    }
    if m.alt {
        r |= MOD_ALT;
    }
    if m.shift {
        r |= MOD_SHIFT;
    }
    if m.meta {
        r |= MOD_WIN;
    }
    r
}

impl WinHotkey {
    pub fn start(handler: Arc<dyn HotkeyHandler>) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<Cmd>();
        let registry: Arc<Mutex<HashMap<u32, Hotkey>>> = Arc::new(Mutex::new(HashMap::new()));
        let registry_th = registry.clone();

        let thread = thread::spawn(move || {
            let registry = registry_th;
            loop {
                // Drain any pending registration commands without blocking.
                while let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        Cmd::Register(id, hk) => {
                            let vk = key_to_vk(hk.key);
                            let mods = mods_to_raw(hk.modifiers);
                            unsafe {
                                RegisterHotKey(0, id as i32, mods, vk);
                            }
                            registry.lock().unwrap().insert(id, hk);
                        }
                        Cmd::Unregister(id) => {
                            unsafe {
                                UnregisterHotKey(0, id as i32);
                            }
                            registry.lock().unwrap().remove(&id);
                        }
                        Cmd::Stop => {
                            unsafe {
                                PostQuitMessage(0);
                            }
                            break;
                        }
                    }
                }

                let mut msg = unsafe { std::mem::zeroed::<MSG>() };
                // SAFETY: standard Windows message pump; `msg` is a valid out-param.
                let res = unsafe { GetMessageW(&mut msg, 0, 0, 0) };
                if res == 0 || res == -1 {
                    break; // WM_QUIT or error
                }
                if msg.message == WM_HOTKEY {
                    let id = msg.wParam as u32;
                    let hk = registry.lock().unwrap().get(&id).copied();
                    if let Some(hk) = hk {
                        handler.on_hotkey(id, &hk);
                    }
                }
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        });

        Ok(Self {
            tx,
            thread: Some(thread),
        })
    }

    pub fn register(&self, id: u32, hotkey: &Hotkey) -> Result<()> {
        self.tx
            .send(Cmd::Register(id, *hotkey))
            .map_err(|e| RdesktopError::GlobalInput(format!("hotkey channel: {e}")))
    }

    pub fn unregister(&self, id: u32) -> Result<()> {
        self.tx
            .send(Cmd::Unregister(id))
            .map_err(|e| RdesktopError::GlobalInput(format!("hotkey channel: {e}")))
    }

    pub fn stop(mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
