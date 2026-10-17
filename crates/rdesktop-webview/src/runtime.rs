//! Thread-safe access to a running native renderer. Operations execute on the
//! event-loop thread; callers never move a WebView or window between threads.
use rdesktop_core::{RdesktopError, Result, WindowConfig};
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::{Duration, Instant};
use tao::window::Window;
use wry::WebView;

const MAX_REQUESTS: usize = 256;
type WindowTask = Box<dyn FnOnce(Option<(&Window, &WebView)>) + Send>;
type Wake = Arc<dyn Fn() + Send + Sync>;
pub type WebViewSetup =
    Box<dyn for<'a> FnOnce(wry::WebViewBuilder<'a>) -> wry::WebViewBuilder<'a> + Send>;

#[derive(Default)]
pub struct NativeWindowOptions {
    pub url: String,
    pub initialization_script: String,
    /// Remote browser windows must not receive the application's privileged IPC.
    pub ipc_enabled: bool,
    /// Enable OS file-drop events. On Windows this replaces HTML drag/drop;
    /// leave false when the application uses DOM file inputs or draggable UI.
    pub native_drag_drop: bool,
    /// Keep remote/plugin browser partitions separate from the trusted host.
    pub data_directory: Option<std::path::PathBuf>,
    pub setup: Option<WebViewSetup>,
}

pub(crate) enum Request {
    Create {
        id: u64,
        config: WindowConfig,
        options: Box<NativeWindowOptions>,
        reply: mpsc::SyncSender<std::result::Result<u64, String>>,
        deadline: Instant,
    },
    Execute {
        id: u64,
        task: WindowTask,
    },
    Close(u64),
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeEvent {
    CloseRequested,
    Destroyed,
    Focused(bool),
}
pub type NativeEventHandler = Arc<dyn Fn(u64, NativeEvent) + Send + Sync>;

#[derive(Default)]
struct Shared {
    requests: Mutex<VecDeque<Request>>,
    wake: Mutex<Option<Wake>>,
    next_id: AtomicU64,
    closed: AtomicBool,
    ui_thread: Mutex<Option<std::thread::ThreadId>>,
}

/// A bounded command queue. Synchronous calls must be made from application
/// workers, never from the native event loop. Shutdown and missing-window errors
/// are returned to the caller rather than silently dropping operations.
#[derive(Clone, Default)]
pub struct NativeRuntime {
    shared: Arc<Shared>,
}

impl NativeRuntime {
    fn enqueue(&self, request: Request) -> Result<()> {
        let mut queue = self
            .shared
            .requests
            .lock()
            .map_err(|_| RdesktopError::Config("native queue poisoned".into()))?;
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(RdesktopError::Config("native renderer has stopped".into()));
        }
        if queue.len() >= MAX_REQUESTS {
            return Err(RdesktopError::Config("native queue is full".into()));
        }
        queue.push_back(request);
        drop(queue);
        if let Some(wake) = self.shared.wake.lock().unwrap().as_ref() {
            wake();
        }
        Ok(())
    }

    pub fn create_window(&self, config: WindowConfig, options: NativeWindowOptions) -> Result<u64> {
        self.require_worker()?;
        // IDs in the upper half cannot collide with Renderer::create_window.
        let id = (1 << 63) | self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, receive) = mpsc::sync_channel(1);
        self.enqueue(Request::Create {
            id,
            config,
            options: Box::new(options),
            reply,
            deadline: Instant::now() + Duration::from_secs(30),
        })?;
        receive
            .recv_timeout(Duration::from_secs(30))
            .map_err(|e| RdesktopError::Config(format!("native creation failed: {e}")))?
            .map_err(RdesktopError::Config)
    }

    pub fn with_window<T: Send + 'static>(
        &self,
        id: u64,
        operation: impl FnOnce(&Window, &WebView) -> T + Send + 'static,
    ) -> Result<T> {
        self.require_worker()?;
        let (send, receive) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let task_cancelled = cancelled.clone();
        self.enqueue(Request::Execute {
            id,
            task: Box::new(move |entry| {
                if task_cancelled.load(Ordering::Acquire) {
                    return;
                }
                let result = entry.map(|(window, webview)| operation(window, webview));
                let _ = send.send(result);
            }),
        })?;
        let result = receive.recv_timeout(Duration::from_secs(30));
        if result.is_err() {
            cancelled.store(true, Ordering::Release);
        }
        result
            .map_err(|e| RdesktopError::Config(format!("native operation failed: {e}")))?
            .ok_or_else(|| RdesktopError::Config("native window no longer exists".into()))
    }

    pub fn close_window(&self, id: u64) -> Result<()> {
        self.enqueue(Request::Close(id))
    }
    pub fn exit(&self) -> Result<()> {
        self.enqueue(Request::Exit)
    }
    pub(crate) fn set_wake(&self, wake: Wake) {
        *self.shared.ui_thread.lock().unwrap() = Some(std::thread::current().id());
        *self.shared.wake.lock().unwrap() = Some(wake);
    }
    fn require_worker(&self) -> Result<()> {
        if *self.shared.ui_thread.lock().unwrap() == Some(std::thread::current().id()) {
            return Err(RdesktopError::Config(
                "synchronous native calls must run on a worker".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn shutdown(&self) {
        let mut queue = self.shared.requests.lock().unwrap();
        self.shared.closed.store(true, Ordering::Release);
        queue.clear();
        self.shared.wake.lock().unwrap().take();
    }
    pub(crate) fn drain(&self) -> Vec<Request> {
        self.shared.requests.lock().unwrap().drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_loop_reentrancy_and_calls_after_shutdown_fail_immediately() {
        let runtime = NativeRuntime::default();
        runtime.set_wake(Arc::new(|| {}));
        assert!(runtime
            .with_window(1, |_, _| ())
            .unwrap_err()
            .to_string()
            .contains("worker"));
        runtime.shutdown();
        assert!(runtime
            .close_window(1)
            .unwrap_err()
            .to_string()
            .contains("stopped"));
    }
    #[test]
    fn queue_is_bounded_and_missing_windows_return_errors() {
        let runtime = NativeRuntime::default();
        for _ in 0..MAX_REQUESTS {
            runtime.close_window(1).unwrap();
        }
        assert!(runtime.close_window(1).is_err());
        assert_eq!(runtime.drain().len(), MAX_REQUESTS);
        let worker_runtime = runtime.clone();
        let worker = std::thread::spawn(move || worker_runtime.with_window(7, |_, _| 42));
        loop {
            if let Some(Request::Execute { task, .. }) = runtime.drain().pop() {
                task(None);
                break;
            }
            std::thread::yield_now();
        }
        assert!(worker
            .join()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("no longer exists"));
    }
}
