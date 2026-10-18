#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use rdesktop_core::{
    config::AppConfig,
    ipc::{FnIpcHandler, IpcMessage, IpcResponse},
    renderer::Renderer,
};
use rdesktop_node::{renderer::RendererBridge, NodeConfig};
use rdesktop_webview::{NativeEvent, NativeWindowOptions, WebViewRenderer};
use std::{path::PathBuf, sync::Arc, time::Duration};

fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(
        std::env::var_os("RDESKTOP_PLUGIN_ROOT")
            .ok_or_else(|| anyhow::anyhow!("select RDESKTOP_PLUGIN_ROOT"))?,
    );
    let bridge = RendererBridge::start(
        NodeConfig {
            executable: PathBuf::from(
                std::env::var_os("RDESKTOP_TEST_NODE")
                    .ok_or_else(|| anyhow::anyhow!("select an explicit Node executable"))?,
            ),
            executable_sha256: std::env::var("RDESKTOP_TEST_NODE_SHA256")?,
            preload: root.join(
                std::env::var("RDESKTOP_PLUGIN_PRELOAD").unwrap_or_else(|_| "preload.cjs".into()),
            ),
            package_root: root,
            timeout: Duration::from_secs(3),
        },
        std::env::var("RDESKTOP_PLUGIN_ENTRY").unwrap_or_else(|_| "index.html".into()),
    )?;
    let mut config = AppConfig::default();
    config.window.title = "rdesktop Native Node Plugin".into();
    config.window.width = 800;
    config.window.height = 600;
    let origin = bridge
        .url()
        .split('/')
        .take(3)
        .collect::<Vec<_>>()
        .join("/");
    config.renderer.trusted_origins = vec![origin];
    let mut renderer = WebViewRenderer::new(&config)?;
    renderer.init()?;
    renderer.enable_native_agent_from_env()?;
    renderer.set_resident(false);
    let native = renderer.native_runtime();
    let exiting = native.clone();
    renderer.set_native_event_handler(Arc::new(move |_, event| {
        if event == NativeEvent::CloseRequested {
            let _ = exiting.exit();
        }
    }));
    let handler_runtime = native.clone();
    renderer.set_ipc_handler(Box::new(FnIpcHandler::new(move |message: IpcMessage| {
        let result = if message.cmd == "plugin_done" {
            handler_runtime
                .exit()
                .map(|_| serde_json::json!(true))
                .map_err(|error| error.to_string())
        } else {
            Err("unknown fixture command".into())
        };
        IpcResponse {
            id: message.id,
            success: result.is_ok(),
            data: result.unwrap_or_else(|error| serde_json::json!({"error":error})),
        }
    })));
    let mut initialization = String::new();
    #[cfg(debug_assertions)]
    if let Ok(folder) = std::env::var("RDESKTOP_PLUGIN_FIXTURE_FOLDER") {
        // A fixture host adapter, not a shipped uTools compatibility claim.
        initialization = format!(
            r#"window.utools=Object.freeze({{
            getCurrentFolderPath:()=>{}, isDarkColors:()=>false,
            onPluginEnter:callback=>queueMicrotask(()=>callback({{code:'新建文件夹',type:'text',payload:''}})),
            showNotification:message=>{{document.documentElement.dataset.notification=message;}},
            outPlugin:()=>window.__RDESKTOP_INVOKE__('plugin_done',{{}}), hideMainWindow:()=>{{}}
        }});"#,
            serde_json::to_string(&folder)?
        );
    }
    initialization.push_str(bridge.initialization_script());
    let url = bridge.url().to_owned();
    let window_config = config.window.clone();
    let window_thread = std::thread::spawn(move || {
        native.create_window(
            window_config,
            NativeWindowOptions {
                url,
                initialization_script: initialization,
                ipc_enabled: true,
                ..Default::default()
            },
        )
    });
    Box::new(renderer).run()?;
    window_thread
        .join()
        .map_err(|_| anyhow::anyhow!("native creation worker panicked"))??;
    drop(bridge);
    Ok(())
}
