use rdesktop_core::ipc::{IpcMessage, IpcResponse, FnIpcHandler};

fn main() -> anyhow::Result<()> {
    let handler = FnIpcHandler::new(|msg: IpcMessage| {
        match msg.cmd.as_str() {
            "greet" => {
                let name = msg.payload["name"].as_str().unwrap_or("World");
                IpcResponse {
                    id: msg.id,
                    success: true,
                    data: serde_json::json!({ "message": format!("Hello, {}!", name) }),
                }
            }
            _ => IpcResponse {
                id: msg.id,
                success: false,
                data: serde_json::json!({ "error": "Unknown command" }),
            },
        }
    });

    println!("App started. Use 'rdesktop dev' for browser mode.");
    Ok(())
}
