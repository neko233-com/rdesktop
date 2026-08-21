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

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::server::{
    DevServerState, RecordingSnapshot, RecordingStatus, DEFAULT_RECORDING_MAX_DURATION_SECONDS,
    MAX_RECORDING_MAX_DURATION_SECONDS,
};

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
#[derive(Debug, Clone, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
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

/// Optional request body for starting a recording. The server owns the
/// recording identity; agents may safely send `{}` more than once.
#[derive(Debug, Deserialize, Default)]
pub struct RecordingStartRequest {
    pub fps: Option<u32>,
    /// Safety limit for forgotten recordings. Defaults to five minutes.
    pub max_duration_seconds: Option<u64>,
}

/// Optional session guard for stopping a recording.
#[derive(Debug, Deserialize, Default)]
pub struct RecordingStopRequest {
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RecordingStartedRequest {
    pub session_id: String,
    pub mime_type: String,
}

#[derive(Debug, Deserialize)]
pub struct RecordingCompleteRequest {
    pub session_id: String,
    pub mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RecordingErrorRequest {
    pub session_id: String,
    pub error: String,
}

/// GET /__rdesktop__/agent/dom
///
/// Returns a full DOM snapshot of the current page.
/// The snapshot is collected from the browser via the bridge script.
pub async fn get_dom(State(state): State<DevServerState>) -> impl IntoResponse {
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
    State(state): State<DevServerState>,
    Json(action): Json<AgentAction>,
) -> impl IntoResponse {
    tracing::info!(
        action = ?action.action,
        selector = %action.selector,
        "Agent action received"
    );

    state.pending_actions.lock().await.push(action.clone());

    let result = ActionResult {
        success: true,
        error: None,
        side_effects: vec![format!(
            "Action {:?} on '{}' queued",
            action.action, action.selector
        )],
    };

    Json(result).into_response()
}

/// GET /__rdesktop__/agent/action/pending
///
/// Drain actions queued by agents. The bridge polls this endpoint.
pub async fn pending_actions(State(state): State<DevServerState>) -> impl IntoResponse {
    let mut actions = state.pending_actions.lock().await;
    Json(std::mem::take(&mut *actions)).into_response()
}

/// GET /__rdesktop__/agent/state
///
/// Get the current application state.
pub async fn get_state(State(state): State<DevServerState>) -> impl IntoResponse {
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
pub async fn take_screenshot(State(_state): State<DevServerState>) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "message": "Screenshot not implemented in browser mode.",
            "hint": "Use Playwright's page.screenshot() directly."
        })),
    )
        .into_response()
}

/// GET /__rdesktop__/agent/recording
///
/// Return the one recording session owned by this dev server.
pub async fn get_recording(State(state): State<DevServerState>) -> impl IntoResponse {
    Json(state.recording.snapshot().await).into_response()
}

/// GET /__rdesktop__/agent/recording/poll
///
/// Alias used by the browser bridge to discover start/stop commands.
pub async fn poll_recording(State(state): State<DevServerState>) -> impl IntoResponse {
    Json(state.recording.snapshot().await).into_response()
}

/// POST /__rdesktop__/agent/recording/start
///
/// Start the single recording, or return the existing session when recording
/// is already active. This is intentionally idempotent.
pub async fn start_recording(
    State(state): State<DevServerState>,
    request: Option<Json<RecordingStartRequest>>,
) -> impl IntoResponse {
    let request = request.map(|Json(request)| request).unwrap_or_default();
    let fps = request.fps.unwrap_or(30).clamp(1, 60);
    let max_duration_seconds = request
        .max_duration_seconds
        .unwrap_or(DEFAULT_RECORDING_MAX_DURATION_SECONDS)
        .clamp(1, MAX_RECORDING_MAX_DURATION_SECONDS);
    let max_duration = std::time::Duration::from_secs(max_duration_seconds);
    match state.recording.start_with_options(fps, max_duration).await {
        Ok((recording, reused)) => {
            if !reused {
                if let Some(session_id) = recording.session_id.clone() {
                    let recording_store = state.recording.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(max_duration).await;
                        if let Err(error) = recording_store.stop(Some(&session_id)).await {
                            tracing::warn!(%error, "recording auto-stop failed");
                        }
                    });
                }
            }
            Json(serde_json::json!({
                "ok": true,
                "reused": reused,
                "auto_stop_seconds": max_duration_seconds,
                "recording": recording,
            }))
            .into_response()
        }
        Err(error) => json_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

