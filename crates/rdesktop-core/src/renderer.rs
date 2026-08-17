use crate::config::{RendererConfig, WindowConfig};
use crate::ipc::IpcHandler;
use crate::window::WindowHandle;

/// The kind of renderer being used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererKind {
    /// System WebView (WebView2/WebKit)
    WebView,
    /// Chrome Embedded Framework
    Chrome,
}

impl From<&RendererConfig> for RendererKind {
    fn from(config: &RendererConfig) -> Self {
        match config {
            RendererConfig::WebView => Self::WebView,
            RendererConfig::Chrome => Self::Chrome,
        }
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

    /// Run the main event loop. This blocks until the application exits.
    fn run(self: Box<Self>) -> crate::Result<()>;

    /// Get the renderer kind.
    fn kind(&self) -> RendererKind;
}
