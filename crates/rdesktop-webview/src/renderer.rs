use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use rdesktop_core::config::{AppConfig, WindowConfig};
use rdesktop_core::ipc::{IpcHandler, IpcMessage};
use rdesktop_core::renderer::{Renderer, RendererKind, ResizeEdge};
use rdesktop_core::window::WindowHandle;
use rdesktop_core::{RdesktopError, Result};

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Window, WindowBuilder, WindowId};
use wry::http::{Request, Response};
#[cfg(target_os = "windows")]
use wry::WebViewBuilderExtWindows;
use wry::{WebView, WebViewBuilder};

struct WindowEntry {
    window: Window,
    webview: WebView,
}

fn serve_asset(root: &Path, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let request_path = request.uri().path().trim_start_matches('/');
    let request_path = percent_encoding::percent_decode_str(request_path).decode_utf8_lossy();
    let relative = Path::new(request_path.as_ref());

    let invalid_path = relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    });
    if invalid_path {
        return asset_response(403, "text/plain; charset=utf-8", b"forbidden".to_vec());
    }

    let relative = if request_path.is_empty() {
        Path::new("index.html")
    } else {
        relative
    };
    let path = root.join(relative);
    match fs::read(&path) {
        Ok(bytes) => asset_response(200, content_type(&path), bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            asset_response(404, "text/plain; charset=utf-8", b"not found".to_vec())
        }
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "Failed to serve native asset");
            asset_response(
                500,
                "text/plain; charset=utf-8",
                b"asset read failed".to_vec(),
            )
        }
    }
}

fn asset_response(status: u16, content_type: &str, body: Vec<u8>) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Cache-Control", "no-cache")
        .body(Cow::Owned(body))
        .expect("valid native asset response")
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Wry's Windows backend exposes custom protocols through an HTTP origin.
/// Initial navigation applies this conversion internally, but subsequent
/// `WebView::load_url` calls do not. Keep runtime navigation consistent with
/// the initial page load so `rdesktop://localhost/...` works on WebView2 too.
fn native_asset_url(url: &str, has_asset_root: bool) -> String {
    #[cfg(target_os = "windows")]
    if has_asset_root {
        if let Some(rest) = url.strip_prefix("rdesktop://") {
            return format!("http://rdesktop.{rest}");
        }
    }

    url.to_string()
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
    asset_root: Option<PathBuf>,
    /// External outbox for native → frontend pushes (e.g. a Node extension
    /// host asking the UI to show a message or apply an editor edit). Drained
    /// every frame by the event loop, same as `ipc_response_queue`.
    outbox: Arc<Mutex<Vec<String>>>,
}

