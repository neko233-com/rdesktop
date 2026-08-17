use serde::{Deserialize, Serialize};

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

/// Handler for IPC messages from the frontend.
pub trait IpcHandler: Send + Sync {
    /// Handle an IPC message and return a response.
    fn handle(&self, message: IpcMessage) -> IpcResponse;
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
