//! Windows bundler: NSIS installer, WiX MSI, and portable EXE.
//!
//! NSIS and WiX require their respective compilers on PATH or the
//! RDESKTOP_NSIS/RDESKTOP_CANDLE/RDESKTOP_LIGHT overrides. WebView applications
//! require an installed WebView2 runtime; this bundler does not embed one.

use std::path::{Path, PathBuf};

use rdesktop_core::config::AppConfig;

use crate::common::{BundleResult, Bundler};
use crate::config::BundleTarget;

#[derive(Default)]
pub struct WindowsBundler;

impl WindowsBundler {
    pub fn new() -> Self {
        Self
    }

    /// Create a portable Windows executable.
    /// Copies the application's executable. Applications must embed their own
    /// frontend assets and resources before invoking the bundler.
    fn bundle_portable(
        &self,
        config: &AppConfig,
        binary_path: &Path,
    ) -> anyhow::Result<BundleResult> {
        validate_config(config)?;
        let output_dir = PathBuf::from("target/release/bundle/windows");
        std::fs::create_dir_all(&output_dir)?;

        let exe_name = format!("{}.exe", config.name.replace(' ', "_"));
        let output_path = output_dir.join(&exe_name);

        // Copy the binary
        std::fs::copy(binary_path, &output_path)?;

        let size = std::fs::metadata(&output_path)?.len();

        tracing::info!(path = %output_path.display(), size, "Portable EXE created");

        Ok(BundleResult {
            path: output_path,
            target: BundleTarget::WindowsPortable,
            size,
        })
    }

    /// Create an NSIS installer.
    /// NSIS (Nullsoft Scriptable Install System) produces a single .exe installer.
    fn bundle_nsis(&self, config: &AppConfig, binary_path: &Path) -> anyhow::Result<BundleResult> {
        let output_dir = PathBuf::from("target/release/bundle/nsis");
        std::fs::create_dir_all(&output_dir)?;

        let installer_name = format!(
            "{}-{}-setup.exe",
            config.name.replace(' ', "_"),
            config.version
        );
        let output_path = output_dir.join(&installer_name);

        // Generate NSIS script
        let nsis_script = self.generate_nsis_script(config, binary_path)?;
        let script_path = output_dir.join("installer.nsi");
        // NSIS otherwise interprets UTF-8 as the machine's ANSI code page.
        std::fs::write(&script_path, format!("\u{feff}{nsis_script}"))?;

        compile_installer(
            "RDESKTOP_NSIS",
            "makensis",
            &[script_path.as_os_str()],
            &output_path,
            b"MZ",
        )?;
        let size = std::fs::metadata(&output_path)?.len();

        Ok(BundleResult {
            path: output_path,
            target: BundleTarget::WindowsNsis,
            size,
        })
    }

    /// Create a WiX MSI installer.
    fn bundle_wix(&self, config: &AppConfig, binary_path: &Path) -> anyhow::Result<BundleResult> {
        let output_dir = PathBuf::from("target/release/bundle/wix");
        std::fs::create_dir_all(&output_dir)?;

        let installer_name = format!("{}-{}.msi", config.name.replace(' ', "_"), config.version);
        let output_path = output_dir.join(&installer_name);

        // Generate WiX XML manifest
        let wix_xml = self.generate_wix_manifest(config, binary_path)?;
        let manifest_path = output_dir.join("main.wxs");
        std::fs::write(&manifest_path, wix_xml)?;

        tracing::info!(path = %output_path.display(), "WiX manifest generated");

        let object_path = output_dir.join("main.wixobj");
        compile_installer(
            "RDESKTOP_CANDLE",
            "candle",
            &[
                "-nologo".as_ref(),
                "-arch".as_ref(),
                "x64".as_ref(),
                "-out".as_ref(),
                object_path.as_os_str(),
                manifest_path.as_os_str(),
            ],
            &object_path,
            b"<?xml",
        )?;
        compile_installer(
            "RDESKTOP_LIGHT",
            "light",
            &[
                "-nologo".as_ref(),
                "-out".as_ref(),
                output_path.as_os_str(),
                object_path.as_os_str(),
            ],
            &output_path,
            &[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1],
        )?;
        let size = std::fs::metadata(&output_path)?.len();

        Ok(BundleResult {
            path: output_path,
            target: BundleTarget::WindowsWix,
            size,
        })
    }

