//! Development server implementation.
//!
//! Serves the frontend as a local web page with hot reload and Agent API.
//! This is the core of rdesktop's Agent-first development story.
//!
//! The dev server does three things:
//! 1. Serves frontend static files (HTML/CSS/JS)
//! 2. Injects the rdesktop bridge script for IPC
//! 3. Provides Agent API endpoints for AI agent interaction

use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::body::Body;
use axum::extract::{Request, State as AxumState};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio::sync::{watch, Mutex, Notify, RwLock};
use tower_http::cors::CorsLayer;

use rdesktop_core::config::DevConfig;
use rdesktop_core::ipc::IpcHandler;

use crate::agent_api;
use crate::native_recorder::NativeRecorder;

/// Recordings are intentionally bounded so a forgotten `stop` cannot keep
/// producing a large debug artifact forever.
pub(crate) const DEFAULT_RECORDING_MAX_DURATION_SECONDS: u64 = 300;
pub(crate) const MAX_RECORDING_MAX_DURATION_SECONDS: u64 = 3600;
const MAX_SCREENSHOT_BYTES: usize = 16 * 1024 * 1024;

/// A single immutable PNG frame published by a native renderer.
#[derive(Debug, Clone)]
pub struct PublishedScreenshot {
    pub generation: u64,
    pub png: Vec<u8>,
}

struct ScreenshotPublisherInner {
    next_generation: AtomicU64,
    frames: watch::Sender<Option<PublishedScreenshot>>,
}

/// Publishes native renderer frames to the Agent API without making the HTTP
/// server poll a file that may still be written by the renderer.
#[derive(Clone)]
pub struct ScreenshotPublisher {
    inner: Arc<ScreenshotPublisherInner>,
}

impl ScreenshotPublisher {
    pub fn new() -> Self {
        let (frames, _) = watch::channel(None);
        Self {
            inner: Arc::new(ScreenshotPublisherInner {
                next_generation: AtomicU64::new(0),
                frames,
            }),
        }
    }

    /// Publish a complete PNG frame. Oversized or empty frames are rejected so
    /// a broken renderer cannot turn the Agent endpoint into an unbounded IPC
    /// sink.
    pub fn publish_png(&self, png: &[u8]) {
        if png.is_empty() || png.len() > MAX_SCREENSHOT_BYTES {
            tracing::warn!(
                bytes = png.len(),
                "rdesktop Agent rejected invalid screenshot frame"
            );
            return;
        }
        let generation = self
            .inner
            .next_generation
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        self.inner.frames.send_replace(Some(PublishedScreenshot {
            generation,
            png: png.to_vec(),
        }));
    }

    pub fn generation(&self) -> u64 {
        self.inner.next_generation.load(Ordering::Relaxed)
    }

    pub async fn latest(&self) -> Option<PublishedScreenshot> {
        self.inner.frames.subscribe().borrow().clone()
    }

    pub async fn wait_for_next(
        &self,
        after_generation: u64,
        timeout: std::time::Duration,
    ) -> Option<PublishedScreenshot> {
        let mut receiver = self.inner.frames.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(frame) = receiver.borrow().clone() {
                if frame.generation > after_generation {
                    return Some(frame);
                }
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match tokio::time::timeout(remaining, receiver.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => return None,
            }
        }
    }
}

impl Default for ScreenshotPublisher {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared state for the development server.
#[derive(Clone)]
pub struct DevServerState {
    /// The last captured DOM snapshot (for agent queries).
    pub last_dom_snapshot: Arc<RwLock<Option<String>>>,

    /// The last captured application state.
    pub last_app_state: Arc<RwLock<Option<serde_json::Value>>>,

    /// The frontend directory path.
    pub frontend_dir: PathBuf,

    /// Whether frontend file polling is enabled.
    pub hot_reload: bool,

    /// Shared queue of actions waiting for the browser bridge.
    pub pending_actions: Arc<Mutex<Vec<agent_api::AgentAction>>>,

    /// Action IDs currently waiting for a bridge execution receipt.
    pub action_waiters: Arc<Mutex<HashSet<String>>>,

