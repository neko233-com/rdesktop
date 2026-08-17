//! rdesktop wallpaper example.
//!
//! Demonstrates Phase 0 of the deep-desktop roadmap:
//! - `WindowKind::Wallpaper` (desktop layer, behind icons, click-through)
//! - `transparent` window so the WebGPU canvas shows through
//! - `renderer.webgpu = true` so the frontend can drive native shaders
//!
//! On Windows this reparents the window beneath the desktop `WorkerW` host and
//! sets `WS_EX_LAYERED | WS_EX_TRANSPARENT`; on macOS it drops to
//! `kCGDesktopWindowLevel` and ignores mouse events. Pointer input falls
//! through to the desktop, exactly like Wallpaper Engine.

use rdesktop_core::config::{AppConfig, RendererConfig, WindowConfig, WindowKind};
use rdesktop_core::ipc::{FnIpcHandler, IpcMessage, IpcResponse};
use rdesktop_core::renderer::Renderer;
use rdesktop_webview::WebViewRenderer;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = AppConfig {
        identifier: "com.example.wallpaper".to_string(),
        name: "rdesktop Wallpaper".to_string(),
        version: "0.1.0".to_string(),
        renderer: RendererConfig {
            webgpu: true,
            ..Default::default()
        },
        window: WindowConfig {
            title: "rdesktop Wallpaper".to_string(),
            width: 1920,
            height: 1080,
            transparent: true,
            decorations: false,
            resizable: false,
            kind: WindowKind::Wallpaper,
            click_through: true,
            ..Default::default()
        },
        ..Default::default()
    };

    // A wallpaper rarely needs inbound IPC, but the bridge is wired so the
    // frontend can still report lifecycle/perf telemetry if desired.
    let handler = FnIpcHandler::new(|msg: IpcMessage| IpcResponse {
        id: msg.id,
        success: true,
        data: serde_json::json!({ "ok": true }),
    });

    let mut renderer = WebViewRenderer::new(&config)?;
    renderer.init()?;
    renderer.set_ipc_handler(Box::new(handler));

    let frontend_html = include_str!("../frontend/index.html");
    let handle = renderer.create_window(&config.window)?;
    renderer.load_html(handle, frontend_html)?;

    // Blocks until the window is closed. Because the window is click-through,
    // "closed" must be driven programmatically (e.g. a tray quit command).
    Box::new(renderer).run()?;
    Ok(())
}
