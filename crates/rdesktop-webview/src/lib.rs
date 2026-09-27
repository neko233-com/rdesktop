//! rdesktop-webview: WebView backend using wry + tao.
//!
//! Uses WebView2 on Windows, WebKit on macOS, and WebKitGTK on Linux.
//! This is the default lightweight renderer.

pub mod renderer;
#[cfg(target_os = "windows")]
mod windows_drag;
#[cfg(target_os = "windows")]
mod windows_frame;

pub use renderer::WebViewRenderer;