    /// Bridge execution receipts consumed by `?wait=true` callers.
    pub action_results: Arc<Mutex<HashMap<String, agent_api::ActionResult>>>,

    /// Wakes action callers when the bridge posts a receipt.
    pub action_result_notify: Arc<Notify>,

    /// Monotonically increasing frontend version used by hot reload.
    pub reload_generation: Arc<AtomicU64>,

    /// Last observed frontend file signature.
    pub frontend_signature: Arc<RwLock<u64>>,

    /// The one and only recording session for this dev server.
    pub recording: Arc<RecordingStore>,

    /// The latest native PNG frame and its generation counter.
    pub screenshot_publisher: ScreenshotPublisher,

    /// Optional compatibility path used by hosts that also persist frames.
    pub screenshot_path: Option<PathBuf>,

    /// Optional host IPC handler for the native Agent bridge.
    pub ipc_handler: Option<Arc<dyn IpcHandler>>,
}

impl DevServerState {
    fn new(
        frontend_dir: PathBuf,
        hot_reload: bool,
        screenshot_publisher: ScreenshotPublisher,
        screenshot_path: Option<PathBuf>,
        ipc_handler: Option<Arc<dyn IpcHandler>>,
    ) -> Self {
        let recording_path = frontend_dir
            .parent()
            .unwrap_or(&frontend_dir)
            .join(".rdesktop")
            .join("recording.mp4");

        Self {
            last_dom_snapshot: Arc::new(RwLock::new(None)),
            last_app_state: Arc::new(RwLock::new(None)),
            frontend_dir,
            hot_reload,
            pending_actions: Arc::new(Mutex::new(Vec::new())),
            action_waiters: Arc::new(Mutex::new(HashSet::new())),
            action_results: Arc::new(Mutex::new(HashMap::new())),
            action_result_notify: Arc::new(Notify::new()),
            reload_generation: Arc::new(AtomicU64::new(0)),
            frontend_signature: Arc::new(RwLock::new(0)),
            recording: Arc::new(if cfg!(windows) {
                RecordingStore::new_native(recording_path)
            } else {
                RecordingStore::new(recording_path)
            }),
            screenshot_publisher,
            screenshot_path,
            ipc_handler,
        }
    }
}

/// Lifecycle of the single development recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingStatus {
    Idle,
    Recording,
    StopRequested,
    Finalizing,
    Completed,
    Failed,
}

/// A stable snapshot returned to agents.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecordingSnapshot {
    pub status: RecordingStatus,
    pub session_id: Option<String>,
    pub path: String,
    pub download_url: String,
    pub mime_type: Option<String>,
    /// True when the dev server is capturing the native desktop directly.
    /// False means the browser bridge owns MediaRecorder capture.
    pub native: bool,
    pub bytes: u64,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug)]
struct RecordingData {
    status: RecordingStatus,
    session_id: Option<String>,
    mime_type: Option<String>,
    bytes: u64,
    error: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
}

/// Owns the single recording file and its state.
///
/// The output stem is deliberately fixed. This makes recording start/stop
/// idempotent and prevents a dev session from accumulating timestamped files.
/// Windows native capture uses the fixed MP4 path; browser fallback follows
/// the browser's native MIME type.
pub struct RecordingStore {
    output_stem: PathBuf,
    partial_path: PathBuf,
    native_partial_path: PathBuf,
    data: Mutex<RecordingData>,
    file: Mutex<Option<tokio::fs::File>>,
    native: Option<Arc<NativeRecorder>>,
    next_id: AtomicU64,
}

impl RecordingStore {
    fn new(output_path: PathBuf) -> Self {
        Self::with_native(output_path, None)
    }

    fn new_native(output_path: PathBuf) -> Self {
        Self::with_native(output_path, Some(Arc::new(NativeRecorder::new())))
    }

    fn with_native(output_path: PathBuf, native: Option<Arc<NativeRecorder>>) -> Self {
        let output_stem = output_path.with_extension("");
        let partial_path = output_stem.with_extension("partial");
        let native_partial_path = output_stem.with_extension("partial.mp4");
        Self {
            output_stem,
            partial_path,
            native_partial_path,
            data: Mutex::new(RecordingData {
                status: RecordingStatus::Idle,
                session_id: None,
                mime_type: None,
                bytes: 0,
                error: None,
                started_at: None,
                finished_at: None,
            }),
            file: Mutex::new(None),
            native,
            next_id: AtomicU64::new(1),
        }
    }

