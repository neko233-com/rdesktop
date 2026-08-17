//! Chrome/Chromium backend via Chrome DevTools Protocol (CDP).
//!
//! Launches Chrome in headless mode, captures screenshots via CDP,
//! and renders them to native tao windows using Windows GDI.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use rdesktop_core::config::{AppConfig, WindowConfig};
use rdesktop_core::ipc::{IpcHandler, IpcMessage};
use rdesktop_core::renderer::{Renderer, RendererKind, ResizeEdge};
use rdesktop_core::window::WindowHandle;
use rdesktop_core::Result;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchMouseEventParams, MouseButton,
    DispatchMouseEventType, DispatchKeyEventType,
};
use chromiumoxide::page::Page;
use futures::StreamExt;
use tao::event::{ElementState, Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Window, WindowBuilder, WindowId};
use tokio::runtime::Runtime;

// ── Windows GDI FFI ──────────────────────────────────────────────
#[cfg(target_os = "windows")]
#[allow(non_snake_case)]
mod gdi {
    use std::ffi::c_void;

    #[repr(C)]
    pub struct BITMAPINFOHEADER {
        pub biSize: u32,
        pub biWidth: i32,
        pub biHeight: i32,
        pub biPlanes: u16,
        pub biBitCount: u16,
        pub biCompression: u32,
        pub biSizeImage: u32,
        pub biXPelsPerMeter: i32,
        pub biYPelsPerMeter: i32,
        pub biClrUsed: u32,
        pub biClrImportant: u32,
    }

    #[repr(C)]
    pub struct RECT {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    pub const BI_RGB: u32 = 0;
    pub const DIB_RGB_COLORS: u32 = 0;
    pub const SRCCOPY: u32 = 0x00CC0020;

    pub type HWND = *mut c_void;
    pub type HDC = *mut c_void;

    #[link(name = "user32")]
    extern "system" {
        pub fn GetDC(hwnd: HWND) -> HDC;
        pub fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
        pub fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> i32;
    }

    #[link(name = "gdi32")]
    extern "system" {
        pub fn StretchDIBits(
            hdc: HDC,
            x_dest: i32, y_dest: i32,
            dest_width: i32, dest_height: i32,
            x_src: i32, y_src: i32,
            src_width: i32, src_height: i32,
            bits: *const c_void,
            bits_info: *const c_void,
            usage: u32,
            rop: u32,
        ) -> i32;
    }
}
// ─────────────────────────────────────────────────────────────────

struct ChromePage {
    page: Page,
    width: u32,
    height: u32,
    pixels: Vec<u8>, // BGRA
    mouse_pos: (f64, f64),
}

enum PendingOp {
    LoadUrl(u64, String),
    LoadHtml(u64, String),
    EvalScript(u64, String),
    SendToFrontend(u64, String),
}

pub struct CefRenderer {
    _config: AppConfig,
    pending_pages: RefCell<Vec<(u64, WindowConfig)>>,
    pending_ops: RefCell<Vec<PendingOp>>,
    next_id: RefCell<u64>,
    ipc_handler: Option<Arc<dyn IpcHandler>>,
    mod_shift: RefCell<bool>,
    mod_caps: RefCell<bool>,
}

impl CefRenderer {
    pub fn new(config: &AppConfig) -> Result<Self> {
        Ok(Self {
            _config: config.clone(),
            pending_pages: RefCell::new(Vec::new()),
            pending_ops: RefCell::new(Vec::new()),
            next_id: RefCell::new(1),
            ipc_handler: None,
            mod_shift: RefCell::new(false),
            mod_caps: RefCell::new(false),
        })
    }

    fn alloc_id(&self) -> u64 {
        let mut id = self.next_id.borrow_mut();
        let v = *id;
        *id += 1;
        v
    }

    fn find_chrome() -> Option<String> {
        let lad = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let cands: Vec<String> = if cfg!(target_os = "windows") {
            vec![
                r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
                r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe".into(),
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into(),
                format!(r"{}\Google\Chrome\Application\chrome.exe", lad),
            ]
        } else if cfg!(target_os = "macos") {
            vec![
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into(),
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".into(),
            ]
        } else {
            vec![
                "/usr/bin/google-chrome".into(),
                "/usr/bin/chromium".into(),
                "/usr/bin/chromium-browser".into(),
            ]
        };
        cands.into_iter().find(|p| std::path::Path::new(p).exists())
    }

