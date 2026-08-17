use std::collections::HashMap;
use std::sync::Arc;

use rdesktop_core::config::{AppConfig, WindowConfig};
use rdesktop_core::ipc::IpcHandler;
use rdesktop_core::renderer::{Renderer, RendererKind};
use rdesktop_core::window::WindowHandle;
use rdesktop_core::{RdesktopError, Result};

use tao::event_loop::EventLoop;
use tao::window::{Window, WindowBuilder};
use wry::{WebView, WebViewBuilder};

struct WindowEntry {
    window: Window,
    webview: WebView,
}

/// WebView-based renderer using wry + tao.
///
/// Platform backends:
/// - Windows: WebView2 (Edge Chromium)
/// - macOS: WKWebView (WebKit)
/// - Linux: WebKitGTK
pub struct WebViewRenderer {
    config: AppConfig,
    ipc_handler: Option<Arc<dyn IpcHandler>>,
    windows: HashMap<u64, WindowEntry>,
    next_window_id: u64,
}

impl WebViewRenderer {
    pub fn new(config: &AppConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
            ipc_handler: None,
            windows: HashMap::new(),
            next_window_id: 1,
        })
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_window_id;
        self.next_window_id += 1;
        id
    }
}

impl Renderer for WebViewRenderer {
    fn init(&mut self) -> Result<()> {
        tracing::info!("Initializing WebView renderer");
        Ok(())
    }

    fn create_window(&mut self, config: &WindowConfig) -> Result<WindowHandle> {
        let id = self.next_id();

        let event_loop = EventLoop::new();
        let window = WindowBuilder::new()
            .with_title(&config.title)
            .with_inner_size(tao::dpi::LogicalSize::new(config.width, config.height))
            .with_resizable(config.resizable)
            .with_decorations(config.decorations)
            .with_transparent(config.transparent)
            .with_always_on_top(config.always_on_top)
            .build(&event_loop)
            .map_err(|e| RdesktopError::WindowCreation(e.to_string()))?;

        let webview_builder = WebViewBuilder::new()
            .with_url("about:blank")
            .with_devtools(cfg!(debug_assertions));

        let webview = webview_builder
            .build(&window)
            .map_err(|e| RdesktopError::WebView(e.to_string()))?;

        self.windows.insert(
            id,
            WindowEntry {
                window,
                webview,
            },
        );

        tracing::info!(window_id = id, "Window created");
        Ok(WindowHandle::new(id))
    }

    fn load_url(&self, window: WindowHandle, url: &str) -> Result<()> {
        let entry = self
            .windows
            .get(&window.id())
            .ok_or_else(|| RdesktopError::WindowCreation("Window not found".to_string()))?;
        entry.webview.load_url(url);
        Ok(())
    }

    fn load_html(&self, window: WindowHandle, html: &str) -> Result<()> {
        let entry = self
            .windows
            .get(&window.id())
            .ok_or_else(|| RdesktopError::WindowCreation("Window not found".to_string()))?;
        entry.webview.load_html(html);
        Ok(())
    }

    fn eval_script(&self, window: WindowHandle, script: &str) -> Result<()> {
        let entry = self
            .windows
            .get(&window.id())
            .ok_or_else(|| RdesktopError::WindowCreation("Window not found".to_string()))?;
        entry
            .webview
            .evaluate_script(script)
            .map_err(|e| RdesktopError::Ipc(e.to_string()))?;
        Ok(())
    }

    fn set_ipc_handler(&mut self, handler: Box<dyn IpcHandler>) {
        self.ipc_handler = Some(Arc::from(handler));
    }

    fn send_to_frontend(&self, window: WindowHandle, message: &str) -> Result<()> {
        let escaped = message.replace('\\', "\\\\").replace('\'', "\\'");
        self.eval_script(
            window,
            &format!("window.__RDESKTOP_IPC__ && window.__RDESKTOP_IPC__('{escaped}')"),
        )
    }

    fn set_title(&self, window: WindowHandle, title: &str) -> Result<()> {
        let entry = self
            .windows
            .get(&window.id())
            .ok_or_else(|| RdesktopError::WindowCreation("Window not found".to_string()))?;
        entry.window.set_title(title);
        Ok(())
    }

    fn set_size(&self, window: WindowHandle, width: u32, height: u32) -> Result<()> {
        let entry = self
            .windows
            .get(&window.id())
            .ok_or_else(|| RdesktopError::WindowCreation("Window not found".to_string()))?;
        entry
            .window
            .set_inner_size(tao::dpi::LogicalSize::new(width, height));
        Ok(())
    }

    fn set_resizable(&self, window: WindowHandle, resizable: bool) -> Result<()> {
        let entry = self
            .windows
            .get(&window.id())
            .ok_or_else(|| RdesktopError::WindowCreation("Window not found".to_string()))?;
        entry.window.set_resizable(resizable);
        Ok(())
    }

    fn set_visible(&self, window: WindowHandle, visible: bool) -> Result<()> {
        let entry = self
            .windows
            .get(&window.id())
            .ok_or_else(|| RdesktopError::WindowCreation("Window not found".to_string()))?;
        entry.window.set_visible(visible);
        Ok(())
    }

    fn close_window(&mut self, window: WindowHandle) -> Result<()> {
        self.windows.remove(&window.id());
        tracing::info!(window_id = window.id(), "Window closed");
        Ok(())
    }

    fn run(self: Box<Self>) -> Result<()> {
        tracing::info!("Starting WebView event loop");
        // In production, this would run tao's event loop
        tracing::info!("WebView renderer shutting down");
        Ok(())
    }

    fn kind(&self) -> RendererKind {
        RendererKind::WebView
    }
}
