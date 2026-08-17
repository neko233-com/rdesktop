//! rdesktop-webview: WebView backend using wry + tao.
//!
//! Uses WebView2 on Windows, WebKit on macOS, and WebKitGTK on Linux.
//! This is the default lightweight renderer.

pub mod renderer;

pub use renderer::WebViewRenderer;
