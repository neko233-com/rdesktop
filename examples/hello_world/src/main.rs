use rdesktop_core::config::{AppConfig, WindowConfig};
use rdesktop_core::ipc::{IpcMessage, IpcResponse, FnIpcHandler};
use rdesktop_core::renderer::Renderer;
use rdesktop_webview::WebViewRenderer;

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
        println!("Received IPC: {} -> {}", msg.cmd, msg.payload);

        let response_data = match msg.cmd.as_str() {
            "greet" => {
                let name = msg.payload["name"].as_str().unwrap_or("World");
                serde_json::json!({ "message": format!("Hello, {}!", name) })
            }
            "add" => {
                let a = msg.payload["a"].as_f64().unwrap_or(0.0);
                let b = msg.payload["b"].as_f64().unwrap_or(0.0);
                serde_json::json!({ "result": a + b })
            }
            _ => serde_json::json!({ "error": "Unknown command" }),
        };

        IpcResponse {
            id: msg.id,
            success: true,
            data: response_data,
        }
    });

    // Build the renderer
    let mut renderer = WebViewRenderer::new(&config)?;
    renderer.init()?;
    renderer.set_ipc_handler(Box::new(handler));

    // Create the main window and load the frontend HTML
    let frontend_html = include_str!("../frontend/index.html");
    let handle = renderer.create_window(&config.window)?;
    renderer.load_html(handle, frontend_html)?;

    // Enter the event loop (blocks until all windows are closed)
    Box::new(renderer).run()?;

    Ok(())
}