    async fn prepare(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.output_stem.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // These are transient files from a previous interrupted session. The
        // finalized fixed output is intentionally kept for agent inspection.
        tokio::fs::remove_file(&self.partial_path).await.ok();
        tokio::fs::remove_file(&self.native_partial_path).await.ok();
        Ok(())
    }

    fn final_path(&self, mime_type: Option<&str>) -> PathBuf {
        let extension = if mime_type
            .map(|mime_type| mime_type.starts_with("video/mp4"))
            .unwrap_or(false)
        {
            "mp4"
        } else {
            "webm"
        };
        self.output_stem.with_extension(extension)
    }

    pub(crate) async fn snapshot(&self) -> RecordingSnapshot {
        let data = self.data.lock().await;
        RecordingSnapshot {
            status: data.status,
            session_id: data.session_id.clone(),
            path: self
                .final_path(data.mime_type.as_deref())
                .display()
                .to_string(),
            download_url: "/__rdesktop__/agent/recording/file".to_string(),
            mime_type: data.mime_type.clone(),
            native: self.native.is_some(),
            bytes: data.bytes,
            error: data.error.clone(),
            started_at: data.started_at.clone(),
            finished_at: data.finished_at.clone(),
        }
    }

    pub(crate) async fn start_with_options(
        &self,
        fps: u32,
        max_duration: std::time::Duration,
    ) -> anyhow::Result<(RecordingSnapshot, bool)> {
        let mut data = self.data.lock().await;
        if matches!(
            data.status,
            RecordingStatus::Recording
                | RecordingStatus::StopRequested
                | RecordingStatus::Finalizing
        ) {
            return Ok((self.snapshot_from_data(&data), true));
        }

        self.prepare().await?;
        tokio::fs::remove_file(self.final_path(Some("video/mp4")))
            .await
            .ok();
        tokio::fs::remove_file(self.final_path(Some("video/webm")))
            .await
            .ok();

        let id = format!(
            "{}-{}",
            unix_millis(),
            self.next_id.fetch_add(1, Ordering::Relaxed)
        );
        let native = self.native.clone();
        data.status = RecordingStatus::Recording;
        data.session_id = Some(id);
        data.mime_type = native.as_ref().map(|_| "video/mp4".to_string());
        data.bytes = 0;
        data.error = None;
        data.started_at = Some(timestamp());
        data.finished_at = None;

        if let Some(native) = native {
            // Media Foundation selects its MP4 sink from the filename
            // extension, so the transient native path also ends in `.mp4`.
            // It is renamed to the fixed output only after Finalize succeeds.
            if let Err(error) = native
                .start(self.native_partial_path.clone(), fps.max(1), max_duration)
                .await
            {
                data.status = RecordingStatus::Failed;
                data.error = Some(error.to_string());
                data.finished_at = Some(timestamp());
                tokio::fs::remove_file(&self.native_partial_path).await.ok();
                return Err(error);
            }
        } else {
            let mut file = self.file.lock().await;
            *file = Some(
                tokio::fs::OpenOptions::new()
                    .create(true)
                    .truncate(true)
                    .write(true)
                    .open(&self.partial_path)
                    .await?,
            );
        }

        let snapshot = self.snapshot_from_data(&data);
        Ok((snapshot, false))
    }

    fn snapshot_from_data(&self, data: &RecordingData) -> RecordingSnapshot {
        RecordingSnapshot {
            status: data.status,
            session_id: data.session_id.clone(),
            path: self
                .final_path(data.mime_type.as_deref())
                .display()
                .to_string(),
            download_url: "/__rdesktop__/agent/recording/file".to_string(),
            mime_type: data.mime_type.clone(),
            native: self.native.is_some(),
            bytes: data.bytes,
            error: data.error.clone(),
            started_at: data.started_at.clone(),
            finished_at: data.finished_at.clone(),
        }
    }

