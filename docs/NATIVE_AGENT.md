# Develop against the real desktop

Windows x64 WebView2 is the current native Agent implementation. `rdesktop dev`
starts the application's Rust executable and its real WebViews. `--browser`
explicitly selects the separate browser preview. Both remain available.

```powershell
rdesktop init my-app
cd my-app
rdesktop dev
# Keep the real windows hidden at startup:
rdesktop dev --headless
# Use an existing frontend server with its own HMR:
rdesktop dev --frontend-url http://127.0.0.1:5173
```

The CLI prints the discovery **file path**, never its token. In another terminal:

```powershell
rdesktop agent --session .rdesktop/agent-cli-1234.json windows
rdesktop agent --session .rdesktop/agent-cli-1234.json dom --window 1
rdesktop agent --session .rdesktop/agent-cli-1234.json elements --window 1
rdesktop agent --session .rdesktop/agent-cli-1234.json accessibility --window 1
rdesktop agent --session .rdesktop/agent-cli-1234.json uia --window 1
rdesktop agent --session .rdesktop/agent-cli-1234.json screenshot --window 1 --output main.png
rdesktop agent --session .rdesktop/agent-cli-1234.json action --params-file action.json
```

`action.json` is an object such as:

```json
{"window":"1","action":"fill","selector":"#name","value":"Agent 中文"}
```

All window IDs are decimal **strings**. Dynamically created IDs can exceed
JavaScript's safe integer range. A target may be omitted only while exactly one
window exists. Closing a window invalidates its ID; titles do not confer identity.

## Integrate an existing application

Enable `rdesktop-webview/native-agent` through an application feature named
`native-agent`. After constructing the renderer, call:

```rust
renderer.enable_native_agent_from_env()?;
```

The method enables the server only with Windows, the `native-agent` feature,
debug assertions, and `RDESKTOP_AGENT=1`. It is inert in ordinary release builds,
even when the feature is accidentally enabled and debug environment variables
are present. Distribution builds must leave debug assertions off. No remote
debugging port, HTTP bridge, console wrapper or credentials are injected into
the production frontend.

In debug builds, load `RDESKTOP_DEV_URL` instead of bundled assets when present.
The CLI serves `frontend/` with file hot reload, or passes `--frontend-url`
through to the app. Native reload adds only a reload script; the application's
real `window.__RDESKTOP_INVOKE__` continues to call its Rust handler. The
unauthenticated frontend asset port disables browser Agent endpoints.

The CLI sets `RDESKTOP_AGENT_SESSION` to a unique `.rdesktop/agent-cli-PID.json`.
Launching a debug executable directly uses `.rdesktop/agent-APP_PID.json` by
default. Discovery files are current-user local credentials and belong in
`.gitignore`. Existing files are never overwritten. Normal shutdown removes
only the current session's file. Ctrl+C asks the app to exit; the CLI never
force-kills a resident application.

## HTTP contract

Read `url` and `token` from the discovery JSON. The listener binds an ephemeral
`127.0.0.1` port. Every request requires `Authorization: Bearer TOKEN`, including
health and info. Browser `Origin` headers and a mismatched `Host` are rejected.
There is no permissive CORS layer or unauthenticated CDP socket.

The routes below use `/__rdesktop__/agent/`. POST bodies are JSON objects.
Read commands also support GET with `window` and `selector` query parameters.

| Command | Parameters / result |
| --- | --- |
| `windows` | Live IDs, URLs, titles, visibility/focus/state, DPI, physical client/outer bounds |
| `dom` | Actual HTML, URL, title and readiness from the selected native WebView |
| `elements` | `selector`; bounded elements with bounds, font/colors, labels, visibility and values |
| `state` | Viewport, focused markup and the app's optional `window.__RDESKTOP_AGENT_STATE__` |
| `accessibility` | Entire WebView2 Accessibility domain: full/partial/root/ancestor/child/query trees, enable and disable; `method` and `params` |
| `uia` | Windows UI Automation raw/control/content tree rooted at the selected native window, names, roles, focus, bounds, help/ARIA properties, values and standard pattern IDs |
| `audit` | Run locally loaded axe-core; returns violations, passes, incomplete and inapplicable rules, plus manual-review flag |
| `evaluate` | `expression`; awaits a Promise and returns its JSON value; exceptions fail the request |
| `action` | `action`, `selector`, `value`, `target_selector`, `delta_x`, `delta_y` as appropriate |
| `cdp` | `method`, `params`; raw WebView2 CDP call, including accessibility, network, performance and detailed input |
| `events` | Last 512 engine console, exception, log, failed-network, Accessibility and intercepted-drag events; sequence strings and optional window filter |
| `screenshot` | Real WebView content as PNG, including hidden startup windows |
| `window` | `operation`: show/hide/focus/minimize/restore/maximize/resize/zoom/close; resize uses logical pixels, zoom accepts `factor` 0.25..5 |
| `reload` | Reload the selected WebView without restarting Rust |
| `ipc` | `cmd`, `payload`; traverses the selected WebView's real IPC bridge |
| `exit` | Request normal application exit |
| `recording` | Current bounded Windows desktop recording state |
| `recording/start` | `fps`, `max_duration_seconds`; starts or reuses one session, maximum one hour |
| `recording/stop` | Optional `session_id`; finalizes that session |
| `recording/file` | GET only; stream the completed desktop MP4 |

