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

Terminal233 uses this repository as its rdesktop framework source. The client pins the exact git
revision `01359a93b4337698ee4f43093f0c7bc78bb1b99a` for `rdesktop-core` and `rdesktop-webview`.
The local working tree used for framework development is
`C:\Users\14170\Desktop\Code\neko233-Projects\rdesktop`; do not edit Cargo's checkout cache.

Terminal233 has two intentionally interchangeable launch paths:

- the Windows desktop path uses rdesktop WebView/IPC and native pure-Rust `puressh`;
- the browser IDE path uses the same Terminal233 IDE components and the localhost Node `ssh2js`
  adapter, so layout, connection flow, terminal events, and SFTP contracts stay aligned.

The current local desktop validation target is Windows `x86_64` only. Windows ARM64 is a future
tag-only CI target and must not be produced locally until the user explicitly says “发布”. 32-bit
Windows targets are permanently out of scope.

## Project status

rdesktop is an active `0.1.x` project. The browser-based development server, Agent API, renderer abstractions, and visual debugging workflow are the primary focus of the current release line. Native build and installer integrations are still evolving and should be evaluated for the target platform before being used in a production distribution pipeline.

The project favors explicit behavior and a small, composable workspace over a single opaque runtime. APIs may change before `1.0.0`.

## Why rdesktop?

Most desktop web frameworks optimize for one of two things: a small binary or maximum browser compatibility. rdesktop focuses on a third requirement: keeping the application inspectable throughout the agent-driven development loop.

- **Use the browser while developing.** Agents can inspect the DOM, query elements, execute structured actions, and read application state through HTTP.
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

`rdesktop dev` serves the frontend in a normal browser and injects the rdesktop bridge. This gives an agent a conventional browser target while keeping the application’s IPC and interaction model visible.

The development server supports:

- DOM snapshots and CSS-selector element queries;
- structured actions such as click, type, fill, scroll, hover, focus, select, press, and drag;
- application state snapshots and IPC requests;
- hot reload when frontend files change;
- native screenshot publication with optional wait-for-paint semantics;
- a local HTTP API that works with Playwright, Puppeteer, MCP tools, or ordinary scripts.

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
- global hotkeys and opt-in global input hooks;
- structured IPC between Rust and the frontend.

## Quick start

### Install the CLI

```bash
cargo install rdesktop-cli --version 0.1.5
```

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

The server normally listens on `http://localhost:1420` and opens the browser automatically. To keep the process headless for an agent or CI job:

```bash
rdesktop dev --no-open
```

Useful CLI commands are:

```text
rdesktop init <name>                 Create a project skeleton
rdesktop dev [--path <dir>]         Run the browser development server
rdesktop build [--chrome]           Build the native application path
rdesktop bundle --target <target>   Generate a platform bundle
rdesktop info                       Inspect rdesktop.toml
```

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

The development server base URL is normally `http://localhost:1420`. The structured endpoints live below `/__rdesktop__/agent/`.

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
