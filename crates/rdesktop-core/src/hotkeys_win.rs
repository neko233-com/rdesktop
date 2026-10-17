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
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WM_HOTKEY,
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
    Register(
        u32,
        Hotkey,
        mpsc::SyncSender<std::result::Result<(), String>>,
    ),
    Unregister(u32, mpsc::SyncSender<std::result::Result<(), String>>),
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
        let thread = thread::Builder::new()
            .name("rdesktop-hotkeys".into())
            .spawn(move || {
                let mut registry = HashMap::<u32, Hotkey>::new();
                let mut pressed = HashMap::<u32, Hotkey>::new();
                loop {
                    // A bounded receive keeps registration, release detection, and
                    // shutdown responsive even when Windows has no messages.
                    let command = match rx.recv_timeout(Duration::from_millis(8)) {
                        Ok(command) => Some(command),
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                        Err(mpsc::RecvTimeoutError::Disconnected) => Some(Cmd::Stop),
                    };
                    match command {
                        Some(Cmd::Register(id, hk, reply)) => {
                            let valid = (1..=0xBFFF).contains(&id);
                            let registered = valid
                                && unsafe {
                                    RegisterHotKey(
                                        0,
                                        id as i32,
                                        mods_to_raw(hk.modifiers) | 0x4000,
                                        key_to_vk(hk.key),
                                    )
                                } != 0;
                            let result = if registered {
                                registry.insert(id, hk);
                                Ok(())
                            } else {
                                Err(format!(
                                    "Could not register hotkey {}: {}",
                                    hk,
                                    std::io::Error::last_os_error()
                                ))
                            };
                            let _ = reply.send(result);
                        }
                        Some(Cmd::Unregister(id, reply)) => {
                            let result = if registry.contains_key(&id) {
                                if unsafe { UnregisterHotKey(0, id as i32) } != 0 {
                                    registry.remove(&id);
                                    pressed.remove(&id);
                                    Ok(())
                                } else {
                                    Err(std::io::Error::last_os_error().to_string())
                                }
                            } else {
                                Ok(())
                            };
                            let _ = reply.send(result);
                        }
                        Some(Cmd::Stop) => {
                            for id in registry.keys() {
                                unsafe {
                                    UnregisterHotKey(0, *id as i32);
                                }
                            }
                            break;
                        }
                        None => {}
                    }
                    let mut msg = unsafe { std::mem::zeroed::<MSG>() };
                    while unsafe { PeekMessageW(&mut msg, 0, 0, 0, PM_REMOVE) } != 0 {
                        if msg.message == WM_HOTKEY {
                            let id = msg.wParam as u32;
                            if let Some(hk) = registry.get(&id).copied() {
                                if pressed.insert(id, hk).is_none() {
                                    handler.on_hotkey(id, &hk);
                                }
                            }
                        }
                        unsafe {
                            TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                    pressed.retain(|id, hk| {
                        let down = |vk| unsafe { GetAsyncKeyState(vk) } < 0;
                        let held = down(key_to_vk(hk.key) as i32)
                            && (!hk.modifiers.ctrl || down(0x11))
                            && (!hk.modifiers.alt || down(0x12))
                            && (!hk.modifiers.shift || down(0x10))
                            && (!hk.modifiers.meta || down(0x5b) || down(0x5c));
                        if !held {
                            handler.on_hotkey_released(*id, hk);
                        }
                        held
                    });
                }
            })
            .map_err(|e| RdesktopError::GlobalInput(e.to_string()))?;
        Ok(Self {
            tx,
            thread: Some(thread),
        })
    }
    pub fn register(&self, id: u32, hotkey: &Hotkey) -> Result<()> {
        let (reply, receive) = mpsc::sync_channel(1);
        self.tx
            .send(Cmd::Register(id, *hotkey, reply))
            .map_err(|e| RdesktopError::GlobalInput(e.to_string()))?;
        receive
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| RdesktopError::GlobalInput(e.to_string()))?
            .map_err(RdesktopError::GlobalInput)
    }
    pub fn unregister(&self, id: u32) -> Result<()> {
        let (reply, receive) = mpsc::sync_channel(1);
        self.tx
            .send(Cmd::Unregister(id, reply))
            .map_err(|e| RdesktopError::GlobalInput(e.to_string()))?;
        receive
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| RdesktopError::GlobalInput(e.to_string()))?
            .map_err(RdesktopError::GlobalInput)
    }
    pub fn stop(mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Handler;
    impl HotkeyHandler for Handler {
        fn on_hotkey(&self, _: u32, _: &Hotkey) {}
    }
    #[test]
    fn reports_conflicts_and_releases_ownership_on_unregister() {
        let first = WinHotkey::start(Arc::new(Handler)).unwrap();
        let second = WinHotkey::start(Arc::new(Handler)).unwrap();
        let hotkey = (13..=24)
            .map(|number| Hotkey {
                modifiers: Modifiers {
                    ctrl: true,
                    alt: true,
                    shift: true,
                    meta: false,
                },
                key: Key::F(number),
            })
            .find(|key| first.register(0x2340, key).is_ok())
            .expect("available native test hotkey");
        assert!(second.register(0x2341, &hotkey).is_err());
        first.unregister(0x2340).unwrap();
        second.register(0x2341, &hotkey).unwrap();
        second.unregister(0x2341).unwrap();
        first.stop();
        second.stop();
    }
}
