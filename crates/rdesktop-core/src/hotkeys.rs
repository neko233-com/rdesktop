//! Global hotkey manager — registers OS-level key combinations that fire even
//! when the application window is not focused.
//!
//! This goes beyond Tauri v2's `global-shortcut`, which only covers a fixed
//! subset. rdesktop owns the platform integration directly:
//!
//! - **Windows**: `RegisterHotKey` + a dedicated message-pump thread that
//!   receives `WM_HOTKEY` and dispatches to the handler.
//! - **macOS**: a `CGEventTap` on `kCGHIDEventTap` that recognises the combo
//!   and consumes the event so it never reaches other apps.
//!
//! Both paths feed the same [`HotkeyHandler`] callback, so application code is
//! platform-agnostic.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

/// Active modifier keys for a hotkey / input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Windows key (Win) on Windows, Command (⌘) on macOS.
    pub meta: bool,
}

impl Modifiers {
    pub fn is_empty(&self) -> bool {
        !(self.ctrl || self.alt || self.shift || self.meta)
    }

    /// Build from a Windows `MOD_*` bitmask.
    pub fn from_raw_win(m: u32) -> Self {
        Self {
            ctrl: m & 0x0002 != 0,
            alt: m & 0x0001 != 0,
            shift: m & 0x0004 != 0,
            meta: m & 0x0008 != 0,
        }
    }

    /// Build from a macOS `CGEventFlags` bitmask.
    pub fn from_raw_mac(f: u64) -> Self {
        const CMD: u64 = 0x100000; // kCGEventFlagMaskCommand
        const SHIFT: u64 = 0x20000; // kCGEventFlagMaskShift
        const ALT: u64 = 0x80000; // kCGEventFlagMaskAlternate
        const CTRL: u64 = 0x40000; // kCGEventFlagMaskControl
        Self {
            ctrl: f & CTRL != 0,
            alt: f & ALT != 0,
            shift: f & SHIFT != 0,
            meta: f & CMD != 0,
        }
    }
}

/// A platform-independent key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Key {
    Letter(char),
    Digit(u8),
    F(u8),
    Space,
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
}

impl Key {
    /// Parse a single key token (e.g. `"K"`, `"F5"`, `"Space"`, `"Up"`).
    pub fn from_token(tok: &str) -> Option<Key> {
        let t = tok.trim();
        if t.len() == 1 {
            let c = t.as_bytes()[0];
            if c.is_ascii_alphabetic() {
                return Some(Key::Letter(c.to_ascii_lowercase() as char));
            }
            if c.is_ascii_digit() {
                return Some(Key::Digit(c - b'0'));
            }
        }
        match t.to_ascii_lowercase().as_str() {
            "space" => Some(Key::Space),
            "enter" | "return" => Some(Key::Enter),
            "escape" | "esc" => Some(Key::Escape),
            "tab" => Some(Key::Tab),
            "backspace" | "back" => Some(Key::Backspace),
            "delete" | "del" => Some(Key::Delete),
            "up" | "arrowup" => Some(Key::ArrowUp),
            "down" | "arrowdown" => Some(Key::ArrowDown),
            "left" | "arrowleft" => Some(Key::ArrowLeft),
            "right" | "arrowright" => Some(Key::ArrowRight),
            "home" => Some(Key::Home),
            "end" => Some(Key::End),
            "pageup" | "prior" => Some(Key::PageUp),
            "pagedown" | "next" => Some(Key::PageDown),
            "insert" | "ins" => Some(Key::Insert),
            _ => {
                if t.len() > 1 && t.starts_with('f') {
                    if let Ok(n) = t[1..].parse::<u8>() {
                        if n >= 1 && n <= 24 {
                            return Some(Key::F(n));
                        }
                    }
                }
                None
            }
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Letter(c) => write!(f, "{}", c.to_ascii_uppercase()),
            Key::Digit(d) => write!(f, "{}", d),
            Key::F(n) => write!(f, "F{}", n),
            Key::Space => write!(f, "Space"),
            Key::Enter => write!(f, "Enter"),
            Key::Escape => write!(f, "Escape"),
            Key::Tab => write!(f, "Tab"),
            Key::Backspace => write!(f, "Backspace"),
            Key::Delete => write!(f, "Delete"),
            Key::ArrowUp => write!(f, "Up"),
            Key::ArrowDown => write!(f, "Down"),
            Key::ArrowLeft => write!(f, "Left"),
            Key::ArrowRight => write!(f, "Right"),
            Key::Home => write!(f, "Home"),
            Key::End => write!(f, "End"),
            Key::PageUp => write!(f, "PageUp"),
            Key::PageDown => write!(f, "PageDown"),
            Key::Insert => write!(f, "Insert"),
        }
    }
}

