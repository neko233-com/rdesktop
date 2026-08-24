use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A message sent from the frontend (JavaScript) to the backend (Rust).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcMessage {
    /// Unique message ID for request-response correlation
    pub id: String,

    /// The command/method to invoke
    pub cmd: String,

    /// JSON payload
    pub payload: serde_json::Value,
}

/// A response sent from the backend to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    /// Correlation ID matching the request
    pub id: String,

    /// Whether the command succeeded
    pub success: bool,

    /// Response data (on success) or error message (on failure)
    pub data: serde_json::Value,
}

/// A thread-safe sink used by asynchronous IPC handlers to deliver a response
/// back to the renderer event loop. The renderer owns the queue and decides
/// when the response is evaluated in the frontend; handlers never touch a
/// WebView or a tao window directly.
pub type IpcResponseSender = Arc<dyn Fn(IpcResponse) + Send + Sync + 'static>;

/// Handler for IPC messages from the frontend.
pub trait IpcHandler: Send + Sync {
    /// Handle an IPC message and return a response.
    fn handle(&self, message: IpcMessage) -> IpcResponse;

    /// Handle an IPC message without blocking the renderer event loop.
    ///
    /// Renderers invoke this method from a worker thread. Existing handlers
    /// remain synchronous by default, while handlers that already use an
    /// async runtime can override this method and call `respond` when their
    /// operation completes. Responses are correlated by `IpcResponse.id`, so
    /// asynchronous replies may safely arrive out of order.
    fn handle_async(&self, message: IpcMessage, respond: IpcResponseSender) {
        respond(self.handle(message));
    }
}

/// A function-based IPC handler.
pub struct FnIpcHandler<F>
where
    F: Fn(IpcMessage) -> IpcResponse + Send + Sync,
{
    handler: F,
}

impl<F> FnIpcHandler<F>
where
    F: Fn(IpcMessage) -> IpcResponse + Send + Sync,
{
    pub fn new(handler: F) -> Self {
        Self { handler }
    }
}

impl<F> IpcHandler for FnIpcHandler<F>
where
    F: Fn(IpcMessage) -> IpcResponse + Send + Sync,
{
    fn handle(&self, message: IpcMessage) -> IpcResponse {
        (self.handler)(message)
    }
}

#[cfg(test)]
mod tests {
    use super::{FnIpcHandler, IpcHandler, IpcMessage, IpcResponse, IpcResponseSender};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[test]
    fn default_async_dispatch_preserves_sync_handler_contract() {
        let handler = FnIpcHandler::new(|message: IpcMessage| IpcResponse {
            id: message.id,
            success: true,
            data: json!({ "method": message.cmd }),
        });
        let received = Arc::new(Mutex::new(None));
        let sink_target = received.clone();
        let sink: IpcResponseSender = Arc::new(move |response| {
            *sink_target.lock().unwrap() = Some(response);
        });

        handler.handle_async(
            IpcMessage {
                id: "async-1".to_string(),
                cmd: "ping".to_string(),
                payload: json!({}),
            },
            sink,
        );

        let response = received.lock().unwrap().take().unwrap();
        assert_eq!(response.id, "async-1");
        assert_eq!(response.data["method"], "ping");
    }
}
