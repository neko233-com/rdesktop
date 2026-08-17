use thiserror::Error;

#[derive(Error, Debug)]
pub enum RdesktopError {
    #[error("Window creation failed: {0}")]
    WindowCreation(String),

    #[error("Renderer initialization failed: {0}")]
    RendererInit(String),

    #[error("IPC error: {0}")]
    Ipc(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Bundler error: {0}")]
    Bundler(String),

    #[error("CEF error: {0}")]
    Cef(String),

    #[error("WebView error: {0}")]
    WebView(String),

    #[error("Unsupported platform: {0}")]
    UnsupportedPlatform(String),

    #[error("Global input error: {0}")]
    GlobalInput(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, RdesktopError>;
