#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(all(feature = "chrome", not(feature = "webview")))]
use rdesktop_cef::CefRenderer as AppRenderer;
use rdesktop_core::config::{AppConfig, WindowConfig};
use rdesktop_core::ipc::{FnIpcHandler, IpcMessage, IpcResponse};
use rdesktop_core::renderer::Renderer;
#[cfg(feature = "webview")]
use rdesktop_webview::WebViewRenderer as AppRenderer;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = AppConfig {
        identifier: "com.example.hello_world".to_string(),
        name: "Hello rdesktop".to_string(),
        version: "0.1.0".to_string(),
        window: WindowConfig {
            title: "Hello rdesktop".to_string(),
            width: 1280,
            height: 720,
            ..Default::default()
        },
        ..Default::default()
    };

    // Set up IPC handler
    let handler = FnIpcHandler::new(|msg: IpcMessage| {
        println!("Received IPC command: {}", msg.cmd);

        let (success, response_data) = match msg.cmd.as_str() {
            "greet" => {
                let name = msg.payload["name"].as_str().unwrap_or("World");
                (
                    true,
                    serde_json::json!({ "message": format!("Hello, {}!", name) }),
                )
            }
            "add" => {
                let a = msg.payload["a"].as_f64().unwrap_or(0.0);
                let b = msg.payload["b"].as_f64().unwrap_or(0.0);
                (true, serde_json::json!({ "result": a + b }))
            }
            _ => (false, serde_json::json!({ "error": "Unknown command" })),
        };

        IpcResponse {
            id: msg.id,
            success,
            data: response_data,
        }
    });

    // Build the renderer
    let mut renderer = AppRenderer::new(&config)?;
    renderer.init()?;
    renderer.set_ipc_handler(Box::new(handler));
    #[cfg(feature = "webview")]
    renderer.enable_native_agent_from_env()?;

    // Create the main window and load the frontend HTML
    let frontend_html = include_str!("../frontend/index.html");
    let handle = renderer.create_window(&config.window)?;
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("RDESKTOP_DEV_URL") {
        renderer.load_url(handle, &url)?;
        return Box::new(renderer).run().map_err(Into::into);
    }
    renderer.load_html(handle, frontend_html)?;

    // Enter the event loop (blocks until all windows are closed)
    Box::new(renderer).run()?;

    Ok(())
}
