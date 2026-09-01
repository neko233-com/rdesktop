//! Application configuration for rdesktop.
//!
//! This module defines all configuration types used by the framework.
//! Configuration can be loaded from `rdesktop.toml` or constructed programmatically.
//!
//! ## Agent-First Development
//!
//! rdesktop supports a special `dev` mode designed for AI agent workflows:
//! - `rdesktop dev` starts a local HTTP server serving the frontend
//! - The app runs in the user's browser (localhost:PORT)
//! - AI agents can use Playwright/Puppeteer MCP tools to inspect and interact
//! - No native window needed during development
//! - Same code works in both dev (browser) and prod (native window) modes

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for the entire rdesktop application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Application identifier (reverse domain notation, e.g. "com.example.myapp")
    pub identifier: String,

    /// Application name displayed to users
    pub name: String,

    /// Application version
    pub version: String,

    /// Renderer to use (default: WebView)
    #[serde(default)]
    pub renderer: RendererConfig,

    /// Window configuration
    #[serde(default)]
    pub window: WindowConfig,

    /// Development server configuration (Agent-first)
    #[serde(default)]
    pub dev: DevConfig,

    /// Bundle configuration for packaging
    #[serde(default)]
    pub bundle: BundleConfig,

    /// Custom IPC command handlers
    #[serde(default)]
    pub commands: HashMap<String, CommandConfig>,

    /// Global hotkeys registered at the OS level (fire even when the window is
    /// unfocused). Each entry parses its `combo` via `Hotkey::from_str`
    /// (e.g. "Ctrl+Shift+K", "Alt+F4", "Meta+Space").
    #[serde(default)]
    pub hotkeys: Vec<HotkeyConfig>,

    /// Global input hook configuration (system-wide keyboard/mouse capture).
    /// Off by default — enable explicitly to observe raw input events.
    #[serde(default)]
    pub global_input: GlobalInputConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            identifier: "com.example.app".to_string(),
            name: "rdesktop App".to_string(),
            version: "0.1.0".to_string(),
            renderer: RendererConfig::default(),
            window: WindowConfig::default(),
            dev: DevConfig::default(),
            bundle: BundleConfig::default(),
            commands: HashMap::new(),
            hotkeys: Vec::new(),
            global_input: GlobalInputConfig::default(),
        }
    }
}

/// Renderer backend selection.
///
/// - `WebView`: Uses system WebView (WebView2/WebKit). Lightweight (~5MB).
/// - `Chrome`: Uses Chrome Embedded Framework. Pixel-perfect (~150MB).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RendererKind {
    /// Use system WebView (WebView2 on Windows, WebKit on macOS/Linux)
    /// Default, lightweight, ~5MB overhead
    #[serde(rename = "webview")]
    WebView,

    /// Use Chrome Embedded Framework for cross-platform pixel consistency
    /// Larger bundle (~150MB) but guaranteed identical rendering
    #[serde(rename = "chrome")]
    Chrome,
}

impl Default for RendererKind {
    fn default() -> Self {
        Self::WebView
    }
}

/// Renderer configuration (deserialized from `[renderer]` section in TOML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RendererConfig {
    /// Which renderer backend to use
    #[serde(default)]
    pub kind: RendererKind,

    /// Enable WebGPU in the web context so the frontend can drive native
    /// shaders (Wallpaper-Engine-style effects). Passed as Chromium flags.
    #[serde(default = "default_true")]
    pub webgpu: bool,

    /// Origins allowed to navigate the packaged WebView and invoke native IPC. An empty list
    /// preserves the framework's legacy unrestricted behavior; production applications should
    /// set their exact local asset origin (for example `rdesktop://localhost`).
    #[serde(default)]
    pub trusted_origins: Vec<String>,

    /// Maximum UTF-8 byte length accepted from one WebView IPC message.
    #[serde(default = "default_max_ipc_message_bytes")]
    pub max_ipc_message_bytes: usize,

    /// Maximum number of application IPC handlers that may execute concurrently.
    #[serde(default = "default_max_ipc_in_flight")]
    pub max_ipc_in_flight: usize,
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            kind: RendererKind::default(),
            webgpu: true,
            trusted_origins: Vec::new(),
            max_ipc_message_bytes: default_max_ipc_message_bytes(),
            max_ipc_in_flight: default_max_ipc_in_flight(),
        }
    }
}

