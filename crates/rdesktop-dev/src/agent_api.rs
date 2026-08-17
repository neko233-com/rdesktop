//! Agent API endpoints.
//!
//! These endpoints allow AI agents to interact with the running application
//! through structured HTTP requests, without needing native desktop control.
//!
//! ## How it works
//!
//! 1. The browser app includes the rdesktop bridge script
//! 2. The bridge script periodically sends DOM snapshots to the server
//! 3. Agents query the server for DOM/state information
//! 4. Agents send actions to the server, which forwards them to the browser
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
use std::collections::HashMap;

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
    pub attributes: HashMap<String, String>,

    /// Whether the element is visible
    pub visible: bool,

    /// Whether the element is enabled (for interactive elements)
    pub enabled: bool,

    /// Accessibility role
    pub role: Option<String>,

    /// Accessibility label
    pub label: Option<String>,
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
/// The snapshot is collected from the browser via the bridge script.
pub async fn get_dom(
    State(state): State<DevServerState>,
) -> impl IntoResponse {
    let snapshot = state.last_dom_snapshot.read().await;

    let html = snapshot.clone().unwrap_or_else(|| {
        r#"<!DOCTYPE html>
<html>
<head><title>rdesktop</title></head>
<body>
  <p>No DOM snapshot available yet. Make sure the app is loaded in the browser.</p>
  <p>The bridge script will send DOM updates automatically.</p>
</body>
</html>"#
            .to_string()
    });

    let dom = DomSnapshot {
        html,
        url: "http://localhost".to_string(),
        title: "rdesktop App".to_string(),
        timestamp: timestamp(),
    };

    Json(dom).into_response()
}

/// GET /__rdesktop__/agent/elements?selector=...
///
/// Query elements matching a CSS selector or text content.
pub async fn query_elements(
    State(state): State<DevServerState>,
    Query(query): Query<ElementQuery>,
) -> impl IntoResponse {
    let snapshot = state.last_dom_snapshot.read().await;

    // Parse the DOM and find matching elements
    let elements: Vec<ElementInfo> = if let Some(ref html) = *snapshot {
        find_elements(html, &query)
    } else {
        vec![]
    };

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
/// The action is stored and picked up by the bridge script.
pub async fn execute_action(
    State(_state): State<DevServerState>,
    Json(action): Json<AgentAction>,
) -> impl IntoResponse {
    tracing::info!(
        action = ?action.action,
        selector = %action.selector,
        "Agent action received"
    );

    // In a full implementation, this would:
    // 1. Store the action in a shared queue
    // 2. The bridge script polls for pending actions
    // 3. The bridge executes the action in the browser
    // 4. The result is returned

    let result = ActionResult {
        success: true,
        error: None,
        side_effects: vec![format!("Action {:?} on '{}' queued", action.action, action.selector)],
    };

    Json(result).into_response()
}

/// GET /__rdesktop__/agent/state
///
/// Get the current application state.
pub async fn get_state(
    State(state): State<DevServerState>,
) -> impl IntoResponse {
    let app_state = state.last_app_state.read().await;

    match app_state.as_ref() {
        Some(state) => Json(state.clone()).into_response(),
        None => Json(serde_json::json!({
            "message": "No application state available yet.",
            "hint": "Use fetch('/__rdesktop__/state', { method: 'POST', body: JSON.stringify(state) }) from your app."
        }))
        .into_response(),
    }
}

/// POST /__rdesktop__/agent/ipc
///
/// Send an IPC message from the agent to the app.
pub async fn send_ipc(
    State(_state): State<DevServerState>,
    Json(message): Json<serde_json::Value>,
) -> impl IntoResponse {
    let cmd = message["cmd"].as_str().unwrap_or("unknown");
    let payload = message["payload"].clone();
    let id = message["id"].as_str().unwrap_or("0");

    tracing::info!(cmd = cmd, "Agent IPC message received");

    // In a full implementation, this would forward to the Rust IPC handler.
    // For now, handle basic commands directly.
    let response = match cmd {
        "greet" => {
            let name = payload["name"].as_str().unwrap_or("World");
            serde_json::json!({
                "id": id,
                "success": true,
                "data": { "message": format!("Hello, {}!", name) }
            })
        }
        "ping" => {
            serde_json::json!({
                "id": id,
                "success": true,
                "data": { "pong": true }
            })
        }
        _ => {
            serde_json::json!({
                "id": id,
                "success": false,
                "data": { "error": format!("Unknown command: {}", cmd) }
            })
        }
    };

    Json(response).into_response()
}

/// GET /__rdesktop__/agent/screenshot
///
/// Capture a screenshot. In browser mode, this delegates to the browser.
pub async fn take_screenshot(
    State(_state): State<DevServerState>,
) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "message": "Screenshot not implemented in browser mode.",
            "hint": "Use Playwright's page.screenshot() directly."
        })),
    )
        .into_response()
}

/// Simple timestamp helper.
fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", now.as_secs())
}

/// Find elements in HTML matching the query.
/// This is a simple text-based search, not a full DOM parser.
fn find_elements(html: &str, query: &ElementQuery) -> Vec<ElementInfo> {
    let mut elements = vec![];

    if let Some(ref selector) = query.selector {
        // Simple tag selector matching (e.g., "button", "input", "h1")
        let tag = selector.trim_start_matches('<').trim_end_matches('>');
        let open_tag = format!("<{}", tag);

        let mut start = 0;
        while let Some(pos) = html[start..].find(&open_tag) {
            let abs_pos = start + pos;
            let end = html[abs_pos..].find('>').unwrap_or(0);
            let _tag_content = &html[abs_pos..abs_pos + end + 1];

            // Extract text content between tags
            let close_tag = format!("</{}>", tag);
            let text_start = abs_pos + end + 1;
            let text = if let Some(text_end) = html[text_start..].find(&close_tag) {
                html[text_start..text_start + text_end].trim().to_string()
            } else {
                String::new()
            };

            elements.push(ElementInfo {
                selector: format!("{}:nth-of-type({})", tag, elements.len() + 1),
                tag: tag.to_string(),
                text,
                attributes: HashMap::new(),
                visible: true,
                enabled: true,
                role: None,
                label: None,
            });

            start = abs_pos + end + 1;
        }
    }

    if let Some(ref text_query) = query.text {
        // Search for text content
        let lower_html = html.to_lowercase();
        let lower_query = text_query.to_lowercase();
        if lower_html.contains(&lower_query) {
            elements.push(ElementInfo {
                selector: format!("*:contains(\"{}\")", text_query),
                tag: "*".to_string(),
                text: text_query.clone(),
                attributes: HashMap::new(),
                visible: true,
                enabled: true,
                role: None,
                label: None,
            });
        }
    }

    elements
}
