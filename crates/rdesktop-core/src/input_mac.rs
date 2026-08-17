//! macOS implementation of global input hooks.
//!
//! NOTE: shipped as a compile-clean stub. The production implementation uses a
//! `CGEventTap` on `kCGHIDEventTap` covering `kCGEventKeyDown`, `kCGEventKeyUp`,
//! `kCGEventMouseDown`, `kCGEventMouseUp`, and `kCGEventMouseMoved`, forwarding
//! each event to the [`GlobalInputHandler`]. It must run on the main-thread run
//! loop. Deferred until a macOS toolchain is available to verify the
//! `core-graphics` FFI.

use super::input::GlobalInputHandler;
use crate::error::{RdesktopError, Result};
use std::sync::Arc;

pub struct MacInput {
    _handler: Arc<dyn GlobalInputHandler>,
}

impl MacInput {
    pub fn start(handler: Arc<dyn GlobalInputHandler>, _include_mouse_move: bool) -> Result<Self> {
        let _ = handler;
        Err(RdesktopError::UnsupportedPlatform(
            "global input hooks on macOS require a CGEventTap implementation (pending macOS build)"
                .into(),
        ))
    }

    pub fn stop(self) {}
}
