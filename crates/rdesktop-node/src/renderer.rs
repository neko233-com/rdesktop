//! Experimental CommonJS projection into a real native WebView. All Node
//! authority belongs to one explicitly trusted package and its private origin.
use crate::{NodeConfig, NodeError, NodeRuntime};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const LOADER: &str = include_str!("renderer.js");
const MAX_BODY: usize = 4 * 1024 * 1024;
const MAX_ASSET: u64 = 16 * 1024 * 1024;

/// Controls implicit Node globals in page scripts. Preloads and CommonJS
/// dependencies retain lexical Node globals in either mode. This is a loading
/// policy, not a sandbox: a preload may deliberately export Node authority.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NodeGlobals {
    #[default]
    Page,
    PreloadOnly,
}

/// Opt-in bridge; it does not bundle Node, a browser, Electron or uTools APIs.
/// The initial script loads the original preload into the page's JS context.
pub struct RendererBridge {
    url: String,
    script: String,
    node: Arc<NodeRuntime>,
    stopped: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

struct Service {
    root: PathBuf,
    prefix: String,
    origin: String,
    token: String,
    node: Arc<NodeRuntime>,
    document: Mutex<Option<String>>,
}

/// A renderer's Node module session hosted by an application's existing private
/// asset server. The application must enforce POST, frame/body limits and lease
/// retirement; this object enforces origin, capability and document ownership.
pub struct RendererModules {
    node: Arc<NodeRuntime>,
    origin: String,
    token: String,
    script: String,
    document: Mutex<Option<String>>,
}

impl RendererModules {
    pub fn start(config: NodeConfig, base_url: &str) -> Result<Self, NodeError> {
        Self::start_with_globals(config, base_url, NodeGlobals::Page)
    }
    pub fn start_with_globals(
        config: NodeConfig,
        base_url: &str,
        globals: NodeGlobals,
    ) -> Result<Self, NodeError> {
        if config.timeout > Duration::from_secs(5) {
            return Err(NodeError::Configuration(
                "renderer Node calls must use a deadline of at most 5000 ms".into(),
            ));
        }
        let authority = base_url
            .strip_prefix("http://127.0.0.1:")
            .and_then(|value| value.split_once('/'))
            .filter(|(port, path)| {
                port.parse::<u16>().is_ok_and(|port| port != 0)
                    && base_url.ends_with('/')
                    && !path.contains(['?', '#', '\\', '\r', '\n'])
                    && !path.split('/').any(|part| matches!(part, "." | ".."))
            })
            .ok_or_else(|| {
                NodeError::Configuration(
                    "module base must be a private 127.0.0.1 HTTP lease ending in /".into(),
                )
            })?;
        let origin = format!("http://127.0.0.1:{}", authority.0);
        let node = Arc::new(NodeRuntime::start_modules(config)?);
        let metadata = node.module_request(json!({"operation":"bootstrap"}))?;
        let token = metadata["token"]
            .as_str()
            .filter(|token| token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or_else(|| NodeError::Protocol("invalid renderer capability".into()))?
            .to_owned();
        let options = json!({"base":base_url,"token":token,"preload":metadata["preload"],"node":metadata["node"],"exposeNodeGlobals":globals == NodeGlobals::Page});
        Ok(Self {
            node,
            origin,
            token,
            script: format!("({LOADER})({options});"),
            document: Mutex::new(None),
        })
    }
    pub fn initialization_script(&self) -> &str {
        &self.script
    }
    pub fn node_process_id(&self) -> u32 {
        self.node.process_id()
    }
    pub fn respond(
        &self,
        origin: &str,
        capability: &str,
        document: &str,
        request: Value,
    ) -> (u16, Value) {
        module_reply(
            &self.node,
            &self.origin,
            &self.token,
            &self.document,
            origin,
            capability,
            document,
            request,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn module_reply(
    node: &NodeRuntime,
    expected_origin: &str,
    token: &str,
    current_document: &Mutex<Option<String>>,
    origin: &str,
    capability: &str,
    document: &str,
    request: Value,
) -> (u16, Value) {
    let denied = |message| {
        (
            403,
            json!({"success":false,"error":{"name":"Error","message":message}}),
        )
    };
    if origin != expected_origin || capability != token {
        return denied("Node calls require the current private plugin origin.");
    }
    if document.len() != 32 || !document.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return denied("Missing renderer document identity.");
    }
    {
        let mut current = current_document
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if request["operation"] == "attach" && current.is_none() {
            *current = Some(document.into());
        }
        if current.as_deref() != Some(document) {
            return denied("Node lease belongs to another renderer document.");
        }
    }
    if request["operation"] == "attach" {
        return (200, json!({"success":true,"data":{}}));
    }
    let body = match node.module_request(request) {
        Ok(value) => json!({"success":true,"data":value}),
        Err(NodeError::Preload(error)) => {
            json!({"success":false,"error":{"name":error.name,"message":error.message,"code":error.code}})
        }
        Err(NodeError::Busy) => {
            json!({"success":false,"error":{"name":"Error","code":"RDESKTOP_NODE_BUSY","message":"Node runtime is busy; this operation did not begin."}})
        }
        Err(error) => json!({"success":false,"error":{"name":"Error","message":error.to_string()}}),
    };
    (200, body)
}

impl RendererBridge {
    pub fn start(config: NodeConfig, entry: impl AsRef<Path>) -> Result<Self, NodeError> {
        Self::start_with_globals(config, entry, NodeGlobals::Page)
    }
    pub fn start_with_globals(
        config: NodeConfig,
        entry: impl AsRef<Path>,
        globals: NodeGlobals,
    ) -> Result<Self, NodeError> {
        // Sync browser calls must have a short, explicit worker deadline.
        if config.timeout > Duration::from_secs(5) {
            return Err(NodeError::Configuration(
                "renderer Node calls must use a deadline of at most 5000 ms".into(),
            ));
        }
        let root = fs::canonicalize(&config.package_root)?;
        let entry = fs::canonicalize(root.join(entry))?;
        if !entry.starts_with(&root) || !entry.is_file() {
            return Err(NodeError::Configuration(
                "renderer entry must be a file inside the package".into(),
            ));
        }
        let node = Arc::new(NodeRuntime::start_modules(config)?);
        let metadata = node.module_request(json!({"operation":"bootstrap"}))?;
        let token = metadata["token"]
            .as_str()
            .filter(|token| token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or_else(|| NodeError::Protocol("invalid renderer capability".into()))?
            .to_owned();
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let origin = format!("http://{}", listener.local_addr()?);
        let prefix = format!("/{token}/");
        let relative = entry
            .strip_prefix(&root)
            .map_err(|error| NodeError::Configuration(error.to_string()))?;
        let url = format!(
            "{origin}{prefix}{}",
            relative.to_string_lossy().replace('\\', "/")
        );
        let options = json!({"base": format!("{origin}{prefix}"), "token":token,"preload":metadata["preload"],"node":metadata["node"],"exposeNodeGlobals":globals == NodeGlobals::Page});
        let script = format!("({LOADER})({options});");
        let service = Arc::new(Service {
            root,
            prefix,
            origin,
            token,
            node: node.clone(),
            document: Mutex::new(None),
        });
        let stopped = Arc::new(AtomicBool::new(false));
        let mut bridge = Self {
            url,
            script,
            node,
            stopped,
            threads: vec![],
        };
        let (sender, receiver) = mpsc::sync_channel::<TcpStream>(8);
        let receiver = Arc::new(Mutex::new(receiver));
        for _ in 0..4 {
            let receiver = receiver.clone();
            let service = service.clone();
            let stopped = bridge.stopped.clone();
            bridge.threads.push(
                std::thread::Builder::new()
                    .name("rdesktop-plugin-http".into())
                    .spawn(move || {
                        while !stopped.load(Ordering::Acquire) {
                            let next = receiver
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .recv_timeout(Duration::from_millis(50));
                            match next {
                                Ok(mut stream) => {
                                    if let Err(error) = service.serve(&mut stream) {
                                        if cfg!(debug_assertions) {
                                            eprintln!("rdesktop plugin transport: {error}");
                                        }
                                    }
                                }
                                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                                Err(mpsc::RecvTimeoutError::Timeout) => {}
                            }
                        }
                    })?,
            );
        }
        let stopped = bridge.stopped.clone();
        bridge.threads.push(
            std::thread::Builder::new()
                .name("rdesktop-plugin-listener".into())
                .spawn(move || {
                    while !stopped.load(Ordering::Acquire) {
                        match listener.accept() {
                            Ok((mut stream, _)) => {
                                // WinSock accepted sockets inherit FIONBIO from
                                // the listener. Restore blocking reads before
                                // applying deadlines, including split uploads.
                                if stream.set_nonblocking(false).is_err() {
                                    continue;
                                }
                                if let Err(mpsc::TrySendError::Full(stream_back)) =
                                    sender.try_send(stream)
                                {
                                    stream = stream_back;
                                    let _ = response(
                                        &mut stream,
                                        503,
                                        "text/plain",
                                        b"Plugin request queue is full.",
                                    );
                                }
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                std::thread::sleep(Duration::from_millis(10))
                            }
                            Err(_) => break,
                        }
                    }
                })?,
        );
        Ok(bridge)
    }
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn initialization_script(&self) -> &str {
        &self.script
    }
    pub fn node_process_id(&self) -> u32 {
        self.node.process_id()
    }
}

impl Drop for RendererBridge {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        let _ = self.node.close();
    }
}

fn response(stream: &mut TcpStream, status: u16, mime: &str, body: &[u8]) -> std::io::Result<()> {
    write!(stream, "HTTP/1.1 {status} Response\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self' data: blob:; script-src 'self' 'unsafe-inline' 'unsafe-eval'; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; object-src 'none'; base-uri 'self'\r\n\r\n", body.len())?;
    stream.write_all(body)
}

impl Service {
    fn serve(&self, stream: &mut TcpStream) -> std::io::Result<()> {
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        stream.set_write_timeout(Some(Duration::from_secs(3)))?;
        let mut bytes = Vec::new();
        let mut chunk = [0; 4096];
        let deadline = Instant::now() + Duration::from_secs(5);
        let header_end = loop {
            if bytes.len() > 8192 {
                return response(stream, 413, "text/plain", b"Request headers too large.");
            }
            if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break end + 4;
            }
            let read = bounded_read(stream, &mut chunk, deadline)?;
            if read == 0 {
                return Ok(());
            }
            bytes.extend_from_slice(&chunk[..read]);
        };
        let header = String::from_utf8_lossy(&bytes[..header_end]);
        let mut lines = header.split("\r\n");
        let request: Vec<_> = lines
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .collect();
        if request.len() != 3 || request[2] != "HTTP/1.1" {
            return response(stream, 400, "text/plain", b"Invalid request.");
        }
        let method = request[0];
        let target = request[1];
        let mut headers = std::collections::HashMap::new();
        for line in lines.filter(|line| !line.is_empty()) {
            let Some((name, value)) = line.split_once(':') else {
                return response(stream, 400, "text/plain", b"Invalid header.");
            };
            if headers
                .insert(name.to_ascii_lowercase(), value.trim())
                .is_some()
            {
                return response(stream, 400, "text/plain", b"Duplicate header.");
            }
        }
        if headers.get("host").copied() != self.origin.strip_prefix("http://")
            || headers.contains_key("transfer-encoding")
        {
            return response(stream, 403, "text/plain", b"Invalid plugin origin.");
        }
        let Some(relative) = target
            .split('?')
            .next()
            .and_then(|path| path.strip_prefix(&self.prefix))
        else {
            return response(stream, 404, "text/plain", b"Unknown plugin lease.");
        };
        if relative == "__node" {
            if method != "POST"
                || headers.get("origin").copied() != Some(&self.origin)
                || headers.get("x-rdesktop-node").copied() != Some(&self.token)
            {
                return response(
                    stream,
                    403,
                    "text/plain",
                    b"Node calls require the current private plugin origin.",
                );
            }
            let length = match headers
                .get("content-length")
                .and_then(|value| value.parse::<usize>().ok())
            {
                Some(value) if value <= MAX_BODY => value,
                _ => return response(stream, 413, "text/plain", b"Invalid Node request size."),
            };
            let mut body = bytes[header_end..].to_vec();
            while body.len() < length {
                let remaining = chunk.len().min(length - body.len());
                let read = bounded_read(stream, &mut chunk[..remaining], deadline)?;
                if read == 0 {
                    return response(stream, 400, "text/plain", b"Incomplete Node request.");
                }
                body.extend_from_slice(&chunk[..read]);
            }
            if body.len() != length {
                return response(stream, 400, "text/plain", b"Unexpected request bytes.");
            }
            let request: Value = match serde_json::from_slice(&body) {
                Ok(value) => value,
                Err(_) => return response(stream, 400, "text/plain", b"Invalid Node JSON."),
            };
            let (status, body) = module_reply(
                &self.node,
                &self.origin,
                &self.token,
                &self.document,
                headers.get("origin").copied().unwrap_or_default(),
                headers.get("x-rdesktop-node").copied().unwrap_or_default(),
                headers
                    .get("x-rdesktop-document")
                    .copied()
                    .unwrap_or_default(),
                request,
            );
            return response(
                stream,
                status,
                "application/json",
                &serde_json::to_vec(&body)?,
            );
        }
        if method != "GET" {
            return response(
                stream,
                405,
                "text/plain",
                b"Only GET assets and POST Node calls are supported.",
            );
        }
        let Some(relative) = decode_path(relative) else {
            return response(stream, 400, "text/plain", b"Invalid asset path.");
        };
        let relative = Path::new(&relative);
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return response(stream, 403, "text/plain", b"Invalid asset path.");
        }
        let path = match fs::canonicalize(self.root.join(relative)) {
            Ok(path) if path.starts_with(&self.root) => path,
            _ => return response(stream, 404, "text/plain", b"Unknown package asset."),
        };
        if !path.is_file() || path.metadata()?.len() > MAX_ASSET {
            return response(stream, 413, "text/plain", b"Asset is not a bounded file.");
        }
        let mime = match path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
        {
            "html" => "text/html; charset=utf-8",
            "js" | "cjs" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "json" => "application/json",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "svg" => "image/svg+xml",
            "ico" => "image/x-icon",
            "woff" => "font/woff",
            "woff2" => "font/woff2",
            "wasm" => "application/wasm",
            _ => "application/octet-stream",
        };
        let mut body = Vec::new();
        fs::File::open(path)?
            .take(MAX_ASSET + 1)
            .read_to_end(&mut body)?;
        if body.len() as u64 > MAX_ASSET {
            return response(stream, 413, "text/plain", b"Asset exceeded its size limit.");
        }
        response(stream, 200, mime, &body)
    }
}

fn bounded_read(
    stream: &mut TcpStream,
    bytes: &mut [u8],
    deadline: Instant,
) -> std::io::Result<usize> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "plugin request deadline expired",
            )
        })?;
    stream.set_read_timeout(Some(remaining.min(Duration::from_secs(3))))?;
    stream.read(bytes)
}

