//! Chrome/Chromium backend via Chrome DevTools Protocol (CDP).

use std::collections::HashMap;

use rdesktop_core::config::{AppConfig, WindowConfig};
use rdesktop_core::ipc::IpcHandler;
use rdesktop_core::renderer::{Renderer, RendererKind, ResizeEdge};
use rdesktop_core::window::WindowHandle;
use rdesktop_core::Result;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::page::Page;
use futures::StreamExt;
use tokio::runtime::Runtime;

struct ChromeWindowEntry {
    page: Page,
    _width: u32,
    _height: u32,
}

/// Chrome/Chromium renderer using CDP.
pub struct CefRenderer {
    _config: AppConfig,
    runtime: Runtime,
    browser: Option<Browser>,
    pages: HashMap<u64, ChromeWindowEntry>,
    ipc_handler: Option<std::sync::Arc<dyn IpcHandler>>,
    next_window_id: u64,
}

impl CefRenderer {
    pub fn new(config: &AppConfig) -> Result<Self> {
        let runtime = Runtime::new()
            .map_err(|e| rdesktop_core::RdesktopError::Cef(format!("Failed to create tokio runtime: {}", e)))?;

        Ok(Self {
            _config: config.clone(),
            runtime,
            browser: None,
            pages: HashMap::new(),
            ipc_handler: None,
            next_window_id: 1,
        })
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_window_id;
        self.next_window_id += 1;
        id
    }

    fn find_chrome() -> Option<String> {
        let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();

        let candidates: Vec<String> = if cfg!(target_os = "windows") {
            vec![
                r"C:\Program Files\Google\Chrome\Application\chrome.exe".to_string(),
                r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe".to_string(),
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".to_string(),
                format!(r"{}\Google\Chrome\Application\chrome.exe", local_app_data),
            ]
        } else if cfg!(target_os = "macos") {
            vec![
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".to_string(),
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".to_string(),
                "/Applications/Chromium.app/Contents/MacOS/Chromium".to_string(),
            ]
        } else {
            vec![
                "/usr/bin/google-chrome".to_string(),
                "/usr/bin/google-chrome-stable".to_string(),
                "/usr/bin/chromium".to_string(),
                "/usr/bin/chromium-browser".to_string(),
                "/snap/bin/chromium".to_string(),
            ]
        };

        for path in &candidates {
            if std::path::Path::new(path).exists() {
                return Some(path.clone());
            }
        }

        None
    }

    fn bridge_script() -> &'static str {
        r#"
        (function() {
            if (window.__RDESKTOP_BRIDGE__) return;
            window.__RDESKTOP_BRIDGE__ = true;
            window.__RDESKTOP_RESOLVE__ = {};

            window.__RDESKTOP_INVOKE__ = function(cmd, payload) {
                return new Promise(function(resolve, reject) {
                    var id = Math.random().toString(36).slice(2);
                    window.__RDESKTOP_RESOLVE__[id] = resolve;
                    window.dispatchEvent(new CustomEvent('__rdesktop_ipc__', {
                        detail: JSON.stringify({ id: id, cmd: cmd, payload: payload || {} })
                    }));
                    setTimeout(function() {
                        if (window.__RDESKTOP_RESOLVE__[id]) {
                            delete window.__RDESKTOP_RESOLVE__[id];
                            reject(new Error('IPC timeout'));
                        }
                    }, 30000);
                });
            };

            window.__RDESKTOP_IPC__ = function(message) {
                try {
                    var data = typeof message === 'string' ? JSON.parse(message) : message;
                    if (data.id && window.__RDESKTOP_RESOLVE__[data.id]) {
                        window.__RDESKTOP_RESOLVE__[data.id](data);
                        delete window.__RDESKTOP_RESOLVE__[data.id];
                    }
                } catch (e) {
                    console.error('rdesktop IPC error:', e);
                }
            };

            window.__RDESKTOP_WINDOW__ = {
                minimize: function() { console.log('rdesktop: minimize'); },
                maximize: function() { console.log('rdesktop: maximize'); },
                close: function() { window.close(); },
                startDrag: function() { console.log('rdesktop: startDrag'); },
                startResize: function(edge) { console.log('rdesktop: startResize', edge); },
                setFullscreen: function(fs) { console.log('rdesktop: setFullscreen', fs); },
                isMaximized: false,
                isFullscreen: false
            };
        })();
        "#
    }
}

impl Renderer for CefRenderer {
    fn init(&mut self) -> Result<()> {
        tracing::info!("Initializing Chrome renderer (CDP)");

        let chrome_path = Self::find_chrome().ok_or_else(|| {
            rdesktop_core::RdesktopError::Cef(
                "Chrome/Chromium not found. Install Google Chrome or Microsoft Edge.".to_string(),
            )
        })?;

        tracing::info!(path = %chrome_path, "Found Chrome executable");

        let config = BrowserConfig::builder()
            .chrome_executable(&chrome_path)
            .no_sandbox()
            .new_headless_mode()
            .build()
            .map_err(|e| rdesktop_core::RdesktopError::Cef(format!("BrowserConfig error: {}", e)))?;

        let (browser, mut handler) = self.runtime.block_on(async {
            Browser::launch(config).await
        }).map_err(|e| rdesktop_core::RdesktopError::Cef(format!("Failed to launch Chrome: {}", e)))?;

        // Spawn the handler as a background task
        self.runtime.spawn(async move {
            while let Some(_event) = handler.next().await {
                // Process CDP events
            }
        });

        self.browser = Some(browser);
        tracing::info!("Chrome browser launched successfully");

        Ok(())
    }

