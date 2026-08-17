/// Target platform for bundling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleTarget {
    /// Windows NSIS installer (.exe)
    WindowsNsis,
    /// Windows WiX MSI installer (.msi)
    WindowsWix,
    /// Windows portable executable (single .exe)
    WindowsPortable,
    /// macOS .app bundle in DMG
    MacOsDmg,
    /// macOS .app bundle only
    MacOsApp,
    /// Linux AppImage
    LinuxAppImage,
    /// Linux .deb package
    LinuxDeb,
    /// Linux .rpm package
    LinuxRpm,
}

impl BundleTarget {
    /// Get all targets for the current platform.
    pub fn for_current_platform() -> Vec<Self> {
        if cfg!(target_os = "windows") {
            vec![Self::WindowsNsis, Self::WindowsPortable]
        } else if cfg!(target_os = "macos") {
            vec![Self::MacOsDmg]
        } else if cfg!(target_os = "linux") {
            vec![Self::LinuxAppImage]
        } else {
            vec![]
        }
    }

    /// Get the file extension for this target.
    pub fn extension(&self) -> &str {
        match self {
            Self::WindowsNsis => "exe",
            Self::WindowsWix => "msi",
            Self::WindowsPortable => "exe",
            Self::MacOsDmg => "dmg",
            Self::MacOsApp => "app",
            Self::LinuxAppImage => "AppImage",
            Self::LinuxDeb => "deb",
            Self::LinuxRpm => "rpm",
        }
    }
}