/// Development server configuration.
///
/// This is the core of rdesktop's Agent-first development story.
/// During `rdesktop dev`, the app is served as a web page that AI agents
/// can interact with using mature browser automation tools (Playwright, Puppeteer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevConfig {
    /// Port for the development server (default: 1420, same as Tauri)
    #[serde(default = "default_dev_port")]
    pub port: u16,

    /// Host to bind to (default: "localhost")
    /// Use "0.0.0.0" to allow remote agent access
    #[serde(default = "default_dev_host")]
    pub host: String,

    /// Whether to open the browser automatically
    #[serde(default = "default_true")]
    pub open_browser: bool,

    /// Enable hot reload on file changes
    #[serde(default = "default_true")]
    pub hot_reload: bool,

    /// Enable the Agent MCP endpoint for structured interaction
    /// When enabled, exposes /__rdesktop__/agent/* endpoints for:
    ///   - DOM inspection (without screenshots)
    ///   - Element querying (CSS selectors, text content)
    ///   - Action execution (click, type, scroll)
    ///   - State snapshots (full DOM + computed styles)
    #[serde(default = "default_true")]
    pub agent_mode: bool,

    /// Enable the devtools overlay in browser mode
    #[serde(default = "default_true")]
    pub devtools: bool,
}

impl Default for DevConfig {
    fn default() -> Self {
        Self {
            port: default_dev_port(),
            host: default_dev_host(),
            open_browser: true,
            hot_reload: true,
            agent_mode: true,
            devtools: true,
        }
    }
}

/// Window layer/kind for native mode.
///
/// Extends Tauri v2's model with a `Wallpaper` layer (desktop-level,
/// click-through) and an `Overlay` layer (always-on-top HUD), enabling
/// Wallpaper-Engine-style and HUD/PIP scenarios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowKind {
    /// Standard application window (default)
    #[serde(rename = "normal")]
    Normal,

    /// Always-on-top overlay (HUD / PIP / floating toolbar)
    #[serde(rename = "overlay")]
    Overlay,

    /// Desktop wallpaper layer: sits behind icons, clicks pass through.
    /// Implies `click_through = true`.
    #[serde(rename = "wallpaper")]
    Wallpaper,
}

impl Default for WindowKind {
    fn default() -> Self {
        Self::Normal
    }
}

/// 32-bit RGBA window icon data used by native window backends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowIcon {
    /// Pixels in row-major RGBA order.
    pub rgba: Vec<u8>,
    /// Icon width in pixels.
    pub width: u32,
    /// Icon height in pixels.
    pub height: u32,
}

/// Window configuration for native mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    /// Window title
    #[serde(default = "default_title")]
    pub title: String,

    /// Window width in logical pixels
    #[serde(default = "default_width")]
    pub width: u32,

    /// Window height in logical pixels
    #[serde(default = "default_height")]
    pub height: u32,

    /// Whether the window is resizable
    #[serde(default = "default_true")]
    pub resizable: bool,

    /// Whether the window has decorations (title bar, borders)
    #[serde(default = "default_true")]
    pub decorations: bool,

    /// Whether the window is transparent
    #[serde(default)]
    pub transparent: bool,

    /// Whether the window is always on top
    #[serde(default)]
    pub always_on_top: bool,

    /// Whether to show in taskbar
    #[serde(default = "default_true")]
    pub visible_on_all_workspaces: bool,

    /// Optional native icon shown in the title bar, taskbar and background previews.
    #[serde(default)]
    pub icon: Option<WindowIcon>,

    /// Window layer/kind. See [`WindowKind`].
    #[serde(default)]
    pub kind: WindowKind,

    /// Click-through: pointer events fall through to whatever is behind the
    /// window. Required for wallpaper; can also be set explicitly on overlays.
    #[serde(default)]
    pub click_through: bool,

    /// Minimum window size (width, height)
    pub min_size: Option<(u32, u32)>,

    /// Maximum window size (width, height)
    pub max_size: Option<(u32, u32)>,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: default_title(),
            width: default_width(),
            height: default_height(),
            resizable: default_true(),
            decorations: default_true(),
            transparent: false,
            always_on_top: false,
            visible_on_all_workspaces: true,
            icon: None,
            kind: WindowKind::default(),
            click_through: false,
            min_size: None,
            max_size: None,
        }
    }
}

