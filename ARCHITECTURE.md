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
    └── rdesktop-core   (CEF backend - stub)
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

## Chrome Embedded (CEF) Architecture

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

### CEF Integration Steps

1. **Binary distribution**: CEF binaries are ~150MB and must be platform-specific
2. **Subprocess model**: CEF requires separate browser/renderer/GPU processes
3. **Off-screen rendering**: CEF renders to a buffer that's composited into the native window
4. **IPC bridge**: Messages between Rust and JavaScript go through CEF's IPC mechanism

> CEF backend is currently a stub. WebView is the production-ready backend.

## Platform Support

| Platform | WebView | Chrome | Status |
|---|---|---|---|
| Windows 10/11 | WebView2 (Edge) | CEF (stub) | WebView ✅ |
| macOS 10.15+ | WKWebView | CEF (stub) | Planned |
| Linux (X11/Wayland) | WebKitGTK | CEF (stub) | Planned |

## Future Roadmap

- [ ] Full CEF binary integration
- [ ] macOS/Linux WebView testing
- [ ] Code signing support (macOS/Windows)
- [ ] Auto-update mechanism
- [ ] Plugin system
- [ ] Hot reload in dev mode (file watcher + WebSocket)
