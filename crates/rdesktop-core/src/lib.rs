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
pub mod global;
pub mod hotkeys;
pub mod input;
pub mod simulate;
pub mod ipc;
pub mod renderer;
pub mod window;
pub mod window_extras;

#[cfg(windows)]
pub mod hotkeys_win;
#[cfg(target_os = "macos")]
pub mod hotkeys_mac;
#[cfg(windows)]
pub mod input_win;
#[cfg(target_os = "macos")]
pub mod input_mac;
#[cfg(windows)]
pub mod simulate_win;
#[cfg(target_os = "macos")]
pub mod simulate_mac;

pub use app::{App, AppBuilder, WindowContent};
pub use config::{AppConfig, BundleConfig, CommandConfig, DevConfig, GlobalInputConfig, HotkeyConfig, RendererConfig, RendererKind as ConfigRendererKind, WindowConfig, WindowKind};
pub use error::{RdesktopError, Result};
pub use event::Event;
pub use global::{Outbox, PushHandler};
pub use hotkeys::{Hotkey, HotkeyHandler, HotkeyManager, Key, Modifiers};
pub use input::{GlobalInput, GlobalInputEvent, GlobalInputHandler, KeyState, MouseButton};
pub use simulate::InputSimulator;
pub use ipc::{IpcHandler, IpcMessage, IpcResponse};
pub use renderer::{Renderer, RendererKind, ResizeEdge};
pub use window::WindowHandle;
pub use window_extras::apply_window_attributes;