/// A global hotkey: a set of modifiers plus a primary key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    pub modifiers: Modifiers,
    pub key: Key,
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        if self.modifiers.ctrl {
            parts.push("Ctrl".into());
        }
        if self.modifiers.alt {
            parts.push("Alt".into());
        }
        if self.modifiers.shift {
            parts.push("Shift".into());
        }
        if self.modifiers.meta {
            parts.push("Meta".into());
        }
        parts.push(self.key.to_string());
        write!(f, "{}", parts.join("+"))
    }
}

impl FromStr for Hotkey {
    type Err = String;
    /// Parse `"Ctrl+Shift+K"`, `"Alt+F4"`, `"Meta+Space"`, etc.
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let mut modifiers = Modifiers::default();
        let mut key: Option<Key> = None;
        for tok in s.split('+') {
            let tok = tok.trim();
            if tok.is_empty() {
                continue;
            }
            match tok.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers.ctrl = true,
                "alt" | "option" => modifiers.alt = true,
                "shift" => modifiers.shift = true,
                "meta" | "win" | "cmd" | "super" | "command" => modifiers.meta = true,
                other => {
                    key = Some(Key::from_token(other).ok_or_else(|| format!("unknown key: {tok}"))?);
                }
            }
        }
        let key = key.ok_or_else(|| "hotkey requires a non-modifier key".to_string())?;
        Ok(Hotkey { modifiers, key })
    }
}

/// Callback invoked when a registered hotkey fires.
pub trait HotkeyHandler: Send + Sync {
    fn on_hotkey(&self, id: u32, hotkey: &Hotkey);
}

/// Platform-independent manager for global hotkeys.
pub struct HotkeyManager {
    handler: Arc<dyn HotkeyHandler>,
    inner: Mutex<Option<PlatformHotkey>>,
}

impl HotkeyManager {
    pub fn new(handler: Arc<dyn HotkeyHandler>) -> Self {
        Self {
            handler,
            inner: Mutex::new(None),
        }
    }

    /// Register a hotkey with the given application-defined `id`.
    pub fn register(&self, id: u32, hotkey: &Hotkey) -> Result<()> {
        let mut guard = self.inner.lock().unwrap();
        match guard.as_ref() {
            Some(p) => p.register(id, hotkey),
            None => {
                let p = PlatformHotkey::start(self.handler.clone())?;
                p.register(id, hotkey)?;
                *guard = Some(p);
                Ok(())
            }
        }
    }

    /// Unregister a previously registered hotkey by `id`.
    pub fn unregister(&self, id: u32) -> Result<()> {
        if let Some(p) = self.inner.lock().unwrap().as_ref() {
            p.unregister(id)?;
        }
        Ok(())
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        if let Some(p) = self.inner.lock().unwrap().take() {
            p.stop();
        }
    }
}

// ── Platform implementations ──────────────────────────────────────────────

enum PlatformHotkey {
    #[cfg(windows)]
    Windows(crate::hotkeys_win::WinHotkey),
    #[cfg(target_os = "macos")]
    Macos(crate::hotkeys_mac::MacHotkey),
    #[cfg(not(any(windows, target_os = "macos")))]
    Unsupported,
}

impl PlatformHotkey {
    fn start(handler: Arc<dyn HotkeyHandler>) -> Result<Self> {
        #[cfg(windows)]
        {
            Ok(PlatformHotkey::Windows(crate::hotkeys_win::WinHotkey::start(handler)?))
        }
        #[cfg(target_os = "macos")]
        {
            Ok(PlatformHotkey::Macos(crate::hotkeys_mac::MacHotkey::start(handler)?))
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = handler;
            Err(crate::error::RdesktopError::UnsupportedPlatform(
                "global hotkeys are only supported on Windows and macOS".into(),
            ))
        }
    }

    fn register(&self, id: u32, hotkey: &Hotkey) -> Result<()> {
        match self {
            #[cfg(windows)]
            PlatformHotkey::Windows(w) => w.register(id, hotkey),
            #[cfg(target_os = "macos")]
            PlatformHotkey::Macos(m) => m.register(id, hotkey),
            #[cfg(not(any(windows, target_os = "macos")))]
            PlatformHotkey::Unsupported => Ok(()),
        }
    }

    fn unregister(&self, id: u32) -> Result<()> {
        match self {
            #[cfg(windows)]
            PlatformHotkey::Windows(w) => w.unregister(id),
            #[cfg(target_os = "macos")]
            PlatformHotkey::Macos(m) => m.unregister(id),
            #[cfg(not(any(windows, target_os = "macos")))]
            PlatformHotkey::Unsupported => Ok(()),
        }
    }

    fn stop(self) {
        match self {
            #[cfg(windows)]
            PlatformHotkey::Windows(w) => w.stop(),
            #[cfg(target_os = "macos")]
            PlatformHotkey::Macos(m) => m.stop(),
            #[cfg(not(any(windows, target_os = "macos")))]
            PlatformHotkey::Unsupported => {}
        }
    }
}
