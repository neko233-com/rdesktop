//! rdesktop-core: Core abstractions for the rdesktop dual-engine desktop framework.
//!
//! This crate defines the renderer trait that both WebView and Chrome Embedded
//! backends must implement, along with shared types for IPC, window management,
//! and application lifecycle.
//!
//! ## Architecture Overview
//!
//! rdesktop supports two modes:
//!
//! - **Native mode**: Opens a real window (WebView or Chrome Embedded)
//! - **Dev mode**: Serves the app as a web page for browser-based development
//!
//! Both modes share the same IPC and configuration types, so application code
//! works identically in development and production.

pub mod app;
pub mod config;
pub mod error;
pub mod event;
pub mod ipc;
pub mod renderer;
pub mod window;

pub use app::{App, AppBuilder, WindowContent};
pub use config::{AppConfig, BundleConfig, CommandConfig, DevConfig, RendererConfig, RendererKind as ConfigRendererKind, WindowConfig};
pub use error::{RdesktopError, Result};
pub use event::Event;
pub use ipc::{IpcHandler, IpcMessage, IpcResponse};
pub use renderer::{Renderer, RendererKind};
pub use window::WindowHandle;
