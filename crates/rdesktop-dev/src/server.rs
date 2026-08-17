//! Development server implementation.
//!
//! Serves the frontend as a local web page with hot reload and agent API.
//! This is the core of rdesktop's Agent-first development story.

use std::path::PathBuf;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::{Json, Router};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use rdesktop_core::config::DevConfig;

use crate::agent_api;

/// Shared state for the development server.
#[derive(Clone)]
pub struct DevServerState {
    /// The last captured DOM snapshot (for agent queries).
    pub last_dom_snapshot: Arc<RwLock<Option<String>>>,

    /// The last captured application state.
    pub last_app_state: Arc<RwLock<Option<serde_json::Value>>>,
}

/// Development server that serves the app in browser mode.
///
/// This is NOT the production renderer. It's a development tool that allows
/// AI agents (and humans) to interact with the app via a browser, using
/// mature browser automation tools like Playwright and Puppeteer.
pub struct DevServer {
    config: DevConfig,
    frontend_dir: PathBuf,
}

impl DevServer {
    /// Create a new DevServer.
    ///
    /// # Arguments
    /// * `config` - Development server configuration
    /// * `frontend_dir` - Path to the directory containing frontend assets (index.html, etc.)
    pub fn new(config: DevConfig, frontend_dir: PathBuf) -> Self {
        Self {
            config,
            frontend_dir,
        }
    }

    /// Start the development server.
    ///
    /// This will:
    /// 1. Serve the frontend assets from the configured directory
    /// 2. Inject the rdesktop bridge script (for IPC in browser mode)
    /// 3. Enable the Agent API endpoints if configured
    /// 4. Start hot-reload file watching if configured
    ///
    /// # Returns
    /// The URL where the server is listening (e.g., "http://localhost:1420")
    pub async fn start(&self) -> anyhow::Result<String> {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let url = format!("http://{}", addr);

        let state = DevServerState {
            last_dom_snapshot: Arc::new(RwLock::new(None)),
            last_app_state: Arc::new(RwLock::new(None)),
        };

        // Build the router
        let mut app = Router::new()
            // Agent API endpoints (always available in dev mode)
            .route("/__rdesktop__/agent/dom", get(agent_api::get_dom))
            .route("/__rdesktop__/agent/elements", get(agent_api::query_elements))
            .route("/__rdesktop__/agent/action", post(agent_api::execute_action))
            .route("/__rdesktop__/agent/state", get(agent_api::get_state))
            .route("/__rdesktop__/agent/ipc", post(agent_api::send_ipc))
            .route("/__rdesktop__/agent/screenshot", get(agent_api::take_screenshot))
            // Health check
            .route("/__rdesktop__/health", get(|| async { "ok" }))
            // Dev info
            .route("/__rdesktop__/info", get(dev_info))
            .layer(CorsLayer::permissive())
            .with_state(state);

        // Serve static frontend files
        if self.frontend_dir.exists() {
            app = app.fallback_service(
                tower_http::services::ServeDir::new(&self.frontend_dir)
                    .not_found_service(tower_http::services::ServeDir::new(
                        self.frontend_dir.join("index.html"),
                    )),
            );
        }

        tracing::info!("rdesktop dev server starting at {}", url);
        tracing::info!("Agent API available at {}/__rdesktop__/agent/", url);

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!("Listening on {}", addr);

        // Spawn the server
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Open browser if configured
        if self.config.open_browser {
            if let Err(e) = open::that(&url) {
                tracing::warn!("Failed to open browser: {}", e);
            }
        }

        Ok(url)
    }
}

/// Dev server info endpoint.
async fn dev_info() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "framework": "rdesktop",
        "mode": "development",
        "version": env!("CARGO_PKG_VERSION"),
        "agent_api": true,
        "endpoints": {
            "dom": "/__rdesktop__/agent/dom",
            "elements": "/__rdesktop__/agent/elements?selector=<css>",
            "action": "/__rdesktop__/agent/action",
            "state": "/__rdesktop__/agent/state",
            "ipc": "/__rdesktop__/agent/ipc",
            "screenshot": "/__rdesktop__/agent/screenshot",
        }
    }))
}