fn decode_path(value: &str) -> Option<String> {
    let mut result = Vec::new();
    let mut bytes = value.as_bytes().iter().copied();
    while let Some(byte) = bytes.next() {
        let byte = if byte == b'%' {
            (bytes.next()? as char).to_digit(16)? * 16 + (bytes.next()? as char).to_digit(16)?
        } else {
            byte as u32
        } as u8;
        if matches!(byte, 0 | b'\\' | b':') {
            return None;
        }
        result.push(byte);
    }
    String::from_utf8(result).ok()
}

#[cfg(test)]
mod tests {
    use super::decode_path;
    #[test]
    fn encoded_asset_paths_do_not_create_windows_namespaces() {
        assert_eq!(
            decode_path("assets/%E4%B8%AD%E6%96%87.js").as_deref(),
            Some("assets/中文.js")
        );
        for path in ["a%00", "%5Csecret", "C%3A/file", "%z0", "%FF", "a%"] {
            assert!(decode_path(path).is_none());
        }
    }

    #[test]
    #[ignore = "requires an explicitly pinned local Node executable"]
    fn externally_hosted_sessions_keep_the_origin_capability_and_document_gate() {
        use super::*;
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("preload.cjs"),
            "window.nodeDocument=document.nodeType;",
        )
        .unwrap();
        let session = RendererModules::start(
            NodeConfig {
                executable: std::env::var_os("RDESKTOP_TEST_NODE").unwrap().into(),
                executable_sha256: std::env::var("RDESKTOP_TEST_NODE_SHA256").unwrap(),
                package_root: root.path().into(),
                preload: root.path().join("preload.cjs"),
                timeout: Duration::from_secs(3),
            },
            "http://127.0.0.1:45678/v1/owned/",
        )
        .unwrap();
        let doc = "11111111111111111111111111111111";
        assert_eq!(
            session
                .respond(
                    "https://unrelated.example",
                    &session.token,
                    doc,
                    json!({"operation":"attach"})
                )
                .0,
            403
        );
        assert_eq!(
            session
                .respond(&session.origin, "wrong", doc, json!({"operation":"attach"}))
                .0,
            403
        );
        assert_eq!(
            session
                .respond(
                    &session.origin,
                    &session.token,
                    doc,
                    json!({"operation":"events"})
                )
                .0,
            403
        );
        assert_eq!(
            session
                .respond(
                    &session.origin,
                    &session.token,
                    doc,
                    json!({"operation":"attach"})
                )
                .0,
            200
        );
        assert_eq!(
            session
                .respond(
                    &session.origin,
                    &session.token,
                    "22222222222222222222222222222222",
                    json!({"operation":"attach"})
                )
                .0,
            403
        );
        let response = session.respond(
            &session.origin,
            &session.token,
            doc,
            json!({"operation":"preload"}),
        );
        assert_eq!(response.0, 200);
        assert!(response.1["data"]["source"]
            .as_str()
            .unwrap()
            .contains("document.nodeType"));
        assert!(session
            .initialization_script()
            .contains("http://127.0.0.1:45678/v1/owned/"));
    }
}
