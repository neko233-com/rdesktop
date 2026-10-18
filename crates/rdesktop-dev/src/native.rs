//! Authenticated, loopback-only transport for a real native development renderer.
//! Hosts must compile this integration out of distribution builds. This server
//! does not open a browser, inject an HTTP bridge, or expose a CDP network port.
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path as RoutePath, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{oneshot, Semaphore};

use crate::server::RecordingStore;

pub const NATIVE_AGENT_PREFIX: &str = "/__rdesktop__/agent";
const MAX_RESPONSE: usize = 32 * 1024 * 1024;

/// The renderer owns window identity, threading, protocol calls and input.
/// Calls run on bounded HTTP workers, never on the native event-loop thread.
pub trait NativeAgentBackend: Send + Sync + 'static {
    fn capabilities(&self) -> Value {
        json!({})
    }
    fn request(&self, command: &str, params: Value) -> Result<Value, String>;
}

#[derive(Clone, Serialize, Deserialize)]
pub struct NativeAgentSession {
    pub version: String,
    pub pid: u32,
    pub url: String,
    pub token: String,
    pub mode: String,
}

#[derive(Clone)]
struct AgentState {
    backend: Arc<dyn NativeAgentBackend>,
    session: NativeAgentSession,
    capacity: Arc<Semaphore>,
    recording: Arc<RecordingStore>,
}

/// Owns the listener and discovery file. Dropping it requests graceful shutdown;
/// the server thread finalizes its one bounded desktop recording before exiting.
pub struct NativeAgentServer {
    shutdown: Option<oneshot::Sender<()>>,
    finished: mpsc::Receiver<()>,
    discovery: PathBuf,
    token: String,
}

impl NativeAgentServer {
    pub fn start(backend: Arc<dyn NativeAgentBackend>, discovery: PathBuf) -> anyhow::Result<Self> {
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random).map_err(|e| anyhow::anyhow!("agent entropy: {e}"))?;
        let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let (shutdown, stop) = oneshot::channel();
        let (ready, started) = mpsc::sync_channel(1);
        let (done, finished) = mpsc::sync_channel(1);
        let server_token = token.clone();
        let server_discovery = discovery.clone();
        std::thread::Builder::new()
            .name("rdesktop-native-agent".into())
            .spawn(move || {
                let failed = ready.clone();
                let result = (|| -> anyhow::Result<()> {
                    let runtime = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2)
                        .enable_all()
                        .build()?;
                    runtime.block_on(async move {
                        let listener =
                            tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                                .await?;
                        let session = NativeAgentSession {
                            version: env!("CARGO_PKG_VERSION").into(),
                            pid: std::process::id(),
                            url: format!("http://{}", listener.local_addr()?),
                            token: server_token,
                            mode: "native-webview2".into(),
                        };
                        write_discovery(&server_discovery, &session)?;
                        let recording = Arc::new(RecordingStore::new_native(
                            server_discovery.with_extension("recording.mp4"),
                        ));
                        let state = AgentState {
                            backend,
                            session,
                            capacity: Arc::new(Semaphore::new(16)),
                            recording: recording.clone(),
                        };
                        ready.send(Ok(())).ok();
                        let result = axum::serve(listener, router(state))
                            .with_graceful_shutdown(async {
                                let _ = stop.await;
                            })
                            .await;
                        // Safe even when no recording was started.
                        recording.stop(None).await.ok();
                        result.map_err(Into::into)
                    })
                })();
                if let Err(error) = result {
                    let _ = failed.send(Err(error.to_string()));
                }
                let _ = done.send(());
            })?;
        match started.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(Self {
                shutdown: Some(shutdown),
                finished,
                discovery,
                token,
            }),
            Ok(Err(error)) => anyhow::bail!(error),
            Err(error) => anyhow::bail!("native agent startup: {error}"),
        }
    }

    pub fn shutdown(&mut self) {
        if let Some(stop) = self.shutdown.take() {
            let _ = stop.send(());
        }
        remove_owned_discovery(&self.discovery, &self.token);
    }

    /// Call after the native queue has stopped and windows have been dropped.
    pub fn finish(&mut self) {
        self.shutdown();
        let _ = self.finished.recv_timeout(Duration::from_secs(3));
    }
}

impl Drop for NativeAgentServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn write_discovery(path: &Path, session: &NativeAgentSession) -> anyhow::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Never replace another running instance's credentials.
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().unwrap_or_else(|| Path::new(".")))?;
    file.write_all(&serde_json::to_vec_pretty(session)?)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path).map_err(|error| error.error)?;
    Ok(())
}

fn remove_owned_discovery(path: &Path, token: &str) {
    let owned = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<NativeAgentSession>(&bytes).ok())
        .is_some_and(|session| session.pid == std::process::id() && session.token == token);
    if owned {
        let _ = std::fs::remove_file(path);
    }
}

