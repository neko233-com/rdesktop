//! rdesktop-bundler: Cross-platform packaging and installer generation.
//!
//! Windows NSIS/MSI packaging invokes installed NSIS and WiX 3 compilers.
//! Compiler failures and invalid artifacts fail the build. The portable target
//! copies the application executable; applications embed their own assets.
//! Other platform backends have separate tooling requirements and validation.

pub mod common;
pub mod config;
pub mod linux;
pub mod macos;
pub mod windows;

pub use common::{BundleResult, Bundler};
pub use config::BundleTarget;
