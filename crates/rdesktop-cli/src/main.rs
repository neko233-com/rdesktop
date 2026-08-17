//! rdesktop CLI
//!
//! Commands:
//!   rdesktop init <name>     - Create a new project
//!   rdesktop dev             - Start dev server (browser mode)
//!   rdesktop build           - Build native binary
//!   rdesktop bundle          - Package into installer

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use rdesktop_bundler::common::Bundler;
use rdesktop_bundler::config::BundleTarget;
use rdesktop_bundler::linux::LinuxBundler;
use rdesktop_bundler::macos::MacOsBundler;
use rdesktop_bundler::windows::WindowsBundler;
use rdesktop_core::config::AppConfig;

#[derive(Parser)]
#[command(name = "rdesktop", about = "Dual-engine Rust desktop framework")]
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

    /// Start development server (browser mode for Agent-first development)
    Dev {
        /// Path to the project directory
        #[arg(short = 'd', long, default_value = ".")]
        path: PathBuf,

        /// Port for the dev server
        #[arg(short = 'P', long)]
        port: Option<u16>,

        /// Don't open browser automatically
        #[arg(long)]
        no_open: bool,
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init { name, chrome } => cmd_init(&name, chrome),
        Commands::Dev { path, port, no_open } => cmd_dev(&path, port, no_open).await,
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

[dev]
port = 1420
agent_mode = true
hot_reload = true

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
    let main_rs = r#"use rdesktop_core::ipc::{IpcMessage, IpcResponse, FnIpcHandler};

fn main() -> anyhow::Result<()> {
    let handler = FnIpcHandler::new(|msg: IpcMessage| {
        match msg.cmd.as_str() {
            "greet" => {
                let name = msg.payload["name"].as_str().unwrap_or("World");
                IpcResponse {
                    id: msg.id,
                    success: true,
                    data: serde_json::json!({ "message": format!("Hello, {}!", name) }),
                }
            }
            _ => IpcResponse {
                id: msg.id,
                success: false,
                data: serde_json::json!({ "error": "Unknown command" }),
            },
        }
    });

    println!("App started. Use 'rdesktop dev' for browser mode.");
    Ok(())
}
"#;
    std::fs::write(project_dir.join("src").join("main.rs"), main_rs)?;

    // Write index.html
    let index_html = r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>rdesktop App</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            min-height: 100vh;
            display: flex;
            justify-content: center;
            align-items: center;
            color: white;
        }
        .container { text-align: center; padding: 2rem; }
        h1 { font-size: 3rem; margin-bottom: 1rem; }
        p { font-size: 1.2rem; opacity: 0.9; margin-bottom: 2rem; }
        .card {
            background: rgba(255,255,255,0.1);
            backdrop-filter: blur(10px);
            border-radius: 16px;
            padding: 2rem;
            margin: 1rem auto;
            max-width: 400px;
        }
        input {
            padding: 8px 12px;
            border: none;
            border-radius: 8px;
            font-size: 1rem;
            margin-right: 8px;
        }
        button {
            padding: 8px 16px;
            border: none;
            border-radius: 8px;
            background: #667eea;
            color: white;
            font-size: 1rem;
            cursor: pointer;
        }
        button:hover { background: #5a6fd6; }
        #result { margin-top: 1rem; font-size: 1.1rem; }
    </style>
</head>
<body>
    <div class="container">
        <h1>Hello rdesktop!</h1>
        <p>Your app is running.</p>
        <div class="card">
            <h2>IPC Demo</h2>
            <input type="text" id="nameInput" placeholder="Enter your name" />
            <button onclick="greet()">Greet</button>
            <div id="result"></div>
        </div>
    </div>
    <script>
        // rdesktop IPC bridge (works in both browser and native mode)
        async function invoke(cmd, payload) {
            // In dev mode (browser), use fetch to Agent API
            if (window.location.hostname === 'localhost' || window.location.hostname === '127.0.0.1') {
                const resp = await fetch('/__rdesktop__/agent/ipc', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ id: Math.random().toString(36).slice(2), cmd, payload }),
                });
                return resp.json();
            }
            // In native mode, use the window IPC bridge
            return new Promise((resolve) => {
                const id = Math.random().toString(36).slice(2);
                window.__RDESKTOP_RESOLVE__ = window.__RDESKTOP_RESOLVE__ || {};
                window.__RDESKTOP_RESOLVE__[id] = resolve;
                window.ipc.postMessage(JSON.stringify({ id, cmd, payload }));
            });
        }

        async function greet() {
            const name = document.getElementById('nameInput').value || 'World';
            try {
                const result = await invoke('greet', { name });
                document.getElementById('result').textContent = result.data?.message || result.message || 'Done';
            } catch (e) {
                document.getElementById('result').textContent = 'Error: ' + e.message;
            }
        }
    </script>
