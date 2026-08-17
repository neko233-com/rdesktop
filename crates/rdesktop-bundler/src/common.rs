use std::path::PathBuf;

use rdesktop_core::config::AppConfig;

use crate::config::BundleTarget;

/// Result of a bundling operation.
#[derive(Debug)]
pub struct BundleResult {
    /// Path to the generated bundle
    pub path: PathBuf,

    /// The target that was built
    pub target: BundleTarget,

    /// Size in bytes
    pub size: u64,
}

/// Trait for platform-specific bundlers.
pub trait Bundler {
    /// Bundle the application for the given target.
    fn bundle(&self, config: &AppConfig, target: &BundleTarget, binary_path: &PathBuf) -> anyhow::Result<BundleResult>;
}