fn router(state: AgentState) -> Router {
    Router::new()
        .route(
            "/__rdesktop__/health",
            get(|| async { Json(json!({"ok": true, "mode": "native"})) }),
        )
        .route("/__rdesktop__/info", get(info))
        .route("/__rdesktop__/agent/{command}", get(read).post(call))
        .route("/__rdesktop__/agent/recording/start", post(start_recording))
        .route("/__rdesktop__/agent/recording/stop", post(stop_recording))
        .route("/__rdesktop__/agent/recording/file", get(recording_file))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .with_state(state)
}

async fn authenticate(State(state): State<AgentState>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    // No browser-origin callers or DNS-rebinding hosts; no permissive CORS.
    if headers.contains_key(header::ORIGIN)
        || headers.get(header::HOST).and_then(|h| h.to_str().ok())
            != state.session.url.strip_prefix("http://")
    {
        return failure(
            StatusCode::FORBIDDEN,
            "native agent accepts loopback CLI requests only",
        );
    }
    let expected = format!("Bearer {}", state.session.token);
    if headers.get(header::AUTHORIZATION).map(|h| h.as_bytes()) != Some(expected.as_bytes()) {
        return failure(
            StatusCode::UNAUTHORIZED,
            "native agent authentication required",
        );
    }
    next.run(request).await
}

async fn info(State(state): State<AgentState>) -> Json<Value> {
    Json(
        json!({"framework": "rdesktop", "version": state.session.version,
        "pid": state.session.pid, "mode": state.session.mode,
        "commands": ["windows", "dom", "elements", "state", "accessibility", "uia", "audit", "evaluate", "action", "cdp", "events", "screenshot", "window", "reload", "ipc", "exit", "recording"],
        "capabilities": state.backend.capabilities(),
        "input": "CDP engine input and declared owned-window UIA patterns; external OS file-drop requires desktop verification",
        "screenshot": "native WebView content PNG; excludes OS dialogs and title bar"}),
    )
}

async fn read(
    State(state): State<AgentState>,
    RoutePath(command): RoutePath<String>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if command == "uia"
        && query
            .get("operation")
            .is_some_and(|operation| operation != "snapshot")
    {
        return failure(
            StatusCode::METHOD_NOT_ALLOWED,
            "use POST for targeted UIA operations",
        );
    }
    let params = query
        .into_iter()
        .map(|(key, value)| {
            let parsed = if command == "uia"
                && ["max_nodes", "max_depth", "patterns"].contains(&key.as_str())
            {
                serde_json::from_str(&value).unwrap_or(Value::String(value))
            } else {
                Value::String(value)
            };
            (key, parsed)
        })
        .collect();
    if !matches!(
        command.as_str(),
        "windows"
            | "dom"
            | "elements"
            | "state"
            | "accessibility"
            | "uia"
            | "events"
            | "screenshot"
            | "recording"
    ) {
        return failure(StatusCode::METHOD_NOT_ALLOWED, "use POST for this command");
    }
    dispatch(state, command, Value::Object(params)).await
}

async fn call(
    State(state): State<AgentState>,
    RoutePath(command): RoutePath<String>,
    Json(params): Json<Value>,
) -> Response {
    dispatch(state, command, params).await
}

async fn dispatch(state: AgentState, command: String, params: Value) -> Response {
    if !params.is_object() {
        return failure(StatusCode::BAD_REQUEST, "parameters must be an object");
    }
    if command == "recording" {
        return Json(json!(state.recording.snapshot().await)).into_response();
    }
    let Ok(permit) = state.capacity.clone().try_acquire_owned() else {
        return failure(StatusCode::TOO_MANY_REQUESTS, "native agent is busy");
    };
    let screenshot = command == "screenshot";
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        state.backend.request(&command, params)
    })
    .await;
    match result {
        Ok(Ok(value)) => {
            let bytes = if screenshot {
                match value["data"]
                    .as_str()
                    .and_then(|data| base64::engine::general_purpose::STANDARD.decode(data).ok())
                {
                    Some(bytes) => bytes,
                    None => {
                        return failure(StatusCode::BAD_GATEWAY, "renderer returned an invalid PNG")
                    }
                }
            } else {
                serde_json::to_vec(&value).unwrap_or_default()
            };
            if bytes.len() > MAX_RESPONSE {
                return failure(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "native response exceeds 32 MiB",
                );
            }
            Response::builder()
                .header(
                    header::CONTENT_TYPE,
                    if screenshot {
                        "image/png"
                    } else {
                        "application/json"
                    },
                )
                .header(header::CACHE_CONTROL, "no-store")
                .body(Body::from(bytes))
                .unwrap()
        }
        Ok(Err(error)) => failure(StatusCode::UNPROCESSABLE_ENTITY, &error),
        Err(_) => failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "native agent worker failed",
        ),
    }
}