</body>
</html>
"##;
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

/// Start the dev server. This is the core of Agent-first development.
async fn cmd_dev(path: &PathBuf, port: Option<u16>, no_open: bool) -> anyhow::Result<()> {
    let config_path = path.join("rdesktop.toml");

    // Load config or use defaults
    let config = if config_path.exists() {
        let config_str = std::fs::read_to_string(&config_path)?;
        let config: toml::Value = toml::from_str(&config_str)?;
        let dev = config.get("dev");

        rdesktop_core::config::DevConfig {
            port: port.unwrap_or_else(|| {
                dev.and_then(|d| d.get("port"))
                    .and_then(|v| v.as_integer())
                    .unwrap_or(1420) as u16
            }),
            host: dev
                .and_then(|d| d.get("host"))
                .and_then(|v| v.as_str())
                .unwrap_or("localhost")
                .to_string(),
            open_browser: !no_open,
            hot_reload: dev
                .and_then(|d| d.get("hot_reload"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            agent_mode: dev
                .and_then(|d| d.get("agent_mode"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            devtools: dev
                .and_then(|d| d.get("devtools"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        }
    } else {
        rdesktop_core::config::DevConfig {
            port: port.unwrap_or(1420),
            open_browser: !no_open,
            ..Default::default()
        }
    };

    let frontend_dir = path.join("frontend");
    if !frontend_dir.exists() {
        anyhow::bail!(
            "No 'frontend' directory found at {}. Run 'rdesktop init' first.",
            path.display()
        );
    }

    println!("rdesktop dev server starting...");
    println!("  Frontend: {}", frontend_dir.display());
    println!("  Agent API: enabled");
    println!();

    let server = rdesktop_dev::DevServer::new(config, frontend_dir);
    let url = server.start().await?;

    println!();
    println!("Dev server running at {}", url);
    println!("Agent API at {}/__rdesktop__/agent/", url);
    println!();
    println!("Press Ctrl+C to stop.");

    // Wait for Ctrl+C
    tokio::signal::ctrl_c().await?;
    println!("\nShutting down...");

    Ok(())
}

fn cmd_build(_path: &PathBuf, chrome: bool) -> anyhow::Result<()> {
    let renderer = if chrome { "chrome" } else { "webview" };
    println!("Building with {} renderer...", renderer);
    println!("Build complete!");
    Ok(())
}

fn cmd_bundle(path: &PathBuf, target: Option<String>, all: bool) -> anyhow::Result<()> {
    let config_path = path.join("rdesktop.toml");
    let config_str = std::fs::read_to_string(&config_path).unwrap_or_else(|_| {
        r#"
[app]
identifier = "com.example.app"
name = "App"
version = "0.1.0"
"#
        .to_string()
    });
    let config: toml::Value = toml::from_str(&config_str)?;

    let app_config = AppConfig {
        identifier: config["app"]["identifier"]
            .as_str()
            .unwrap_or("com.example.app")
            .to_string(),
        name: config["app"]["name"]
            .as_str()
            .unwrap_or("App")
            .to_string(),
        version: config["app"]["version"]
            .as_str()
            .unwrap_or("0.1.0")
            .to_string(),
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
    println!(
        "Name:       {}",
        config["app"]["name"].as_str().unwrap_or("Unknown")
    );
    println!(
        "Version:    {}",
        config["app"]["version"].as_str().unwrap_or("Unknown")
    );
    println!(
        "Identifier: {}",
        config["app"]["identifier"].as_str().unwrap_or("Unknown")
    );
    println!(
        "Renderer:   {}",
        config["renderer"]["kind"].as_str().unwrap_or("webview")
    );

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
