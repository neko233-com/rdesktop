use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rdesktop_core::config::{AppConfig, WindowConfig};
use rdesktop_core::ipc::{IpcHandler, IpcMessage};
use rdesktop_core::renderer::{Renderer, RendererKind};
use rdesktop_core::window::WindowHandle;
use rdesktop_core::Result;

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Window, WindowBuilder, WindowId};
use wry::{WebView, WebViewBuilder};

struct WindowEntry {
    window: Window,
    webview: WebView,
}

/// Pending operation queued before the event loop starts.
enum PendingOp {
    LoadUrl(u64, String),
    LoadHtml(u64, String),
    EvalScript(u64, String),
    SetTitle(u64, String),
    SetSize(u64, u32, u32),
    SetResizable(u64, bool),
    SetVisible(u64, bool),
    SendToFrontend(u64, String),
    Close(u64),
}

/// Shared IPC response queue.
/// The IPC handler pushes responses here; the event loop drains and evaluates them.
type IpcResponseQueue = Arc<Mutex<Vec<String>>>;

/// WebView-based renderer using wry + tao.
///
/// Platform backends:
/// - Windows: WebView2 (Edge Chromium)
/// - macOS: WKWebView (WebKit)
/// - Linux: WebKitGTK
///
/// ## Lifecycle
///
/// 1. `new()` + `init()` - create renderer
/// 2. `set_ipc_handler()` - wire up IPC
/// 3. `create_window()` - queue window creation (returns handle immediately)
/// 4. `load_url()` / `load_html()` - queue content loading
/// 5. `run()` - enters event loop, processes all queued operations, blocks until exit
pub struct WebViewRenderer {
    config: AppConfig,
    ipc_handler: Option<Arc<dyn IpcHandler>>,
    pending_windows: RefCell<Vec<(u64, WindowConfig)>>,
    pending_ops: RefCell<Vec<PendingOp>>,
    next_window_id: RefCell<u64>,
}

impl WebViewRenderer {
    pub fn new(config: &AppConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
            ipc_handler: None,
            pending_windows: RefCell::new(Vec::new()),
            pending_ops: RefCell::new(Vec::new()),
            next_window_id: RefCell::new(1),
        })
    }

    fn next_id(&self) -> u64 {
        let mut id = self.next_window_id.borrow_mut();
        let current = *id;
        *id += 1;
        current
    }

    /// JavaScript bridge injected into every WebView.
    /// Provides `window.__RDESKTOP_INVOKE__(cmd, payload)` for IPC.
    fn bridge_script() -> &'static str {
        r#"
        (function() {
            if (window.__RDESKTOP_BRIDGE__) return;
            window.__RDESKTOP_BRIDGE__ = true;
            window.__RDESKTOP_RESOLVE__ = {};

            window.__RDESKTOP_INVOKE__ = function(cmd, payload) {
                return new Promise(function(resolve, reject) {
                    var id = Math.random().toString(36).slice(2);
                    window.__RDESKTOP_RESOLVE__[id] = resolve;
                    if (window.ipc && window.ipc.postMessage) {
                        window.ipc.postMessage(JSON.stringify({ id: id, cmd: cmd, payload: payload || {} }));
                    }
                    setTimeout(function() {
                        if (window.__RDESKTOP_RESOLVE__[id]) {
                            delete window.__RDESKTOP_RESOLVE__[id];
                            reject(new Error('IPC timeout'));
                        }
                    }, 30000);
                });
            };

            window.__RDESKTOP_IPC__ = function(message) {
                try {
                    var data = typeof message === 'string' ? JSON.parse(message) : message;
                    if (data.id && window.__RDESKTOP_RESOLVE__[data.id]) {
                        window.__RDESKTOP_RESOLVE__[data.id](data);
                        delete window.__RDESKTOP_RESOLVE__[data.id];
                    }
                } catch (e) {
                    console.error('rdesktop IPC error:', e);
                }
            };
        })();
        "#
    }
}

impl Renderer for WebViewRenderer {
    fn init(&mut self) -> Result<()> {
        tracing::info!("Initializing WebView renderer");
        Ok(())
    }

    fn create_window(&mut self, config: &WindowConfig) -> Result<WindowHandle> {
        let id = self.next_id();
        self.pending_windows.borrow_mut().push((id, config.clone()));
        tracing::info!(window_id = id, "Window queued for creation");
        Ok(WindowHandle::new(id))
    }