/// POST /__rdesktop__/agent/recording/stop
///
/// Stop and finalize the native recorder, or request the browser bridge to
/// flush and finalize its MediaRecorder. Repeating this call is safe.
pub async fn stop_recording(
    State(state): State<DevServerState>,
    request: Option<Json<RecordingStopRequest>>,
) -> impl IntoResponse {
    let session_id = request.and_then(|Json(request)| request.session_id);
    match state.recording.stop(session_id.as_deref()).await {
        Ok(recording) => Json(serde_json::json!({
            "ok": true,
            "recording": recording,
        }))
        .into_response(),
        Err(error) => json_error(StatusCode::CONFLICT, error.to_string()),
    }
}

/// POST /__rdesktop__/agent/recording/started
///
/// Tell the server which browser MediaRecorder MIME type was selected.
pub async fn recording_started(
    State(state): State<DevServerState>,
    Json(request): Json<RecordingStartedRequest>,
) -> impl IntoResponse {
    match state
        .recording
        .mark_started(&request.session_id, &request.mime_type)
        .await
    {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(error) => json_error(StatusCode::CONFLICT, error.to_string()),
    }
}

/// POST /__rdesktop__/agent/recording/chunk
///
/// Append one MediaRecorder Blob to the single `.partial` file. Chunks are
/// serialized by the store so concurrent browser callbacks cannot interleave.
pub async fn recording_chunk(
    State(state): State<DevServerState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(session_id) = header_value(&headers, "x-rdesktop-recording-id") else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "missing recording session header".to_string(),
        );
    };
    if body.is_empty() {
        return Json(serde_json::json!({ "ok": true, "bytes": 0 })).into_response();
    }
    match state.recording.append_chunk(&session_id, &body).await {
        Ok(bytes) => Json(serde_json::json!({ "ok": true, "bytes": bytes })).into_response(),
        Err(error) => json_error(StatusCode::CONFLICT, error.to_string()),
    }
}

/// POST /__rdesktop__/agent/recording/complete
pub async fn recording_complete(
    State(state): State<DevServerState>,
    Json(request): Json<RecordingCompleteRequest>,
) -> impl IntoResponse {
    match state
        .recording
        .complete(&request.session_id, request.mime_type.as_deref())
        .await
    {
        Ok(recording) => recording_response(recording),
        Err(error) => json_error(StatusCode::CONFLICT, error.to_string()),
    }
}

/// POST /__rdesktop__/agent/recording/error
pub async fn recording_error(
    State(state): State<DevServerState>,
    Json(request): Json<RecordingErrorRequest>,
) -> impl IntoResponse {
    match state
        .recording
        .fail(&request.session_id, request.error)
        .await
    {
        Ok(recording) => recording_response(recording),
        Err(error) => json_error(StatusCode::CONFLICT, error.to_string()),
    }
}

/// GET /__rdesktop__/agent/recording/file
pub async fn recording_file(State(state): State<DevServerState>) -> impl IntoResponse {
    let recording = state.recording.snapshot().await;
    if recording.status != RecordingStatus::Completed {
        return json_error(
            StatusCode::NOT_FOUND,
            format!("recording is not complete: {:?}", recording.status),
        );
    }

    match tokio::fs::read(&recording.path).await {
        Ok(bytes) => axum::response::Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                recording.mime_type.as_deref().unwrap_or("video/webm"),
            )
            .header(
                header::CONTENT_DISPOSITION,
                if recording
                    .mime_type
                    .as_deref()
                    .map(|mime| mime.starts_with("video/mp4"))
                    .unwrap_or(false)
                {
                    "attachment; filename=recording.mp4"
                } else {
                    "attachment; filename=recording.webm"
                },
            )
            .body(axum::body::Body::from(bytes))
            .expect("recording response is valid")
            .into_response(),
        Err(error) => json_error(StatusCode::NOT_FOUND, error.to_string()),
    }
}

fn recording_response(recording: RecordingSnapshot) -> axum::response::Response {
    Json(serde_json::json!({
        "ok": recording.status == RecordingStatus::Completed,
        "recording": recording,
    }))
    .into_response()
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn json_error(status: StatusCode, error: String) -> axum::response::Response {
    (
        status,
        Json(serde_json::json!({ "ok": false, "error": error })),
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
