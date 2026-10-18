#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use rdesktop_core::ipc::{FnIpcHandler, IpcHandler, IpcMessage, IpcResponse, IpcResponseSender};
use rdesktop_core::{config::AppConfig, renderer::Renderer};
use rdesktop_webview::{NativeWindowOptions, WebViewRenderer};
use std::sync::{
    atomic::{AtomicI32, Ordering},
    Arc,
};

struct FixtureHandler {
    inner: Box<dyn IpcHandler>,
    runtime: rdesktop_webview::NativeRuntime,
    dialog_result: Arc<AtomicI32>,
}
impl IpcHandler for FixtureHandler {
    fn handle(&self, message: IpcMessage) -> IpcResponse {
        self.inner.handle(message)
    }
    fn handle_from_window(&self, window: u64, message: IpcMessage, respond: IpcResponseSender) {
        if message.cmd != "native_dialog" {
            self.inner.handle_from_window(window, message, respond);
            return;
        }
        #[cfg(windows)]
        let result = {
            use rdesktop_webview::tao::platform::windows::WindowExtWindows;
            let state = self.dialog_result.clone();
            self.runtime
                .dispatch_window(window, move |window, _| {
                    state.store(-1, Ordering::Release);
                    let text: Vec<u16> =
                        "This is a real Windows modal dialog. Close it through native UIA."
                            .encode_utf16()
                            .chain(Some(0))
                            .collect();
                    let title: Vec<u16> = "rdesktop native modal fixture"
                        .encode_utf16()
                        .chain(Some(0))
                        .collect();
                    let code = unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                            window.hwnd() as _,
                            text.as_ptr(),
                            title.as_ptr(),
                            windows_sys::Win32::UI::WindowsAndMessaging::MB_OKCANCEL
                                | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONINFORMATION,
                        )
                    };
                    state.store(code, Ordering::Release);
                })
                .map(|_| serde_json::json!({"queued":true}))
                .map_err(|error| error.to_string())
        };
        #[cfg(not(windows))]
        let result: Result<serde_json::Value, String> =
            Err("native modal fixture is Windows-only".into());
        respond(IpcResponse {
            id: message.id,
            success: result.is_ok(),
            data: result.unwrap_or_else(|error| serde_json::json!({"error":error})),
        });
    }
}

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
    let dialog_result = Arc::new(AtomicI32::new(0));
    let result_for_handler = dialog_result.clone();
    let handler = Box::new(FnIpcHandler::new(move |msg: IpcMessage| {
        let result = match msg.cmd.as_str() {
            "delay" => {
                match msg.payload["milliseconds"]
                    .as_u64()
                    .filter(|value| *value <= 20_000)
                {
                    Some(milliseconds) => {
                        std::thread::sleep(std::time::Duration::from_millis(milliseconds));
                        Ok(
                            serde_json::json!({"milliseconds":milliseconds,"pid":std::process::id()}),
                        )
                    }
                    None => Err("milliseconds must be an integer from 0 to 20000".into()),
                }
            }
            "greet" => Ok(
                serde_json::json!({"message":format!("Hello, {}!",msg.payload["name"].as_str().unwrap_or("World")),"pid":std::process::id()}),
            ),
            "native_dialog_status" => {
                Ok(serde_json::json!({"result":result_for_handler.load(Ordering::Acquire)}))
            }
            "second" => native
                .create_window(
                    rdesktop_core::WindowConfig {
                        title: "rdesktop Secondary Agent".into(),
                        width: 640,
                        height: 480,
                        ..Default::default()
                    },
                    NativeWindowOptions {
                        url: secondary_url
                            .clone()
                            .unwrap_or_else(|| "about:blank".into()),
                        ipc_enabled: true,
                        ..Default::default()
                    },
                )
                .and_then(|id| {
                    if secondary_url.is_none() {
                        native
                            .with_window(id, |_, view| {
                                view.load_html(include_str!("../frontend/index.html"))
                                    .map_err(|e| e.to_string())
                            })?
                            .map_err(rdesktop_core::RdesktopError::Config)?;
                    }
                    Ok(serde_json::json!({"window":id.to_string()}))
                })
                .map_err(|e| e.to_string()),
            _ => Err("unknown command".into()),
        };
        IpcResponse {
            id: msg.id,
            success: result.is_ok(),
            data: result.unwrap_or_else(|error| serde_json::json!({"error":error})),
        }
    }));
    renderer.set_ipc_handler(Box::new(FixtureHandler {
        inner: handler,
        runtime: renderer.native_runtime(),
        dialog_result,
    }));
    let window = renderer.create_window(&config.window)?;
    if let Some(url) = url {
        renderer.load_url(window, &url)?;
    } else {
        renderer.load_html(window, include_str!("../frontend/index.html"))?;
    }
    Box::new(renderer).run().map_err(Into::into)
}