impl WebViewRenderer {
    pub fn new(config: &AppConfig) -> Result<Self> {
        Ok(Self {
            _config: config.clone(),
            ipc_handler: None,
            pending_windows: RefCell::new(Vec::new()),
            pending_ops: RefCell::new(Vec::new()),
            next_window_id: RefCell::new(1),
            asset_root: None,
            outbox: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Register a local directory as the renderer's `rdesktop://` asset root.
    ///
    /// Native WebViews cannot reliably load Vite module assets from
    /// `file://` or `NavigateToString()` because of origin and module-CORS
    /// rules. Serving the built frontend through a framework-owned protocol
    /// gives the page a stable origin on every desktop backend.
    pub fn set_asset_root(&mut self, root: impl Into<PathBuf>) -> Result<()> {
        let requested_root = root.into();
        let root = std::fs::canonicalize(&requested_root).map_err(|error| {
            RdesktopError::Config(format!(
                "asset root is not accessible ({}): {error}",
                requested_root.display()
            ))
        })?;
        if !root.is_dir() {
            return Err(RdesktopError::Config(format!(
                "asset root is not a directory: {}",
                root.display()
            )));
        }
        self.asset_root = Some(root);
        Ok(())
    }

    /// Attach an external outbox so other runtimes (e.g. a Node extension
    /// host) can push messages to the frontend. Each entry is a JSON string
    /// emitted as `window.__RDESKTOP_IPC__(<json>)`.
    pub fn set_outbox(&mut self, outbox: Arc<Mutex<Vec<String>>>) {
        self.outbox = outbox;
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
                    } else if (window.__RDESKTOP_PUSH__) {
                        // Unnamed push (e.g. extension host → UI event).
                        window.__RDESKTOP_PUSH__(data);
                    }
                } catch (e) {
                    console.error('rdesktop IPC error:', e);
                }
            };

            // ── Window Controls (frameless / custom title bar) ──
            var postWindowCommand = function(action, extra) {
                if (!window.ipc || !window.ipc.postMessage) return;
                var payload = extra || {};
                payload.__window__ = true;
                payload.action = action;
                window.ipc.postMessage(JSON.stringify({
                    id: 'window-' + Math.random().toString(36).slice(2),
                    cmd: 'rdesktop.window',
                    payload: payload
                }));
            };

            window.__RDESKTOP_WINDOW__ = {
                minimize: function() {
                    postWindowCommand('minimize');
                },
                maximize: function() {
                    postWindowCommand('maximize');
                },
                close: function() {
                    postWindowCommand('close');
                },
                startDrag: function() {
                    postWindowCommand('start_drag');
                },
                startResize: function(edge) {
                    postWindowCommand('start_resize', { edge: edge || 'bottom-right' });
                },
                setFullscreen: function(fs) {
                    postWindowCommand('set_fullscreen', { value: !!fs });
                },
                isMaximized: false,
                isFullscreen: false
            };
        })();
        "#
    }

    /// Parse a window control payload from the IPC handler.
    /// Returns Some(WindowCommand) if it's a window command, None otherwise.
    fn parse_window_payload(
        payload: &serde_json::Value,
        rdesktop_id: u64,
    ) -> Option<WindowCommand> {
        // Check if the payload has __window__ flag
        if payload
            .get("__window__")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let action = match payload["action"].as_str()? {
                "minimize" => WindowAction::Minimize,
                "maximize" => WindowAction::Maximize,
                "close" => WindowAction::Close,
                "start_drag" => WindowAction::StartDrag,
                "start_resize" => {
                    let edge_str = payload["edge"].as_str().unwrap_or("bottom-right");
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
                    let val = payload["value"].as_bool().unwrap_or(false);
                    WindowAction::SetFullscreen(val)
                }
                _ => return None,
            };
            return Some(WindowCommand {
                rdesktop_id,
                action,
            });
        }
        None
    }

    fn parse_window_command(msg: &IpcMessage, rdesktop_id: u64) -> Option<WindowCommand> {
        Self::parse_window_payload(&msg.payload, rdesktop_id)
    }

    fn parse_legacy_window_command(
        raw: &serde_json::Value,
        rdesktop_id: u64,
    ) -> Option<WindowCommand> {
        Self::parse_window_payload(raw, rdesktop_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_formal_window_command_envelope() {
        let message = IpcMessage {
            id: "window-test".to_string(),
            cmd: "rdesktop.window".to_string(),
            payload: serde_json::json!({
                "__window__": true,
                "action": "close"
            }),
        };

        assert!(WebViewRenderer::parse_window_command(&message, 1).is_some());
    }

    #[test]
    fn parses_legacy_top_level_window_command() {
        let raw = serde_json::json!({
            "__window__": true,
            "action": "minimize"
        });

        assert!(WebViewRenderer::parse_legacy_window_command(&raw, 1).is_some());
    }

    #[test]
    fn normalizes_runtime_asset_navigation_for_the_native_backend() {
        assert_eq!(
            native_asset_url("rdesktop://localhost/index.html", true),
            if cfg!(target_os = "windows") {
                "http://rdesktop.localhost/index.html"
            } else {
                "rdesktop://localhost/index.html"
            }
        );
        assert_eq!(
            native_asset_url("https://example.com", true),
            "https://example.com"
        );
        assert_eq!(
            native_asset_url("rdesktop://localhost/index.html", false),
            "rdesktop://localhost/index.html"
        );
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
        let webgpu_enabled = self._config.renderer.webgpu;
        let asset_root = self.asset_root.clone();
        let pending_windows: Vec<(u64, WindowConfig)> =
            self.pending_windows.borrow_mut().drain(..).collect();
        let pending_ops: Vec<PendingOp> = self.pending_ops.borrow_mut().drain(..).collect();

        let ipc_response_queue: IpcResponseQueue = Arc::new(Mutex::new(Vec::new()));
        let ipc_queue_for_handler = ipc_response_queue.clone();

        // External outbox for native → frontend pushes (Node extension host, etc.)
        let outbox_for_loop = self.outbox.clone();

        // ── Phase 2: global hotkeys & input hooks ───────────────────────
        // Wired through the shared outbox so the frontend receives them as
        // `window.__RDESKTOP_PUSH__` events (`rdesktop.globalHotkey` /
        // `rdesktop.globalInput`). Managers live for the whole event loop.
        let global_handler = rdesktop_core::PushHandler::new(self.outbox.clone());
        let _hotkey_manager = {
            let mgr = rdesktop_core::HotkeyManager::new(global_handler.clone());
            for (i, hc) in self._config.hotkeys.iter().enumerate() {
                if let Ok(hk) = hc.combo.parse::<rdesktop_core::Hotkey>() {
                    let id = i as u32 + 1;
                    if let Err(e) = mgr.register(id, &hk) {
                        tracing::warn!("failed to register hotkey {:?}: {}", hc.combo, e);
                    }
                } else {
                    tracing::warn!("invalid hotkey combo: {:?}", hc.combo);
                }
            }
            mgr
        };
        let _input_manager = if self._config.global_input.enabled {
            let mut inp = rdesktop_core::GlobalInput::new(global_handler.clone());
            if self._config.global_input.mouse_move {
                inp = inp.with_mouse_move(true);
            }
            match inp.start() {
                Ok(()) => Some(inp),
                Err(e) => {
                    tracing::warn!("failed to start global input: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Window command queue for IPC-triggered window operations
        let window_cmd_queue: WindowCommandQueue = Arc::new(Mutex::new(Vec::new()));
        let window_cmd_queue_for_ipc = window_cmd_queue.clone();

        // Build a map of rdesktop_id -> first tao_id for the IPC handler
        // (the IPC handler needs to know which window to operate on)
        let first_window_id: Arc<Mutex<Option<u64>>> = Arc::new(Mutex::new(None));

        let event_loop = EventLoopBuilder::new().build();
        let event_loop_proxy = event_loop.create_proxy();
        let mut windows: HashMap<WindowId, WindowEntry> = HashMap::new();
        let mut rdesktop_to_tao: HashMap<u64, WindowId> = HashMap::new();
        let mut tao_to_rdesktop: HashMap<WindowId, u64> = HashMap::new();

        event_loop.run(move |event, event_loop_target, control_flow| {
            *control_flow = ControlFlow::Wait;

            match event {
                Event::NewEvents(StartCause::Init) => {
                    let event_loop_proxy = event_loop_proxy.clone();
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
                            .with_window_icon(rdesktop_core::window_icon(window_config))
                            .build(event_loop_target)
                        {
                            Ok(w) => w,
                            Err(e) => {
                                tracing::error!("Failed to create window {}: {}", rdesktop_id, e);
                                continue;
                            }
                        };

                        let tao_id = window.id();

                        // Realize wallpaper/overlay/click-through window attributes.
                        rdesktop_core::apply_window_attributes(&window, window_config);

                        let mut builder = WebViewBuilder::new()
                            .with_url("about:blank")
                            .with_devtools(cfg!(debug_assertions))
                            .with_initialization_script(Self::bridge_script());

                        if let Some(root) = asset_root.clone() {
                            builder = builder.with_custom_protocol(
                                "rdesktop".to_string(),
                                move |_webview_id, request| serve_asset(&root, request),
                            );
                        }

                        // Enable WebGPU in the web context when requested, so the
                        // frontend can drive native shaders (wallpaper effects).
                        if window_config.transparent {
                            builder = builder.with_transparent(true);
                        }
                        // Enable WebGPU in the web context so the frontend can
                        // drive native shaders (wallpaper effects). On Windows
                        // WebView2/Edge needs the feature flag; on macOS WKWebView
                        // exposes WebGPU natively and on Linux WebKitGTK enables it
                        // via a different path, so the args are Windows-only.
                        #[cfg(target_os = "windows")]
                        if webgpu_enabled {
                            builder = builder.with_additional_browser_args(
                                "--enable-features=Vulkan,WebGPU --enable-unsafe-webgpu",
                            );
                        }

                        // Wire up IPC handler
                        if let Some(ref handler) = ipc_handler {
                            let handler = handler.clone();
                            let queue = ipc_queue_for_handler.clone();
                            let win_queue = window_cmd_queue_for_ipc.clone();
                            let wake_proxy = event_loop_proxy.clone();
                            let _first_id = first_window_id.clone();
                            let rd_id = *rdesktop_id;

                            builder =
                                builder.with_ipc_handler(move |req: wry::http::Request<String>| {
                                    let body = req.body();

                                    // Parse the JSON once so both the formal IPC envelope and
                                    // legacy top-level window commands remain supported.
                                    if let Ok(raw) = serde_json::from_str::<serde_json::Value>(body)
                                    {
                                        if let Some(cmd) =
                                            WebViewRenderer::parse_legacy_window_command(
                                                &raw, rd_id,
                                            )
                                        {
                                            if let Ok(mut q) = win_queue.lock() {
                                                q.push(cmd);
                                            }
                                            let _ = wake_proxy.send_event(());
                                            return;
                                        }

                                        if let Ok(msg) = serde_json::from_value::<IpcMessage>(raw) {
                                            // Formal window command or regular IPC message.
                                            if let Some(cmd) =
                                                WebViewRenderer::parse_window_command(&msg, rd_id)
                                            {
                                                if let Ok(mut q) = win_queue.lock() {
                                                    q.push(cmd);
                                                }
                                            } else {
                                                let response = handler.handle(msg);
                                                if let Ok(json) = serde_json::to_string(&response) {
                                                    if let Ok(mut q) = queue.lock() {
                                                        q.push(json);
                                                    }
                                                }
                                            }
                                            let _ = wake_proxy.send_event(());
                                        }
                                    }
                                });
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
                        Self::apply_op(op, &windows, &rdesktop_to_tao, asset_root.as_deref());
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
                            if let Ok(js) = serde_json::to_string(&json) {
                                let script = format!("window.__RDESKTOP_IPC__({js})");
                                let _ = entry.webview.evaluate_script(&script);
                            }
                        }
                    }

                    // Drain external outbox (native → frontend pushes)
                    let outbox_msgs: Vec<String> = {
                        let mut queue = outbox_for_loop.lock().unwrap();
                        queue.drain(..).collect()
                    };
                    for json in outbox_msgs {
                        if let Some(entry) = windows.values().next() {
                            if let Ok(js) = serde_json::to_string(&json) {
                                let script = format!("window.__RDESKTOP_IPC__({js})");
                                let _ = entry.webview.evaluate_script(&script);
                            }
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
                                        *control_flow = ControlFlow::Exit;
                                    }
                                    WindowAction::StartDrag => {
                                        let _ = entry.window.drag_window();
                                    }
                                    WindowAction::StartResize(dir) => {
                                        let _ = entry.window.drag_resize_window(dir);
                                    }
                                    WindowAction::SetFullscreen(fs) => {
                                        if fs {
                                            entry.window.set_fullscreen(Some(
                                                tao::window::Fullscreen::Borderless(None),
                                            ));
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
        asset_root: Option<&Path>,
    ) {
        match op {
            PendingOp::LoadUrl(rd_id, url) => {
                if let Some(entry) = rdesktop_to_tao.get(rd_id).and_then(|id| windows.get(id)) {
                    let native_url = native_asset_url(url, asset_root.is_some());
                    let _ = entry.webview.load_url(&native_url);
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
                    if let Ok(js) = serde_json::to_string(msg) {
                        let script = format!("window.__RDESKTOP_IPC__({js})");
                        let _ = entry.webview.evaluate_script(&script);
                    }
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