`GET /__rdesktop__/info` reports capabilities and capture scope. JSON requests
are capped at 256 KiB, native responses at 32 MiB, outstanding requests at 16,
and event payloads at 16 KiB. CDP completion has a 15-second deadline; queued
expired operations are skipped. Lost windows and invalid calls fail explicitly.

Structured click, type, fill, clear, hover, focus, scroll, press and drag use CDP
engine input. Clicks require one visible, enabled, unobscured element. `select`
sets a DOM select's value and dispatches DOM events, reported as `dom-select`.
Use raw CDP for complete key/modifier, touch and protocol parameters. A native
engine method's availability is determined by the installed WebView2 runtime;
unsupported methods return errors rather than simulated success.

Targets can use `selector`, or an exact computed `role` plus `name` resolved
through the native AX tree, including Shadow DOM. Focus does not click or
toggle the element. Read-only/non-editable filling, disabled/hidden actions and
missing select options fail explicitly. `press` accepts a CDP modifier bitmask
(Alt=1, Control=2, Meta=4, Shift=8), including Shift+Tab. Concurrent structured
input to the same window is rejected rather than interleaved.

HTML drag uses the engine's intercepted drag data and trusted drag events;
both endpoints must be visible together. It releases the button and resets
interception on failure. This does not verify an external Windows file drop.

For automated accessibility auditing, install the official engine locally and
pass its file to the CLI. Nothing is fetched into an end user's application:

```powershell
npm install --save-dev axe-core@4.14.0
rdesktop agent --session .rdesktop/agent-cli-PID.json audit --window 1 --axe-source node_modules/axe-core/axe.min.js --output audit.json
```

The CLI loads the engine in bounded chunks, includes accessible same-origin
frames and deletes its temporary source buffer. All standard axe rules run by
default. `context` and `options` can narrow an audit explicitly. `incomplete`
results and inaccessible frames require review; a zero-violation report is not
a claim that every possible assistive technology or application is accessible.

UIA reports up to 1,000 nodes by default (maximum 5,000), 64 levels (maximum
128) and a five-second traversal budget. It probes standard pattern IDs
10000..10034, reports truncation, and redacts password values. `patterns:false`
can skip probing; `view` chooses raw/control/content. No UIA request starts at
the desktop root or visits another application's windows. Snapshot node IDs
are not persistent action identifiers.

The UIA root's current process ID and native handle must match the selected
owned window before descendants are traversed. Values are returned only when
UIA explicitly identifies a control as non-password; an unknown password
property is redacted too.

## What this proves

Hidden mode uses the same native WebView2 and Rust host as visible mode. It
still needs a Windows desktop session and is not a Linux/server-only renderer.
An application may explicitly show a hidden startup window later.

Windows may expose only its outer UIA providers while a window is hidden. The
test fixture exposes 102 UIA nodes visibly and seven while hidden; its real
engine AX tree retains 145 nodes in both modes. These are distinct reported
sources. The framework does not manufacture hidden OS accessibility nodes or
silently show a window to populate them. Narrator/NVDA speech is a separate
manual acceptance check, not an inference from an AX snapshot or axe report.

PNG captures contain the WebView content, not OS title bars, file dialogs,
menus outside the WebView, or external file-drop targets. Engine input is not
physical Windows input. Desktop recordings capture the entire virtual desktop,
not a hidden WebView. Use real desktop automation and visual checks for those
OS interactions. Browser preview cannot prove them either.

## Repeatable verification

From the repository root, with Node.js installed:

```powershell
cargo run -p rdesktop-cli -- dev --path examples/native_agent --port 1438
node scripts/verify-native-agent.mjs examples/native_agent/.rdesktop/agent-cli-PID.json .cache/native-visible
npm ci --ignore-scripts
node scripts/verify-native-accessibility.mjs examples/native_agent/.rdesktop/agent-cli-PID.json .cache/a11y-visible
# Repeat with dev --headless and a different artifact directory.
cargo fmt --all -- --check
cargo check --workspace --features rdesktop-webview/native-agent
cargo test --workspace --features rdesktop-webview/native-agent
cargo clippy --workspace --all-targets --features rdesktop-webview/native-agent -- -D warnings
```

For the separate, explicit desktop recording check, use
`node scripts/verify-native-recording.mjs SESSION .cache/recording-proof`.
This captures the entire virtual desktop locally for two seconds and validates
MP4 finalization, streaming, session checks and idempotency. Never publish its
video as a release artifact.

The fixture checks actual Rust IPC, Unicode/trusted engine input, keyboard
submission, two-window isolation and 64-bit identity, native resizing, PNG,
console events, repeated hot reload with the same PID, and closed-ID rejection.
It restores the exact frontend bytes after its owned hot-reload test. Artifacts
and local credentials stay ignored. macOS/Linux native Agent support is not
claimed by these Windows checks.
