//! macOS implementation of system-wide input simulation.
//!
//! Pending a real `CGEvent`-based implementation (`CGEventCreateKeyboardEvent` /
//! `CGEventCreateMouseEvent` posted to `kCGHIDEventTap`). Compiles cleanly and
//! returns [`crate::error::RdesktopError::UnsupportedPlatform`] so the framework
//! stays cross-platform; the real backend must be filled in and verified on
//! macOS.

use crate::error::{RdesktopError, Result};
use crate::hotkeys::Key;
use crate::input::MouseButton;

pub struct MacSim;

impl MacSim {
    pub fn new() -> Result<Self> {
        Err(RdesktopError::UnsupportedPlatform(
            "macOS input simulation requires a CGEvent-based implementation (pending real macOS build)".into(),
        ))
    }

    fn unsupported<T>() -> Result<T> {
        Err(RdesktopError::UnsupportedPlatform(
            "macOS input simulation is not yet implemented (CGEvent pending real macOS build)".into(),
        ))
    }

    pub fn press_key(&self, _key: Key) -> Result<()> {
        Self::unsupported()
    }
    pub fn release_key(&self, _key: Key) -> Result<()> {
        Self::unsupported()
    }
    pub fn tap_combo(&self, _keys: &[Key]) -> Result<()> {
        Self::unsupported()
    }
    pub fn type_text(&self, _text: &str) -> Result<()> {
        Self::unsupported()
    }
    pub fn move_mouse(&self, _x: i32, _y: i32, _absolute: bool) -> Result<()> {
        Self::unsupported()
    }
    pub fn press_mouse(&self, _button: MouseButton) -> Result<()> {
        Self::unsupported()
    }
    pub fn release_mouse(&self, _button: MouseButton) -> Result<()> {
        Self::unsupported()
    }
    pub fn scroll(&self, _delta: i32) -> Result<()> {
        Self::unsupported()
    }
}