    fn bridge_script() -> &'static str {
        r#"
        (function() {
            if (window.__RDESKTOP_BRIDGE__) return;
            window.__RDESKTOP_BRIDGE__ = true;
            window.__RDESKTOP_RESOLVE__ = {};
            window.__RDESKTOP_QUEUE__ = [];
            window.__RDESKTOP_INVOKE__ = function(cmd, payload) {
                return new Promise(function(resolve, reject) {
                    var id = Math.random().toString(36).slice(2);
                    window.__RDESKTOP_RESOLVE__[id] = resolve;
                    window.__RDESKTOP_QUEUE__.push({ id: id, cmd: cmd, payload: payload || {} });
                    setTimeout(function() {
                        if (window.__RDESKTOP_RESOLVE__[id]) {
                            delete window.__RDESKTOP_RESOLVE__[id];
                            reject(new Error('IPC timeout'));
                        }
                    }, 30000);
                });
            };
            window.__rdesktop_take__ = function() {
                var q = window.__RDESKTOP_QUEUE__ || [];
                window.__RDESKTOP_QUEUE__ = [];
                return JSON.stringify(q);
            };
            window.__RDESKTOP_IPC__ = function(message) {
                try {
                    var data = typeof message === 'string' ? JSON.parse(message) : message;
                    if (data.id && window.__RDESKTOP_RESOLVE__[data.id]) {
                        window.__RDESKTOP_RESOLVE__[data.id](data);
                        delete window.__RDESKTOP_RESOLVE__[data.id];
                    }
                } catch (e) { console.error('rdesktop IPC error:', e); }
            };
            window.__RDESKTOP_WINDOW__ = {
                minimize: function() {},
                maximize: function() {},
                close: function() { window.close(); },
                startDrag: function() {},
                startResize: function() {},
                setFullscreen: function() {},
                isMaximized: false,
                isFullscreen: false
            };
        })();
        "#
    }

    fn decode_png(png_bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
        let dec = png::Decoder::new(std::io::Cursor::new(png_bytes));
        let mut reader = dec.read_info().ok()?;
        let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
        let info = reader.next_frame(&mut buf).ok()?;
        let (w, h) = (info.width, info.height);
        let rgba = &buf[..info.buffer_size()];
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for c in rgba.chunks_exact(4) {
            bgra.extend_from_slice(&[c[2], c[1], c[0], 255]);
        }
        Some((w, h, bgra))
    }

    #[cfg(target_os = "windows")]
    fn blit(window: &Window, pixels: &[u8], w: u32, h: u32) {
        use tao::platform::windows::WindowExtWindows;
        use gdi::*;

        let hwnd = window.hwnd() as HWND;
        unsafe {
            let hdc = GetDC(hwnd);
            if hdc.is_null() { return; }

            let bmi = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            };

            let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            GetClientRect(hwnd, &mut rect);

            StretchDIBits(
                hdc,
                0, 0,
                rect.right, rect.bottom,
                0, 0,
                w as i32, h as i32,
                pixels.as_ptr() as *const _,
                &bmi as *const _ as *const _,
                DIB_RGB_COLORS,
                SRCCOPY,
            );

            ReleaseDC(hwnd, hdc);
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn blit(_window: &Window, _pixels: &[u8], _w: u32, _h: u32) {
        tracing::warn!("Chrome GDI rendering: Windows only. Other platforms pending.");
    }
}

impl Renderer for CefRenderer {
    fn init(&mut self) -> Result<()> { Ok(()) }

    fn create_window(&mut self, config: &WindowConfig) -> Result<WindowHandle> {
        let id = self.alloc_id();
        self.pending_pages.borrow_mut().push((id, config.clone()));
        Ok(WindowHandle::new(id))
    }

