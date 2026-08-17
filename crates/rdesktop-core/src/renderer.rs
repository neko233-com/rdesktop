use crate::config::{RendererConfig, RendererKind as ConfigRendererKind, WindowConfig};
use crate::ipc::IpcHandler;
use crate::window::WindowHandle;

/// The kind of renderer being used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererKind {
    /// System WebView (WebView2/WebKit)
    WebView,
    /// Chrome Embedded (CDP)
    Chrome,
}

impl From<&ConfigRendererKind> for RendererKind {
    fn from(kind: &ConfigRendererKind) -> Self {
        match kind {
            ConfigRendererKind::WebView => Self::WebView,
            ConfigRendererKind::Chrome => Self::Chrome,
        }
    }
}

impl From<&RendererConfig> for RendererKind {
    fn from(config: &RendererConfig) -> Self {
        Self::from(&config.kind)
    }
}

/// Core trait that both WebView and Chrome backends must implement.
///
/// This provides a unified API for creating windows, loading content,
/// executing JavaScript, and handling IPC regardless of the underlying
/// rendering engine.
pub trait Renderer {
    /// Initialize the renderer.
    fn init(&mut self) -> crate::Result<()>;

    /// Create a new window with the given configuration.
    fn create_window(&mut self, config: &WindowConfig) -> crate::Result<WindowHandle>;

    /// Load a URL in the specified window.
    fn load_url(&self, window: WindowHandle, url: &str) -> crate::Result<()>;

    /// Load HTML content directly.
    fn load_html(&self, window: WindowHandle, html: &str) -> crate::Result<()>;

    /// Execute JavaScript in the specified window.
    fn eval_script(&self, window: WindowHandle, script: &str) -> crate::Result<()>;

    /// Set the IPC handler for messages from the frontend.
    fn set_ipc_handler(&mut self, handler: Box<dyn IpcHandler>);

    /// Send a message to the frontend JavaScript.
    fn send_to_frontend(&self, window: WindowHandle, message: &str) -> crate::Result<()>;

    /// Set the window title.
    fn set_title(&self, window: WindowHandle, title: &str) -> crate::Result<()>;

    /// Set the window size.
    fn set_size(&self, window: WindowHandle, width: u32, height: u32) -> crate::Result<()>;

    /// Set whether the window is resizable.
    fn set_resizable(&self, window: WindowHandle, resizable: bool) -> crate::Result<()>;

    /// Show or hide the window.
    fn set_visible(&self, window: WindowHandle, visible: bool) -> crate::Result<()>;

    /// Close a window.
    fn close_window(&mut self, window: WindowHandle) -> crate::Result<()>;

    // ── Frameless / Custom Title Bar ────────────────────────────────

    /// Minimize the window.
    fn minimize_window(&self, window: WindowHandle) -> crate::Result<()>;

    /// Toggle maximize/restore.
    fn maximize_window(&self, window: WindowHandle) -> crate::Result<()>;

    /// Check whether the window is currently maximized.
    fn is_maximized(&self, window: WindowHandle) -> crate::Result<bool>;

    /// Toggle fullscreen mode.
    fn set_fullscreen(&self, window: WindowHandle, fullscreen: bool) -> crate::Result<()>;

    /// Check whether the window is currently fullscreen.
    fn is_fullscreen(&self, window: WindowHandle) -> crate::Result<bool>;

    /// Begin an interactive window drag.
    ///
    /// Call this from a `mousedown` handler on a custom title bar element
    /// to allow the user to drag the window from any region.
    fn start_drag(&self, window: WindowHandle) -> crate::Result<()>;

    /// Begin an interactive window resize.
    ///
    /// `edge` specifies which edge/corner to resize from.
    fn start_resize(&self, window: WindowHandle, edge: ResizeEdge) -> crate::Result<()>;

    /// Set whether the window has OS decorations (title bar + borders).
    fn set_decorations(&self, window: WindowHandle, decorations: bool) -> crate::Result<()>;

    /// Set the window's always-on-top state.
    fn set_always_on_top(&self, window: WindowHandle, always: bool) -> crate::Result<()>;

    /// Enable or disable click-through: when enabled, pointer events fall
    /// through the window to whatever is behind it (used by wallpaper and
    /// click-through overlays). Applied at window creation by default; this
    /// method allows toggling it at runtime where the platform supports it.
    ///
    /// Default implementation is a no-op; backends override it to call the
    /// platform-specific window API.
    fn set_click_through(&self, _window: WindowHandle, _enabled: bool) -> crate::Result<()> {
        Ok(())
    }

    // ── Lifecycle ───────────────────────────────────────────────────

    /// Run the main event loop. This blocks until the application exits.
    fn run(self: Box<Self>) -> crate::Result<()>;

    /// Get the renderer kind.
    fn kind(&self) -> RendererKind;
}

/// Edge or corner for interactive resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}