    fn load_url(&self, window: WindowHandle, url: &str) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::LoadUrl(window.id(), url.to_string()));
        Ok(())
    }

    fn load_html(&self, window: WindowHandle, html: &str) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::LoadHtml(window.id(), html.to_string()));
        Ok(())
    }

    fn eval_script(&self, window: WindowHandle, script: &str) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::EvalScript(window.id(), script.to_string()));
        Ok(())
    }

    fn set_ipc_handler(&mut self, handler: Box<dyn IpcHandler>) {
        self.ipc_handler = Some(Arc::from(handler));
    }

    fn send_to_frontend(&self, window: WindowHandle, message: &str) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::SendToFrontend(window.id(), message.to_string()));
        Ok(())
    }

    fn set_title(&self, window: WindowHandle, title: &str) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::SetTitle(window.id(), title.to_string()));
        Ok(())
    }

    fn set_size(&self, window: WindowHandle, width: u32, height: u32) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::SetSize(window.id(), width, height));
        Ok(())
    }

    fn set_resizable(&self, window: WindowHandle, resizable: bool) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::SetResizable(window.id(), resizable));
        Ok(())
    }

    fn set_visible(&self, window: WindowHandle, visible: bool) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::SetVisible(window.id(), visible));
        Ok(())
    }

    fn close_window(&mut self, window: WindowHandle) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::Close(window.id()));
        Ok(())
    }

    fn run(mut self: Box<Self>) -> Result<()> {
        tracing::info!("Starting WebView event loop");

        let _config = self.config.clone();
        let ipc_handler = self.ipc_handler.take();
        let pending_windows: Vec<(u64, WindowConfig)> =
            self.pending_windows.borrow_mut().drain(..).collect();
        let pending_ops: Vec<PendingOp> = self.pending_ops.borrow_mut().drain(..).collect();

        // Shared queue for IPC responses that need to be sent back to webviews
        let ipc_response_queue: IpcResponseQueue = Arc::new(Mutex::new(Vec::new()));
        let ipc_queue_for_handler = ipc_response_queue.clone();

        let event_loop = EventLoopBuilder::new().build();
        let mut windows: HashMap<WindowId, WindowEntry> = HashMap::new();
        let mut rdesktop_to_tao: HashMap<u64, WindowId> = HashMap::new();
        let mut tao_to_rdesktop: HashMap<WindowId, u64> = HashMap::new();

        event_loop.run(move |event, event_loop_target, control_flow| {
            *control_flow = ControlFlow::Wait;

            match event {
                Event::NewEvents(StartCause::Init) => {
                    // Create all pending windows
                    for (rdesktop_id, window_config) in &pending_windows {
                        let window = match WindowBuilder::new()
                            .with_title(&window_config.title)
                            .with_inner_size(tao::dpi::LogicalSize::new(
                                window_config.width,
                                window_config.height,
                            ))
                            .with_resizable(window_config.resizable)
                            .with_decorations(window_config.decorations)
                            .with_transparent(window_config.transparent)
                            .with_always_on_top(window_config.always_on_top)
                            .build(event_loop_target)
                        {
                            Ok(w) => w,
                            Err(e) => {
                                tracing::error!("Failed to create window {}: {}", rdesktop_id, e);
                                continue;
                            }
                        };

                        let tao_id = window.id();

                        // Build WebView with IPC handler
                        let mut builder = WebViewBuilder::new()
                            .with_url("about:blank")
                            .with_devtools(cfg!(debug_assertions))
                            .with_initialization_script(Self::bridge_script());

                        // Wire up IPC handler
                        // wry's IPC handler is Fn(Request<String>) - no return value.
                        // Responses are sent back via a shared queue that the event loop drains.
                        if let Some(ref handler) = ipc_handler {
                            let handler = handler.clone();
                            let queue = ipc_queue_for_handler.clone();
                            builder = builder.with_ipc_handler(
                                move |req: wry::http::Request<String>| {
                                    let body = req.body();
                                    match serde_json::from_str::<IpcMessage>(body) {
                                        Ok(msg) => {
                                            let response = handler.handle(msg);
                                            if let Ok(json) = serde_json::to_string(&response) {
                                                // Queue the response - the event loop will
                                                // drain and evaluate it on the webview
                                                if let Ok(mut q) = queue.lock() {
                                                    q.push(json);
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!("Invalid IPC message: {}", e);
                                        }
                                    }
                                },
                            );
                        }

                        let webview = match builder.build(&window) {
                            Ok(wv) => wv,
                            Err(e) => {
                                tracing::error!("Failed to create webview {}: {}", rdesktop_id, e);
                                continue;
                            }
                        };

                        windows.insert(tao_id, WindowEntry { window, webview });
                        rdesktop_to_tao.insert(*rdesktop_id, tao_id);
                        tao_to_rdesktop.insert(tao_id, *rdesktop_id);

                        tracing::info!(rdesktop_id = rdesktop_id, ?tao_id, "Window created");
                    }

                    // Process pending operations
                    for op in &pending_ops {
                        match op {
                            PendingOp::LoadUrl(rd_id, url) => {
                                if let Some(tao_id) = rdesktop_to_tao.get(rd_id) {
                                    if let Some(entry) = windows.get(tao_id) {
                                        let _ = entry.webview.load_url(url);
                                    }
                                }
                            }
                            PendingOp::LoadHtml(rd_id, html) => {
                                if let Some(tao_id) = rdesktop_to_tao.get(rd_id) {
                                    if let Some(entry) = windows.get(tao_id) {
                                        let _ = entry.webview.load_html(html);
                                    }
                                }
                            }
                            PendingOp::EvalScript(rd_id, script) => {
                                if let Some(tao_id) = rdesktop_to_tao.get(rd_id) {
                                    if let Some(entry) = windows.get(tao_id) {
                                        let _ = entry.webview.evaluate_script(script);
                                    }
                                }
                            }
                            PendingOp::SetTitle(rd_id, title) => {
                                if let Some(tao_id) = rdesktop_to_tao.get(rd_id) {
                                    if let Some(entry) = windows.get(tao_id) {
                                        entry.window.set_title(title);
                                    }
                                }
                            }
                            PendingOp::SetSize(rd_id, w, h) => {
                                if let Some(tao_id) = rdesktop_to_tao.get(rd_id) {
                                    if let Some(entry) = windows.get(tao_id) {
                                        entry
                                            .window
                                            .set_inner_size(tao::dpi::LogicalSize::new(*w, *h));
                                    }
                                }
                            }
                            PendingOp::SetResizable(rd_id, resizable) => {
                                if let Some(tao_id) = rdesktop_to_tao.get(rd_id) {
                                    if let Some(entry) = windows.get(tao_id) {
                                        entry.window.set_resizable(*resizable);
                                    }
                                }
                            }
                            PendingOp::SetVisible(rd_id, visible) => {
                                if let Some(tao_id) = rdesktop_to_tao.get(rd_id) {
                                    if let Some(entry) = windows.get(tao_id) {
                                        entry.window.set_visible(*visible);
                                    }
                                }
                            }
                            PendingOp::SendToFrontend(rd_id, msg) => {
                                if let Some(tao_id) = rdesktop_to_tao.get(rd_id) {
                                    if let Some(entry) = windows.get(tao_id) {
                                        let escaped =
                                            msg.replace('\\', "\\\\").replace('\'', "\\'");
                                        let script =
                                            format!("window.__RDESKTOP_IPC__('{escaped}')");
                                        let _ = entry.webview.evaluate_script(&script);
                                    }
                                }
                            }
                            PendingOp::Close(rd_id) => {
                                if let Some(tao_id) = rdesktop_to_tao.remove(rd_id) {
                                    tao_to_rdesktop.remove(&tao_id);
                                    windows.remove(&tao_id);
                                }
                            }
                        }
                    }
                }

                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    window_id,
                    ..
                } => {
                    if let Some(rd_id) = tao_to_rdesktop.remove(&window_id) {
                        rdesktop_to_tao.remove(&rd_id);
                    }
                    windows.remove(&window_id);
                    if windows.is_empty() {
                        tracing::info!("All windows closed, exiting");
                        *control_flow = ControlFlow::Exit;
                    }
                }

                Event::WindowEvent {
                    event: WindowEvent::Resized(size),
                    window_id,
                    ..
                } => {
                    if let Some(entry) = windows.get(&window_id) {
                        let _ = entry.webview.set_bounds(wry::Rect {
                            position: tao::dpi::LogicalPosition::<i32>::new(0, 0).into(),
                            size: tao::dpi::LogicalSize::new(size.width, size.height).into(),
                        });
                    }
                }

                Event::WindowEvent {
                    event: WindowEvent::ScaleFactorChanged { new_inner_size, .. },
                    window_id,
                    ..
                } => {
                    if let Some(entry) = windows.get(&window_id) {
                        let _ = entry.webview.set_bounds(wry::Rect {
                            position: tao::dpi::LogicalPosition::<i32>::new(0, 0).into(),
                            size: tao::dpi::LogicalSize::new(
                                new_inner_size.width,
                                new_inner_size.height,
                            )
                            .into(),
                        });
                    }
                }

                // On each event, drain the IPC response queue and send responses to webviews
                Event::MainEventsCleared => {
                    let responses: Vec<String> = {
                        let mut queue = ipc_response_queue.lock().unwrap();
                        queue.drain(..).collect()
                    };
                    for json in responses {
                        // Send to the primary (first) window's webview
                        if let Some(entry) = windows.values().next() {
                            let escaped = json.replace('\\', "\\\\").replace('\'', "\\'");
                            let script =
                                format!("window.__RDESKTOP_IPC__('{escaped}')");
                            let _ = entry.webview.evaluate_script(&script);
                        }
                    }
                }

                Event::LoopDestroyed => {
                    tracing::info!("WebView event loop destroyed");
                }

                _ => {}
            }
        });
    }

    fn kind(&self) -> RendererKind {
        RendererKind::WebView
    }
}
