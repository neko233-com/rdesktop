//! rdesktop-cef: Chrome/Chromium backend for cross-platform pixel-perfect rendering.
//!
//! This renderer controls a Chrome/Chromium instance via the Chrome DevTools Protocol (CDP).
//! It provides identical rendering across Windows, macOS, and Linux since it uses the
//! same Chromium engine everywhere.
//!
//! ## Architecture
//!
//! - Chrome is launched as a headless subprocess
//! - Communication happens via CDP (WebSocket)
//! - Pages are created and controlled via the CDP API
//! - The IPC bridge is injected via `page.evaluate()`
//!
//! ## Requirements
//!
//! - Google Chrome, Microsoft Edge, or Chromium must be installed
//! - The renderer auto-detects the Chrome executable on the system
//!
//! ## Trade-offs vs WebView
//!
//! | Aspect | WebView | Chrome (CDP) |
//! |--------|---------|-------------|
//! | Bundle size | ~5MB | ~150MB (Chrome required) |
//! | Rendering | Platform-specific | Identical everywhere |
//! | Native window | Yes (tao) | Headless (screenshot-based) |
//! | Performance | Native | Slight overhead (CDP + screenshot) |

pub mod renderer;

pub use renderer::CefRenderer;