    fn load_url(&self, w: WindowHandle, url: &str) -> Result<()> {
        self.pending_ops.borrow_mut().push(PendingOp::LoadUrl(w.id(), url.into())); Ok(())
    }
    fn load_html(&self, w: WindowHandle, html: &str) -> Result<()> {
        self.pending_ops.borrow_mut().push(PendingOp::LoadHtml(w.id(), html.into())); Ok(())
    }
    fn eval_script(&self, w: WindowHandle, script: &str) -> Result<()> {
        self.pending_ops.borrow_mut().push(PendingOp::EvalScript(w.id(), script.into())); Ok(())
    }
    fn set_ipc_handler(&mut self, h: Box<dyn IpcHandler>) {
        self.ipc_handler = Some(Arc::from(h));
    }
    fn send_to_frontend(&self, w: WindowHandle, msg: &str) -> Result<()> {
        self.pending_ops.borrow_mut().push(PendingOp::SendToFrontend(w.id(), msg.into())); Ok(())
    }
    fn set_title(&self, _w: WindowHandle, _t: &str) -> Result<()> { Ok(()) }
    fn set_size(&self, _w: WindowHandle, _x: u32, _y: u32) -> Result<()> { Ok(()) }
    fn set_resizable(&self, _w: WindowHandle, _r: bool) -> Result<()> { Ok(()) }
    fn set_visible(&self, _w: WindowHandle, _v: bool) -> Result<()> { Ok(()) }
    fn close_window(&mut self, _w: WindowHandle) -> Result<()> { Ok(()) }
    fn minimize_window(&self, _w: WindowHandle) -> Result<()> { Ok(()) }
    fn maximize_window(&self, _w: WindowHandle) -> Result<()> { Ok(()) }
    fn is_maximized(&self, _w: WindowHandle) -> Result<bool> { Ok(false) }
    fn set_fullscreen(&self, _w: WindowHandle, _f: bool) -> Result<()> { Ok(()) }
    fn is_fullscreen(&self, _w: WindowHandle) -> Result<bool> { Ok(false) }
    fn start_drag(&self, _w: WindowHandle) -> Result<()> { Ok(()) }
    fn start_resize(&self, _w: WindowHandle, _e: ResizeEdge) -> Result<()> { Ok(()) }
    fn set_decorations(&self, _w: WindowHandle, _d: bool) -> Result<()> { Ok(()) }
    fn set_always_on_top(&self, _w: WindowHandle, _a: bool) -> Result<()> { Ok(()) }

