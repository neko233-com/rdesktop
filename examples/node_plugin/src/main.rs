#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use rdesktop_core::{
    config::AppConfig,
    ipc::{FnIpcHandler, IpcMessage, IpcResponse},
    renderer::Renderer,
};
use rdesktop_node::{
    renderer::{NodeGlobals, RendererBridge},
    NodeConfig,
};
use rdesktop_webview::{NativeEvent, NativeWindowOptions, WebViewRenderer};
use std::{path::PathBuf, sync::Arc, time::Duration};

fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(
        std::env::var_os("RDESKTOP_PLUGIN_ROOT")
            .ok_or_else(|| anyhow::anyhow!("select RDESKTOP_PLUGIN_ROOT"))?,
    );
    let globals = match std::env::var("RDESKTOP_PLUGIN_NODE_GLOBALS").as_deref() {
        Ok("preload-only") => NodeGlobals::PreloadOnly,
        Ok("page") | Err(_) => NodeGlobals::Page,
        _ => anyhow::bail!("Node globals must be page or preload-only"),
    };
    let bridge = RendererBridge::start_with_globals(
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
        globals,
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
    let embedded = std::env::var_os("RDESKTOP_PLUGIN_EMBEDDED").is_some_and(|value| value == "1");
    #[cfg(debug_assertions)]
    let hidden = std::env::var_os("RDESKTOP_HEADLESS").is_some_and(|value| value == "1");
    #[cfg(not(debug_assertions))]
    let hidden = false;
    renderer.set_resident(embedded);
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
    let profile = std::env::var_os("RDESKTOP_PLUGIN_FIXTURE_FOLDER")
        .map(PathBuf::from)
        .map(|root| root.join("webview-profile"));
    let window_thread = std::thread::spawn(move || {
        let parent = if embedded {
            let mut owner = window_config.clone();
            owner.title = "Native plugin owner".into();
            owner.width = 900;
            owner.height = 700;
            let id = native.create_window(
                owner,
                NativeWindowOptions {
                    url: "about:blank".into(),
                    ..Default::default()
                },
            )?;
            if !hidden {
                native.with_window(id, |window, _| window.set_visible(true))?;
            }
            Some(id)
        } else {
            None
        };
        let mut child_config = window_config;
        if embedded {
            child_config.decorations = false;
            child_config.resizable = false;
        }
        let child = native.create_window(
            child_config,
            NativeWindowOptions {
                url,
                initialization_script: initialization,
                ipc_enabled: true,
                parent_window: parent,
                data_directory: embedded.then_some(profile).flatten(),
                ..Default::default()
            },
        )?;
        if let Some(parent) = parent {
            if native
                .set_child_bounds(parent, child, 0, 0, 800, 600)
                .is_ok()
                || native
                    .set_child_bounds(child, u64::MAX, 0, 0, 800, 600)
                    .is_ok()
                || native
                    .set_child_bounds(child, parent, 0, 0, 8193, 600)
                    .is_ok()
            {
                return Err(rdesktop_core::RdesktopError::Config(
                    "invalid child placement was accepted".into(),
                ));
            }
            native.set_child_bounds(child, parent, 0, 0, 800, 600)?;
        }
        Ok::<_, rdesktop_core::RdesktopError>(child)
    });
    Box::new(renderer).run()?;
    window_thread
        .join()
        .map_err(|_| anyhow::anyhow!("native creation worker panicked"))??;
    drop(bridge);
    Ok(())
}
