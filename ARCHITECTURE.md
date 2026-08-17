# rdesktop Architecture

## Design Philosophy

rdesktop is built on three core principles:

1. **Renderer as a choice**: The rendering engine should be configurable, not hardcoded
2. **Batteries included**: Windows EXE/installer generation should "just work"
3. **Progressive disclosure**: Simple things should be simple, complex things are possible

## System Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     User Application                         │
│                    (rdesktop-core::App)                       │
├─────────────────────────────────────────────────────────────┤
│                      Renderer Trait                          │
│  ┌──────────────────┐    ┌──────────────────┐               │
│  │  WebView Backend  │    │  Chrome Backend   │               │
│  │  (rdesktop-webview)│    │  (rdesktop-cef)   │               │
│  ├──────────────────┤    ├──────────────────┤               │
│  │ Windows: WebView2│    │ CEF (Chromium)   │               │
│  │ macOS: WKWebView │    │ Off-screen render│               │
│  │ Linux: WebKitGTK │    │ OpenGL/Vulkan    │               │
│  └──────────────────┘    └──────────────────┘               │
├─────────────────────────────────────────────────────────────┤
│                      Window Management                       │
│  ┌──────────────────┐                                       │
│  │     tao           │  (Tauri's window library fork)        │
│  └──────────────────┘                                       │
├─────────────────────────────────────────────────────────────┤
│                      Dev Server (Agent-First)                │
│  ┌──────────────────┐                                       │
│  │  rdesktop-dev     │  HTTP server + Agent API              │
│  │  (axum)           │  Browser-based development            │
│  └──────────────────┘                                       │
├─────────────────────────────────────────────────────────────┤
│                      Bundler                                 │
│  ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐   │
│  │ NSIS │ │ WiX  │ │Porta.│ │ DMG  │ │AppIm.│ │ DEB  │   │
│  └──────┘ └──────┘ └──────┘ └──────┘ └──────┘ └──────┘   │
└─────────────────────────────────────────────────────────────┘
```

## Crate Dependency Graph

```
rdesktop-cli
    ├── rdesktop-bundler
    │   └── rdesktop-core
    ├── rdesktop-dev
    │   └── rdesktop-core
    └── rdesktop-core

rdesktop-webview
    ├── rdesktop-core
    ├── wry 0.56       (WebView rendering)
    └── tao 0.36       (Window management)

rdesktop-cef
    └── rdesktop-core   (Chrome/Edge backend via CDP, chromiumoxide 0.9)
        ├── chromiumoxide 0.9  (Chrome DevTools Protocol binding)
        ├── tao 0.36           (window management)
        ├── tokio              (async runtime for CDP)
        └── png 0.18           (screenshot decode) + windows-sys (GDI blit)
```

## Renderer Abstraction

The `Renderer` trait is the central abstraction:

```rust
pub trait Renderer {
    fn init(&mut self) -> Result<()>;
    fn create_window(&mut self, config: &WindowConfig) -> Result<WindowHandle>;
    fn load_url(&self, window: WindowHandle, url: &str) -> Result<()>;
    fn load_html(&self, window: WindowHandle, html: &str) -> Result<()>;
    fn eval_script(&self, window: WindowHandle, script: &str) -> Result<()>;
    fn set_ipc_handler(&mut self, handler: Box<dyn IpcHandler>);
    fn send_to_frontend(&self, window: WindowHandle, message: &str) -> Result<()>;
    fn set_title(&self, window: WindowHandle, title: &str) -> Result<()>;
    fn set_size(&self, window: WindowHandle, width: u32, height: u32) -> Result<()>;
    fn set_resizable(&self, window: WindowHandle, resizable: bool) -> Result<()>;
    fn set_visible(&self, window: WindowHandle, visible: bool) -> Result<()>;
    fn close_window(&mut self, window: WindowHandle) -> Result<()>;
    fn run(self: Box<Self>) -> Result<()>;  // Consumes self, enters event loop
    fn kind(&self) -> RendererKind;
}
```

### Lifecycle

```
1. Renderer::new(config)
2. renderer.init()
3. renderer.set_ipc_handler(handler)
4. renderer.create_window(config) → WindowHandle  // Queued
5. renderer.load_url(handle, url)                  // Queued
6. Box::new(renderer).run()                        // Enters event loop, processes queue
```

The `run()` method consumes the renderer and enters the platform event loop.
All operations before `run()` are queued and processed when the event loop starts.

## IPC Architecture

```
┌─────────────┐     IPC Message      ┌─────────────┐
│  Frontend    │ ──────────────────>  │  Backend     │
│  (JavaScript)│                      │  (Rust)      │
│              │ <──────────────────  │              │
└─────────────┘     IPC Response     └─────────────┘
```

### JavaScript Bridge

Every WebView gets an injected bridge script:

```javascript
// Call from JavaScript
const result = await window.__RDESKTOP_INVOKE__('greet', { name: 'World' });
```

The bridge uses `window.ipc.postMessage()` (wry's IPC) to send messages to Rust.
Responses are delivered back via `window.__RDESKTOP_IPC__(jsonString)`.

### IPC Message Format

```json
// Request (JS → Rust)
{ "id": "abc123", "cmd": "greet", "payload": { "name": "World" } }

// Response (Rust → JS)
{ "id": "abc123", "success": true, "data": { "message": "Hello, World!" } }
```

### IPC Response Flow (WebView Backend)

1. JS calls `window.__RDESKTOP_INVOKE__(cmd, payload)`
2. Bridge calls `window.ipc.postMessage(json)` → wry IPC handler
3. Rust `IpcHandler.handle(msg)` → `IpcResponse`
4. Response queued in `IpcResponseQueue`
5. Event loop drains queue, calls `evaluate_script("window.__RDESKTOP_IPC__(json)")`

## Agent-First Development

rdesktop supports browser-based development for AI agent workflows:

```
┌─────────────────────────────────────────────┐
│              rdesktop dev                     │
│  ┌─────────────────────────────────────────┐│
│  │         Dev Server (axum)                ││
│  │  ┌───────────────────────────────────┐  ││
│  │  │  Static Files (frontend/)          │  ││
│  │  ├───────────────────────────────────┤  ││
│  │  │  Agent API (/__rdesktop__/agent/)  │  ││
│  │  │  - DOM snapshots                   │  ││
│  │  │  - Element queries                 │  ││
│  │  │  - Action execution                │  ││
│  │  │  - IPC bridge                      │  ││
│  │  └───────────────────────────────────┘  ││
│  └─────────────────────────────────────────┘│
│         ↑                                    │
│    Browser (localhost:1420)                   │
│         ↑                                    │
│    AI Agent (Playwright/Puppeteer MCP)        │
└─────────────────────────────────────────────┘
```

### Agent API Endpoints

| Endpoint | Method | Description |
|---|---|---|
| `/__rdesktop__/health` | GET | Health check |
| `/__rdesktop__/info` | GET | Dev server info |
| `/__rdesktop__/agent/dom` | GET | Full DOM snapshot |
| `/__rdesktop__/agent/elements?selector=` | GET | Query elements by CSS |
| `/__rdesktop__/agent/action` | POST | Execute UI action |
| `/__rdesktop__/agent/state` | GET | Application state |
| `/__rdesktop__/agent/ipc` | POST | Send IPC message |
| `/__rdesktop__/agent/screenshot` | GET | Screenshot (via Playwright) |

## Bundler Design

The bundler is designed to be self-contained:

1. **No external dependencies** where possible (NSIS/WiX tools bundled)
2. **Platform-aware defaults** (auto-selects appropriate targets)
3. **Single command** to produce a distributable package

### Windows Bundler Strategy

```
rdesktop bundle
    │
    ├── Portable EXE (default)
    │   └── Single .exe with embedded resources
    │
    ├── NSIS Installer
    │   ├── Generates .nsi script
    │   ├── Bundles NSIS compiler
    │   └── Produces setup.exe
    │
    └── WiX MSI
        ├── Generates .wxs manifest
        ├── Bundles WiX toolset
        └── Produces installer.msi
```

### Key Differences from Tauri's Bundler

| Aspect | Tauri | rdesktop |
|--------|-------|----------|
| NSIS setup | Manual download | Bundled |
| WiX setup | Manual download | Bundled |
| Direct EXE | Not supported | First-class support |
| WebView2 runtime | Must be installed | Auto-bootstrapper embedded |
| Dev mode | Native window only | Browser mode (Agent-first) |

## Chrome / Edge Backend (CDP via chromiumoxide)

CEF requires a multi-process architecture:

```
┌─────────────────────────────────────────┐
│              Main Process                │
│  ┌─────────────────────────────────┐    │
│  │         Native Window            │    │
│  │  ┌───────────────────────────┐  │    │
│  │  │    OpenGL/Vulkan Surface   │  │    │
│  │  │  ┌─────────────────────┐  │  │    │
│  │  │  │   CEF Render Buffer  │  │  │    │
│  │  │  └─────────────────────┘  │  │    │
│  │  └───────────────────────────┘  │    │
│  └─────────────────────────────────┘    │
│                    │                      │
│              IPC (shared memory)          │
│                    │                      │
│  ┌─────────────────────────────────┐    │
│  │         CEF Browser Process      │    │
│  │  ┌───────────────────────────┐  │    │
│  │  │      Chromium Engine       │  │    │
│  │  └───────────────────────────┘  │    │
│  └─────────────────────────────────┘    │
└─────────────────────────────────────────┘
```

### Design notes

The Chrome backend does **not** embed the CEF SDK. Instead it drives an
installed Chrome/Edge over the Chrome DevTools Protocol (CDP) using
`chromiumoxide`, which avoids shipping ~150MB of CEF binaries:

1. **Rendering**: Each frame captures a PNG screenshot via `Page.captureScreenshot`,
   decodes it (`png` crate, RGBA→BGRA), and blits to the native tao window with
   `StretchDIBits` (Windows GDI). Efficient composited-GPU rendering is a future
   optimization; screenshot-blit is the current portable approach.
2. **Input**: Mouse move/button/wheel and keyboard events are translated to CDP
   `Input.dispatch*` commands. Keyboard `physical_key` is mapped to CDP `key`/
   `text`; Shift/CapsLock state is tracked so `Shift+1` yields `"!"` and
   `Shift+a` yields `"A"`.
3. **IPC (pull-based)**: The injected bridge exposes `window.__RDESKTOP_INVOKE__`
   (returns a promise) and queues calls in `window.__RDESKTOP_QUEUE__`. Rust polls
   `window.__rdesktop_take__()` each frame, dispatches to the `Arc<dyn IpcHandler>`,
   and pushes the `IpcResponse` back via `window.__RDESKTOP_IPC__(json)`.
   `send_to_frontend` uses the same push path. A pull model is used (rather than
   CDP `Runtime.bindingCalled`) because the chromiumoxide `Handler` stream only
   yields `Result<()>` and the event stream is not `Send`, which complicates
   moving it into the `FnMut` loop.
4. **Platform**: GDI blit and Chrome auto-discovery are Windows-first; other
   platforms fall back to a `tracing::warn!` no-op for blit.

> The Chrome backend is functional on Windows (headless Chrome/Edge + GDI).
> WebView remains the default production backend; Chrome is opt-in.

## Platform Support

| Platform | WebView | Chrome | Status |
|---|---|---|---|
| Windows 10/11 | WebView2 (Edge) | CDP (headless Chrome/Edge + GDI) ✅ | WebView ✅ / Chrome ✅ |
| macOS 10.15+ | WKWebView | CDP (pending GDI port) | WebView ✅ / Chrome Planned |
| Linux (X11/Wayland) | WebKitGTK | CDP (pending GDI port) | WebView Planned / Chrome Planned |

## Future Roadmap

- [ ] Full CEF binary integration
- [ ] macOS/Linux WebView testing
- [ ] Code signing support (macOS/Windows)
- [ ] Auto-update mechanism
- [ ] Plugin system
- [ ] Hot reload in dev mode (file watcher + WebSocket)
