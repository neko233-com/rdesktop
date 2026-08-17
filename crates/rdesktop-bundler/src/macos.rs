//! macOS bundler: .app bundle + DMG creation.

use std::path::{Path, PathBuf};

use rdesktop_core::config::AppConfig;

use crate::common::{BundleResult, Bundler};
use crate::config::BundleTarget;

pub struct MacOsBundler;

impl MacOsBundler {
    pub fn new() -> Self {
        Self
    }

    fn bundle_app(&self, config: &AppConfig, binary_path: &Path) -> anyhow::Result<BundleResult> {
        let app_name = format!("{}.app", config.name);
        let app_dir = PathBuf::from("target/release/bundle/macos").join(&app_name);
        let contents_dir = app_dir.join("Contents");
        let macos_dir = contents_dir.join("MacOS");
        let resources_dir = contents_dir.join("Resources");

        std::fs::create_dir_all(&macos_dir)?;
        std::fs::create_dir_all(&resources_dir)?;

        // Copy binary
        let binary_dest = macos_dir.join(&config.name);
        std::fs::copy(binary_path, &binary_dest)?;

        // Generate Info.plist
        let plist = self.generate_info_plist(config);
        std::fs::write(contents_dir.join("Info.plist"), plist)?;

        // Copy icon if specified
        if let Some(icon) = &config.bundle.icon {
            let icon_src = Path::new(icon);
            if icon_src.exists() {
                let icon_dest = resources_dir.join(format!("{}.icns", config.name));
                std::fs::copy(icon_src, &icon_dest)?;
            }
        }

        let size = dir_size(&app_dir)?;

        tracing::info!(path = %app_dir.display(), size, "macOS .app bundle created");

        Ok(BundleResult {
            path: app_dir,
            target: BundleTarget::MacOsApp,
            size,
        })
    }

    fn bundle_dmg(&self, config: &AppConfig, binary_path: &Path) -> anyhow::Result<BundleResult> {
        // First create the .app bundle
        let _app_result = self.bundle_app(config, binary_path)?;

        let dmg_name = format!("{}-{}.dmg", config.name, config.version);
        let dmg_path = PathBuf::from("target/release/bundle/macos").join(&dmg_name);

        // In production, this would use hdiutil or create-dmg to create the DMG
        tracing::info!(path = %dmg_path.display(), "DMG creation (stub)");

        std::fs::write(&dmg_path, b"DMG placeholder")?;
        let size = std::fs::metadata(&dmg_path)?.len();

        Ok(BundleResult {
            path: dmg_path,
            target: BundleTarget::MacOsDmg,
            size,
        })
    }

    fn generate_info_plist(&self, config: &AppConfig) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>{name}</string>
    <key>CFBundleIdentifier</key>
    <string>{identifier}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>{name}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>LSMinimumSystemVersion</key>
    <string>10.15</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSRequiresAquaSystemAppearance</key>
    <false/>
    {icon_entry}
    {copyright_entry}
</dict>
</plist>"#,
            name = config.name,
            identifier = config
                .bundle
                .macos_bundle_id
                .as_deref()
                .unwrap_or(&config.identifier),
            version = config.version,
            icon_entry = config
                .bundle
                .icon
                .as_ref()
                .map(|_| format!("<key>CFBundleIconFile</key>\n    <string>{}</string>", config.name))
                .unwrap_or_default(),
            copyright_entry = config
                .bundle
                .copyright
                .as_ref()
                .map(|c| format!("<key>NSHumanReadableCopyright</key>\n    <string>{c}</string>"))
                .unwrap_or_default(),
        )
    }
}

impl Bundler for MacOsBundler {
    fn bundle(&self, config: &AppConfig, target: &BundleTarget, binary_path: &PathBuf) -> anyhow::Result<BundleResult> {
        match target {
            BundleTarget::MacOsApp => self.bundle_app(config, binary_path),
            BundleTarget::MacOsDmg => self.bundle_dmg(config, binary_path),
            _ => anyhow::bail!("Invalid target for macOS bundler: {:?}", target),
        }
    }
}

fn dir_size(path: &Path) -> anyhow::Result<u64> {
    let mut size = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            size += dir_size(&entry.path())?;
        } else {
            size += entry.metadata()?.len();
        }
    }
    Ok(size)
}
