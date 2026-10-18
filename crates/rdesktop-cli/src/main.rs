//! rdesktop CLI
//!
//! Commands:
//!   rdesktop init <name>     - Create a new project
//!   rdesktop dev             - Start dev server (browser mode)
//!   rdesktop build           - Build native binary
//!   rdesktop bundle          - Package into installer
//!   rdesktop icons           - Generate multi-size ICO/PNG assets

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use rdesktop_assets::generate_icons;
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

    /// Start native development (use --browser for the browser preview)
    Dev {
        /// Path to the project directory
        #[arg(short = 'd', long, default_value = ".")]
        path: PathBuf,

        /// Port for the dev server
        #[arg(short = 'P', long)]
        port: Option<u16>,

        /// Don't open the explicit browser preview automatically
        #[arg(long, requires = "browser")]
        no_open: bool,

        /// Real desktop process and WebView2 Agent API (default)
        #[arg(long, conflicts_with = "browser")]
        native: bool,

        /// Serve only the browser preview
        #[arg(long)]
        browser: bool,

        /// Start real native windows hidden; keeps the Windows WebView2 engine
        #[arg(long, conflicts_with = "browser")]
        headless: bool,

        /// Existing frontend dev server (for example Vite with HMR)
        #[arg(long, conflicts_with = "browser")]
        frontend_url: Option<String>,
    },

    /// Call a running native app without opening a browser
    Agent {
        /// Discovery JSON printed by `rdesktop dev`; contains a local credential
        #[arg(long)]
        session: PathBuf,
        /// info, windows, dom, elements, state, evaluate, action, cdp, events,
        /// accessibility, uia, audit, screenshot, native-windows, native-screenshot,
        /// window, reload, ipc, exit or recording[/start|/stop|/file]
        command: String,
        #[arg(long)]
        window: Option<u64>,
        /// JSON object (use --params-file for multiline input)
        #[arg(long, conflicts_with = "params_file")]
        params: Option<String>,
        #[arg(long)]
        params_file: Option<PathBuf>,
        /// Local official axe-core/axe.min.js to run an accessibility audit
        #[arg(long, requires = "command")]
        axe_source: Option<PathBuf>,
        /// Required for PNG/MP4; writes bytes without printing them
        #[arg(long)]
        output: Option<PathBuf>,
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
        /// Exact native executable; otherwise resolve Cargo's release target
        #[arg(long)]
        binary: Option<PathBuf>,
    },

    /// Show information about the current project
    Info {
        /// Path to the project directory
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },

    /// Generate Windows ICO and desktop PNG assets from one PNG source
    #[command(name = "icons", visible_alias = "icon")]
    Icons {
        /// Source PNG (transparent background is recommended)
        #[arg(short, long, value_name = "PNG")]
        input: PathBuf,

        /// Directory for generated assets
        #[arg(short, long, default_value = "resources/icons", value_name = "DIR")]
        output_dir: PathBuf,

        /// Output file stem, for example `app` -> `app.ico`
        #[arg(short, long, default_value = "app")]
        name: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init { name, chrome } => cmd_init(&name, chrome),
        Commands::Dev {
            path,
            port,
            no_open,
            native: _,
            browser,
            headless,
            frontend_url,
        } => {
            if browser {
                cmd_dev(&path, port, no_open).await
            } else {
                cmd_native_dev(&path, port, headless, frontend_url).await
            }
        }
        Commands::Agent {
            session,
            command,
            window,
            params,
            params_file,
            axe_source,
            output,
        } => {
            let params = if let Some(path) = params_file {
                std::fs::read_to_string(path)?
            } else {
                params.unwrap_or_else(|| "{}".into())
            };
            let mut params: serde_json::Value = serde_json::from_str(&params)?;
            anyhow::ensure!(params.is_object(), "parameters must be an object");
            if let Some(window) = window {
                params["window"] = window.to_string().into();
            }
            if let Some(source) = axe_source {
                anyhow::ensure!(command == "audit", "--axe-source is only used by audit");
                load_axe(&session, &params, &source).await?;
            }
            agent_request(&session, &command, params, output.as_deref()).await
        }
        Commands::Build { path, chrome } => cmd_build(&path, chrome),
        Commands::Bundle {
            path,
            target,
            all,
            binary,
        } => cmd_bundle(&path, target, all, binary),
        Commands::Info { path } => cmd_info(&path),
        Commands::Icons {
            input,
            output_dir,
            name,
        } => cmd_icons(&input, &output_dir, &name),
    }
}

