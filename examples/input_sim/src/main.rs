//! rdesktop Phase 3 example: system-wide input simulation ("keyboard/mouse driver").
//!
//! Demonstrates injecting keyboard and mouse input into the OS — the output
//! counterpart to Phase 2's global input capture. The frontend buttons call
//! backend IPC commands (`simulate.*`) that drive [`InputSimulator`].
//!
//! Run with the default WebView backend:
//!   cargo run -p input_sim
//! Or with the Chrome/CDP backend:
//!   cargo run -p input_sim --no-default-features --features chrome
//!
//! Tip: click another window (e.g. Notepad) before pressing the buttons so the
//! synthetic input lands somewhere visible.

use rdesktop_core::config::{AppConfig, WindowConfig};
use rdesktop_core::ipc::{FnIpcHandler, IpcMessage, IpcResponse};
use rdesktop_core::{InputSimulator, Key, MouseButton, Renderer};
use std::sync::Arc;

#[cfg(feature = "chrome")]
use rdesktop_cef::CefRenderer;
#[cfg(all(feature = "webview", not(feature = "chrome")))]
use rdesktop_webview::WebViewRenderer;

fn handle_sim(sim: &InputSimulator, msg: IpcMessage) -> IpcResponse {
    let ok = |data: serde_json::Value| IpcResponse {
        id: msg.id.clone(),
        success: true,
        data,
    };
    let err = |e: String| IpcResponse {
        id: msg.id.clone(),
        success: false,
        data: serde_json::json!({ "error": e }),
    };
    match msg.cmd.as_str() {
        "simulate.text" => {
            let text = msg
                .payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            match sim.type_text(&text) {
                Ok(()) => ok(serde_json::json!({ "typed": text.chars().count() })),
                Err(e) => err(e.to_string()),
            }
        }
        "simulate.key" => {
            match msg
                .payload
                .get("key")
                .and_then(|v| v.as_str())
                .and_then(Key::from_token)
            {
                Some(k) => match sim.tap_key(k) {
                    Ok(()) => ok(serde_json::json!({ "key": k.to_string() })),
                    Err(e) => err(e.to_string()),
                },
                None => err("invalid key token".into()),
            }
        }
        "simulate.combo" => {
            let keys: Vec<Key> = msg
                .payload
                .get("keys")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().and_then(Key::from_token))
                        .collect()
                })
                .unwrap_or_default();
            match sim.tap_combo(&keys) {
                Ok(()) => ok(serde_json::json!({ "keys": keys.len() })),
                Err(e) => err(e.to_string()),
            }
        }
        "simulate.click" => {
            let button = match msg.payload.get("button").and_then(|v| v.as_str()) {
                Some("right") => MouseButton::Right,
                Some("middle") => MouseButton::Middle,
                Some("x1") => MouseButton::X1,
                Some("x2") => MouseButton::X2,
                _ => MouseButton::Left,
            };
            match sim.click(button) {
                Ok(()) => ok(serde_json::json!({ "button": format!("{:?}", button) })),
                Err(e) => err(e.to_string()),
            }
        }
        "simulate.move" => {
            let x = msg.payload.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = msg.payload.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let absolute = msg
                .payload
                .get("absolute")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            match sim.move_mouse(x, y, absolute) {
                Ok(()) => ok(serde_json::json!({ "moved": [x, y] })),
                Err(e) => err(e.to_string()),
            }
        }
        "simulate.scroll" => {
            let delta = msg
                .payload
                .get("delta")
                .and_then(|v| v.as_i64())
                .unwrap_or(120) as i32;
            match sim.scroll(delta) {
                Ok(()) => ok(serde_json::json!({ "delta": delta })),
                Err(e) => err(e.to_string()),
            }
        }
        other => err(format!("unknown command: {other}")),
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = AppConfig {
        identifier: "com.example.input_sim".to_string(),
        name: "rdesktop input sim".to_string(),
        version: "0.1.0".to_string(),
        window: WindowConfig {
            title: "rdesktop — Input Simulation".to_string(),
            width: 820,
            height: 540,
            ..Default::default()
        },
        ..Default::default()
    };

    let sim =
        Arc::new(InputSimulator::new().expect("input simulation is unsupported on this platform"));
    let handler_sim = sim.clone();
    let handler = FnIpcHandler::new(move |msg: IpcMessage| handle_sim(&handler_sim, msg));

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