    pub(crate) async fn request_stop(
        &self,
        session_id: Option<&str>,
    ) -> anyhow::Result<RecordingSnapshot> {
        let mut data = self.data.lock().await;
        if let Some(expected) = session_id {
            if data.session_id.as_deref() != Some(expected) {
                anyhow::bail!("recording session does not match the active session");
            }
        }
        if data.status == RecordingStatus::Recording {
            data.status = RecordingStatus::StopRequested;
        }
        Ok(self.snapshot_from_data(&data))
    }

    /// Stop the recording. Native capture can finalize synchronously because
    /// the encoder is owned by the server; browser capture still needs the
    /// bridge to flush its MediaRecorder chunks.
    pub(crate) async fn stop(&self, session_id: Option<&str>) -> anyhow::Result<RecordingSnapshot> {
        let Some(native) = self.native.clone() else {
            return self.request_stop(session_id).await;
        };

        {
            let mut data = self.data.lock().await;
            if let Some(expected) = session_id {
                if data.session_id.as_deref() != Some(expected) {
                    anyhow::bail!("recording session does not match the active session");
                }
            }
            if matches!(
                data.status,
                RecordingStatus::Idle | RecordingStatus::Completed | RecordingStatus::Failed
            ) {
                return Ok(self.snapshot_from_data(&data));
            }
            if data.status == RecordingStatus::Finalizing {
                return Ok(self.snapshot_from_data(&data));
            }
            data.status = RecordingStatus::Finalizing;
        }

        let result = match native.stop().await {
            Ok(_) => {
                let final_path = self.final_path(Some("video/mp4"));
                tokio::fs::remove_file(&final_path).await.ok();
                tokio::fs::remove_file(self.final_path(Some("video/webm")))
                    .await
                    .ok();
                tokio::fs::rename(&self.native_partial_path, &final_path).await?;
                Ok(tokio::fs::metadata(&final_path).await?.len())
            }
            Err(error) => Err(error),
        };

        let mut data = self.data.lock().await;
        match result {
            Ok(bytes) => {
                data.status = RecordingStatus::Completed;
                data.bytes = bytes;
                data.error = None;
            }
            Err(error) => {
                data.status = RecordingStatus::Failed;
                data.error = Some(error.to_string());
                tokio::fs::remove_file(&self.native_partial_path).await.ok();
                tokio::fs::remove_file(self.final_path(Some("video/mp4")))
                    .await
                    .ok();
            }
        }
        data.finished_at = Some(timestamp());
        Ok(self.snapshot_from_data(&data))
    }

    pub(crate) async fn mark_started(
        &self,
        session_id: &str,
        mime_type: &str,
    ) -> anyhow::Result<()> {
        if self.native.is_some() {
            anyhow::bail!("native recording does not accept browser metadata")
        }
        let mut data = self.data.lock().await;
        if data.session_id.as_deref() != Some(session_id) {
            anyhow::bail!("recording session does not match the active session");
        }
        if matches!(
            data.status,
            RecordingStatus::Recording | RecordingStatus::StopRequested
        ) {
            data.mime_type = Some(mime_type.to_string());
            return Ok(());
        }
        anyhow::bail!("recording is not accepting browser metadata")
    }

    pub(crate) async fn append_chunk(&self, session_id: &str, chunk: &[u8]) -> anyhow::Result<u64> {
        if self.native.is_some() {
            anyhow::bail!("native recording does not accept browser media chunks")
        }
        {
            let data = self.data.lock().await;
            if data.session_id.as_deref() != Some(session_id) {
                anyhow::bail!("recording session does not match the active session");
            }
            if !matches!(
                data.status,
                RecordingStatus::Recording | RecordingStatus::StopRequested
            ) {
                anyhow::bail!("recording is not accepting media chunks");
            }
        }

        use tokio::io::AsyncWriteExt;
        let mut file_guard = self.file.lock().await;
        let file = file_guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("recording file is not open"))?;
        file.write_all(chunk).await?;
        file.flush().await?;
        drop(file_guard);

