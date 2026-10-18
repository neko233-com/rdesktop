//! rdesktop-webview: WebView backend using wry + tao.
//!
//! Uses WebView2 on Windows, WebKit on macOS, and WebKitGTK on Linux.
//! This is the default lightweight renderer.

#[cfg(all(windows, debug_assertions, feature = "native-agent"))]
mod native_accessibility;
#[cfg(all(windows, debug_assertions, feature = "native-agent"))]
mod native_agent;
pub mod renderer;
pub mod runtime;
pub use runtime::{NativeEvent, NativeEventHandler, NativeRuntime};
pub use runtime::{NativeWindowOptions, WebViewSetup};
pub use {tao, wry};
#[cfg(target_os = "windows")]
mod windows_drag;
#[cfg(target_os = "windows")]
mod windows_frame;

pub use renderer::WebViewRenderer;