    fn generate_nsis_script(
        &self,
        config: &AppConfig,
        binary_path: &Path,
    ) -> anyhow::Result<String> {
        validate_config(config)?;
        let binary_path = compiler_path(binary_path)?;
        let binary = nsis_literal(&binary_path.to_string_lossy())?;
        let output = nsis_literal(
            &std::env::current_dir()?
                .join(format!(
                    "target/release/bundle/nsis/{}-{}-setup.exe",
                    config.name.replace(' ', "_"),
                    config.version
                ))
                .to_string_lossy(),
        )?;
        let (icon_line, icon_file, shortcut, icon_delete) = match installer_icon(config)? {
            Some(path) => {
                let path = nsis_literal(&path.to_string_lossy())?;
                (
                    format!("Icon \"{path}\"\nUninstallIcon \"{path}\""),
                    format!("File /oname=icon.ico \"{path}\""),
                    format!("CreateDirectory \"$SMPROGRAMS\\{name}\"\n    CreateShortCut \"$SMPROGRAMS\\{name}\\{name}.lnk\" \"$INSTDIR\\{name}.exe\" \"\" \"$INSTDIR\\icon.ico\"", name = config.name),
                    "Delete \"$INSTDIR\\icon.ico\"",
                )
            }
            None => (String::new(), String::new(), String::new(), ""),
        };
        let publisher = config.bundle.copyright.as_deref().unwrap_or(&config.name);
        let publisher = nsis_literal(publisher)?;

        Ok(format!(
            r#"
!include "MUI2.nsh"
!include "LogicLib.nsh"
Unicode true

Name "{name}"
OutFile "{output}"
InstallDir "$LOCALAPPDATA\Programs\{name}"
RequestExecutionLevel user

{icon}

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
    SetShellVarContext current
FunctionEnd

; A loaded image cannot be opened for exclusive writes. Request a normal exit
; instead of terminating the application or replacing it on reboot.
!macro CheckApplicationClosed
    ${{If}} ${{FileExists}} "$INSTDIR\{name}.exe"
        System::Call 'kernel32::CreateFileW(w "$INSTDIR\{name}.exe", i 0x40000000, i 0, p 0, i 3, i 0, p 0) p.r0'
        ${{If}} $0 == -1
            MessageBox MB_OK|MB_ICONEXCLAMATION "Exit {name} before continuing." /SD IDOK
            SetErrorLevel 10
            Abort
        ${{EndIf}}
        System::Call 'kernel32::CloseHandle(p r0)'
    ${{EndIf}}
!macroend

Section "Install"
    SetShellVarContext current
    !insertmacro CheckApplicationClosed
    SetOutPath "$INSTDIR"
    File "/oname={name}.exe" "{binary}"
    {icon_file}

    ; Create uninstaller
    WriteUninstaller "$INSTDIR\uninstall.exe"

    ; Start menu shortcuts
    {shortcut}

    ; Registry for Add/Remove Programs
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{name}" "DisplayName" "{name}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{name}" "UninstallString" '"$INSTDIR\uninstall.exe"'
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{name}" "DisplayVersion" "{version}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{name}" "Publisher" "{publisher}"
SectionEnd

Section "Uninstall"
    SetShellVarContext current
    !insertmacro CheckApplicationClosed
    Delete "$INSTDIR\{name}.exe"
    {icon_delete}
    Delete "$INSTDIR\uninstall.exe"
    RMDir "$INSTDIR"

    Delete "$SMPROGRAMS\{name}\{name}.lnk"
    RMDir "$SMPROGRAMS\{name}"

    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{name}"
SectionEnd
"#,
            name = config.name,
            version = config.version,
            icon = icon_line,
            binary = binary,
            output = output,
        ))
    }