        let mut data = self.data.lock().await;
        data.bytes = data.bytes.saturating_add(chunk.len() as u64);
        Ok(data.bytes)
    }

    pub(crate) async fn complete(
        &self,
        session_id: &str,
        mime_type: Option<&str>,
    ) -> anyhow::Result<RecordingSnapshot> {
        if self.native.is_some() {
            anyhow::bail!("native recording is finalized by the server stop operation")
        }
        let mime_type = {
            let mut data = self.data.lock().await;
            if data.session_id.as_deref() != Some(session_id) {
                anyhow::bail!("recording session does not match the active session");
            }
            if data.status == RecordingStatus::Completed {
                return Ok(self.snapshot_from_data(&data));
            }
            if data.status == RecordingStatus::Finalizing {
                return Ok(self.snapshot_from_data(&data));
            }
            if let Some(mime_type) = mime_type {
                data.mime_type = Some(mime_type.to_string());
            }
            data.status = RecordingStatus::Finalizing;
            data.mime_type.clone().unwrap_or_default()
        };

        // Close the file before rename/conversion. This is required on Windows.
        self.file.lock().await.take();
        let final_path = self.final_path(Some(&mime_type));
        let result = {
            tokio::fs::remove_file(self.final_path(Some("video/mp4")))
                .await
                .ok();
            tokio::fs::remove_file(self.final_path(Some("video/webm")))
                .await
                .ok();
            tokio::fs::rename(&self.partial_path, &final_path)
                .await
                .map_err(anyhow::Error::from)
        };

        let mut data = self.data.lock().await;
        match result {
            Ok(()) => {
                data.status = RecordingStatus::Completed;
                data.finished_at = Some(timestamp());
                data.error = None;
            }
            Err(error) => {
                data.status = RecordingStatus::Failed;
                data.finished_at = Some(timestamp());
                data.error = Some(error.to_string());
                tokio::fs::remove_file(&self.partial_path).await.ok();
            }
        }
        Ok(self.snapshot_from_data(&data))
    }

    pub(crate) async fn fail(
        &self,
        session_id: &str,
        error: String,
    ) -> anyhow::Result<RecordingSnapshot> {
        self.file.lock().await.take();
        let mut data = self.data.lock().await;
        if data.session_id.as_deref() != Some(session_id) {
            anyhow::bail!("recording session does not match the active session");
        }
        if matches!(
            data.status,
            RecordingStatus::Completed | RecordingStatus::Failed
        ) {
            return Ok(self.snapshot_from_data(&data));
        }
        data.status = RecordingStatus::Failed;
        data.error = Some(error);
        data.finished_at = Some(timestamp());
        drop(data);
        tokio::fs::remove_file(&self.partial_path).await.ok();
        tokio::fs::remove_file(&self.native_partial_path).await.ok();
        tokio::fs::remove_file(self.final_path(Some("video/mp4")))
            .await
            .ok();
        tokio::fs::remove_file(self.final_path(Some("video/webm")))
            .await
            .ok();
        let data = self.data.lock().await;
        Ok(self.snapshot_from_data(&data))
    }
}

/// Development server that serves the app in browser mode.
///
/// This is NOT the production renderer. It's a development tool that allows
/// AI agents (and humans) to interact with the app via a browser.
pub struct DevServer {
    config: DevConfig,
    frontend_dir: PathBuf,
    recording: Arc<Mutex<Option<Arc<RecordingStore>>>>,
    ipc_handler: Option<Arc<dyn IpcHandler>>,
    screenshot_path: Option<PathBuf>,
    screenshot_publisher: ScreenshotPublisher,
}

impl DevServer {
    /// Create a new DevServer.
    pub fn new(config: DevConfig, frontend_dir: PathBuf) -> Self {
        Self {
            config,
            frontend_dir,
            recording: Arc::new(Mutex::new(None)),
            ipc_handler: None,
            screenshot_path: None,
            screenshot_publisher: ScreenshotPublisher::new(),
        }
    }

    /// Create a server whose Agent IPC endpoint forwards to the native host.
    pub fn new_with_handler(
        config: DevConfig,
        frontend_dir: PathBuf,
        handler: Arc<dyn IpcHandler>,
    ) -> Self {
        Self {
            config,
            frontend_dir,
            recording: Arc::new(Mutex::new(None)),
            ipc_handler: Some(handler),
            screenshot_path: None,
            screenshot_publisher: ScreenshotPublisher::new(),
        }
    }

