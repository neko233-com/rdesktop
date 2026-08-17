use std::path::PathBuf;

use clap::{Parser, Subcommand};

use rdesktop_bundler::config::BundleTarget;
use rdesktop_bundler::windows::WindowsBundler;
use rdesktop_bundler::macos::MacOsBundler;
use rdesktop_bundler::linux::LinuxBundler;
use rdesktop_bundler::common::Bundler;
use rdesktop_core::config::AppConfig;

#[derive(Parser)]
#[command(name = "rdesktop", about = "rdesktop - A dual-engine Rust desktop framework")]
#[command(version, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new rdesktop project
    Init {
        /// Project name
        name: String,

        /// Use Chrome Embedded renderer instead of WebView
        #[arg(long)]
        chrome: bool,
    },

    /// Run the application in development mode
    Dev {
        /// Path to the project directory
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },

    /// Build the application for release
    Build {
        /// Path to the project directory
        #[arg(short, long, default_value = ".")]
        path: PathBuf,

        /// Build with Chrome renderer
        #[arg(long)]
        chrome: bool,
    },

    /// Bundle the application into an installer/package
    Bundle {
        /// Path to the project directory
        #[arg(short, long, default_value = ".")]
        path: PathBuf,

        /// Target format (nsis, wix, portable, dmg, app, appimage, deb, rpm)
        #[arg(short, long)]
        target: Option<String>,

        /// Bundle for all platform-appropriate targets
        #[arg(long)]
        all: bool,
    },

    /// Show information about the current project
    Info {
        /// Path to the project directory
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init { name, chrome } => cmd_init(&name, chrome),
        Commands::Dev { path } => cmd_dev(&path),
        Commands::Build { path, chrome } => cmd_build(&path, chrome),
        Commands::Bundle { path, target, all } => cmd_bundle(&path, target, all),
        Commands::Info { path } => cmd_info(&path),
    }
}

fn cmd_init(name: &str, chrome: bool) -> anyhow::Result<()> {
    let renderer = if chrome { "chrome" } else { "webview" };

    let config = format!(
        r#"[app]
identifier = "com.example.{name}"
name = "{name}"
version = "0.1.0"

[renderer]
kind = "{renderer}"

[window]
title = "{name}"
width = 1280
height = 720
resizable = true

[bundle]
windows_installer = "nsis"
linux_packages = ["appimage"]
"#,
        name = name,
        renderer = renderer,
    );

    let project_dir = PathBuf::from(name);
    std::fs::create_dir_all(&project_dir)?;
    std::fs::create_dir_all(project_dir.join("src"))?;
    std::fs::create_dir_all(project_dir.join("frontend"))?;

    std::fs::write(project_dir.join("rdesktop.toml"), config)?;

    // Write main.rs
    let main_rs = r#"use rdesktop_core::config::AppConfig;
use rdesktop_core::ipc::{IpcMessage, IpcResponse, FnIpcHandler};

fn main() -> anyhow::Result<()> {
    let config = AppConfig {
        identifier: "com.example.app".to_string(),
        name: "My App".to_string(),
        version: "0.1.0".to_string(),
        ..Default::default()
    };

    let handler = FnIpcHandler::new(|msg: IpcMessage| {
        IpcResponse {
            id: msg.id,
            success: true,
            data: serde_json::json!({ "echo": msg.payload }),
        }
    });

    App::builder(config)
        .with_ipc_handler(Box::new(handler))
        .build()?
        .run()
}
"#;
    std::fs::write(project_dir.join("src").join("main.rs"), main_rs)?;

    // Write index.html
    let index_html = r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>rdesktop App</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            min-height: 100vh;
            margin: 0;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
        }
        .container { text-align: center; }
        h1 { font-size: 3rem; margin-bottom: 0.5rem; }
        p { font-size: 1.2rem; opacity: 0.9; }
    </style>
</head>
<body>
    <div class="container">
        <h1>Hello rdesktop!</h1>
        <p>Your app is running.</p>
    </div>
    <script>
        // IPC bridge
        window.__RDESKTOP_IPC__ = function(message) {
            console.log('IPC from backend:', message);
        };

        async function invoke(cmd, payload) {
            return new Promise((resolve) => {
                const id = Math.random().toString(36).slice(2);
                window.__RDESKTOP_IPC_RESOLVE__ = window.__RDESKTOP_IPC_RESOLVE__ || {};
                window.__RDESKTOP_IPC_RESOLVE__[id] = resolve;
                window.ipc.postMessage(JSON.stringify({ id, cmd, payload }));
            });
        }
    </script>