    fn generate_wix_manifest(
        &self,
        config: &AppConfig,
        binary_path: &Path,
    ) -> anyhow::Result<String> {
        validate_config(config)?;
        let binary = xml_literal(&compiler_path(binary_path)?.to_string_lossy());
        let upgrade_code =
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, config.identifier.as_bytes())
                .hyphenated()
                .to_string();
        let component_guid = uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_DNS,
            format!("{}:windows-application", config.identifier).as_bytes(),
        );
        let (icon_definition, icon_file, shortcut) = match installer_icon(config)? {
            Some(path) => {
                let path = xml_literal(&path.to_string_lossy());
                (
                    format!("<Icon Id=\"AppIcon\" SourceFile=\"{path}\" /><Property Id=\"ARPPRODUCTICON\" Value=\"AppIcon\" />"),
                    format!("<File Id=\"AppIconFile\" Source=\"{path}\" Name=\"icon.ico\" />"),
                    format!("<Shortcut Id=\"AppShortcut\" Directory=\"ProgramMenuFolder\" Name=\"{name}\" Target=\"[INSTALLFOLDER]{name}.exe\" WorkingDirectory=\"INSTALLFOLDER\" Icon=\"AppIcon\" />", name = config.name),
                )
            }
            None => (String::new(), String::new(), String::new()),
        };
        Ok(format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi">
  <Product Id="*" Name="{name}" Language="1033" Version="{version}"
           Manufacturer="{manufacturer}" UpgradeCode="{upgrade_code}">

    <Package InstallerVersion="500" Compressed="yes" InstallScope="perUser" Platform="x64" />

    <MajorUpgrade DowngradeErrorMessage="A newer version is already installed." />
    <MediaTemplate EmbedCab="yes" />
    <Property Id="MSIRESTARTMANAGERCONTROL" Value="Disable" />
    <Property Id="REBOOT" Value="ReallySuppress" />
    {icon_definition}

    <Feature Id="ProductFeature" Title="{name}" Level="1">
      <ComponentGroupRef Id="ProductComponents" />
    </Feature>
  </Product>

  <Fragment>
    <Directory Id="TARGETDIR" Name="SourceDir">
      <Directory Id="LocalAppDataFolder">
        <Directory Id="INSTALLFOLDER" Name="{name}" />
      </Directory>
      <Directory Id="ProgramMenuFolder" />
    </Directory>
  </Fragment>

  <Fragment>
    <ComponentGroup Id="ProductComponents" Directory="INSTALLFOLDER">
      <Component Id="MainExecutable" Guid="{component_guid}" Win64="yes">
        <File Id="EXE" Source="{binary}" Name="{name}.exe" />
        {icon_file}
        {shortcut}
        <RegistryValue Root="HKCU" Key="Software\{identifier}" Name="InstallDir" Value="[INSTALLFOLDER]" Type="string" KeyPath="yes" />
        <RemoveFolder Id="RemoveApplicationDirectory" On="uninstall" />
      </Component>
    </ComponentGroup>
  </Fragment>
</Wix>"#,
            name = config.name,
            version = config.version,
            manufacturer = xml_literal(config.bundle.copyright.as_deref().unwrap_or(&config.name)),
            upgrade_code = upgrade_code,
            binary = binary,
            identifier = xml_literal(&config.identifier),
        ))
    }
}

impl Bundler for WindowsBundler {
    fn bundle(
        &self,
        config: &AppConfig,
        target: &BundleTarget,
        binary_path: &Path,
    ) -> anyhow::Result<BundleResult> {
        match target {
            BundleTarget::WindowsPortable => self.bundle_portable(config, binary_path),
            BundleTarget::WindowsNsis => self.bundle_nsis(config, binary_path),
            BundleTarget::WindowsWix => self.bundle_wix(config, binary_path),
            _ => anyhow::bail!("Invalid target for Windows bundler: {:?}", target),
        }
    }
}