    /// Keep a stable on-disk copy for humans and external image viewers. The
    /// Agent endpoint itself serves the in-memory published frame.
    pub fn with_screenshot_path(mut self, path: PathBuf) -> Self {
        self.screenshot_path = Some(path);
        self
    }

    pub fn screenshot_publisher(&self) -> ScreenshotPublisher {
        self.screenshot_publisher.clone()
    }

    /// Start the development server.
    ///
    /// Returns the URL where the server is listening.
    pub async fn start(&self) -> anyhow::Result<String> {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let url = format!("http://{}", addr);

        let state = DevServerState::new(
            self.frontend_dir.clone(),
            self.config.hot_reload,
            self.screenshot_publisher.clone(),
            self.screenshot_path.clone(),
            self.ipc_handler.clone(),
        );
        *self.recording.lock().await = Some(state.recording.clone());
        state.recording.prepare().await?;

        if self.config.hot_reload {
            let signature = frontend_signature(&state.frontend_dir);
            *state.frontend_signature.write().await = signature;
            let watch_state = state.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_millis(300));
                loop {
                    interval.tick().await;
                    let current = frontend_signature(&watch_state.frontend_dir);
                    let mut previous = watch_state.frontend_signature.write().await;
                    if *previous != current {
                        *previous = current;
                        watch_state
                            .reload_generation
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }

        // Build the router
        let app = Router::new()
            // Agent API endpoints
            .route("/__rdesktop__/agent/dom", get(agent_api::get_dom))
            .route(
                "/__rdesktop__/agent/elements",
                get(agent_api::query_elements),
            )
            .route(
                "/__rdesktop__/agent/action",
                post(agent_api::execute_action),
            )
            .route("/__rdesktop__/agent/state", get(agent_api::get_state))
            .route("/__rdesktop__/agent/ipc", post(agent_api::send_ipc))
            .route(
                "/__rdesktop__/agent/screenshot",
                get(agent_api::take_screenshot),
            )
            .route(
                "/__rdesktop__/agent/recording",
                get(agent_api::get_recording),
            )
            .route(
                "/__rdesktop__/agent/recording/start",
                post(agent_api::start_recording),
            )
            .route(
                "/__rdesktop__/agent/recording/stop",
                post(agent_api::stop_recording),
            )
            .route(
                "/__rdesktop__/agent/recording/status",
                get(agent_api::get_recording),
            )
            .route(
                "/__rdesktop__/agent/recording/poll",
                get(agent_api::poll_recording),
            )
            .route(
                "/__rdesktop__/agent/recording/started",
                post(agent_api::recording_started),
            )
            .route(
                "/__rdesktop__/agent/recording/chunk",
                post(agent_api::recording_chunk),
            )
            .route(
                "/__rdesktop__/agent/recording/complete",
                post(agent_api::recording_complete),
            )
            .route(
                "/__rdesktop__/agent/recording/error",
                post(agent_api::recording_error),
            )
            .route(
                "/__rdesktop__/agent/recording/file",
                get(agent_api::recording_file),
            )
            .route(
                "/__rdesktop__/agent/action/pending",
                get(agent_api::pending_actions),
            )
            .route(
                "/__rdesktop__/agent/action/result",
                post(agent_api::report_action_result),
            )
            // Health check
            .route("/__rdesktop__/health", get(|| async { "ok" }))
            // Dev info
            .route("/__rdesktop__/info", get(dev_info))
            .route("/__rdesktop__/reload", get(reload_status))
            .route("/__rdesktop__/bridge.js", get(bridge_script))
            // State update from browser
            .route("/__rdesktop__/state", post(update_state))
            .route("/__rdesktop__/dom", post(update_dom))
            // Enable CORS for all routes
            .layer(CorsLayer::permissive())
            // Serve static files and inject the bridge into HTML documents.
            .fallback(serve_frontend)
            .with_state(state.clone());

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

    /// Finalize or discard an active debug recording before the dev process
    /// exits. Native MP4 recording is finalized; browser fallback recordings
    /// are marked failed so their partial chunks do not remain as garbage.
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        let recording = self.recording.lock().await.take();
        let Some(recording) = recording else {
            return Ok(());
        };

        let snapshot = recording.snapshot().await;
        let Some(session_id) = snapshot.session_id else {
            return Ok(());
        };
        match snapshot.status {
            RecordingStatus::Recording
            | RecordingStatus::StopRequested
            | RecordingStatus::Finalizing => {
                if snapshot.native {
                    recording.stop(Some(&session_id)).await?;
                } else {
                    recording
                        .fail(
                            &session_id,
                            "dev server shut down before browser recording finalized".to_string(),
                        )
                        .await?;
                }
            }
            RecordingStatus::Idle | RecordingStatus::Completed | RecordingStatus::Failed => {}
        }
        Ok(())
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
            "action_result": "/__rdesktop__/agent/action/result",
            "state": "/__rdesktop__/agent/state",
            "ipc": "/__rdesktop__/agent/ipc",
            "screenshot": "/__rdesktop__/agent/screenshot",
            "recording": "/__rdesktop__/agent/recording",
            "recording_start": "/__rdesktop__/agent/recording/start",
            "recording_stop": "/__rdesktop__/agent/recording/stop",
            "recording_file": "/__rdesktop__/agent/recording/file",
        }
    }))
}

