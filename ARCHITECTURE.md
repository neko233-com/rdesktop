# rdesktop Architecture

## Design Philosophy

rdesktop is built on three core principles:

1. **Renderer as a choice**: The rendering engine should be configurable, not hardcoded
2. **Batteries included**: Windows EXE/installer generation should "just work"
3. **Progressive disclosure**: Simple things should be simple, complex things should be possible

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
│  ┌──────────────────┐    ┌──────────────────┐               │
│  │     tao           │    │    winit          │               │
│  │  (Tauri's fork)   │    │  (alternative)    │               │
│  └──────────────────┘    └──────────────────┘               │
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
    └── rdesktop-core

rdesktop-webview
    ├── rdesktop-core
    ├── wry
    └── tao

rdesktop-cef
    └── rdesktop-core
```

## Renderer Abstraction

The `Renderer` trait is the central abstraction:

```rust
pub trait Renderer: Send {
    fn init(&mut self) -> Result<()>;
    fn create_window(&mut self, config: &WindowConfig) -> Result<WindowHandle>;
    fn load_url(&self, window: WindowHandle, url: &str) -> Result<()>;
    fn load_html(&self, window: WindowHandle, html: &str) -> Result<()>;
    fn eval_script(&self, window: WindowHandle, script: &str) -> Result<()>;
    fn set_ipc_handler(&mut self, handler: Box<dyn IpcHandler>);
    fn send_to_frontend(&self, window: WindowHandle, message: &str) -> Result<()>;
    fn set_title(&self, window: WindowHandle, title: &str) -> Result<()>;
    fn set_size(&self, window: WindowHandle, width: u32, height: u32) -> Result<()>;
    fn close_window(&mut self, window: WindowHandle) -> Result<()>;
    fn run(self: Box<Self>) -> Result<()>;
    fn kind(&self) -> RendererKind;
}
```

Both backends implement this trait, so the application code is renderer-agnostic.

## IPC Architecture

```
┌─────────────┐     IPC Message      ┌─────────────┐
│  Frontend    │ ──────────────────>  │  Backend     │
│  (JavaScript)│                      │  (Rust)      │
│              │ <──────────────────  │              │
└─────────────┘     IPC Response     └─────────────┘
```

IPC messages are JSON-encoded:

```json
// Request
{
    "id": "abc123",
    "cmd": "greet",
    "payload": { "name": "World" }
}

// Response
{
    "id": "abc123",
    "success": true,
    "data": { "message": "Hello, World!" }
}
```

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

## Future Roadmap

- [ ] Full CEF binary integration
- [ ] Automatic WebView2 bootstrapper for Windows
- [ ] Code signing support (macOS/Windows)
- [ ] Auto-update mechanism
- [ ] Plugin system
- [ ] Mobile support (iOS/Android) via wry's existing mobile backends
