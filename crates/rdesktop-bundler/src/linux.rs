//! Linux bundler: AppImage, .deb, and .rpm packages.

use std::path::{Path, PathBuf};

use rdesktop_core::config::AppConfig;

use crate::common::{BundleResult, Bundler};
use crate::config::BundleTarget;

pub struct LinuxBundler;

impl LinuxBundler {
    pub fn new() -> Self {
        Self
    }

    fn bundle_appimage(&self, config: &AppConfig, binary_path: &Path) -> anyhow::Result<BundleResult> {
        let appimage_name = format!("{}-{}.AppImage", config.name, config.version);
        let output_dir = PathBuf::from("target/release/bundle/linux");
        std::fs::create_dir_all(&output_dir)?;
        let output_path = output_dir.join(&appimage_name);

        // Create AppDir structure
        let appdir = output_dir.join(format!("{}.AppDir", config.name));
        let usr_bin = appdir.join("usr/bin");
        let usr_share = appdir.join("usr/share");
        std::fs::create_dir_all(&usr_bin)?;
        std::fs::create_dir_all(&usr_share)?;

        // Copy binary
        let binary_dest = usr_bin.join(&config.name);
        std::fs::copy(binary_path, &binary_dest)?;

        // Create .desktop file
        let desktop = self.generate_desktop_file(config);
        std::fs::write(appdir.join(format!("{}.desktop", config.name)), desktop)?;

        // Create AppRun symlink
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(format!("usr/bin/{}", config.name), appdir.join("AppRun"))?;
        }

        // In production, appimagetool would be called here
        tracing::info!(path = %output_path.display(), "AppImage created (stub)");

        std::fs::write(&output_path, b"AppImage placeholder")?;
        let size = std::fs::metadata(&output_path)?.len();

        Ok(BundleResult {
            path: output_path,
            target: BundleTarget::LinuxAppImage,
            size,
        })
    }

    fn bundle_deb(&self, config: &AppConfig, binary_path: &Path) -> anyhow::Result<BundleResult> {
        let deb_name = format!("{}_{}_amd64.deb", config.name.to_lowercase().replace(' ', "-"), config.version);
        let output_dir = PathBuf::from("target/release/bundle/linux");
        let output_path = output_dir.join(&deb_name);

        // Create deb structure
        let deb_dir = output_dir.join("deb");
        let debian_dir = deb_dir.join("DEBIAN");
        let usr_bin = deb_dir.join("usr/bin");
        std::fs::create_dir_all(&debian_dir)?;
        std::fs::create_dir_all(&usr_bin)?;

        // Copy binary
        std::fs::copy(binary_path, usr_bin.join(&config.name))?;

        // Generate control file
        let control = self.generate_deb_control(config);
        std::fs::write(debian_dir.join("control"), control)?;

        tracing::info!(path = %output_path.display(), ".deb package created (stub)");

        std::fs::write(&output_path, b"deb placeholder")?;
        let size = std::fs::metadata(&output_path)?.len();

        Ok(BundleResult {
            path: output_path,
            target: BundleTarget::LinuxDeb,
            size,
        })
    }

    fn bundle_rpm(&self, config: &AppConfig, _binary_path: &Path) -> anyhow::Result<BundleResult> {
        let rpm_name = format!("{}-{}-1.x86_64.rpm", config.name.to_lowercase().replace(' ', "-"), config.version);
        let output_dir = PathBuf::from("target/release/bundle/linux");
        let output_path = output_dir.join(&rpm_name);

        tracing::info!(path = %output_path.display(), ".rpm package created (stub)");

        std::fs::write(&output_path, b"rpm placeholder")?;
        let size = std::fs::metadata(&output_path)?.len();

        Ok(BundleResult {
            path: output_path,
            target: BundleTarget::LinuxRpm,
            size,
        })
    }

    fn generate_desktop_file(&self, config: &AppConfig) -> String {
        format!(
            r#"[Desktop Entry]
Type=Application
Name={name}
Exec={name}
Icon={name}
Categories=Utility;
Terminal=false
"#,
            name = config.name,
        )
    }

    fn generate_deb_control(&self, config: &AppConfig) -> String {
        format!(
            r#"Package: {name}
Version: {version}
Section: utils
Priority: optional
Architecture: amd64
Maintainer: {maintainer}
Description: {description}
"#,
            name = config.name.to_lowercase().replace(' ', "-"),
            version = config.version,
            maintainer = config.bundle.copyright.as_deref().unwrap_or("Unknown"),
            description = format!("{} desktop application", config.name),
        )
    }
}

impl Bundler for LinuxBundler {
    fn bundle(&self, config: &AppConfig, target: &BundleTarget, binary_path: &PathBuf) -> anyhow::Result<BundleResult> {
        match target {
            BundleTarget::LinuxAppImage => self.bundle_appimage(config, binary_path),
            BundleTarget::LinuxDeb => self.bundle_deb(config, binary_path),
            BundleTarget::LinuxRpm => self.bundle_rpm(config, binary_path),
            _ => anyhow::bail!("Invalid target for Linux bundler: {:?}", target),
        }
    }
}
