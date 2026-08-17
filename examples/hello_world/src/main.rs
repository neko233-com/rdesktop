use rdesktop_core::config::{AppConfig, RendererConfig, WindowConfig};
use rdesktop_core::ipc::{IpcMessage, IpcResponse, FnIpcHandler};

fn main() -> anyhow::Result<()> {
    let _config = AppConfig {
        identifier: "com.example.hello_world".to_string(),
        name: "Hello rdesktop".to_string(),
        version: "0.1.0".to_string(),
        renderer: RendererConfig::default(), // WebView by default
        window: WindowConfig {
            title: "Hello rdesktop".to_string(),
            width: 1280,
            height: 720,
            ..Default::default()
        },
        ..Default::default()
    };

    // Set up IPC handler
    let _handler = FnIpcHandler::new(|msg: IpcMessage| {
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

    // Build and run the app
    // In a real implementation, this would use the App builder:
    //
    // rdesktop_core::App::builder(config)
    //     .with_ipc_handler(Box::new(handler))
    //     .build()?
    //     .run()?;

    println!("rdesktop Hello World example");
    println!("This demonstrates the dual-engine architecture:");
    println!("  - Default: WebView (WebView2/WebKit)");
    println!("  - Optional: Chrome Embedded (pixel-perfect cross-platform)");
    println!();
    println!("To run with Chrome renderer:");
    println!("  cargo run -p hello_world --features chrome");
    println!();
    println!("To run in dev mode (browser):");
    println!("  rdesktop dev");

    Ok(())
}
