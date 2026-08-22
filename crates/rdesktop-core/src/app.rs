use crate::config::{AppConfig, WindowConfig};
use crate::ipc::IpcHandler;
use crate::renderer::Renderer;

/// Content to load in the main window.
pub enum WindowContent {
    /// Load a URL.
    Url(String),
    /// Load HTML directly.
    Html(String),
}

/// Builder for creating an rdesktop application.
pub struct AppBuilder {
    config: AppConfig,
    renderer: Option<Box<dyn Renderer>>,
    ipc_handler: Option<Box<dyn IpcHandler>>,
    content: Option<WindowContent>,
    setup_fn: Option<Box<dyn FnOnce(&mut dyn Renderer) -> crate::Result<()>>>,
}

impl AppBuilder {
    /// Create a new AppBuilder with the given configuration.
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            renderer: None,
            ipc_handler: None,
            content: None,
            setup_fn: None,
        }
    }

    /// Set the renderer to use.
    pub fn with_renderer(mut self, renderer: Box<dyn Renderer>) -> Self {
        self.renderer = Some(renderer);
        self
    }

    /// Set the IPC handler for frontend-to-backend communication.
    pub fn with_ipc_handler(mut self, handler: Box<dyn IpcHandler>) -> Self {
        self.ipc_handler = Some(handler);
        self
    }

    /// Set the URL to load in the main window.
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.content = Some(WindowContent::Url(url.into()));
        self
    }

    /// Set the HTML to load in the main window.
    pub fn with_html(mut self, html: impl Into<String>) -> Self {
        self.content = Some(WindowContent::Html(html.into()));
        self
    }

    /// Set a setup function that runs after renderer initialization.
    pub fn with_setup<F>(mut self, setup: F) -> Self
    where
        F: FnOnce(&mut dyn Renderer) -> crate::Result<()> + 'static,
    {
        self.setup_fn = Some(Box::new(setup));
        self
    }

    /// Build the application.
    pub fn build(self) -> crate::Result<App> {
        let renderer = self.renderer.ok_or_else(|| {
            crate::RdesktopError::RendererInit(
                "No renderer provided. Use with_renderer().".to_string(),
            )
        })?;

        Ok(App {
            config: self.config,
            renderer,
            ipc_handler: self.ipc_handler,
            content: self.content,
            setup_fn: self.setup_fn,
        })
    }
}

/// The main application struct.
pub struct App {
    config: AppConfig,
    renderer: Box<dyn Renderer>,
    ipc_handler: Option<Box<dyn IpcHandler>>,
    content: Option<WindowContent>,
    setup_fn: Option<Box<dyn FnOnce(&mut dyn Renderer) -> crate::Result<()>>>,
}

impl App {
    /// Create a new AppBuilder.
    pub fn builder(config: AppConfig) -> AppBuilder {
        AppBuilder::new(config)
    }

    /// Run the application.
    pub fn run(mut self) -> crate::Result<()> {
        // Initialize the renderer
        self.renderer.init()?;

        // Set IPC handler if provided
        if let Some(handler) = self.ipc_handler {
            self.renderer.set_ipc_handler(handler);
        }

        // Create the main window
        let window_config = WindowConfig {
            title: self.config.name.clone(),
            ..self.config.window.clone()
        };
        let handle = self.renderer.create_window(&window_config)?;

        // Load content
        match self.content {
            Some(WindowContent::Url(url)) => {
                self.renderer.load_url(handle, &url)?;
            }
            Some(WindowContent::Html(html)) => {
                self.renderer.load_html(handle, &html)?;
            }
            None => {
                // Load about:blank by default
                self.renderer.load_url(handle, "about:blank")?;
            }
        }

        // Run setup function if provided
        if let Some(setup) = self.setup_fn {
            setup(self.renderer.as_mut())?;
        }

        // Run the event loop (blocks until all windows are closed)
        self.renderer.run()
    }
}