fn validate_config(config: &AppConfig) -> anyhow::Result<()> {
    anyhow::ensure!(
        !config.name.is_empty()
            && config
                .name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || " ._-".contains(ch)),
        "unsafe Windows product name"
    );
    anyhow::ensure!(
        !config.version.is_empty()
            && config
                .version
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ".+-".contains(ch)),
        "unsafe Windows version"
    );
    Ok(())
}
fn installer_icon(config: &AppConfig) -> anyhow::Result<Option<PathBuf>> {
    config
        .bundle
        .icon
        .as_ref()
        .map(|path| {
            let path = compiler_path(Path::new(path))?;
            let bytes = std::fs::read(&path)?;
            anyhow::ensure!(
                path.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("ico"))
                    && bytes.len() >= 6
                    && bytes.starts_with(&[0, 0, 1, 0])
                    && u16::from_le_bytes([bytes[4], bytes[5]]) > 0,
                "Windows installers require a generated .ico; use rdesktop icons"
            );
            Ok(path)
        })
        .transpose()
}
fn nsis_literal(value: &str) -> anyhow::Result<String> {
    anyhow::ensure!(!value.contains(['\n', '\r', '"']), "unsafe NSIS path");
    Ok(value.replace('$', "$$"))
}
// WiX 3's .NET path handling rejects Rust's canonical verbatim prefixes even
// for short paths. Keep canonical resolution but use compiler-compatible DOS
// or UNC spelling; do not perform textual replacements elsewhere in a path.
fn compiler_path(path: &Path) -> anyhow::Result<PathBuf> {
    let resolved = std::fs::canonicalize(path)?;
    #[cfg(windows)]
    {
        let text = resolved.to_string_lossy();
        if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
            return Ok(PathBuf::from(format!(r"\\{unc}")));
        }
        if let Some(dos) = text.strip_prefix(r"\\?\") {
            return Ok(PathBuf::from(dos));
        }
    }
    Ok(resolved)
}
fn xml_literal(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn compile_installer(
    variable: &str,
    fallback: &str,
    args: &[&std::ffi::OsStr],
    output: &Path,
    magic: &[u8],
) -> anyhow::Result<()> {
    let executable = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
    // A failed compiler must never be mistaken for an older successful build.
    if output.exists() {
        std::fs::remove_file(output)?;
    }
    let mut command = std::process::Command::new(&executable);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let status = command.status().map_err(|e| {
        anyhow::anyhow!(
            "Could not run installer compiler {}: {e}; set {variable} to its executable path",
            fallback
        )
    })?;
    anyhow::ensure!(
        status.success(),
        "installer compiler {fallback} failed: {status}"
    );
    use std::io::Read;
    let file = std::fs::File::open(output)
        .map_err(|e| anyhow::anyhow!("compiler produced no artifact: {e}"))?;
    let size = file.metadata()?.len();
    let mut bytes = Vec::new();
    file.take(64).read_to_end(&mut bytes)?;
    let header = if magic == b"<?xml" {
        bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes)
    } else {
        &bytes
    };
    anyhow::ensure!(
        header.starts_with(magic) && size > magic.len() as u64,
        "compiler output is not the required Windows artifact"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifests_use_the_exact_binary_and_user_scope() {
        let config = AppConfig {
            identifier: "com.example.manifest".into(),
            name: "Manifest Test".into(),
            ..Default::default()
        };
        let path = std::env::current_exe().unwrap();
        let bundler = WindowsBundler::new();
        let nsis = bundler.generate_nsis_script(&config, &path).unwrap();
        assert!(nsis.contains("RequestExecutionLevel user"));
        assert!(nsis.contains("SetShellVarContext current"));
        assert!(nsis.contains(&path.file_name().unwrap().to_string_lossy().to_string()));
        let wix = bundler.generate_wix_manifest(&config, &path).unwrap();
        assert!(wix.contains("InstallScope=\"perUser\""));
        assert!(!wix.contains("00000000-0000-0000-0000-000000000000"));
    }
    #[test]
    fn rejects_script_injection() {
        let config = AppConfig {
            name: "bad\nSection".into(),
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
        assert!(nsis_literal("bad\npath").is_err());
    }

    /// Run explicitly on a Windows build machine with the three compiler
    /// overrides and RDESKTOP_TEST_INSTALLER_ICON pointing at a generated ICO.
    /// Compiles real installers without executing or installing them.
    #[test]
    #[cfg(windows)]
    #[ignore = "requires NSIS, WiX 3 and a generated Windows icon"]
    fn windows_compilers_produce_real_installers() {
        let mut config = AppConfig {
            identifier: "com.rdesktop.compiler-test".into(),
            name: "rdesktop Compiler Test".into(),
            ..Default::default()
        };
        config.bundle.icon =
            Some(std::env::var("RDESKTOP_TEST_INSTALLER_ICON").expect("icon path"));
        // Check XML escaping with metadata that is valid in neither raw XML
        // nor unescaped NSIS source.
        config.bundle.copyright = Some("rdesktop & Contributors".into());
        let bundler = WindowsBundler::new();
        let exe = std::env::current_exe().unwrap();
        for target in [BundleTarget::WindowsNsis, BundleTarget::WindowsWix] {
            let result = bundler.bundle(&config, &target, &exe).unwrap();
            assert!(result.size > 1024);
        }
    }
}
