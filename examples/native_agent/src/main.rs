#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use rdesktop_core::ipc::{FnIpcHandler, IpcMessage, IpcResponse};
use rdesktop_core::{config::AppConfig, renderer::Renderer};
use rdesktop_webview::{NativeWindowOptions, WebViewRenderer};

fn main() -> anyhow::Result<()> {
    let mut config = AppConfig::default();
    config.window.title = "rdesktop Native Agent".into();
    config.window.width = 900;
    config.window.height = 680;
    let mut renderer = WebViewRenderer::new(&config)?;
    renderer.init()?;
    renderer.enable_native_agent_from_env()?;
    let native = renderer.native_runtime();
    #[cfg(debug_assertions)]
    let url = std::env::var("RDESKTOP_DEV_URL").ok();
    #[cfg(not(debug_assertions))]
    let url: Option<String> = None;
    let secondary_url = url.clone();
    renderer.set_ipc_handler(Box::new(FnIpcHandler::new(move |msg: IpcMessage| {
        let result = match msg.cmd.as_str() {
            "greet" => Ok(serde_json::json!({"message":format!("Hello, {}!",msg.payload["name"].as_str().unwrap_or("World")),"pid":std::process::id()})),
            "second" => native.create_window(rdesktop_core::WindowConfig {
                title:"rdesktop Secondary Agent".into(),width:640,height:480,..Default::default()
            },NativeWindowOptions {url:secondary_url.clone().unwrap_or_else(||"about:blank".into()),ipc_enabled:true,..Default::default()})
                .and_then(|id| {
                    if secondary_url.is_none() { native.with_window(id,|_,view|view.load_html(include_str!("../frontend/index.html")).map_err(|e|e.to_string()))?
                        .map_err(rdesktop_core::RdesktopError::Config)?; }
                    Ok(serde_json::json!({"window":id.to_string()}))
                }).map_err(|e|e.to_string()),
            _ => Err("unknown command".into()),
        };
        IpcResponse {id:msg.id,success:result.is_ok(),data:result.unwrap_or_else(|error|serde_json::json!({"error":error}))}
    })));
    let window = renderer.create_window(&config.window)?;
    if let Some(url) = url {
        renderer.load_url(window, &url)?;
    } else {
        renderer.load_html(window, include_str!("../frontend/index.html"))?;
    }
    Box::new(renderer).run().map_err(Into::into)
}
