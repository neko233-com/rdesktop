use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rdesktop_core::config::{AppConfig, WindowConfig};
use rdesktop_core::ipc::{IpcHandler, IpcMessage};
use rdesktop_core::renderer::{Renderer, RendererKind, ResizeEdge};
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
    // Frameless / window control
    Minimize(u64),
    Maximize(u64),
    SetFullscreen(u64, bool),
    StartDrag(u64),
    StartResize(u64, tao::window::ResizeDirection),
    SetDecorations(u64, bool),
    SetAlwaysOnTop(u64, bool),
}

/// Shared IPC response queue.
type IpcResponseQueue = Arc<Mutex<Vec<String>>>;

/// Window control commands from the IPC thread, drained by the event loop.
type WindowCommandQueue = Arc<Mutex<Vec<WindowCommand>>>;

/// A window control command sent from the IPC handler to the event loop.
struct WindowCommand {
    rdesktop_id: u64,
    action: WindowAction,
}

enum WindowAction {
    Minimize,
    Maximize,
    Close,
    StartDrag,
    StartResize(tao::window::ResizeDirection),
    SetFullscreen(bool),
}

/// Convert rdesktop ResizeEdge to tao's ResizeDirection.
fn to_tao_resize(edge: ResizeEdge) -> tao::window::ResizeDirection {
    match edge {
        ResizeEdge::Top => tao::window::ResizeDirection::North,
        ResizeEdge::Bottom => tao::window::ResizeDirection::South,
        ResizeEdge::Left => tao::window::ResizeDirection::West,
        ResizeEdge::Right => tao::window::ResizeDirection::East,
        ResizeEdge::TopLeft => tao::window::ResizeDirection::NorthWest,
        ResizeEdge::TopRight => tao::window::ResizeDirection::NorthEast,
        ResizeEdge::BottomLeft => tao::window::ResizeDirection::SouthWest,
        ResizeEdge::BottomRight => tao::window::ResizeDirection::SouthEast,
    }
}

/// WebView-based renderer using wry + tao.
///
/// Platform backends:
/// - Windows: WebView2 (Edge Chromium)
/// - macOS: WKWebView (WebKit)
/// - Linux: WebKitGTK
///
/// ## Frameless / Custom Title Bar
///
/// Set `decorations = false` in `WindowConfig` to create a frameless window.
/// The frontend can use `window.__RDESKTOP_WINDOW__` to control the window:
///
/// ```javascript
/// window.__RDESKTOP_WINDOW__.minimize()
/// window.__RDESKTOP_WINDOW__.maximize()
/// window.__RDESKTOP_WINDOW__.close()
/// window.__RDESKTOP_WINDOW__.startDrag()       // drag from custom title bar
/// window.__RDESKTOP_WINDOW__.startResize('bottom-right')  // resize from edge
/// ```
pub struct WebViewRenderer {
    _config: AppConfig,
    ipc_handler: Option<Arc<dyn IpcHandler>>,
    pending_windows: RefCell<Vec<(u64, WindowConfig)>>,
    pending_ops: RefCell<Vec<PendingOp>>,
    next_window_id: RefCell<u64>,
}

