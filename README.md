# rdesktop

[![Crates.io](https://img.shields.io/crates/v/rdesktop-cli.svg)](https://crates.io/crates/rdesktop-cli)
[![docs.rs](https://docs.rs/rdesktop-core/badge.svg)](https://docs.rs/rdesktop-core)
[![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/neko233-com/rdesktop)
[![GitHub stars](https://img.shields.io/github/stars/neko233-com/rdesktop.svg?style=social)](https://github.com/neko233-com/rdesktop)

**English** | [Chinese](README-CN.md)

> An agent-first Rust desktop framework with interchangeable WebView and Chromium rendering backends.

rdesktop is designed for desktop applications whose UI is built with web technologies but whose development, testing, and native runtime need to remain observable and scriptable. It provides a common Rust API for windowing, rendering, IPC, input, hotkeys, and development-time visual verification.

## Terminal233 integration

Terminal233 uses this repository as its rdesktop framework source. Applications
should pin an explicit Git revision or immutable release tag for `rdesktop-core`
and `rdesktop-webview`.

Terminal233 has two intentionally interchangeable launch paths:

- the Windows desktop path uses rdesktop WebView/IPC and native pure-Rust `puressh`;
- the browser IDE path uses the same Terminal233 IDE components and the localhost Node `ssh2js`
  adapter, so layout, connection flow, terminal events, and SFTP contracts stay aligned.

The current validated native development and binary distribution target is
Windows 10/11 x64. Other platforms require separate runtime acceptance.

## Project status

Node/Electron plugin migration is a separate, incomplete capability. The new
unpublished `rdesktop-node` module bridge now has a native WebView2/CommonJS
acceptance example. It has not replaced iHub's Electron renderer or completed
Electron/uTools API compatibility; see the [native plugin runtime status and acceptance gates](docs/PLUGIN_RUNTIME.md).

rdesktop is an active `0.1.x` project. Windows development now runs the real native
application by default, with authenticated Agent CLI inspection, engine input,
PNG capture, multi-window targeting and frontend hot reload. The browser preview
remains available explicitly. See [Native Agent development](docs/NATIVE_AGENT.md)
for setup, release isolation and the exact limits of hidden-window verification.
The generated starter includes keyboard submission, persistent labels, live
results, reflow and system-color support. See [actual NVDA and accessibility
acceptance](docs/ASSISTIVE_TECHNOLOGY_VALIDATION.md) for tested cases and limits.

The project favors explicit behavior and a small, composable workspace over a single opaque runtime. APIs may change before `1.0.0`.

## Why rdesktop?

Most desktop web frameworks optimize for one of two things: a small binary or maximum browser compatibility. rdesktop focuses on a third requirement: keeping the application inspectable throughout the agent-driven development loop.

- **Develop against the real desktop.** Agents can inspect and operate the application's actual WebView2 through authenticated loopback HTTP; `--headless` starts the same native windows hidden.
- **Keep native rendering available.** The same application model can target a system WebView or a Chromium-backed renderer.
- **Verify what a person will see.** Native screenshots and Windows desktop MP4 recordings make visual regressions and interaction flows reviewable by both humans and agents.
- **Keep platform behavior explicit.** Window state, IPC, global input, hotkeys, overlays, wallpaper windows, and click-through behavior are represented as Rust APIs and configuration.

rdesktop is not intended to replace every desktop framework. A useful rule of thumb is:

| Requirement | A reasonable fit |
| --- | --- |
| Small internal utility with minimal native integration | Tauri or a system WebView wrapper |
| Existing Electron application and broad web compatibility | Electron |
| Agent-observable development, renderer choice, and native desktop control | rdesktop |

## Highlights

### Interchangeable renderers

The `rdesktop-core` crate defines the renderer and window abstractions used by the workspace.

- `rdesktop-webview` uses the platform WebView through `wry` and `tao`.
- `rdesktop-cef` drives Chrome, Chromium, or Edge through the Chrome DevTools Protocol (CDP) for a consistent Chromium rendering path.
- Application-level IPC and window operations are designed to remain independent of the selected renderer.

```toml
[renderer]
kind = "webview" # lightweight system WebView
# kind = "chrome" # Chromium/CDP rendering path
```

The Chromium path requires a supported Chrome, Chromium, or Edge executable on the host. It is not a bundled CEF distribution.

### Agent-first development server

`rdesktop dev` launches the Rust desktop application. Its native Agent server
calls WebView2's internal CDP API without exposing a remote-debugging port.
`rdesktop dev --browser` serves the separate browser preview and injects the
browser bridge. The native path retains the application's real Rust IPC.

The development server supports:

- DOM snapshots and CSS-selector element queries;
- structured actions such as click, type, fill, scroll, hover, focus, select, press, and drag;
- application state snapshots and IPC requests;
- hot reload when frontend files change;
- native screenshot publication with optional wait-for-paint semantics;
- real Windows UIA patterns and owned OS dialogs, with compositor PNG frames and validated screen coordinates;
- a local HTTP API that works with Playwright, Puppeteer, MCP tools, or ordinary scripts.

### Synchronous and asynchronous IPC

`IpcHandler::handle` remains available for small, synchronous RPC methods.
Native WebView and Chromium renderers dispatch frontend RPC on worker threads
and deliver responses back on the renderer event loop, so a synchronous Git or
network call cannot freeze the desktop window. Handlers that already own an
async runtime can override `IpcHandler::handle_async` and invoke the supplied
`IpcResponseSender` when their work completes. Responses are correlated by
request ID and may arrive out of order; window operations remain event-loop
owned and are never executed from a worker thread.

Frontend calls retain a 120-second default wait and accept an optional bounded
deadline for intentional long-running work:

```javascript
const reply = await window.__RDESKTOP_INVOKE__(
  "download", { artifact: "example" }, { timeoutMs: 240000 },
);
```

`timeoutMs` must be an integer from 1 to 600000. Timeouts reject with
`code = "RDESKTOP_IPC_TIMEOUT"` and the chosen `timeoutMs`. Native responses,
transport errors and serialization failures clean up their pending callbacks
and timers; late replies are not push events. Browser preview applies the same
deadline while aborting its HTTP wait. A timeout does not cancel an already
dispatched Rust operation or retry it. The native Agent/CLI exposes the matching
`timeout_ms` parameter; see [its IPC contract](docs/NATIVE_AGENT.md).

### Visual verification and recording

The Agent API owns one recording session and one fixed output file per development server:

- On Windows, the server captures the virtual desktop and encodes a real H.264 MP4 through GDI and Media Foundation.
- No FFmpeg installation or runtime packaging is required for the Windows path.
- Repeated `start` calls reuse the active session; repeated `stop` calls are safe.
- The default maximum duration is five minutes; the hard limit is one hour.
- Temporary files are cleaned after finalization, startup failure, and graceful shutdown.
- On non-Windows platforms, the browser bridge provides a `MediaRecorder` fallback. The browser may require display-capture permission and may produce WebM rather than MP4.

This bounded, idempotent design is intentional: an agent can retry a request without creating an unbounded collection of debug videos.

### Native desktop primitives

The core workspace includes abstractions and platform implementations for:

- normal, overlay, wallpaper, always-on-top, transparent, and click-through windows;
- custom title-bar dragging and resizing;
- native window icons;
- runtime switching between frameless integrated UI and OS-decorated system window UI via
  `window.__RDESKTOP_WINDOW__.setDecorations(true|false)`;
- global hotkeys and opt-in global input hooks;
- structured IPC between Rust and the frontend.

On Windows WebView2, mark non-interactive custom title-bar regions with
`app-region: drag` (and `-webkit-app-region: drag` for Chromium compatibility). WebView2 then handles dragging, double-click
maximize/restore and the right-click system menu from the original native
mouse event. Keep buttons and menus outside those regions, or give them
`app-region: no-drag` and `-webkit-app-region: no-drag`. Explicitly exclude
buttons, their labels, menus and editable fields; native drag regions consume
the original mouse press before DOM pointer handlers receive it. The
On Windows the `rdesktop:window-move-resize-start` window event reports entry
into the native interactive move/resize loop, including CSS dragging and the
system menu. It is emitted once per loop and can arrive after a quick drag
finishes. Programmatic positioning does not emit it. A consumer can use it
to preserve a deliberate window position while switching surfaces; the event
alone is not proof of a position change.
The `startDrag()` bridge remains available as
a fallback for renderers without native CSS drag regions.

Undecorated Windows WebView windows have a native four-DIP resize border
above the WebView. Its eight resize directions do not wait for JavaScript
IPC. It is hidden while maximized, minimized or fullscreen. Switching window
decorations preserves the client dimensions, and configured minimum/maximum
dimensions include the client area even when Windows retains invisible frame
insets. Minimized icon dimensions are never applied as WebView bounds.

For repeatable development inspection, query `GET /__rdesktop__/agent/dom`
and `GET /__rdesktop__/agent/elements?selector=%5Brole%3Dmenubar%5D` on the
application's loopback Agent port. The DOM can confirm controls and ARIA
shortcuts; verify resizing and window state in the real native window and
its `rdesktop_webview=debug` resize log. Browser snapshots cannot prove
native mouse handling or client-area dimensions.

## Quick start

### Install the CLI

```bash
npm install -g https://github.com/neko233-com/rdesktop/releases/download/v0.1.15/rdesktop-0.1.15.tgz
# Or install the CLI from this immutable source release:
cargo install --git https://github.com/neko233-com/rdesktop.git --tag v0.1.15 rdesktop-cli
```

The GitHub npm tarball contains the Windows x64 CLI. The npm registry publication
is a separate step; use `npm install -g rdesktop@0.1.15` when that version is
available there. Building native applications also requires Rust MSVC, the
Windows SDK and WebView2 Runtime.

### Create a project

```bash
rdesktop init hello-rdesktop
cd hello-rdesktop
```

The initializer creates a minimal `rdesktop.toml`, Rust entry point, and `frontend/index.html`.

### Start the development server

```bash
rdesktop dev
```

On Windows this starts the real application and serves `frontend/` with hot
reload. The CLI prints a local Agent session file path. To start its windows
hidden while retaining the actual native engine:

```bash
rdesktop dev --headless
```

Useful CLI commands are:

```text
rdesktop init <name>                 Create a project skeleton
rdesktop dev [--path <dir>]         Run the native app with hot reload and Agent API
rdesktop dev --browser --no-open    Run the browser preview without opening it
rdesktop agent --session <file> ... Inspect or operate a real native app
rdesktop build [--chrome]           Build the native application path
rdesktop bundle --target <target>   Generate a platform bundle
rdesktop info                       Inspect rdesktop.toml
rdesktop icons --input <png>        Generate multi-size ICO/PNG icon assets
```

### Generate Windows icons from PNG

Prepare one transparent PNG and generate the real `.ico` required by Windows
shortcuts plus 16/24/32/48/64/128/256 pixel PNG variants:

```bash
rdesktop icons \
  --input resources/icons/source.png \
  --output-dir resources/icons \
  --name app
```

The output contains `app.ico` and files such as `app-16.png`. Non-square input
is fitted into a transparent square without silent cropping; source edges over
8192 pixels are rejected. GitX uses the same `.ico` for its Start Menu and
Desktop shortcuts and fail-closed validates it during packaging.

### Preserve WebView data across updates

Portable applications should keep WebView2 state outside the replaceable
program directory. Configure an absolute per-user path before starting the
renderer so cookies, localStorage, IndexedDB, tabs, and preferences survive a
clean application update:

```rust
let mut renderer = WebViewRenderer::new(&config)?;
renderer.set_data_directory(
    std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
        .join("hello-rdesktop")
        .join("WebView2"),
)?;
```

The updater must preserve this directory. Relative paths and paths that are
already regular files are rejected.

The build and installer commands are under active development. Inspect their output and validate the generated artifact for your platform before distribution.

## Configuration

The generated project uses `rdesktop.toml`. A minimal development configuration looks like this:

```toml
[app]
identifier = "com.example.hello"
name = "hello-rdesktop"
version = "0.1.0"

[renderer]
kind = "webview"
webgpu = true

[dev]
host = "localhost"
port = 1420
open_browser = true
hot_reload = true
agent_mode = true
devtools = true

[window]
title = "Hello rdesktop"
width = 1280
height = 720
resizable = true

[global_input]
enabled = false
keyboard = true
mouse = true
mouse_move = false
```

Keep `host = "localhost"` unless remote access is explicitly required. Setting it to `0.0.0.0` exposes the development and Agent endpoints to the network and should only be done on a trusted, isolated network.

## Agent workflow

For native development use `rdesktop agent --session <file> windows` and the
[native command contract](docs/NATIVE_AGENT.md). The authenticated native API
uses an ephemeral port from the session file. The examples below describe the
explicit **browser preview** (`rdesktop dev --browser`), normally on
`http://localhost:1420`; its structured endpoints live below `/__rdesktop__/agent/`.

| Method | Endpoint | Purpose |
| --- | --- | --- |
| `GET` | `/dom` | Read the latest DOM snapshot |
| `GET` | `/elements?selector=...` | Query elements by selector, text, or role |
| `POST` | `/action` | Queue a structured UI action |
| `GET` | `/state` | Read the latest application state snapshot |
| `POST` | `/ipc` | Send an IPC message to the frontend/backend bridge |
| `GET` | `/screenshot` | Read a screenshot; `wait=true&after=<generation>` waits for a newer native frame |
| `GET` | `/recording` | Read the single recording session state |
| `POST` | `/recording/start` | Start or reuse the single recording |
| `POST` | `/recording/stop` | Stop and finalize the recording |
| `GET` | `/recording/file` | Download the finalized recording |

Example interaction:

```bash
# Query a button
curl "http://localhost:1420/__rdesktop__/agent/elements?selector=button"

# Execute an action and wait for a new native frame when available
curl -X POST "http://localhost:1420/__rdesktop__/agent/action?wait=true" \
  -H "content-type: application/json" \
  -d '{"action":"click","selector":"button#submit"}'

# Start one bounded recording
curl -X POST "http://localhost:1420/__rdesktop__/agent/recording/start" \
  -H "content-type: application/json" \
  -d '{"fps":30,"max_duration_seconds":300}'

# Perform the flow, then finalize the same recording
curl -X POST "http://localhost:1420/__rdesktop__/agent/recording/stop" \
  -H "content-type: application/json" \
  -d '{}'

# Download the result
curl -L "http://localhost:1420/__rdesktop__/agent/recording/file" -o recording.mp4
```

For a robust visual assertion, use the structured DOM/state endpoints first, perform an action, wait for the resulting native frame, and use the MP4 only as the durable review artifact. This keeps routine agent runs fast while preserving a human-readable trace when needed.

## Workspace layout

```text
rdesktop/
├── crates/
│   ├── rdesktop-core/       Shared types, renderer traits, IPC, input, and window APIs
│   ├── rdesktop-webview/    WebView2/WebKit/WebKitGTK backend
│   ├── rdesktop-cef/        Chromium/CDP backend
│   ├── rdesktop-dev/        Browser dev server and Agent API
│   ├── rdesktop-bundler/    Platform bundle abstractions and generators
│   ├── rdesktop-assets/     PNG to ICO/desktop icon asset generator
│   └── rdesktop-cli/        `rdesktop` command-line interface
├── examples/                Small example applications
├── test-app/                Local visual and interaction test fixture
├── ARCHITECTURE.md          Design notes and subsystem boundaries
└── README-CN.md             Chinese documentation
```

## Platform notes

| Platform | System WebView | Chromium path | Native MP4 recording |
| --- | --- | --- | --- |
| Windows | WebView2 | Chrome/Chromium/Edge | Supported through GDI + Media Foundation |
| macOS | WKWebView | Chrome/Chromium/Edge | Browser fallback |
| Linux | WebKitGTK | Chrome/Chromium/Edge | Browser fallback |

The exact runtime prerequisites depend on the selected backend. The WebView backend uses the operating system’s webview stack; the Chromium backend requires a locally available compatible browser. Mobile platforms are outside the current scope.

## Build from source

```bash
git clone https://github.com/neko233-com/rdesktop.git
cd rdesktop
cargo check --workspace
cargo test --workspace
cargo run -p rdesktop-cli -- --help
```

For the Windows native recording path, build and run on Windows so the Media Foundation implementation is compiled and exercised by the target platform.

## Contributing

Contributions are welcome. Before opening a pull request:

1. Read [ARCHITECTURE.md](ARCHITECTURE.md) and identify the affected crate.
2. Keep public API changes focused and document behavior that agents or users can observe.
3. Run `cargo fmt --all -- --check`.
4. Run `cargo check --workspace` and `cargo test --workspace`.
5. For Agent API or rendering changes, include a reproducible request sequence and explain how visual behavior was verified.
6. Keep generated recordings, build output, credentials, and local test artifacts out of commits.

Small documentation fixes are also valuable. Please use a clear commit message and describe platform-specific assumptions in the pull request.

## Security and privacy

The Agent API is a development interface, not an internet-facing service. It can inspect and manipulate the running application and can capture the desktop on supported platforms. Bind it to localhost by default, avoid exposing it on shared networks, and review recording contents before sharing them.

Please do not include credentials, private screenshots, or recordings containing sensitive information in issues or pull requests.

## License

rdesktop is licensed under either of:

- [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0)
- [MIT License](https://opensource.org/license/mit/)

at your option.

## Acknowledgements

rdesktop builds on the Rust ecosystem, including `wry`, `tao`, `tokio`, `axum`, `chromiumoxide`, `serde`, and `windows`. Their maintainers and contributors make this project possible.