</body>
</html>
"#;
    std::fs::write(project_dir.join("frontend").join("index.html"), index_html)?;

    println!("Created new rdesktop project: {}", name);
    println!("  Renderer: {}", renderer);
    println!("  Project dir: {}", project_dir.display());
    println!();
    println!("Next steps:");
    println!("  cd {}", name);
    println!("  rdesktop dev");

    Ok(())
}

fn cmd_dev(path: &PathBuf) -> anyhow::Result<()> {
    println!("Starting development server at {}", path.display());
    // In production, this would watch for file changes and rebuild
    Ok(())
}

fn cmd_build(_path: &PathBuf, chrome: bool) -> anyhow::Result<()> {
    let renderer = if chrome { "chrome" } else { "webview" };
    println!("Building with {} renderer...", renderer);

    // In production, this would:
    // 1. Build the Rust binary
    // 2. Bundle the frontend assets
    // 3. Link them together

    println!("Build complete!");
    Ok(())
}

fn cmd_bundle(path: &PathBuf, target: Option<String>, all: bool) -> anyhow::Result<()> {
    // Load config
    let config_path = path.join("rdesktop.toml");
    let config_str = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|_| {
            r#"
[app]
identifier = "com.example.app"
name = "App"
version = "0.1.0"
"#.to_string()
        });
    let config: toml::Value = toml::from_str(&config_str)?;

    let app_config = AppConfig {
        identifier: config["app"]["identifier"].as_str().unwrap_or("com.example.app").to_string(),
        name: config["app"]["name"].as_str().unwrap_or("App").to_string(),
        version: config["app"]["version"].as_str().unwrap_or("0.1.0").to_string(),
        ..Default::default()
    };

    let binary_path = path.join("target/release").join(&app_config.name);

    let targets = if all {
        BundleTarget::for_current_platform()
    } else if let Some(t) = target {
        vec![parse_target(&t)?]
    } else {
        BundleTarget::for_current_platform()
    };

    for t in &targets {
        println!("Bundling for {:?}...", t);
        let result = if cfg!(target_os = "windows") {
            WindowsBundler::new().bundle(&app_config, t, &binary_path)?
        } else if cfg!(target_os = "macos") {
            MacOsBundler::new().bundle(&app_config, t, &binary_path)?
        } else {
            LinuxBundler::new().bundle(&app_config, t, &binary_path)?
        };
        println!("  -> {} ({} bytes)", result.path.display(), result.size);
    }

    Ok(())
}

fn cmd_info(path: &PathBuf) -> anyhow::Result<()> {
    let config_path = path.join("rdesktop.toml");
    if !config_path.exists() {
        println!("No rdesktop.toml found at {}", path.display());
        return Ok(());
    }

    let config_str = std::fs::read_to_string(&config_path)?;
    let config: toml::Value = toml::from_str(&config_str)?;

    println!("rdesktop Project Info");
    println!("=====================");
    println!("Name:       {}", config["app"]["name"].as_str().unwrap_or("Unknown"));
    println!("Version:    {}", config["app"]["version"].as_str().unwrap_or("Unknown"));
    println!("Identifier: {}", config["app"]["identifier"].as_str().unwrap_or("Unknown"));
    println!("Renderer:   {}", config["renderer"]["kind"].as_str().unwrap_or("webview"));

    Ok(())
}

fn parse_target(s: &str) -> anyhow::Result<BundleTarget> {
    match s.to_lowercase().as_str() {
        "nsis" => Ok(BundleTarget::WindowsNsis),
        "wix" | "msi" => Ok(BundleTarget::WindowsWix),
        "portable" | "exe" => Ok(BundleTarget::WindowsPortable),
        "dmg" => Ok(BundleTarget::MacOsDmg),
        "app" => Ok(BundleTarget::MacOsApp),
        "appimage" => Ok(BundleTarget::LinuxAppImage),
        "deb" => Ok(BundleTarget::LinuxDeb),
        "rpm" => Ok(BundleTarget::LinuxRpm),
        _ => anyhow::bail!("Unknown bundle target: {}", s),
    }
}