fn cmd_init(name: &str, chrome: bool) -> anyhow::Result<()> {
    anyhow::ensure!(
        !name.is_empty()
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            && !name.starts_with(|c: char| c.is_ascii_digit()),
        "use a Rust package name containing letters, numbers, hyphens or underscores"
    );
    anyhow::ensure!(
        !Path::new(name).exists(),
        "project directory already exists"
    );
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
    let version = env!("CARGO_PKG_VERSION");
    let cargo = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow = "1"
serde_json = "1"
rdesktop-core = {{ git = "https://github.com/neko233-com/rdesktop.git", tag = "v{version}" }}
rdesktop-webview = {{ git = "https://github.com/neko233-com/rdesktop.git", tag = "v{version}", optional = true }}
rdesktop-cef = {{ git = "https://github.com/neko233-com/rdesktop.git", tag = "v{version}", optional = true }}

[features]
default = ["{renderer}"]
webview = ["dep:rdesktop-webview"]
chrome = ["dep:rdesktop-cef"]
native-agent = ["rdesktop-webview/native-agent"]
"#
    );
    std::fs::write(project_dir.join("Cargo.toml"), cargo)?;
    std::fs::write(project_dir.join(".gitignore"), "/target/\n/.rdesktop/\n")?;

    // Write main.rs
    let main_rs = include_str!("../templates/main.rs.txt");
    std::fs::write(project_dir.join("src").join("main.rs"), main_rs)?;

    // Keep the generated UI reviewable and packaged with the CLI.
    let index_html = include_str!("../templates/index.html");
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
async fn cmd_dev(path: &Path, port: Option<u16>, no_open: bool) -> anyhow::Result<()> {
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
    server.shutdown().await?;

    Ok(())
}

async fn cmd_native_dev(
    path: &Path,
    port: Option<u16>,
    headless: bool,
    frontend_url: Option<String>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        cfg!(windows),
        "native Agent API currently supports Windows x64 WebView2; use --browser elsewhere"
    );
    let path = std::fs::canonicalize(path)?;
    anyhow::ensure!(
        path.join("Cargo.toml").is_file(),
        "native dev requires Cargo.toml; use --browser for a frontend-only preview"
    );
    let (server, url) = if let Some(url) = frontend_url {
        validate_loopback_url(&url)?;
        (None, url)
    } else {
        let frontend = path.join("frontend");
        anyhow::ensure!(
            frontend.is_dir(),
            "frontend/ missing; provide --frontend-url for an existing Vite or other HMR server"
        );
        let server = rdesktop_dev::DevServer::new(
            rdesktop_core::config::DevConfig {
                host: "127.0.0.1".into(),
                port: port.unwrap_or(1420),
                open_browser: false,
                ..Default::default()
            },
            frontend,
        )
        .with_native_frontend();
        let url = server.start().await?;
        (Some(server), url)
    };
    let session = path
        .join(".rdesktop")
        .join(format!("agent-cli-{}.json", std::process::id()));
    println!("Native frontend: {url}");
    println!(
        "Native Agent session (available after startup): {}",
        session.display()
    );
    println!("Windows start hidden: {headless}");
    let mut command = tokio::process::Command::new("cargo");
    command
        .current_dir(&path)
        .args(["run", "--features", "native-agent"])
        .env("RDESKTOP_AGENT", "1")
        .env("RDESKTOP_AGENT_SESSION", &session)
        .env("RDESKTOP_DEV_URL", url)
        .env("RDESKTOP_HEADLESS", if headless { "1" } else { "0" });
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    // The renderer owns its shutdown. Never kill a resident application to end
    // dev mode, including when the CLI itself receives Ctrl+C.
    let result = async {
        let mut child = command.spawn()?;
        let status = tokio::select! {
            result = child.wait() => result?,
            _ = tokio::signal::ctrl_c() => {
                agent_request(&session,"exit",serde_json::json!({}),None).await?;
                tokio::time::timeout(std::time::Duration::from_secs(10),child.wait()).await
                    .map_err(|_|anyhow::anyhow!("native app is still exiting; it has not been forcibly terminated"))??
            }
        };
        anyhow::ensure!(status.success(),"native process failed: {status}");
        Ok(())
    }.await;
    if let Some(server) = server {
        server.shutdown().await?;
    }
    result
}

fn validate_loopback_url(url: &str) -> anyhow::Result<()> {
    let url = reqwest::Url::parse(url)?;
    anyhow::ensure!(
        url.scheme() == "http"
            && matches!(
                url.host_str(),
                Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
            )
            && url.username().is_empty()
            && url.password().is_none(),
        "development URL must be HTTP loopback without credentials"
    );
    Ok(())
}