async fn reload_status(AxumState(state): AxumState<DevServerState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "generation": state.reload_generation.load(Ordering::Relaxed),
        "enabled": state.hot_reload,
    }))
}

async fn bridge_script() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/javascript; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(include_str!("../assets/bridge.js")))
        .expect("static bridge response is valid")
}

async fn serve_frontend(AxumState(state): AxumState<DevServerState>, request: Request) -> Response {
    let request_path = request.uri().path();
    let relative = request_path.trim_start_matches('/');
    if relative.split('/').any(|part| part == "..") || relative.contains('\\') {
        return response_text(StatusCode::BAD_REQUEST, "invalid frontend path");
    }

    let mut file_path = state.frontend_dir.join(if relative.is_empty() {
        "index.html"
    } else {
        relative
    });
    if tokio::fs::metadata(&file_path)
        .await
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false)
    {
        file_path = file_path.join("index.html");
    }

    let bytes = match tokio::fs::read(&file_path).await {
        Ok(bytes) => bytes,
        Err(_) => return response_text(StatusCode::NOT_FOUND, "frontend file not found"),
    };
    let is_html = file_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("html"))
        .unwrap_or(false);
    let body = if is_html {
        let mut html = String::from_utf8_lossy(&bytes).into_owned();
        if !html.contains("/__rdesktop__/bridge.js") {
            let bridge = "<script src=\"/__rdesktop__/bridge.js\"></script>";
            if let Some(index) = html.to_ascii_lowercase().find("</head>") {
                html.insert_str(index, bridge);
            } else {
                html.insert_str(0, bridge);
            }
        }
        Body::from(html)
    } else {
        Body::from(bytes)
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type(&file_path))
        .body(body)
        .expect("frontend response is valid")
}

fn response_text(status: StatusCode, text: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(text.to_string()))
        .expect("text response is valid")
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
    {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn frontend_signature(root: &Path) -> u64 {
    let mut entries = Vec::new();
    collect_frontend_files(root, &mut entries);
    entries.sort();
    let mut hasher = DefaultHasher::new();
    entries.hash(&mut hasher);
    hasher.finish()
}

fn collect_frontend_files(root: &Path, entries: &mut Vec<(String, u64, u64)>) {
    let Ok(read_dir) = std::fs::read_dir(root) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            collect_frontend_files(&path, entries);
        } else if metadata.is_file() {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos() as u64)
                .unwrap_or_default();
            entries.push((path.display().to_string(), modified, metadata.len()));
        }
    }
}

