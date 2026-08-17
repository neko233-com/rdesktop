//! Development server implementation.
//!
//! Serves the frontend as a local web page with hot reload and Agent API.
//! This is the core of rdesktop's Agent-first development story.
//!
//! The dev server does three things:
//! 1. Serves frontend static files (HTML/CSS/JS)
//! 2. Injects the rdesktop bridge script for IPC
//! 3. Provides Agent API endpoints for AI agent interaction

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State as AxumState;
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

    /// The frontend directory path.
    pub frontend_dir: PathBuf,
}

/// Development server that serves the app in browser mode.
///
/// This is NOT the production renderer. It's a development tool that allows
/// AI agents (and humans) to interact with the app via a browser.
pub struct DevServer {
    config: DevConfig,
    frontend_dir: PathBuf,
}

impl DevServer {
    /// Create a new DevServer.
    pub fn new(config: DevConfig, frontend_dir: PathBuf) -> Self {
        Self {
            config,
            frontend_dir,
        }
    }

    /// Start the development server.
    ///
    /// Returns the URL where the server is listening.
    pub async fn start(&self) -> anyhow::Result<String> {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let url = format!("http://{}", addr);

        let state = DevServerState {
            last_dom_snapshot: Arc::new(RwLock::new(None)),
            last_app_state: Arc::new(RwLock::new(None)),
            frontend_dir: self.frontend_dir.clone(),
        };

        // Build the router
        let app = Router::new()
            // Agent API endpoints
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
            // State update from browser
            .route("/__rdesktop__/state", post(update_state))
            .route("/__rdesktop__/dom", post(update_dom))
            // Enable CORS for all routes
            .layer(CorsLayer::permissive())
            .with_state(state.clone())
            // Serve static files as fallback
            .fallback_service(tower_http::services::ServeDir::new(&self.frontend_dir));

        tracing::info!("rdesktop dev server starting at {}", url);
        if self.config.agent_mode {
            tracing::info!("Agent API available at {}/__rdesktop__/agent/", url);
        }

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!("Listening on {}", addr);

        // Spawn the server
        let server_url = url.clone();
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("Server error: {}", e);
            }
        });

        // Open browser if configured
        if self.config.open_browser {
            if let Err(e) = open::that(&url) {
                tracing::warn!("Failed to open browser: {}", e);
            }
        }

        Ok(server_url)
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

/// Update the stored DOM snapshot from the browser.
async fn update_dom(
    AxumState(state): AxumState<DevServerState>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let html = body["html"].as_str().unwrap_or("").to_string();
    let mut snapshot = state.last_dom_snapshot.write().await;
    *snapshot = Some(html);
    Json(serde_json::json!({ "ok": true }))
}

/// Update the stored app state from the browser.
async fn update_state(
    AxumState(state): AxumState<DevServerState>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let mut app_state = state.last_app_state.write().await;
    *app_state = Some(body);
    Json(serde_json::json!({ "ok": true }))
}
