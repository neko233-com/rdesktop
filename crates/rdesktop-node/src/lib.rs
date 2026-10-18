//! Trusted CommonJS preload workers owned by rdesktop, without Electron.
//!
//! This is an execution primitive, not a Node sandbox or a complete Electron
//! compatibility layer. Direct workers export functions assigned to `window`;
//! the experimental renderer module also projects CommonJS into native WebView2.
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{mpsc, Mutex, TryLockError},
    time::{Duration, Instant},
};

const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
const MAX_NODE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_TIMEOUT: Duration = Duration::from_secs(120);
const WORKER: &str = include_str!("worker.cjs");
const MODULES: &str = include_str!("modules.cjs");
const MEMORY: &str = include_str!("memory.cjs");
pub mod renderer;

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("Invalid Node runtime configuration: {0}")]
    Configuration(String),
    #[error("Node runtime I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Node runtime protocol failed: {0}")]
    Protocol(String),
    #[error("Node runtime deadline expired; the worker cannot be reused")]
    Timeout,
    #[error("Node runtime is busy")]
    Busy,
    #[error("Node runtime is closed")]
    Closed,
    #[error("{0}")]
    Preload(#[from] PreloadError),
}

#[derive(Debug, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct PreloadError {
    pub name: String,
    pub message: String,
    pub code: Option<String>,
}

/// The application explicitly chooses and pins the full-authority Node binary
/// and preload. No executable is discovered from PATH or downloaded on startup.
pub struct NodeConfig {
    pub executable: PathBuf,
    pub executable_sha256: String,
    pub package_root: PathBuf,
    pub preload: PathBuf,
    pub timeout: Duration,
}

#[derive(Debug, Deserialize)]
pub struct NodeMetadata {
    pub node: String,
    pub functions: Vec<String>,
    pub electron: Option<String>,
}

pub struct NodeRuntime {
    worker: Mutex<Worker>,
    metadata: NodeMetadata,
    timeout: Duration,
    process_id: u32,
    module_mode: bool,
}

struct Worker {
    child: Child,
    requests: Option<mpsc::SyncSender<Vec<u8>>>,
    responses: mpsc::Receiver<std::result::Result<Value, String>>,
    sequence: u64,
    failed: bool,
}

fn valid_timeout(timeout: Duration) -> Result<(), NodeError> {
    if timeout < Duration::from_millis(1) || timeout > MAX_TIMEOUT {
        return Err(NodeError::Configuration(
            "timeout must be 1..120000 milliseconds".into(),
        ));
    }
    Ok(())
}

fn verified_executable(config: &NodeConfig) -> Result<(PathBuf, File), NodeError> {
    if config.executable_sha256.len() != 64
        || !config
            .executable_sha256
            .bytes()
            .all(|value| value.is_ascii_hexdigit())
    {
        return Err(NodeError::Configuration(
            "expected SHA-256 is required".into(),
        ));
    }
    let path = fs::canonicalize(&config.executable)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Deny replacement while the verified executable is launched.
        options.share_mode(1);
    }
    let mut file = options.open(&path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_NODE_BYTES {
        return Err(NodeError::Configuration(
            "Node executable is not a bounded regular file".into(),
        ));
    }
    let mut digest = Sha256::new();
    let mut chunk = [0; 64 * 1024];
    let mut bytes = 0;
    loop {
        let count = file.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > MAX_NODE_BYTES {
            return Err(NodeError::Configuration(
                "Node executable exceeds its limit".into(),
            ));
        }
        digest.update(&chunk[..count]);
    }
    if !format!("{:x}", digest.finalize()).eq_ignore_ascii_case(&config.executable_sha256) {
        return Err(NodeError::Configuration(
            "Node executable SHA-256 mismatch".into(),
        ));
    }
    Ok((path, file))
}

impl NodeRuntime {
    pub fn start(config: NodeConfig) -> Result<Self, NodeError> {
        Self::start_mode(config, false)
    }

    /// Project Node modules into a native renderer. The preload is selected and
    /// bounded here, but executed by that renderer, where its DOM/callbacks live.
    pub fn start_modules(config: NodeConfig) -> Result<Self, NodeError> {
        Self::start_mode(config, true)
    }

