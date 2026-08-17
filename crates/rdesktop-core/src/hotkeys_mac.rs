//! macOS implementation of the global hotkey manager.
//!
//! NOTE: shipped as a compile-clean stub. The production implementation uses a
//! `CGEventTap` on `kCGHIDEventTap` filtering `kCGEventKeyDown`, recognising the
//! registered combo and returning `NULL` from the callback to swallow the event
//! so it never reaches other apps. It must be installed on the main-thread run
//! loop (tao owns it). Deferred until a macOS toolchain is available to verify
//! the `core-graphics` FFI.

use super::hotkeys::{Hotkey, HotkeyHandler};
use crate::error::{RdesktopError, Result};
use std::sync::Arc;

pub struct MacHotkey {
    _handler: Arc<dyn HotkeyHandler>,
}

impl MacHotkey {
    pub fn start(handler: Arc<dyn HotkeyHandler>) -> Result<Self> {
        let _ = handler;
        Err(RdesktopError::UnsupportedPlatform(
            "global hotkeys on macOS require a CGEventTap implementation (pending macOS build)".into(),
        ))
    }

    pub fn register(&self, _id: u32, _hotkey: &Hotkey) -> Result<()> {
        Ok(())
    }

    pub fn unregister(&self, _id: u32) -> Result<()> {
        Ok(())
    }

    pub fn stop(self) {}
}