fn unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn timestamp() -> String {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rdesktop-dev-{}-{}-{}",
            name,
            std::process::id(),
            unix_millis()
        ))
    }

    #[tokio::test]
    async fn recording_start_is_singleton_and_mp4_stop_is_idempotent() {
        let root = test_path("recording");
        let output = root.join("recording.mp4");
        let store = RecordingStore::new(output.clone());

        let (first, reused) = store
            .start_with_options(
                30,
                std::time::Duration::from_secs(DEFAULT_RECORDING_MAX_DURATION_SECONDS),
            )
            .await
            .expect("start recording");
        assert!(!reused);
        let (second, reused) = store
            .start_with_options(
                30,
                std::time::Duration::from_secs(DEFAULT_RECORDING_MAX_DURATION_SECONDS),
            )
            .await
            .expect("reuse recording");
        assert!(reused);
        assert_eq!(first.session_id, second.session_id);

        let session_id = first.session_id.as_deref().expect("session id");
        store
            .mark_started(session_id, "video/mp4")
            .await
            .expect("mark mime");
        store
            .append_chunk(session_id, b"fake-mp4")
            .await
            .expect("append chunk");
        store
            .request_stop(Some(session_id))
            .await
            .expect("request stop");
        let completed = store
            .complete(session_id, Some("video/mp4"))
            .await
            .expect("complete recording");
        assert_eq!(completed.status, RecordingStatus::Completed);
        assert_eq!(
            tokio::fs::read(&output).await.expect("read mp4"),
            b"fake-mp4"
        );

        let repeated = store
            .complete(session_id, Some("video/mp4"))
            .await
            .expect("repeat complete");
        assert_eq!(repeated.status, RecordingStatus::Completed);
        assert_eq!(repeated.path, completed.path);

        tokio::fs::remove_dir_all(root)
            .await
            .expect("cleanup test files");
    }

    #[tokio::test]
    async fn concurrent_starts_share_one_session() {
        let root = test_path("concurrent");
        let store = Arc::new(RecordingStore::new(root.join("recording.mp4")));
        let first_store = store.clone();
        let second_store = store.clone();
        let duration = std::time::Duration::from_secs(DEFAULT_RECORDING_MAX_DURATION_SECONDS);
        let (first, second) = tokio::join!(
            first_store.start_with_options(30, duration),
            second_store.start_with_options(30, duration)
        );
        let first = first.expect("first start");
        let second = second.expect("second start");
        assert_ne!(first.1, second.1);
        assert_eq!(first.0.session_id, second.0.session_id);
        drop(store);
        tokio::fs::remove_dir_all(root)
            .await
            .expect("cleanup test files");
    }

    #[tokio::test]
    async fn stale_transient_files_are_removed_before_a_new_session() {
        let root = test_path("stale-transients");
        tokio::fs::create_dir_all(&root)
            .await
            .expect("create test directory");
        let store = RecordingStore::new(root.join("recording.mp4"));
        tokio::fs::write(&store.partial_path, b"stale browser bytes")
            .await
            .expect("write browser transient");
        tokio::fs::write(&store.native_partial_path, b"stale native bytes")
            .await
            .expect("write native transient");

        store.prepare().await.expect("prepare recording directory");

        assert!(!tokio::fs::try_exists(&store.partial_path)
            .await
            .expect("check browser transient"));
        assert!(!tokio::fs::try_exists(&store.native_partial_path)
            .await
            .expect("check native transient"));

        tokio::fs::remove_dir_all(root)
            .await
            .expect("cleanup test files");
    }

    #[tokio::test]
    async fn screenshot_publisher_returns_complete_frames_and_waits_for_new_generation() {
        let publisher = ScreenshotPublisher::new();
        assert_eq!(publisher.generation(), 0);
        assert!(publisher.latest().await.is_none());

        let waiter = publisher.clone();
        let pending = tokio::spawn(async move {
            waiter
                .wait_for_next(0, std::time::Duration::from_secs(1))
                .await
        });
        tokio::task::yield_now().await;
        publisher.publish_png(b"complete-png-frame");

        let frame = pending
            .await
            .expect("screenshot waiter")
            .expect("new frame");
        assert_eq!(frame.generation, 1);
        assert_eq!(frame.png, b"complete-png-frame");
        assert_eq!(
            publisher.latest().await.expect("latest frame").generation,
            1
        );
    }
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
