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

    /// Load HTML content with a document base URL for relative assets.
    ///
    /// WebView backends commonly implement `load_html` with an in-memory
    /// document (`NavigateToString` on WebView2). Such documents do not have
    /// a useful filesystem base, so relative Vite assets like
    /// `./assets/index.js` fail to load. This helper injects a `<base>` tag
    /// before delegating to the backend and keeps local-first renderers
    /// portable across WebView2, WKWebView, and WebKitGTK.
    fn load_html_with_base_url(
        &self,
        window: WindowHandle,
        html: &str,
        base_url: &str,
    ) -> crate::Result<()> {
        let html_with_base = html_with_base_url(html, base_url);
        self.load_html(window, &html_with_base)
    }

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

/// Add a document base URL while preserving a caller-provided base tag.
fn html_with_base_url(html: &str, base_url: &str) -> String {
    let lower = html.to_ascii_lowercase();
    if lower.contains("<base ") || lower.contains("<base>") {
        return html.to_string();
    }

    let escaped_base = base_url
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let normalized_base = if escaped_base.ends_with('/') {
        escaped_base
    } else {
        format!("{escaped_base}/")
    };
    let base_tag = format!(r#"<base href="{normalized_base}">"#);

    if let Some(head_start) = lower.find("<head") {
        if let Some(tag_end) = html[head_start..].find('>') {
            let insert_at = head_start + tag_end + 1;
            let mut output = String::with_capacity(html.len() + base_tag.len() + 1);
            output.push_str(&html[..insert_at]);
            output.push('\n');
            output.push_str(&base_tag);
            output.push_str(&html[insert_at..]);
            return output;
        }
    }

    format!("{base_tag}\n{html}")
}

#[cfg(test)]
mod tests {
    use super::html_with_base_url;

    #[test]
    fn injects_base_into_head() {
        let html = "<!doctype html><html><head><title>Test</title></head></html>";
        let result = html_with_base_url(html, "file:///C:/app/frontend");
        assert!(result.contains(r#"<base href="file:///C:/app/frontend/">"#));
        assert!(result.contains("<head>\n<base"));
    }

    #[test]
    fn preserves_existing_base() {
        let html = r#"<head><base href="custom://app/"></head>"#;
        assert_eq!(html_with_base_url(html, "file:///ignored"), html);
    }
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
