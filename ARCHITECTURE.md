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

rdesktop-core
    ├── serde / serde_json / anyhow / thiserror / tracing
    ├── tao 0.36            (Window handle type for apply_window_attributes)
    ├── raw-window-handle / dpi / url
    └── window_extras FFI   (Phase 0: layers + click-through)
        ├── windows-sys 0.52   (WS_EX_*, WorkerW discovery, GDI)  [cfg(windows)]
        └── core-graphics 0.24 + objc 0.2  (desktop level, mouse-ignore) [cfg(macos)]
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

## Deep Desktop Window Layers (Phase 0)

rdesktop extends Tauri v2's window model with first-class support for
Wallpaper-Engine-style and HUD scenarios. The `WindowKind` enum and
click-through attribute are realized by `rdesktop_core::window_extras`
(`apply_window_attributes`, called immediately after the tao window is built
inside each backend's `create_window`).

### WindowKind

| Kind | Behavior | Implies `click_through` |
|------|----------|--------------------------|
| `normal` | Standard application window | no |
| `overlay` | Always-on-top HUD / PIP / floating toolbar | no (set explicitly if needed) |
| `wallpaper` | Desktop layer: behind icons, pointer falls through | yes |

### `apply_window_attributes` dispatch

- **Windows** (`window_extras::windows`):
  - *Click-through*: adds `WS_EX_LAYERED | WS_EX_TRANSPARENT` to the extended
    style via `GetWindowLongPtrW` / `SetWindowLongPtrW`.
  - *Wallpaper*: sends `0x52C` to `Progman` to force Explorer to (re)create the
    dedicated wallpaper host, then `EnumWindows` finds the `WorkerW` sibling that
    does **not** own `SHELLDLL_DefView`; the window is `SetParent`ed to it and
    pinned with `SetWindowPos(HWND_BOTTOM, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)`.
- **macOS** (`window_extras::macos`): `setIgnoresMouseEvents:` for click-through;
  `setLevel:` to `CGWindowLevelForKey(kCGDesktopWindowLevelKey)` + `orderBack:`
  for wallpaper.
- **Other platforms**: `tracing::debug!` no-op.

### WebGPU for frontend-native shaders

`RendererConfig.webgpu` (default `true`) enables the frontend to drive native
shaders. The WebView backend passes `--enable-features=Vulkan,WebGPU
--enable-unsafe-webgpu` on Windows (via `WebViewBuilderExtWindows`); macOS
WKWebView and Linux WebKitGTK expose WebGPU through their own paths.
`examples/wallpaper` renders an animated full-screen WebGPU shader behind the
desktop, with a CSS-gradient fallback when WebGPU is unavailable.

### Caveats

- The Windows `WorkerW` reparent is a well-established Explorer interop
  technique; it is robust on Windows 10/11 but can be disturbed by an Explorer
  restart (handled gracefully — the window simply stays top-most).
- The macOS path is **not compiled on this Windows dev host** and remains
  unverified; it requires `objc` 0.2 + `core-graphics` 0.24 and a macOS build.
- A click-through wallpaper receives no pointer input by design; it must be
  closed programmatically (e.g. via a tray quit command).

## Global Input & Hotkeys (Phase 2)

Phase 2 delivers system-wide input capture and global shortcuts — capabilities
that exceed Tauri v2's `global-shortcut` (which only covers a fixed subset and
no raw input stream). rdesktop owns the platform integration directly and feeds
both backends through a single, renderer-agnostic event contract.

### Module layout (`rdesktop-core`)

```
hotkeys.rs     Platform-agnostic types + manager
  ├─ Modifiers { ctrl, alt, shift, meta }      (from_raw_win / from_raw_mac)
  ├─ Key enum (Letter / Digit / F / Arrow / Function …)
  ├─ Hotkey { modifiers, key }                 (FromStr: "Ctrl+Shift+K", Display)
  ├─ HotkeyHandler trait                        (fn on_hotkey(&self, id, &Hotkey))
  └─ HotkeyManager { register / unregister }    (owns the platform impl)

input.rs       Global input stream + manager
  ├─ KeyState / MouseButton
  ├─ GlobalInputEvent::{ Keyboard, Mouse, MouseMove }
  ├─ GlobalInputHandler trait                   (fn on_event(&self, &GlobalInputEvent))
  └─ GlobalInput { start / stop, with_mouse_move }

#[cfg(windows)]  hotkeys_win.rs / input_win.rs   (real implementations)
#[cfg(macos)]    hotkeys_mac.rs / input_mac.rs    (CGEventTap stubs — unverified)
global.rs      PushHandler + Outbox (the shared delivery contract)
```

### Windows implementation