    fn run(self: Box<Self>) -> Result<()> {
        tracing::info!("Chrome renderer starting");

        let chrome_path = Self::find_chrome().ok_or_else(|| {
            rdesktop_core::RdesktopError::Cef("Chrome not found".into())
        })?;

        let pending_pages = self.pending_pages.borrow_mut().drain(..).collect::<Vec<_>>();
        let pending_ops = self.pending_ops.borrow_mut().drain(..).collect::<Vec<_>>();

        let rt = Runtime::new().map_err(|e| rdesktop_core::RdesktopError::Cef(format!("{}", e)))?;

        let cfg = BrowserConfig::builder()
            .chrome_executable(&chrome_path)
            .no_sandbox()
            .new_headless_mode()
            .build()
            .map_err(|e| rdesktop_core::RdesktopError::Cef(format!("{}", e)))?;

        let (browser, mut handler) = rt.block_on(async {
            Browser::launch(cfg).await
        }).map_err(|e| rdesktop_core::RdesktopError::Cef(format!("{}", e)))?;

        rt.spawn(async move { while let Some(_) = handler.next().await {} });

        // Create Chrome pages
        let mut pages: Vec<(u64, ChromePage)> = Vec::new();
        for (rd_id, wc) in &pending_pages {
            let page = rt.block_on(async {
                browser.new_page("about:blank").await
            }).map_err(|e| rdesktop_core::RdesktopError::Cef(format!("{}", e)))?;

            rt.block_on(async { let _ = page.evaluate(Self::bridge_script()).await; });

            let pixels = rt.block_on(async {
                use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotParams;
                let bytes = page.screenshot(CaptureScreenshotParams::builder().build()).await.ok()?;
                let (_, _, bgra) = Self::decode_png(&bytes)?;
                Some(bgra)
            }).unwrap_or_else(|| vec![0u8; (wc.width * wc.height * 4) as usize]);

            pages.push((*rd_id, ChromePage {
                page, width: wc.width, height: wc.height, pixels, mouse_pos: (0.0, 0.0),
            }));
        }

        // Process pending ops
        for op in &pending_ops {
            let page = pages.iter().find(|(id, _)| match op {
                PendingOp::LoadUrl(i, _) | PendingOp::LoadHtml(i, _)
                | PendingOp::EvalScript(i, _) | PendingOp::SendToFrontend(i, _) => id == i,
            }).map(|(_, p)| &p.page);
            let Some(page) = page else { continue };
            match op {
                PendingOp::LoadUrl(_, url) => { rt.block_on(async { let _ = page.goto(url.as_str()).await; }); }
                PendingOp::LoadHtml(_, html) => { rt.block_on(async { let _ = page.set_content(html.as_str()).await; }); }
                PendingOp::EvalScript(_, script) => { rt.block_on(async { let _ = page.evaluate(script.as_str()).await; }); }
                PendingOp::SendToFrontend(_, msg) => {
                    if let Ok(js) = serde_json::to_string(msg) {
                        let s = format!("window.__RDESKTOP_IPC__({js})");
                        rt.block_on(async { let _ = page.evaluate(s.as_str()).await; });
                    }
                }
            }
        }

        // Enter tao event loop
        let event_loop = EventLoopBuilder::new().build();
        let mut tao_windows: HashMap<WindowId, (Window, u64)> = HashMap::new();

        event_loop.run(move |event, el_target, cf| {
            *cf = ControlFlow::WaitUntil(
                std::time::Instant::now() + std::time::Duration::from_millis(33),
            );

            match event {
                Event::NewEvents(StartCause::Init) => {
                    for (rd_id, wc) in &pending_pages {
                        let window = match WindowBuilder::new()
                            .with_title(&wc.title)
                            .with_inner_size(tao::dpi::LogicalSize::new(wc.width, wc.height))
                            .with_resizable(wc.resizable)
                            .with_decorations(wc.decorations)
                            .with_transparent(wc.transparent)
                            .with_always_on_top(wc.always_on_top)
                            .build(el_target)
                        {
                            Ok(w) => w,
                            Err(e) => { tracing::error!("Window: {}", e); continue; }
                        };

                        if let Some((_, p)) = pages.iter().find(|(id, _)| id == rd_id) {
                            Self::blit(&window, &p.pixels, p.width, p.height);
                        }

                        let tao_id = window.id();
                        tao_windows.insert(tao_id, (window, *rd_id));
                        tracing::info!(rd_id = rd_id, ?tao_id, "Chrome window created");
                    }
                }

                Event::WindowEvent { event: WindowEvent::CursorMoved { position, .. }, window_id, .. } => {
                    if let Some((_, rd_id)) = tao_windows.get(&window_id) {
                        if let Some((_, p)) = pages.iter_mut().find(|(id, _)| id == rd_id) {
                            p.mouse_pos = (position.x, position.y);
                            let params = DispatchMouseEventParams::builder()
                                .r#type(DispatchMouseEventType::MouseMoved)
                                .x(position.x).y(position.y).build().unwrap();
                            rt.block_on(async { let _ = p.page.execute(params).await; });
                        }
                    }
                }

                Event::WindowEvent { event: WindowEvent::MouseInput { state: bs, button, .. }, window_id, .. } => {
                    if let Some((_, rd_id)) = tao_windows.get(&window_id) {
                        if let Some((_, p)) = pages.iter().find(|(id, _)| id == rd_id) {
                            let mt = match bs {
                                ElementState::Pressed => DispatchMouseEventType::MousePressed,
                                ElementState::Released => DispatchMouseEventType::MouseReleased,
                                _ => DispatchMouseEventType::MouseReleased,
                            };
                            let mb = match button {
                                tao::event::MouseButton::Left => MouseButton::Left,
                                tao::event::MouseButton::Right => MouseButton::Right,
                                tao::event::MouseButton::Middle => MouseButton::Middle,
                                _ => return,
                            };
                            let params = DispatchMouseEventParams::builder()
                                .r#type(mt).x(p.mouse_pos.0).y(p.mouse_pos.1).button(mb).build().unwrap();
                            rt.block_on(async { let _ = p.page.execute(params).await; });
                        }
                    }
                }

                Event::WindowEvent { event: WindowEvent::MouseWheel { delta, .. }, window_id, .. } => {
                    if let Some((_, rd_id)) = tao_windows.get(&window_id) {
                        if let Some((_, p)) = pages.iter().find(|(id, _)| id == rd_id) {
                            let (dx, dy) = match delta {
                                tao::event::MouseScrollDelta::LineDelta(dx, dy) => (dx as f64 * 50.0, dy as f64 * 50.0),
                                tao::event::MouseScrollDelta::PixelDelta(pos) => (pos.x, pos.y),
                                _ => (0.0, 0.0),
                            };
                            let params = DispatchMouseEventParams::builder()
                                .r#type(DispatchMouseEventType::MouseWheel).x(p.mouse_pos.0).y(p.mouse_pos.1)
                                .delta_x(dx).delta_y(dy).build().unwrap();
                            rt.block_on(async { let _ = p.page.execute(params).await; });
                        }
                    }
                }

                Event::WindowEvent { event: WindowEvent::KeyboardInput { event: key_event, .. }, window_id, .. } => {
                    if let Some((_, rd_id)) = tao_windows.get(&window_id) {
                        if let Some((_, p)) = pages.iter().find(|(id, _)| id == rd_id) {
                            let ts = match key_event.state {
                                ElementState::Pressed => DispatchKeyEventType::KeyDown,
                                ElementState::Released => DispatchKeyEventType::KeyUp,
                                _ => DispatchKeyEventType::KeyUp,
                            };
                            let physical = format!("{:?}", key_event.physical_key);

                            // Track modifier state so CDP receives the correct
                            // character in `text` (e.g. Shift+1 => "!", Shift+a => "A").
                            if physical == "ShiftLeft" || physical == "ShiftRight" {
                                *self.mod_shift.borrow_mut() = matches!(ts, DispatchKeyEventType::KeyDown);
                            } else if physical == "CapsLock" && matches!(ts, DispatchKeyEventType::KeyDown) {
                                let mut c = self.mod_caps.borrow_mut();
                                *c = !*c;
                            }
                            let shift = *self.mod_shift.borrow();
                            let caps = *self.mod_caps.borrow();

                            let (key_text, text_opt) = cdp_key_event(&physical, shift, caps);
                            let params_builder = DispatchKeyEventParams::builder()
                                .r#type(ts)
                                .key(key_text.clone())
                                .code(physical.clone());
                            let params = if let Some(t) = text_opt {
                                params_builder.text(t).build().unwrap()
                            } else {
                                params_builder.build().unwrap()
                            };
                            rt.block_on(async { let _ = p.page.execute(params).await; });
                        }
                    }
                }

                Event::WindowEvent { event: WindowEvent::CloseRequested, window_id, .. } => {
                    if let Some((_, rd_id)) = tao_windows.remove(&window_id) {
                        if let Some(idx) = pages.iter().position(|(id, _)| *id == rd_id) {
                            let (_, p) = pages.remove(idx);
                            rt.block_on(async { let _ = p.page.close().await; });
                        }
                    }
                    if tao_windows.is_empty() { *cf = ControlFlow::Exit; }
                }

                Event::MainEventsCleared => {
                    // Capture new screenshots and render
                    for (_, p) in pages.iter_mut() {
                        use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotParams;
                        if let Ok(bytes) = rt.block_on(async {
                            p.page.screenshot(CaptureScreenshotParams::builder().build()).await
                        }) {
                            if let Some((w, h, bgra)) = Self::decode_png(&bytes) {
                                p.pixels = bgra;
                                p.width = w;
                                p.height = h;
                            }
                        }
                    }

                    // Frontend → backend IPC: drain queued invokes and dispatch to the handler
                    if let Some(handler) = self.ipc_handler.as_ref() {
                        for (_, p) in pages.iter() {
                            if let Ok(value) = rt.block_on(p.page.evaluate("window.__rdesktop_take__()")) {
                                if let Some(raw) = value.value().and_then(|v| v.as_str()) {
                                    if let Ok(messages) = serde_json::from_str::<Vec<IpcMessage>>(raw) {
                                        for msg in messages {
                                            let response = handler.handle(msg);
                                            if let Ok(json) = serde_json::to_string(&response) {
                                                let script = format!("window.__RDESKTOP_IPC__({})", json);
                                                let _ = rt.block_on(p.page.evaluate(script.as_str()));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Backend → frontend push (live send_to_frontend)
                    let live_ops = self.pending_ops.borrow_mut().drain(..).collect::<Vec<_>>();
                    for op in live_ops {
                        if let PendingOp::SendToFrontend(rd_id, msg) = op {
                            if let Some((_, p)) = pages.iter().find(|(id, _)| *id == rd_id) {
                                if let Ok(js) = serde_json::to_string(&msg) {
                                    let script = format!("window.__RDESKTOP_IPC__({js})");
                                    let _ = rt.block_on(p.page.evaluate(script.as_str()));
                                }
                            }
                        }
                    }

                    for (_, (window, rd_id)) in tao_windows.iter() {
                        if let Some((_, p)) = pages.iter().find(|(id, _)| id == rd_id) {
                            Self::blit(window, &p.pixels, p.width, p.height);
                        }
                    }
                }

                Event::LoopDestroyed => { tracing::info!("Chrome renderer destroyed"); }
                _ => {}
            }
        });
    }

    fn kind(&self) -> RendererKind { RendererKind::Chrome }
}

/// Map a winit `KeyCode` debug name (e.g. "KeyA", "Digit1", "Enter") to the
/// `key`/`text` values expected by CDP `Input.dispatchKeyEvent`.
///
/// Returns `(key, text)` where `text` is `Some` only for printable characters.
/// When `shift` or `caps` is active, letters are uppercased and digits/symbols
/// use their shifted variant, so `Shift+1` produces `"!"` instead of `"1"`.
fn cdp_key_event(code: &str, shift: bool, caps: bool) -> (String, Option<String>) {
    if let Some(c) = code.strip_prefix("Key") {
        let upper = shift ^ caps;
        let ch = if upper { c.to_uppercase() } else { c.to_lowercase() };
        return (ch.clone(), Some(ch));
    }
    if let Some(d) = code.strip_prefix("Digit") {
        const SHIFTED: &[&str] = &[")", "!", "@", "#", "$", "%", "^", "&", "*", "("];
        if let Ok(idx) = d.parse::<usize>() {
            if idx < SHIFTED.len() {
                let s = if shift { SHIFTED[idx].to_string() } else { d.to_string() };
                return (s.clone(), Some(s));
            }
        }
    }
    match code {
        "Enter" => ("Enter".into(), None),
        "Escape" => ("Escape".into(), None),
        "Backspace" => ("Backspace".into(), None),
        "Tab" => ("Tab".into(), None),
        "Space" => (" ".into(), Some(" ".into())),
        "ArrowLeft" => ("ArrowLeft".into(), None),
        "ArrowRight" => ("ArrowRight".into(), None),
        "ArrowUp" => ("ArrowUp".into(), None),
        "ArrowDown" => ("ArrowDown".into(), None),
        "Delete" => ("Delete".into(), None),
        "Home" => ("Home".into(), None),
        "End" => ("End".into(), None),
        "PageUp" => ("PageUp".into(), None),
        "PageDown" => ("PageDown".into(), None),
        "ShiftLeft" | "ShiftRight" => ("Shift".into(), None),
        "ControlLeft" | "ControlRight" => ("Control".into(), None),
        "AltLeft" | "AltRight" => ("Alt".into(), None),
        "MetaLeft" | "MetaRight" => ("Meta".into(), None),
        _ => (code.to_string(), None),
    }
}