/// Bundle configuration for packaging the app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleConfig {
    /// Windows installer format: "nsis", "wix", or "both"
    #[serde(default = "default_windows_installer")]
    pub windows_installer: String,

    /// macOS bundle identifier
    pub macos_bundle_id: Option<String>,

    /// Linux package formats: "appimage", "deb", "rpm", or combinations
    #[serde(default = "default_linux_packages")]
    pub linux_packages: Vec<String>,

    /// Icon path (relative to project root)
    pub icon: Option<String>,

    /// Whether to code-sign on macOS
    #[serde(default)]
    pub macos_sign: bool,

    /// Whether to create a DMG on macOS
    #[serde(default = "default_true")]
    pub macos_dmg: bool,

    /// Copyright notice
    pub copyright: Option<String>,
}

impl Default for BundleConfig {
    fn default() -> Self {
        Self {
            windows_installer: default_windows_installer(),
            macos_bundle_id: None,
            linux_packages: default_linux_packages(),
            icon: None,
            macos_sign: false,
            macos_dmg: true,
            copyright: None,
        }
    }
}

/// Configuration for an IPC command handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandConfig {
    /// Command to execute
    pub command: String,

    /// Working directory (optional)
    pub working_dir: Option<String>,

    /// Environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// A global hotkey declared in configuration.
///
/// `id` is an optional stable identifier echoed back to the handler; `combo`
/// is parsed by [`crate::hotkeys::Hotkey::from_str`] (case-insensitive,
/// `+`-separated modifiers, e.g. `"Ctrl+Shift+K"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    /// Stable identifier for this hotkey (echoed to the handler). If omitted,
    /// the list index is used.
    #[serde(default)]
    pub id: Option<String>,

    /// Key combination string, e.g. "Alt+F4", "Meta+Shift+P".
    pub combo: String,

    /// Optional human-readable title (shown in UI lists).
    #[serde(default)]
    pub title: Option<String>,
}

/// Global input hook configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalInputConfig {
    /// Master switch for global input capture.
    #[serde(default)]
    pub enabled: bool,

    /// Capture keyboard events.
    #[serde(default = "default_true")]
    pub keyboard: bool,

    /// Capture mouse button events.
    #[serde(default = "default_true")]
    pub mouse: bool,

    /// Also forward high-frequency `MouseMove` events (off by default).
    #[serde(default)]
    pub mouse_move: bool,
}

impl Default for GlobalInputConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            keyboard: true,
            mouse: true,
            mouse_move: false,
        }
    }
}

// Default value functions for serde
fn default_title() -> String {
    "rdesktop App".to_string()
}
fn default_width() -> u32 {
    1280
}
fn default_height() -> u32 {
    720
}
fn default_true() -> bool {
    true
}
fn default_max_ipc_message_bytes() -> usize {
    1024 * 1024
}
fn default_max_ipc_in_flight() -> usize {
    32
}
fn default_dev_port() -> u16 {
    1420
}
fn default_dev_host() -> String {
    "localhost".to_string()
}
fn default_windows_installer() -> String {
    "nsis".to_string()
}
fn default_linux_packages() -> Vec<String> {
    vec!["appimage".to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_security_limits_default_when_omitted() {
        let renderer: RendererConfig = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(renderer.trusted_origins.is_empty());
        assert_eq!(renderer.max_ipc_message_bytes, 1024 * 1024);
        assert_eq!(renderer.max_ipc_in_flight, 32);
    }
}
