//! rdesktop-cef: Chrome Embedded Framework backend for cross-platform pixel-perfect rendering.
//!
//! This renderer embeds a full Chromium browser, ensuring identical rendering
//! across Windows, macOS, and Linux. The trade-off is a larger bundle size (~150MB).
//!
//! Architecture:
//! - CEF runs as a separate process (required by Chromium's multi-process model)
//! - Communication between Rust app and CEF happens via IPC (Unix socket / named pipe)
//! - The CEF process renders to a shared texture or window handle

pub mod renderer;

pub use renderer::CefRenderer;