fn agent_http_timeout(
    command: &str,
    params: &serde_json::Value,
) -> anyhow::Result<std::time::Duration> {
    Ok(if command == "ipc" {
        std::time::Duration::from_millis(
            rdesktop_dev::native::ipc_timeout_ms(params).map_err(anyhow::Error::msg)?
                + rdesktop_dev::native::IPC_COMPLETION_GRACE_MS
                + 3_000,
        )
    } else {
        std::time::Duration::from_secs(60)
    })
}

async fn agent_response(
    session_path: &Path,
    command: &str,
    params: serde_json::Value,
) -> anyhow::Result<reqwest::Response> {
    anyhow::ensure!(
        matches!(
            command,
            "info"
                | "windows"
                | "native-windows"
                | "native-screenshot"
                | "dom"
                | "elements"
                | "state"
                | "accessibility"
                | "uia"
                | "audit"
                | "evaluate"
                | "action"
                | "cdp"
                | "events"
                | "screenshot"
                | "window"
                | "reload"
                | "ipc"
                | "exit"
                | "recording"
                | "recording/start"
                | "recording/stop"
                | "recording/file"
        ),
        "unknown native agent command"
    );
    let session: rdesktop_dev::native::NativeAgentSession =
        serde_json::from_slice(&std::fs::read(session_path)?)?;
    validate_loopback_url(&session.url)?;
    let timeout = agent_http_timeout(command, &params)?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .build()?;
    let url = if command == "info" {
        format!("{}/__rdesktop__/info", session.url)
    } else {
        format!(
            "{}{}/{}",
            session.url,
            rdesktop_dev::native::NATIVE_AGENT_PREFIX,
            command
        )
    };
    let request = if matches!(command, "recording/file" | "info") {
        client.get(url)
    } else {
        client.post(url).json(&params)
    };
    let response = request.bearer_auth(session.token).send().await?;
    anyhow::ensure!(
        response.status().is_success(),
        "native agent {}: {}",
        response.status(),
        response.text().await?
    );
    Ok(response)
}

async fn agent_request(
    session_path: &Path,
    command: &str,
    params: serde_json::Value,
    output: Option<&Path>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !matches!(
            command,
            "screenshot" | "native-screenshot" | "recording/file"
        ) || output.is_some(),
        "--output is required for PNG/MP4"
    );
    let mut response = agent_response(session_path, command, params).await?;
    if let Some(output) = output {
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)
            .await?;
        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        println!("Saved {}", output.display());
    } else {
        let bytes = response.bytes().await?;
        anyhow::ensure!(
            bytes.len() <= 32 * 1024 * 1024,
            "native response exceeds 32 MiB"
        );
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::from_slice::<serde_json::Value>(&bytes)?)?
        );
    }
    Ok(())
}

async fn load_axe(session: &Path, params: &serde_json::Value, source: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        std::fs::metadata(source)?.len() <= 3 * 1024 * 1024,
        "axe source exceeds 3 MiB"
    );
    let source = std::fs::read_to_string(source)?;
    let key = format!(
        "__RDESKTOP_AXE_SOURCE_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let invoke = |expression: String| {
        let mut args = serde_json::json!({"expression":expression});
        if let Some(window) = params.get("window") {
            args["window"] = window.clone();
        }
        async move {
            let value = agent_response(session, "evaluate", args)
                .await?
                .json::<serde_json::Value>()
                .await?;
            Ok::<_, anyhow::Error>(value)
        }
    };
    let result = async {
        invoke(format!("(window[{}]='',null)", serde_json::json!(key))).await?;
        let chars: Vec<_> = source.chars().collect();
        for chunk in chars.chunks(16 * 1024) {
            let chunk: String = chunk.iter().collect();
            invoke(format!(
                "(window[{}]+={},null)",
                serde_json::json!(key),
                serde_json::json!(chunk)
            ))
            .await?;
        }
        let version = invoke(format!(
            "(()=>{{const source=window[{}];function install(frame){{frame.eval(source);for(let i=0;i<frame.frames.length;i++){{try{{install(frame.frames[i]);}}catch(_){{/* Cross-origin frames remain reported by axe for manual review. */}}}}}}install(window);return window.axe.version;}})()",
            serde_json::json!(key)
        ))
        .await?;
        anyhow::ensure!(version.is_string(), "axe-core did not initialize");
        Ok(())
    }
    .await;
    let _ = invoke(format!("(delete window[{}],null)", serde_json::json!(key))).await;
    result
}

