//! rdesktop Phase 2 example: global hotkeys & global input hooks.
//!
//! Demonstrates the framework's ability to capture keyboard / mouse events
//! system-wide (beyond what Tauri v2 exposes) and to register global
//! shortcuts that fire even when the window is unfocused.
//!
//! Events are surfaced to the frontend as unnamed `window.__RDESKTOP_PUSH__`
//! calls carrying `{ cmd: "rdesktop.globalHotkey" | "rdesktop.globalInput",
//! payload }`.
//!
//! Run with the default WebView backend:
//!   cargo run -p global_hotkey
//! Or with the Chrome/CDP backend:
//!   cargo run -p global_hotkey --no-default-features --features chrome
//!
//! When both features are enabled, the Chrome backend is used.

use rdesktop_core::config::{AppConfig, GlobalInputConfig, HotkeyConfig, WindowConfig};
use rdesktop_core::ipc::{IpcMessage, IpcResponse, FnIpcHandler};
use rdesktop_core::renderer::Renderer;

#[cfg(feature = "chrome")]
use rdesktop_cef::CefRenderer;
#[cfg(all(feature = "webview", not(feature = "chrome")))]
use rdesktop_webview::WebViewRenderer;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = AppConfig {
        identifier: "com.example.global_hotkey".to_string(),
        name: "rdesktop global input".to_string(),
        version: "0.1.0".to_string(),
        // Register global shortcuts. They fire regardless of focus.
        hotkeys: vec![
            HotkeyConfig {
                id: Some("toggle-overlay".to_string()),
                combo: "Ctrl+Shift+K".to_string(),
                title: Some("Toggle overlay".to_string()),
            },
            HotkeyConfig {
                id: Some("screenshot".to_string()),
                combo: "Alt+PrintScreen".to_string(),
                title: Some("Capture screenshot".to_string()),
            },
        ],
        // Capture keyboard + mouse system-wide. Mouse-move is off by default
        // to avoid flooding the frontend with move events.
        global_input: GlobalInputConfig {
            enabled: true,
            keyboard: true,
            mouse: true,
            mouse_move: false,
        },
        window: WindowConfig {
            title: "rdesktop — Global Hotkey & Input".to_string(),
            width: 900,
            height: 560,
            ..Default::default()
        },
        ..Default::default()
    };

    // The frontend also invokes backend commands; mirror them into the log.
    let handler = FnIpcHandler::new(|msg: IpcMessage| {
        IpcResponse {
            id: msg.id,
            success: true,
            data: serde_json::json!({ "echo": msg.cmd }),
        }
    });

    let frontend_html = include_str!("../frontend/index.html");

    #[cfg(feature = "chrome")]
    {
        let mut renderer = CefRenderer::new(&config)?;
        renderer.init()?;
        renderer.set_ipc_handler(Box::new(handler));
        let handle = renderer.create_window(&config.window)?;
        renderer.load_html(handle, frontend_html)?;
        Box::new(renderer).run()?;
    }

    #[cfg(all(feature = "webview", not(feature = "chrome")))]
    {
        let mut renderer = WebViewRenderer::new(&config)?;
        renderer.init()?;
        renderer.set_ipc_handler(Box::new(handler));
        let handle = renderer.create_window(&config.window)?;
        renderer.load_html(handle, frontend_html)?;
        Box::new(renderer).run()?;
    }

    Ok(())
}