    fn create_window(&mut self, config: &WindowConfig) -> Result<WindowHandle> {
        let id = self.next_id();
        let browser = self.browser.as_ref().ok_or_else(|| {
            rdesktop_core::RdesktopError::Cef("Browser not initialized. Call init() first.".to_string())
        })?;

        let page = self.runtime.block_on(async {
            browser.new_page("about:blank").await
        }).map_err(|e| rdesktop_core::RdesktopError::Cef(format!("Failed to create page: {}", e)))?;

        // Inject the bridge script
        self.runtime.block_on(async {
            let _ = page.evaluate(Self::bridge_script()).await;
        });

        self.pages.insert(id, ChromeWindowEntry {
            page,
            _width: config.width,
            _height: config.height,
        });

        tracing::info!(window_id = id, "Chrome page created");
        Ok(WindowHandle::new(id))
    }

    fn load_url(&self, window: WindowHandle, url: &str) -> Result<()> {
        let entry = self.pages.get(&window.id()).ok_or_else(|| {
            rdesktop_core::RdesktopError::Cef("Window not found".to_string())
        })?;

        self.runtime.block_on(async {
            entry.page.goto(url).await
                .map_err(|e| rdesktop_core::RdesktopError::Cef(format!("Failed to load URL: {}", e)))
        })?;

        Ok(())
    }

    fn load_html(&self, window: WindowHandle, html: &str) -> Result<()> {
        let entry = self.pages.get(&window.id()).ok_or_else(|| {
            rdesktop_core::RdesktopError::Cef("Window not found".to_string())
        })?;

        self.runtime.block_on(async {
            entry.page.set_content(html).await
                .map_err(|e| rdesktop_core::RdesktopError::Cef(format!("Failed to load HTML: {}", e)))
        })?;

        Ok(())
    }

    fn eval_script(&self, window: WindowHandle, script: &str) -> Result<()> {
        let entry = self.pages.get(&window.id()).ok_or_else(|| {
            rdesktop_core::RdesktopError::Cef("Window not found".to_string())
        })?;

        self.runtime.block_on(async {
            entry.page.evaluate(script).await
                .map_err(|e| rdesktop_core::RdesktopError::Cef(format!("Failed to eval script: {}", e)))
        })?;

        Ok(())
    }

    fn set_ipc_handler(&mut self, handler: Box<dyn IpcHandler>) {
        self.ipc_handler = Some(std::sync::Arc::from(handler));
    }

    fn send_to_frontend(&self, window: WindowHandle, message: &str) -> Result<()> {
        let escaped = message.replace('\\', "\\\\").replace('\'', "\\'");
        let script = format!("window.__RDESKTOP_IPC__('{escaped}')");
        self.eval_script(window, &script)
    }

    fn set_title(&self, _window: WindowHandle, _title: &str) -> Result<()> { Ok(()) }
    fn set_size(&self, _window: WindowHandle, _width: u32, _height: u32) -> Result<()> { Ok(()) }
    fn set_resizable(&self, _window: WindowHandle, _resizable: bool) -> Result<()> { Ok(()) }
    fn set_visible(&self, _window: WindowHandle, _visible: bool) -> Result<()> { Ok(()) }

    fn close_window(&mut self, window: WindowHandle) -> Result<()> {
        if let Some(entry) = self.pages.remove(&window.id()) {
            self.runtime.block_on(async {
                let _ = entry.page.close().await;
            });
            tracing::info!(window_id = window.id(), "Chrome page closed");
        }
        Ok(())
    }

    fn minimize_window(&self, _window: WindowHandle) -> Result<()> { Ok(()) }
    fn maximize_window(&self, _window: WindowHandle) -> Result<()> { Ok(()) }
    fn is_maximized(&self, _window: WindowHandle) -> Result<bool> { Ok(false) }
    fn set_fullscreen(&self, _window: WindowHandle, _fullscreen: bool) -> Result<()> { Ok(()) }
    fn is_fullscreen(&self, _window: WindowHandle) -> Result<bool> { Ok(false) }
    fn start_drag(&self, _window: WindowHandle) -> Result<()> { Ok(()) }
    fn start_resize(&self, _window: WindowHandle, _edge: ResizeEdge) -> Result<()> { Ok(()) }
    fn set_decorations(&self, _window: WindowHandle, _decorations: bool) -> Result<()> { Ok(()) }
    fn set_always_on_top(&self, _window: WindowHandle, _always: bool) -> Result<()> { Ok(()) }

    fn run(self: Box<Self>) -> Result<()> {
        tracing::info!("Chrome renderer event loop started");
        tracing::info!("Chrome renderer shutting down");
        Ok(())
    }

    fn kind(&self) -> RendererKind {
        RendererKind::Chrome
    }
}