fn cmd_build(path: &Path, chrome: bool) -> anyhow::Result<()> {
    let renderer = if chrome { "chrome" } else { "webview" };
    println!("Building with {} renderer...", renderer);
    let mut command = std::process::Command::new("cargo");
    command.current_dir(path).args(["build", "--release"]);
    if chrome {
        command.args(["--no-default-features", "--features", "chrome"]);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let status = command.status()?;
    anyhow::ensure!(status.success(), "native build failed: {status}");
    println!("Native release build complete.");
    Ok(())
}

fn cmd_bundle(
    path: &Path,
    target: Option<String>,
    all: bool,
    binary: Option<PathBuf>,
) -> anyhow::Result<()> {
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

    let mut app_config = AppConfig {
        identifier: config["app"]["identifier"]
            .as_str()
            .unwrap_or("com.example.app")
            .to_string(),
        name: config["app"]["name"].as_str().unwrap_or("App").to_string(),
        version: config["app"]["version"]
            .as_str()
            .unwrap_or("0.1.0")
            .to_string(),
        ..Default::default()
    };
    if let Some(bundle) = config.get("bundle") {
        app_config.bundle = bundle.clone().try_into()?;
    }
    if let Some(icon) = app_config.bundle.icon.as_mut() {
        let icon_path = PathBuf::from(&*icon);
        if icon_path.is_relative() {
            *icon = std::fs::canonicalize(path.join(icon_path))?
                .to_string_lossy()
                .into_owned();
        }
    }
    let binary_path = match binary {
        Some(binary) => std::fs::canonicalize(binary)?,
        None => resolve_release_binary(path)?,
    };

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

fn resolve_release_binary(path: &Path) -> anyhow::Result<PathBuf> {
    let manifest = std::fs::canonicalize(path.join("Cargo.toml"))?;
    let mut command = std::process::Command::new("cargo");
    command
        .current_dir(path)
        .args(["metadata", "--no-deps", "--format-version", "1"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output()?;
    anyhow::ensure!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let package = metadata["packages"]
        .as_array()
        .and_then(|packages| {
            packages.iter().find(|package| {
                package["manifest_path"]
                    .as_str()
                    .and_then(|path| std::fs::canonicalize(path).ok())
                    .is_some_and(|path| path == manifest)
            })
        })
        .ok_or_else(|| anyhow::anyhow!("select a Cargo package directory or pass --binary"))?;
    let binaries: Vec<_> = package["targets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|target| {
            target["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
        })
        .collect();
    let selected = package["default_run"]
        .as_str()
        .and_then(|name| {
            binaries
                .iter()
                .find(|target| target["name"] == name)
                .copied()
        })
        .or_else(|| {
            if binaries.len() == 1 {
                Some(binaries[0])
            } else {
                None
            }
        })
        .ok_or_else(|| {
            anyhow::anyhow!("multiple/no binary targets; set package.default-run or pass --binary")
        })?;
    let name = selected["name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("binary has no name"))?;
    let filename = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let target_dir = metadata["target_directory"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Cargo target directory unavailable"))?;
    Ok(std::fs::canonicalize(
        Path::new(target_dir).join("release").join(filename),
    )?)
}

fn cmd_info(path: &Path) -> anyhow::Result<()> {
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

fn cmd_icons(input: &PathBuf, output_dir: &PathBuf, name: &str) -> anyhow::Result<()> {
    let generated = generate_icons(input, output_dir, name).map_err(anyhow::Error::msg)?;
    println!("Generated {}", generated.ico.display());
    for (size, path) in generated.pngs {
        println!("Generated {size:>4}px {}", path.display());
    }
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

#[cfg(test)]
mod native_agent_deadline_tests {
    use super::agent_http_timeout;
    use serde_json::json;

    #[test]
    fn http_wait_covers_frontend_deadline_and_native_completion_grace() {
        assert_eq!(
            agent_http_timeout("ipc", &json!({})).unwrap().as_millis(),
            125_000
        );
        assert_eq!(
            agent_http_timeout("ipc", &json!({"timeout_ms":600_000}))
                .unwrap()
                .as_millis(),
            605_000
        );
        assert_eq!(
            agent_http_timeout("ipc", &json!({"timeout_ms":1}))
                .unwrap()
                .as_millis(),
            5_001
        );
        assert!(agent_http_timeout("ipc", &json!({"timeout_ms":0})).is_err());
        assert_eq!(
            agent_http_timeout("evaluate", &json!({"timeout_ms":0}))
                .unwrap()
                .as_secs(),
            60
        );
    }
}
