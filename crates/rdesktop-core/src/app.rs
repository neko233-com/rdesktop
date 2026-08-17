use crate::config::{AppConfig, WindowConfig};
use crate::ipc::IpcHandler;
use crate::renderer::Renderer;

/// Builder for creating an rdesktop application.
pub struct AppBuilder {
    config: AppConfig,
    renderer: Option<Box<dyn Renderer>>,
    ipc_handler: Option<Box<dyn IpcHandler>>,
    setup_fn: Option<Box<dyn FnOnce(&mut dyn Renderer) -> crate::Result<()>>>,
}

impl AppBuilder {
    /// Create a new AppBuilder with the given configuration.
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            renderer: None,
            ipc_handler: None,
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
        let renderer = self
            .renderer
            .ok_or_else(|| crate::RdesktopError::RendererInit("No renderer provided. Use with_renderer().".to_string()))?;

        Ok(App {
            config: self.config,
            renderer,
            ipc_handler: self.ipc_handler,
            setup_fn: self.setup_fn,
        })
    }
}

/// The main application struct.
pub struct App {
    config: AppConfig,
    renderer: Box<dyn Renderer>,
    ipc_handler: Option<Box<dyn IpcHandler>>,
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
        self.renderer.create_window(&window_config)?;

        // Run setup function if provided
        if let Some(setup) = self.setup_fn {
            setup(self.renderer.as_mut())?;
        }

        // Run the event loop
        self.renderer.run()
    }
}