    fn start_mode(config: NodeConfig, module_mode: bool) -> Result<Self, NodeError> {
        valid_timeout(config.timeout)?;
        let root = fs::canonicalize(&config.package_root)?;
        let preload = fs::canonicalize(&config.preload)?;
        if !root.is_dir()
            || preload == root
            || !preload.starts_with(&root)
            || !preload.is_file()
            || preload.metadata()?.len() > 1024 * 1024
        {
            return Err(NodeError::Configuration(
                "preload must be a bounded file inside its package root".into(),
            ));
        }
        let (executable, verified_file) = verified_executable(&config)?;
        let mut command = Command::new(executable);
        let source = format!("\"use strict\";\n{MEMORY}\n{MODULES}\n{WORKER}");
        command
            .args(["--input-type=commonjs", "--eval", &source])
            .env_remove("NODE_OPTIONS")
            .env_remove("NODE_PATH")
            .env_remove("ELECTRON_RUN_AS_NODE")
            .env_remove("IHUB_SIGNING_PRIVATE_KEY")
            .env_remove("IHUB_SIGNING_PRIVATE_KEY_PASSWORD")
            .env_remove("GH_TOKEN")
            .env_remove("GITHUB_TOKEN")
            .env_remove("NPM_TOKEN")
            .env_remove("NODE_AUTH_TOKEN")
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW on the first spawn.
        }
        let mut child = command.spawn()?;
        drop(verified_file);
        let process_id = child.id();
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| NodeError::Protocol("missing stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| NodeError::Protocol("missing stdout".into()))?;
        let (send, responses) = mpsc::sync_channel(1);
        let writer_errors = send.clone();
        let (requests, outgoing) = mpsc::sync_channel::<Vec<u8>>(1);
        let writer = std::thread::Builder::new()
            .name(format!("rdesktop-node-write-{process_id}"))
            .spawn(move || {
                while let Ok(bytes) = outgoing.recv() {
                    if let Err(error) = stdin.write_all(&bytes).and_then(|_| stdin.flush()) {
                        let _ =
                            writer_errors.send(Err(format!("Node request write failed: {error}")));
                        break;
                    }
                }
            });
        if let Err(error) = writer {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        let reader = std::thread::Builder::new()
            .name(format!("rdesktop-node-{process_id}"))
            .spawn(move || {
                let mut input = BufReader::new(stdout);
                loop {
                    let mut line = Vec::new();
                    let read = (&mut input)
                        .take((MAX_FRAME_BYTES + 2) as u64)
                        .read_until(b'\n', &mut line);
                    let result = match read {
                        Ok(0) => Err("Node response stream closed".into()),
                        Ok(_)
                            if line.len() > MAX_FRAME_BYTES + 1 || line.last() != Some(&b'\n') =>
                        {
                            Err("Node response exceeds its frame limit".into())
                        }
                        Ok(_) => serde_json::from_slice(&line)
                            .map_err(|error| format!("invalid response JSON: {error}")),
                        Err(error) => Err(error.to_string()),
                    };
                    let failed = result.is_err();
                    if send.send(result).is_err() || failed {
                        break;
                    }
                }
            });
        if let Err(error) = reader {
            drop(requests);
            // The worker sees stdin EOF and exits; the fallback is restricted
            // to this newly created worker, never another application.
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        let mut worker = Worker {
            child,
            requests: Some(requests),
            responses,
            sequence: 0,
            failed: false,
        };
        let result = worker.exchange(
            "load",
            json!({"root":root,"preload":preload,"modules":module_mode}),
            config.timeout,
        )?;
        let metadata: NodeMetadata = serde_json::from_value(result)
            .map_err(|error| NodeError::Protocol(error.to_string()))?;
        if metadata.node.is_empty() || metadata.electron.is_some() {
            return Err(NodeError::Configuration(
                "preload worker must be Node.js without Electron".into(),
            ));
        }
        Ok(Self {
            worker: Mutex::new(worker),
            metadata,
            timeout: config.timeout,
            process_id,
            module_mode,
        })
    }

    pub fn metadata(&self) -> &NodeMetadata {
        &self.metadata
    }
    pub fn process_id(&self) -> u32 {
        self.process_id
    }

    /// Bounded module protocol used by the renderer bridge, never exposed as
    /// application IPC to unrelated pages. Trusted modules retain Node authority.
    pub fn module_request(&self, request: Value) -> Result<Value, NodeError> {
        if !self.module_mode {
            return Err(NodeError::Configuration(
                "worker is not in renderer module mode".into(),
            ));
        }
        let mut worker = self.worker.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => NodeError::Busy,
            TryLockError::Poisoned(_) => NodeError::Closed,
        })?;
        worker.exchange("module", json!({"request":request}), self.timeout)
    }

