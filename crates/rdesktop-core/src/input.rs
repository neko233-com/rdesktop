//! Global input hooks — raw keyboard and mouse events captured system-wide,
//! regardless of which window is focused.
//!
//! This is the foundation for "keyboard driver / mouse driver" scenarios
//! (e.g. a Logitech G-Hub-style remapper): rdesktop owns the low-level hook and
//! forwards every event to a [`GlobalInputHandler`], where application code can
//! observe, transform, or suppress it.
//!
//! - **Windows**: `SetWindowsHookExW` with `WH_KEYBOARD_LL` + `WH_MOUSE_LL` on a
//!   dedicated message-pump thread.
//! - **macOS**: a `CGEventTap` on `kCGHIDEventTap` (see `input_mac`).

use crate::error::Result;
use crate::hotkeys::{Key, Modifiers};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Pressed / released state for a key or mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyState {
    Pressed,
    Released,
}

/// Mouse buttons reported by the low-level hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    /// X1 (typically "back")
    X1,
    /// X2 (typically "forward")
    X2,
}

/// A single global input event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum GlobalInputEvent {
    Keyboard {
        key: Key,
        state: KeyState,
        modifiers: Modifiers,
    },
    Mouse {
        button: MouseButton,
        state: KeyState,
        x: i32,
        y: i32,
    },
    MouseMove {
        x: i32,
        y: i32,
    },
}

/// Callback invoked for every global input event.
pub trait GlobalInputHandler: Send + Sync {
    fn on_event(&self, event: GlobalInputEvent);
}

/// Platform-independent manager for global input hooks.
pub struct GlobalInput {
    handler: Arc<dyn GlobalInputHandler>,
    inner: Mutex<Option<PlatformInput>>,
    /// When false, mouse-move spam is suppressed (movement events are high-frequency).
    include_mouse_move: bool,
}

impl GlobalInput {
    pub fn new(handler: Arc<dyn GlobalInputHandler>) -> Self {
        Self {
            handler,
            inner: Mutex::new(None),
            include_mouse_move: false,
        }
    }

    /// Include `MouseMove` events (high frequency — off by default).
    pub fn with_mouse_move(mut self, on: bool) -> Self {
        self.include_mouse_move = on;
        self
    }

    /// Begin listening. Spawns the platform hook machinery.
    pub fn start(&self) -> Result<()> {
        let mut guard = self.inner.lock().unwrap();
        if guard.is_some() {
            return Ok(());
        }
        let p = PlatformInput::start(self.handler.clone(), self.include_mouse_move)?;
        *guard = Some(p);
        Ok(())
    }

    /// Stop listening (also happens on drop).
    pub fn stop(&self) {
        if let Some(p) = self.inner.lock().unwrap().take() {
            p.stop();
        }
    }
}

impl Drop for GlobalInput {
    fn drop(&mut self) {
        if let Some(p) = self.inner.lock().unwrap().take() {
            p.stop();
        }
    }
}

enum PlatformInput {
    #[cfg(windows)]
    Windows(crate::input_win::WinInput),
    #[cfg(target_os = "macos")]
    Macos(crate::input_mac::MacInput),
    #[cfg(not(any(windows, target_os = "macos")))]
    Unsupported,
}

impl PlatformInput {
    fn start(handler: Arc<dyn GlobalInputHandler>, include_mouse_move: bool) -> Result<Self> {
        #[cfg(windows)]
        {
            Ok(PlatformInput::Windows(crate::input_win::WinInput::start(
                handler,
                include_mouse_move,
            )?))
        }
        #[cfg(target_os = "macos")]
        {
            Ok(PlatformInput::Macos(crate::input_mac::MacInput::start(
                handler,
                include_mouse_move,
            )?))
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (handler, include_mouse_move);
            Err(crate::error::RdesktopError::UnsupportedPlatform(
                "global input hooks are only supported on Windows and macOS".into(),
            ))
        }
    }

    fn stop(self) {
        match self {
            #[cfg(windows)]
            PlatformInput::Windows(w) => w.stop(),
            #[cfg(target_os = "macos")]
            PlatformInput::Macos(m) => m.stop(),
            #[cfg(not(any(windows, target_os = "macos")))]
            PlatformInput::Unsupported => {}
        }
    }
}
