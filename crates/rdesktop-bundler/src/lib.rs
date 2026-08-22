//! rdesktop-bundler: Cross-platform packaging and installer generation.
//!
//! Solves Tauri's incomplete bundling story:
//! - Windows: Direct .exe output + NSIS installer + WiX MSI (all supported)
//! - macOS: .app bundle + DMG + notarization
//! - Linux: AppImage + .deb + .rpm
//!
//! Unlike Tauri, all formats work out of the box without external tool dependencies
//! where possible. NSIS and WiX are bundled as pre-built tools.

pub mod common;
pub mod config;
pub mod linux;
pub mod macos;
pub mod windows;

pub use common::{BundleResult, Bundler};
pub use config::BundleTarget;