    /// Call a preload's exported function on a worker thread, not a UI thread.
    /// Arguments are JSON. Results use explicit object/Buffer/undefined/bigint
    /// tags; no asynchronous Node callback or DOM proxy is implied.
    pub fn call(&self, name: &str, args: &[Value]) -> Result<Value, NodeError> {
        self.call_with_timeout(name, args, self.timeout)
    }

    pub fn call_with_timeout(
        &self,
        name: &str,
        args: &[Value],
        timeout: Duration,
    ) -> Result<Value, NodeError> {
        valid_timeout(timeout)?;
        if !self
            .metadata
            .functions
            .iter()
            .any(|candidate| candidate == name)
        {
            return Err(NodeError::Configuration("unknown preload function".into()));
        }
        let mut worker = self.worker.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => NodeError::Busy,
            TryLockError::Poisoned(_) => NodeError::Closed,
        })?;
        worker.exchange("call", json!({"name":name,"args":args}), timeout)
    }

    pub fn close(&self) -> Result<(), NodeError> {
        let mut worker = self.worker.try_lock().map_err(|_| NodeError::Busy)?;
        worker.shutdown();
        Ok(())
    }
}

impl Worker {
    fn exchange(
        &mut self,
        operation: &str,
        fields: Value,
        timeout: Duration,
    ) -> Result<Value, NodeError> {
        if self.failed || self.requests.is_none() {
            return Err(NodeError::Closed);
        }
        self.sequence = self.sequence.checked_add(1).ok_or(NodeError::Closed)?;
        let id = self.sequence.to_string();
        let mut message = fields.as_object().cloned().unwrap_or_default();
        message.insert("version".into(), json!(1));
        message.insert("id".into(), json!(id));
        message.insert("operation".into(), json!(operation));
        let mut bytes =
            serde_json::to_vec(&message).map_err(|error| NodeError::Protocol(error.to_string()))?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(NodeError::Configuration(
                "Node request exceeds its frame limit".into(),
            ));
        }
        bytes.push(b'\n');
        if self
            .requests
            .as_ref()
            .ok_or(NodeError::Closed)?
            .try_send(bytes)
            .is_err()
        {
            self.failed = true;
            self.shutdown();
            return Err(NodeError::Closed);
        }
        let response = match self.responses.recv_timeout(timeout) {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                self.failed = true;
                self.shutdown();
                return Err(NodeError::Protocol(error));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.failed = true;
                self.shutdown();
                return Err(NodeError::Timeout);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.failed = true;
                self.shutdown();
                return Err(NodeError::Closed);
            }
        };
        if response["version"] != 1
            || response["id"] != id
            || !response["success"].is_boolean()
            || response.get("data").is_none()
        {
            self.failed = true;
            self.shutdown();
            return Err(NodeError::Protocol(
                "response identity or envelope mismatch".into(),
            ));
        }
        if response["success"] == true {
            return Ok(response["data"].clone());
        }
        let error = match serde_json::from_value(response["data"].clone()) {
            Ok(error) => error,
            Err(error) => {
                self.failed = true;
                self.shutdown();
                return Err(NodeError::Protocol(format!(
                    "invalid preload error: {error}"
                )));
            }
        };
        Err(NodeError::Preload(error))
    }

    fn shutdown(&mut self) {
        if self.requests.is_some() && !self.failed {
            let _ = self.exchange("shutdown", json!({}), Duration::from_secs(1));
        }
        self.requests.take();
        self.failed = true;
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                _ if Instant::now() >= deadline => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
                _ => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shutdown();
    }
}
