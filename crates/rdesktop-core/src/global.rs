//! Bridge that forwards global hotkey / input events into the renderer's
//! outbox, so the frontend receives them as unnamed `window.__RDESKTOP_PUSH__`
//! messages (matching the IPC contract used by the Node extension host).
//!
//! `PushHandler` implements both [`HotkeyHandler`](crate::hotkeys::HotkeyHandler)
//! and [`GlobalInputHandler`](crate::input::GlobalInputHandler) and writes a
//! JSON envelope into a shared `outbox` queue drained every frame by the
//! backend event loop.

use crate::hotkeys::{Hotkey, HotkeyHandler};
use crate::input::{GlobalInputEvent, GlobalInputHandler};
use serde_json::json;
use std::sync::{Arc, Mutex};

/// Shared queue of JSON strings emitted to the frontend each frame.
pub type Outbox = Arc<Mutex<Vec<String>>>;

/// Forwards global events to the frontend via the renderer outbox.
pub struct PushHandler {
    outbox: Outbox,
}

impl PushHandler {
    pub fn new(outbox: Outbox) -> Arc<Self> {
        Arc::new(Self { outbox })
    }
}

impl HotkeyHandler for PushHandler {
    fn on_hotkey(&self, id: u32, hotkey: &Hotkey) {
        if let Ok(s) =
            serde_json::to_string(&json!({ "cmd": "rdesktop.globalHotkey", "payload": { "id": id, "combo": hotkey.to_string() } }))
        {
            self.outbox.lock().unwrap().push(s);
        }
    }
}

impl GlobalInputHandler for PushHandler {
    fn on_event(&self, event: GlobalInputEvent) {
        if let Ok(s) =
            serde_json::to_string(&json!({ "cmd": "rdesktop.globalInput", "payload": event }))
        {
            self.outbox.lock().unwrap().push(s);
        }
    }
}