- **Global hotkeys** (`hotkeys_win.rs`): `RegisterHotKey(hwnd=0, id, MOD_*, vk)`
  runs on a dedicated thread that pumps `GetMessageW` / `WM_HOTKEY`. This is the
  *application-level* shortcut path — distinct from the low-level hook below.
- **Global input** (`input_win.rs`): `SetWindowsHookExW(WH_KEYBOARD_LL | WH_MOUSE_LL,
  …, 0, 0)` installed on a dedicated thread that pumps messages; the low-level
  hook callback runs on that same thread (a hard Windows requirement). A static
  `Mutex<Option<Arc<InputCtx>>>` shares the handler + modifier state with the
  callbacks. `PostThreadMessageW(WM_QUIT)` tears the thread down.
- `GetAsyncKeyState` reconstructs the live modifier mask for each key event.

### Event delivery contract (both backends)

Both backends share one outbox:

```
HotkeyManager / GlobalInput  ──on_hotkey/on_event──▶  PushHandler
                                                        │  json!({cmd, payload})
                                                        ▼
                                              Arc<Mutex<Vec<String>>>  (Outbox)
                                                        │  drained every frame in MainEventsCleared
                                                        ▼
                              backend emits window.__RDESKTOP_IPC__(json)
                                                        │
                                                        ▼
                              frontend window.__RDESKTOP_PUSH__(data)
```

`PushHandler` (in `global.rs`) implements both `HotkeyHandler` and
`GlobalInputHandler` and pushes JSON envelopes into the shared `Outbox`:

```json
// Global hotkey fired
{ "cmd": "rdesktop.globalHotkey", "payload": { "id": 1, "combo": "Ctrl+Shift+K" } }

// Raw input event
{ "cmd": "rdesktop.globalInput",
  "payload": { "type": "keyboard", "key": "KeyK", "state": "pressed",
               "modifiers": { "ctrl": true, "shift": true, "alt": false, "meta": false } } }
```

Each backend drains `Outbox` every frame (in `MainEventsCleared`) and emits each
entry through the same bridge used for `send_to_frontend`. The bridge forwards
unnamed entries (`data` without an `id`) to `window.__RDESKTOP_PUSH__`, so the
frontend needs no special wiring beyond defining that one handler.

### Config schema

```rust
pub struct AppConfig {
    pub hotkeys: Vec<HotkeyConfig>,          // serde(default)
    pub global_input: GlobalInputConfig,     // serde(default)
}

pub struct HotkeyConfig { pub id: Option<String>, pub combo: String, pub title: Option<String> }
pub struct GlobalInputConfig { pub enabled: bool, pub keyboard: bool, pub mouse: bool, pub mouse_move: bool }
```

Valid combos follow the parser: `Ctrl`, `Alt`, `Shift`, `Meta` (in any order,
`+`-separated) followed by a key token (`A`–`Z`, `0`–`9`, `F1`–`F24`, `Enter`,
`Space`, `Tab`, `Esc`/`Escape`, `Backspace`, `Delete`, `ArrowUp/Down/Left/Right`,
`Home/End/PageUp/PageDown`, `PrintScreen`, …).

### Example

`examples/global_hotkey` registers `Ctrl+Shift+K` + `Alt+PrintScreen` and enables
global keyboard/mouse capture, then renders a live log of hotkey + input events
in the frontend via `window.__RDESKTOP_PUSH__`.

### Caveats

- Windows low-level hooks require the installing thread to pump messages; both
  managers spawn their own thread for exactly this reason.
- The macOS `CGEventTap` paths (`hotkeys_mac.rs` / `input_mac.rs`) are **stubbed**
  and return `UnsupportedPlatform`; they are not compiled/verified on this Windows
  dev host and require a macOS build to complete.
- `mouse_move` is off by default — enabling it floods the outbox with move events.

## Roadmap

### Implemented
- [x] Dual-engine renderer (WebView + Chrome/CEF via CDP)
- [x] Pull-based IPC bridge (both backends)
- [x] Phase 0: `WindowKind` (Normal/Overlay/Wallpaper) + click-through + desktop layer + `webgpu` flag
- [x] Phase 1: Node.js extension host (rcode PoC) + native → frontend Outbox push
- [x] Phase 2: Global mouse/keyboard hook + global hotkeys (Windows; macOS stub)

### Planned (deep desktop)
- [ ] Phase 2 (macOS): complete `CGEventTap` hotkey + input impls; Linux X11/Wayland
- [ ] Phase 3: Input simulation & device remapping (Logitech G-Hub style)
- [ ] Phase 4: Media session (SMTC) + HiFi/WASAPI audio engine
- [ ] Phase 5: Cross-platform alignment; tray / notify / autostart system layer
- [ ] macOS/Linux WebView + Chrome backend verification
- [ ] Code signing (macOS/Windows), auto-update, plugin system
