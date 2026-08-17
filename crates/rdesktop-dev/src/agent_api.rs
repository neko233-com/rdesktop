//! Agent API endpoints.
//!
//! These endpoints allow AI agents to interact with the running application
//! through structured HTTP requests, without needing native desktop control.
//!
//! ## Design Philosophy
//!
//! Instead of requiring agents to take screenshots and use vision models to
//! understand the UI, the Agent API provides direct DOM access and structured
//! state information. This is:
//!
//! - **Faster**: No screenshot encoding/decoding overhead
//! - **More reliable**: Exact element selectors, not pixel coordinates
//! - **More informative**: Full DOM tree, computed styles, accessibility info
//! - **Easier to test**: Standard HTTP endpoints, can be scripted

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::server::DevServerState;

/// Query parameters for element selection.
#[derive(Debug, Deserialize)]
pub struct ElementQuery {
    /// CSS selector to query elements
    pub selector: Option<String>,

    /// Text content to search for
    pub text: Option<String>,

    /// Role attribute to filter by
    pub role: Option<String>,
}

/// An action that an agent can execute on the UI.
#[derive(Debug, Deserialize, Serialize)]
pub struct AgentAction {
    /// The type of action
    pub action: ActionType,

    /// CSS selector of the target element
    pub selector: String,

    /// Value for type/fill actions
    pub value: Option<String>,

    /// Coordinates for scroll actions
    pub coordinates: Option<(f64, f64)>,
}

/// Types of actions agents can execute.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Click,
    DoubleClick,
    RightClick,
    Type,
    Fill,
    Clear,
    Scroll,
    Hover,
    Focus,
    Select,
}

/// Response from a DOM query.
#[derive(Debug, Serialize)]
pub struct DomSnapshot {
    /// The full HTML content
    pub html: String,

    /// The page URL
    pub url: String,

    /// The page title
    pub title: String,

    /// Timestamp of the snapshot
    pub timestamp: String,
}

/// Response from an element query.
#[derive(Debug, Serialize)]
pub struct ElementInfo {
    /// CSS selector that uniquely identifies this element
    pub selector: String,

    /// Tag name
    pub tag: String,

    /// Text content
    pub text: String,

    /// Element attributes
    pub attributes: std::collections::HashMap<String, String>,

    /// Whether the element is visible
    pub visible: bool,

    /// Whether the element is enabled (for interactive elements)
    pub enabled: bool,

    /// Bounding box (if available)
    pub bbox: Option<BoundingBox>,

    /// Accessibility role
    pub role: Option<String>,

    /// Accessibility label
    pub label: Option<String>,
}

/// Bounding box of an element.
#[derive(Debug, Serialize)]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Result of an action execution.
#[derive(Debug, Serialize)]
pub struct ActionResult {
    /// Whether the action succeeded
    pub success: bool,

    /// Error message if the action failed
    pub error: Option<String>,

    /// Any side effects (e.g., navigation that occurred)
    pub side_effects: Vec<String>,
}

/// GET /__rdesktop__/agent/dom
///
/// Returns a full DOM snapshot of the current page.
/// This is injected into the page via the rdesktop bridge script,
/// which periodically sends DOM state to the server.
pub async fn get_dom(
    State(state): State<DevServerState>,
) -> impl IntoResponse {
    let snapshot = state.last_dom_snapshot.read().await;

    match snapshot.as_ref() {
        Some(html) => {
            let dom = DomSnapshot {
                html: html.clone(),
                url: "http://localhost".to_string(), // TODO: track actual URL
                title: "rdesktop App".to_string(),
                timestamp: chrono_like_timestamp(),
            };
            Json(dom).into_response()
        }
        None => {
            let dom = DomSnapshot {
                html: "<html><body><p>No DOM snapshot available yet. Make sure the app is loaded.</p></body></html>".to_string(),
                url: "http://localhost".to_string(),
                title: "rdesktop App".to_string(),
                timestamp: chrono_like_timestamp(),
            };
            Json(dom).into_response()
        }
    }
}

/// GET /__rdesktop__/agent/elements?selector=...
///
/// Query elements matching a CSS selector or text content.
/// Returns structured information about each matching element.
pub async fn query_elements(
    State(_state): State<DevServerState>,
    Query(query): Query<ElementQuery>,
) -> impl IntoResponse {
    // In a full implementation, this would query the actual DOM via the bridge.
    // For now, return a placeholder.
    let elements: Vec<ElementInfo> = vec![];

    Json(serde_json::json!({
        "query": {
            "selector": query.selector,
            "text": query.text,
            "role": query.role,
        },
        "count": elements.len(),
        "elements": elements,
    }))
    .into_response()
}

/// POST /__rdesktop__/agent/action
///
/// Execute a UI action (click, type, scroll, etc.)
pub async fn execute_action(
    State(_state): State<DevServerState>,
    Json(action): Json<AgentAction>,
) -> impl IntoResponse {
    tracing::info!(
        action = ?action.action,
        selector = %action.selector,
        "Agent action received"
    );

    // In production, this would send the action to the browser via WebSocket
    let result = ActionResult {
        success: true,
        error: None,
        side_effects: vec![],
    };

    Json(result).into_response()
}

/// GET /__rdesktop__/agent/state
///
/// Get the current application state (data, not DOM).
/// This is useful for verifying that UI actions had the expected effect.
pub async fn get_state(
    State(state): State<DevServerState>,
) -> impl IntoResponse {
    let app_state = state.last_app_state.read().await;

    match app_state.as_ref() {
        Some(state) => Json(state.clone()).into_response(),
        None => Json(serde_json::json!({
            "message": "No application state available yet.",
            "hint": "Use window.__RDESKTOP_SET_STATE__(state) from your app to report state."
        }))
        .into_response(),
    }
}

/// POST /__rdesktop__/agent/ipc
///
/// Send an IPC message from the agent to the app backend.
/// This allows agents to invoke backend commands directly.
pub async fn send_ipc(
    State(_state): State<DevServerState>,
    Json(message): Json<serde_json::Value>,
) -> impl IntoResponse {
    tracing::info!(?message, "Agent IPC message received");

    // In production, this would forward to the Rust IPC handler
    Json(serde_json::json!({
        "success": true,
        "message": "IPC message forwarded (stub)"
    }))
    .into_response()
}

/// GET /__rdesktop__/agent/screenshot
///
/// Capture a screenshot of the current view.
/// In browser mode, this delegates to the browser's screenshot capability.
pub async fn take_screenshot(
    State(_state): State<DevServerState>,
) -> impl IntoResponse {
    // In production, this would use the browser's screenshot API
    // or CDP (Chrome DevTools Protocol) to capture the view
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "message": "Screenshot not yet implemented",
            "hint": "Use Playwright/Puppeteer's screenshot capability directly."
        })),
    )
        .into_response()
}

/// Simple timestamp helper (avoids chrono dependency).
fn chrono_like_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", now.as_secs())
}