async fn start_recording(
    State(state): State<AgentState>,
    Json(request): Json<crate::agent_api::RecordingStartRequest>,
) -> Response {
    let seconds = request.max_duration_seconds.unwrap_or(300).clamp(1, 3600);
    match state
        .recording
        .start_with_options(
            request.fps.unwrap_or(30).clamp(1, 60),
            Duration::from_secs(seconds),
        )
        .await
    {
        Ok((snapshot, reused)) => {
            Json(json!({"recording": snapshot, "reused": reused})).into_response()
        }
        Err(error) => failure(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn stop_recording(
    State(state): State<AgentState>,
    Json(request): Json<crate::agent_api::RecordingStopRequest>,
) -> Response {
    match state.recording.stop(request.session_id.as_deref()).await {
        Ok(snapshot) => Json(json!(snapshot)).into_response(),
        Err(error) => failure(StatusCode::CONFLICT, &error.to_string()),
    }
}

async fn recording_file(State(state): State<AgentState>) -> Response {
    let snapshot = state.recording.snapshot().await;
    if snapshot.status != crate::server::RecordingStatus::Completed {
        return failure(StatusCode::CONFLICT, "recording is not finalized");
    }
    // Stream the bounded artifact instead of loading a potentially large MP4.
    match tokio::fs::File::open(snapshot.path).await {
        Ok(file) => {
            use tokio_util::io::ReaderStream;
            Response::builder()
                .header(header::CONTENT_TYPE, "video/mp4")
                .body(Body::from_stream(ReaderStream::new(file)))
                .unwrap()
        }
        Err(_) => failure(StatusCode::NOT_FOUND, "recording file is unavailable"),
    }
}

fn failure(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;
    struct Backend;
    impl NativeAgentBackend for Backend {
        fn request(&self, command: &str, params: Value) -> Result<Value, String> {
            Ok(json!({"command":command,"params":params}))
        }
    }
    fn state() -> AgentState {
        AgentState {
            backend: Arc::new(Backend),
            session: NativeAgentSession {
                version: "test".into(),
                pid: 1,
                url: "http://127.0.0.1:1234".into(),
                token: "test-secret".into(),
                mode: "native".into(),
            },
            capacity: Arc::new(Semaphore::new(16)),
            recording: Arc::new(RecordingStore::new_native(
                std::env::temp_dir().join("unused-native-agent.mp4"),
            )),
        }
    }
    #[tokio::test]
    async fn rejects_browser_origins_rebinding_missing_credentials_and_oversized_requests() {
        for (host, auth, origin, expected) in [
            ("127.0.0.1:1234", "", None, 401),
            ("evil.example:1234", "Bearer test-secret", None, 403),
            (
                "127.0.0.1:1234",
                "Bearer test-secret",
                Some("http://evil.example"),
                403,
            ),
            ("127.0.0.1:1234", "Bearer test-secret", None, 200),
        ] {
            let mut request = Request::builder()
                .uri("/__rdesktop__/agent/windows")
                .header("host", host)
                .header("authorization", auth);
            if let Some(origin) = origin {
                request = request.header("origin", origin);
            }
            let response = router(state())
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), expected);
        }
        let response = router(state())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/__rdesktop__/agent/evaluate")
                    .header("host", "127.0.0.1:1234")
                    .header("authorization", "Bearer test-secret")
                    .header("content-type", "application/json")
                    .body(Body::from(" ".repeat(256 * 1024 + 1)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
    #[tokio::test]
    async fn forwards_window_identity_and_rejects_mutations_via_get() {
        let response = router(state())
            .oneshot(
                Request::builder()
                    .uri("/__rdesktop__/agent/dom?window=7")
                    .header("host", "127.0.0.1:1234")
                    .header("authorization", "Bearer test-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["params"]["window"], "7");
        let response = router(state())
            .oneshot(
                Request::builder()
                    .uri("/__rdesktop__/agent/window?window=7")
                    .header("host", "127.0.0.1:1234")
                    .header("authorization", "Bearer test-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn discovery_is_published_complete_without_replacing_another_session() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.json");
        let session = NativeAgentSession {
            version: "test".into(),
            pid: std::process::id(),
            url: "http://127.0.0.1:1234".into(),
            token: "owned-test-token".into(),
            mode: "native".into(),
        };
        write_discovery(&path, &session).unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(serde_json::from_slice::<NativeAgentSession>(&saved).is_ok());
        assert!(write_discovery(&path, &session).is_err());
        remove_owned_discovery(&path, "another-session");
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        remove_owned_discovery(&path, &session.token);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn uia_get_cannot_execute_targeted_operations_and_preserves_typed_limits() {
        for operation in ["invoke", "set-value", "window-close", "inspect", "text"] {
            let response = router(state())
                .oneshot(
                    Request::builder()
                        .uri(format!("/__rdesktop__/agent/uia?operation={operation}"))
                        .header("host", "127.0.0.1:1234")
                        .header("authorization", "Bearer test-secret")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        }
        let response = router(state())
            .oneshot(
                Request::builder()
                    .uri("/__rdesktop__/agent/uia?window=7&max_nodes=32&max_depth=4&patterns=false")
                    .header("host", "127.0.0.1:1234")
                    .header("authorization", "Bearer test-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(value["params"]["max_nodes"], 32);
        assert_eq!(value["params"]["max_depth"], 4);
        assert_eq!(value["params"]["patterns"], false);
    }
}
