use serde::{Deserialize, Serialize};

/// Events that can occur in the application lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    /// Application is about to exit
    Exit,

    /// Window was closed
    WindowClosed { window_id: u64 },

    /// Window was resized
    WindowResized {
        window_id: u64,
        width: u32,
        height: u32,
    },

    /// Window was moved
    WindowMoved { window_id: u64, x: i32, y: i32 },

    /// Window gained focus
    WindowFocused { window_id: u64 },

    /// Window lost focus
    WindowUnfocused { window_id: u64 },

    /// File was dropped on the window
    FileDrop { window_id: u64, paths: Vec<String> },

    /// Custom event from the renderer
    Custom {
        name: String,
        data: serde_json::Value,
    },
}