impl WebViewRenderer {
    pub fn new(config: &AppConfig) -> Result<Self> {
        Ok(Self {
            _config: config.clone(),
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
    fn bridge_script() -> &'static str {
        r#"
        (function() {
            if (window.__RDESKTOP_BRIDGE__) return;
            window.__RDESKTOP_BRIDGE__ = true;
            window.__RDESKTOP_RESOLVE__ = {};

            // ── IPC Bridge ──────────────────────────────────────
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

            // ── Window Controls (frameless / custom title bar) ──
            window.__RDESKTOP_WINDOW__ = {
                minimize: function() {
                    window.ipc && window.ipc.postMessage(JSON.stringify({ __window__: true, action: 'minimize' }));
                },
                maximize: function() {
                    window.ipc && window.ipc.postMessage(JSON.stringify({ __window__: true, action: 'maximize' }));
                },
                close: function() {
                    window.ipc && window.ipc.postMessage(JSON.stringify({ __window__: true, action: 'close' }));
                },
                startDrag: function() {
                    window.ipc && window.ipc.postMessage(JSON.stringify({ __window__: true, action: 'start_drag' }));
                },
                startResize: function(edge) {
                    window.ipc && window.ipc.postMessage(JSON.stringify({ __window__: true, action: 'start_resize', edge: edge || 'bottom-right' }));
                },
                setFullscreen: function(fs) {
                    window.ipc && window.ipc.postMessage(JSON.stringify({ __window__: true, action: 'set_fullscreen', value: !!fs }));
                },
                isMaximized: false,
                isFullscreen: false
            };
        })();
        "#
    }

    /// Parse a window control message from the IPC handler.
    /// Returns Some(WindowAction) if it's a window command, None otherwise.
    fn parse_window_command(msg: &IpcMessage, rdesktop_id: u64) -> Option<WindowCommand> {
        // Check if the payload has __window__ flag
        if msg.payload.get("__window__").and_then(|v| v.as_bool()).unwrap_or(false) {
            let action = match msg.payload["action"].as_str()? {
                "minimize" => WindowAction::Minimize,
                "maximize" => WindowAction::Maximize,
                "close" => WindowAction::Close,
                "start_drag" => WindowAction::StartDrag,
                "start_resize" => {
                    let edge_str = msg.payload["edge"].as_str().unwrap_or("bottom-right");
                    let dir = match edge_str {
                        "top" => tao::window::ResizeDirection::North,
                        "bottom" => tao::window::ResizeDirection::South,
                        "left" => tao::window::ResizeDirection::West,
                        "right" => tao::window::ResizeDirection::East,
                        "top-left" => tao::window::ResizeDirection::NorthWest,
                        "top-right" => tao::window::ResizeDirection::NorthEast,
                        "bottom-left" => tao::window::ResizeDirection::SouthWest,
                        _ => tao::window::ResizeDirection::SouthEast,
                    };
                    WindowAction::StartResize(dir)
                }
                "set_fullscreen" => {
                    let val = msg.payload["value"].as_bool().unwrap_or(false);
                    WindowAction::SetFullscreen(val)
                }
                _ => return None,
            };
            return Some(WindowCommand { rdesktop_id, action });
        }
        None
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

    // ── Frameless / Window Controls ─────────────────────────────

    fn minimize_window(&self, window: WindowHandle) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::Minimize(window.id()));
        Ok(())
    }

    fn maximize_window(&self, window: WindowHandle) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::Maximize(window.id()));
        Ok(())
    }

    fn is_maximized(&self, _window: WindowHandle) -> Result<bool> {
        // This needs to be checked inside the event loop; return false for now.
        // In practice, the frontend can track this via window state events.
        Ok(false)
    }

    fn set_fullscreen(&self, window: WindowHandle, fullscreen: bool) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::SetFullscreen(window.id(), fullscreen));
        Ok(())
    }

    fn is_fullscreen(&self, _window: WindowHandle) -> Result<bool> {
        Ok(false)
    }

    fn start_drag(&self, window: WindowHandle) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::StartDrag(window.id()));
        Ok(())
    }

    fn start_resize(&self, window: WindowHandle, edge: ResizeEdge) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::StartResize(window.id(), to_tao_resize(edge)));
        Ok(())
    }

    fn set_decorations(&self, window: WindowHandle, decorations: bool) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::SetDecorations(window.id(), decorations));
        Ok(())
    }

    fn set_always_on_top(&self, window: WindowHandle, always: bool) -> Result<()> {
        self.pending_ops
            .borrow_mut()
            .push(PendingOp::SetAlwaysOnTop(window.id(), always));
        Ok(())
    }

    // ── Event Loop ──────────────────────────────────────────────

    fn run(mut self: Box<Self>) -> Result<()> {
        tracing::info!("Starting WebView event loop");

        let ipc_handler = self.ipc_handler.take();
        let pending_windows: Vec<(u64, WindowConfig)> =
            self.pending_windows.borrow_mut().drain(..).collect();
        let pending_ops: Vec<PendingOp> = self.pending_ops.borrow_mut().drain(..).collect();

        let ipc_response_queue: IpcResponseQueue = Arc::new(Mutex::new(Vec::new()));
        let ipc_queue_for_handler = ipc_response_queue.clone();

        // Window command queue for IPC-triggered window operations
        let window_cmd_queue: WindowCommandQueue = Arc::new(Mutex::new(Vec::new()));
        let window_cmd_queue_for_ipc = window_cmd_queue.clone();

        // Build a map of rdesktop_id -> first tao_id for the IPC handler
        // (the IPC handler needs to know which window to operate on)
        let first_window_id: Arc<Mutex<Option<u64>>> = Arc::new(Mutex::new(None));

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

                        let mut builder = WebViewBuilder::new()
                            .with_url("about:blank")
                            .with_devtools(cfg!(debug_assertions))
                            .with_initialization_script(Self::bridge_script());

                        // Wire up IPC handler
                        if let Some(ref handler) = ipc_handler {
                            let handler = handler.clone();
                            let queue = ipc_queue_for_handler.clone();
                            let win_queue = window_cmd_queue_for_ipc.clone();
                            let _first_id = first_window_id.clone();
                            let rd_id = *rdesktop_id;

                            builder = builder.with_ipc_handler(
                                move |req: wry::http::Request<String>| {
                                    let body = req.body();

                                    // Try parsing as window command first
                                    if let Ok(msg) = serde_json::from_str::<IpcMessage>(body) {
                                        if let Some(cmd) =
                                            WebViewRenderer::parse_window_command(&msg, rd_id)
                                        {
                                            if let Ok(mut q) = win_queue.lock() {
                                                q.push(cmd);
                                            }
                                            return;
                                        }

                                        // Regular IPC message
                                        let response = handler.handle(msg);
                                        if let Ok(json) = serde_json::to_string(&response) {
                                            if let Ok(mut q) = queue.lock() {
                                                q.push(json);
                                            }
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

                        if first_window_id.lock().unwrap().is_none() {
                            *first_window_id.lock().unwrap() = Some(*rdesktop_id);
                        }

                        tracing::info!(rdesktop_id = rdesktop_id, ?tao_id, "Window created");
                    }

                    // Process pending operations
                    for op in &pending_ops {
                        Self::apply_op(
                            op,
                            &windows,
                            &rdesktop_to_tao,
                        );
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

                Event::MainEventsCleared => {
                    // Drain IPC response queue
                    let responses: Vec<String> = {
                        let mut queue = ipc_response_queue.lock().unwrap();
                        queue.drain(..).collect()
                    };
                    for json in responses {
                        if let Some(entry) = windows.values().next() {
                            let escaped = json.replace('\\', "\\\\").replace('\'', "\\'");
                            let script = format!("window.__RDESKTOP_IPC__('{escaped}')");
                            let _ = entry.webview.evaluate_script(&script);
                        }
                    }

                    // Drain window command queue
                    let commands: Vec<WindowCommand> = {
                        let mut queue = window_cmd_queue.lock().unwrap();
                        queue.drain(..).collect()
                    };
                    for cmd in commands {
                        if let Some(tao_id) = rdesktop_to_tao.get(&cmd.rdesktop_id) {
                            if let Some(entry) = windows.get(tao_id) {
                                match cmd.action {
                                    WindowAction::Minimize => {
                                        entry.window.set_minimized(true);
                                    }
                                    WindowAction::Maximize => {
                                        let is_max = entry.window.is_maximized();
                                        entry.window.set_maximized(!is_max);
                                    }
                                    WindowAction::Close => {
                                        // Will be handled by CloseRequested
                                        // For now, just remove the window
                                    }
                                    WindowAction::StartDrag => {
                                        let _ = entry.window.drag_window();
                                    }
                                    WindowAction::StartResize(dir) => {
                                        let _ = entry.window.drag_resize_window(dir);
                                    }
                                    WindowAction::SetFullscreen(fs) => {
                                        if fs {
                                            entry.window.set_fullscreen(
                                                Some(tao::window::Fullscreen::Borderless(None)),
                                            );
                                        } else {
                                            entry.window.set_fullscreen(None);
                                        }
                                    }
                                }
                            }
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

impl WebViewRenderer {
    /// Apply a pending operation to a window.
    fn apply_op(
        op: &PendingOp,
        windows: &HashMap<WindowId, WindowEntry>,
        rdesktop_to_tao: &HashMap<u64, WindowId>,
    ) {
        match op {
            PendingOp::LoadUrl(rd_id, url) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    let _ = entry.webview.load_url(url);
                }
            }
            PendingOp::LoadHtml(rd_id, html) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    let _ = entry.webview.load_html(html);
                }
            }
            PendingOp::EvalScript(rd_id, script) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    let _ = entry.webview.evaluate_script(script);
                }
            }
            PendingOp::SetTitle(rd_id, title) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    entry.window.set_title(title);
                }
            }
            PendingOp::SetSize(rd_id, w, h) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    entry
                        .window
                        .set_inner_size(tao::dpi::LogicalSize::new(*w, *h));
                }
            }
            PendingOp::SetResizable(rd_id, resizable) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    entry.window.set_resizable(*resizable);
                }
            }
            PendingOp::SetVisible(rd_id, visible) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    entry.window.set_visible(*visible);
                }
            }
            PendingOp::SendToFrontend(rd_id, msg) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    let escaped = msg.replace('\\', "\\\\").replace('\'', "\\'");
                    let script = format!("window.__RDESKTOP_IPC__('{escaped}')");
                    let _ = entry.webview.evaluate_script(&script);
                }
            }
            PendingOp::Close(_rd_id) => {
                // Handled by the caller (removes from maps)
            }
            PendingOp::Minimize(rd_id) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    entry.window.set_minimized(true);
                }
            }
            PendingOp::Maximize(rd_id) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    let is_max = entry.window.is_maximized();
                    entry.window.set_maximized(!is_max);
                }
            }
            PendingOp::SetFullscreen(rd_id, fs) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    if *fs {
                        entry
                            .window
                            .set_fullscreen(Some(tao::window::Fullscreen::Borderless(None)));
                    } else {
                        entry.window.set_fullscreen(None);
                    }
                }
            }
            PendingOp::StartDrag(rd_id) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    let _ = entry.window.drag_window();
                }
            }
            PendingOp::StartResize(rd_id, dir) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    let _ = entry.window.drag_resize_window(*dir);
                }
            }
            PendingOp::SetDecorations(rd_id, decorations) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    entry.window.set_decorations(*decorations);
                }
            }
            PendingOp::SetAlwaysOnTop(rd_id, always) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    entry.window.set_always_on_top(*always);
                }
            }
        }
    }
}
