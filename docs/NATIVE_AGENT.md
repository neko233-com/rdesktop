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
rdesktop agent --session .rdesktop/agent-cli-1234.json info
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
| `windows` | Live IDs, engine-event URLs, Win32 titles/visibility/focus/state, process ID, DPI, physical client/outer and DWM frame bounds; reads remain available during native modals |
| `native-windows` | `window`, optional `max_windows` 1..64 (default 32); discover owned native windows with opaque lifetime IDs, including verified OS broker dialogs |
| `dom` | Actual HTML, URL, title and readiness from the selected native WebView |
| `elements` | `selector`; bounded elements with bounds, font/colors, labels, visibility and values |
| `state` | Viewport, focused markup and the app's optional `window.__RDESKTOP_AGENT_STATE__` |
| `accessibility` | Entire WebView2 Accessibility domain: full/partial/root/ancestor/child/query trees, enable and disable; `method` and `params` |
| `uia` | Windows UI Automation raw/control/content tree, provider runtime IDs, targeted inspection/text and real typed pattern operations within the selected native window |
| `audit` | Run locally loaded axe-core; returns violations, passes, incomplete and inapplicable rules, plus manual-review flag |
| `evaluate` | `expression`; awaits a Promise and returns its JSON value; exceptions fail the request |
| `action` | `action`, `selector`, `value`, `target_selector`, `delta_x`, `delta_y` as appropriate |
| `cdp` | `method`, `params`; raw WebView2 CDP call, including accessibility, network, performance and detailed input |
| `events` | Last 512 engine console, exception, log, failed-network, Accessibility and intercepted-drag events; sequence strings and optional window filter |
| `screenshot` | Real WebView content as PNG, including hidden startup windows |
| `native-screenshot` | Windows Graphics Capture PNG of the selected renderer's OS frame, or its discovered `native_window`; requires visible, non-minimized window |
| `window` | `operation`: show/hide/focus/minimize/restore/maximize/resize/zoom/close; resize uses logical pixels, zoom accepts `factor` 0.25..5 |
| `reload` | Reload the selected WebView without restarting Rust |
| `ipc` | `cmd`, `payload`, optional integer `timeout_ms` 1..600000 (default 120000); traverses the selected WebView's real IPC bridge |
| `exit` | Request normal application exit |
| `recording` | Current bounded Windows desktop recording state |
| `recording/start` | `fps`, `max_duration_seconds`; starts or reuses one session, maximum one hour |
| `recording/stop` | Optional `session_id`; finalizes that session |
| `recording/file` | GET only; stream the completed desktop MP4 |

For an application command that intentionally takes longer than the default,
pass its bounded wait in the IPC parameter file:

```json
{"window":"1","cmd":"download","payload":{},"timeout_ms":240000}
```

`ipc` forwards this as the frontend bridge's third argument
`{timeoutMs:240000}`. Native CDP completion adds 2000 ms of grace, and the CLI's
HTTP wait adds a further 3000 ms. Other `evaluate` and `cdp` operations keep
their existing 15-second native completion limit. The chosen deadline ends
the wait; it does not cancel an already dispatched Rust command or retry it.
Do not retry a timed-out operation with side effects without checking its
application state. Info publishes these bounds in `capabilities.ipc`.

Run `node scripts/verify-native-ipc-deadlines.mjs SESSION OUTPUT RDESKTOP_EXE`
against `examples/native_agent` in visible and hidden modes. It exercises a
real 16-second Rust command through the CLI, concurrent UI/greet responsiveness,
typed short deadlines, actual late-reply arrival without a push event,
serialization cleanup, and invalid bounds. The separate
`node --test scripts/verify-ipc-deadlines.mjs` executes the shipped WebView,
Chrome and browser bridge code with a controlled clock; waits beyond the old
120-second limit are tested there without real multi-minute sleeps. Chrome's
bridge-source tests do not certify its native runtime or accessibility.

`GET /__rdesktop__/info` reports capabilities and capture scope. JSON requests
are capped at 256 KiB, native responses at 32 MiB, outstanding requests at 16,
and event payloads at 16 KiB. General CDP completion has a 15-second deadline;
IPC uses the bounded wait described above. Queued expired operations are skipped.
Lost windows and invalid calls fail explicitly.

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
the desktop root or visits another application's windows. Snapshot `id` values
are local row numbers and are rejected as action targets. Opaque `runtime_id`
arrays identify current provider controls; they are not valid after removal.
Windows may reuse those IDs over time; take fresh observations and do not use
them as persistent identities. Their format is treated as opaque, as specified
by [Microsoft's runtime ID contract](https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationelement-getruntimeid).

The UIA root's current process ID and native handle must match the selected
owned window before descendants are traversed. Values are returned only when
UIA explicitly identifies a control as non-password; an unknown password
property is redacted too.
Snapshot process IDs and hexadecimal `native_window_handle` values are
diagnostic metadata only; they are not accepted action targets.

### Native dialogs and full window frames

`native-windows` discovers windows under the selected renderer's native owner
chain. It scans only the host, the browser process obtained from WebView2 COM,
and physically embedded child-window processes. Windows can create a file
picker in a separate, short-lived OS broker process: those exact windows must
also appear as immediate raw UIA children of the renderer. No desktop-root UIA
search, process-name allowlist or arbitrary HWND/PID target is accepted.
Registered secondary renderers remain separate scopes. Discovery is bounded
by a two-second traversal budget, thread/child caps and the requested window
limit, and reports `truncated` when incomplete. Provider calls have their own
timeouts and can overrun the traversal budget.

Use a fresh opaque ID from discovery, for example:

```json
{"window":"1","native_window":"native-4","operation":"snapshot"}
```

Pass that object to `uia`; use its current provider runtime IDs for native
pattern actions. Pass `window` and `native_window` to `native-screenshot` to
capture the dialog. A lifetime property, current process ID, owner chain and
broker UIA relation are rechecked. Closing or reparenting a dialog expires its
ID; a reused HWND cannot revive it. Windows may also expose owned dialogs in
their renderer owner's raw UIA ancestry. That real relationship is preserved.
The native-dialog input permit is separate from pending renderer input, so a
modal can be canceled while the opening click remains outstanding.

`native-screenshot` uses Windows Graphics Capture directly, without selecting a
desktop region, showing a hidden window or activating it. It includes the OS
frame, limits capture to 16,777,216 pixels and needs Windows 10 1903+ with a
supported graphics session. It returns PNG bytes with
`x-rdesktop-capture-source: windows-graphics-capture` and pixel width/height
headers. `x-rdesktop-frame-x/y/width/height` are included only when DWM screen
bounds remain stable throughout capture and exactly match its pixel size.
When those headers are absent, refresh discovery before mapping image pixels
to screen coordinates. Invisible resize borders in `outer` are not a capture
origin. The WebView-only `screenshot` remains available in hidden mode and
reports `x-rdesktop-capture-source: webview-content`.

For application code that opens a native modal, use
`NativeRuntime::dispatch_window(id, callback)` to queue the window-bound UI
operation. Acceptance means queued; a window closed before execution is
skipped. Unlike synchronous `with_window`, a modal's lifetime is not limited
by the caller's 30-second response deadline. OS inspector and capture modules
remain gated by Windows, debug assertions and the `native-agent` feature.

### Native UIA operations

Use POST (the CLI does this automatically) and an exact `target`. Criteria can
include `name`, numeric UIA `control_type`, `automation_id`, `class_name` and/or
the `runtime_id` array returned by a snapshot. Criteria are combined with AND.
Every request searches the current selected window again. Ambiguous, missing,
truncated and stale-window searches fail before performing an action. No raw
HWND or snapshot row number is accepted as a target. `info` advertises the
backend's operations and scope under `capabilities.uia`.
An exact live runtime ID resolves as soon as it is found; name/property searches
must finish within the limits to prove uniqueness. This lets a known control
remain actionable when the rest of a large document exceeds the snapshot cap.

```json
{"window":"1","operation":"invoke","target":{"name":"Save preferences","control_type":50000}}
```

Save the object as `uia-action.json`, then call:

```powershell
rdesktop agent --session .rdesktop/agent-cli-PID.json uia --params-file uia-action.json
```

| Operation | Additional parameters / native pattern |
| --- | --- |
| `snapshot` | Default operation; `view`, `patterns`, `max_nodes`, `max_depth` |
| `inspect` | Provider Value/read-only, Toggle, ExpandCollapse, SelectionItem, RangeValue, Scroll, Selection, Transform and Window states |
| `text`, `text-selection` | Text pattern; `max_chars` 1..65536 (default 4096); up to 32 selected ranges |
| `focus` | Native `SetFocus`; requires an enabled, visible, keyboard-focusable control |
| `invoke` | Invoke pattern |
| `set-value` | String `value`, up to 65536 UTF-8 bytes, no NUL; Value pattern |
| `set-range-value` | Finite numeric `value` within the provider's writable range |
| `toggle`, `expand`, `collapse` | Toggle / ExpandCollapse patterns |
| `select`, `add-to-selection`, `remove-from-selection` | SelectionItem pattern |
| `scroll` | `horizontal_amount` / `vertical_amount`: `none`, `small-decrement`, `large-decrement`, `small-increment`, `large-increment` |
| `set-scroll-percent` | `horizontal_percent` / `vertical_percent`: -1 (unchanged) or 0..100 |
| `scroll-into-view` | ScrollItem pattern; can also reveal disabled controls |
| `realize` | VirtualizedItem pattern; provider availability determines support |
| `legacy-default-action` | LegacyIAccessible `DoDefaultAction` |
| `multiple-view-set` | Signed 32-bit `value`; MultipleView pattern |
| `window-state` | `state`: `normal`, `minimized`, `maximized`; Window pattern |
| `window-close` | Normal Window pattern close, including normal application prompts |
| `move`, `resize`, `rotate` | Transform pattern; finite `x`/`y`, positive `width`/`height`, or `degrees`; coordinates follow the native UIA provider |
| `text-select` | Text pattern `start`/`end`, 0..1000000 UIA character units; end must not precede start |
| `grid-item` | Grid pattern, zero-based integer `row` and `column` (0..1000000), checked against the current native dimensions |
| `table-headers`, `table-item-headers` | Table/TableItem row and column header element collections; `max_items` 1..256 per array, default 32 |
| `selection` | Selection pattern's actual selected-element collection; `max_items` 1..256, default 32; explicit `strategy` described below |
| `containing-grid`, `selection-container` | GridItem/SelectionItem native owner element |
| `relations` | Native `described-by`, `labeled-by`, `controller-for`, `flows-to` or `flows-from`; required `relation`, optional `max_items` 1..256 (default 32) |

These call the real Windows COM interfaces. Unsupported/read-only/disabled
operations fail explicitly; the framework does not synthesize DOM events or
fall back to CDP clicks. `scroll-into-view` and `realize` may act on disabled or
not-yet-realized items when their provider allows it. Mutations share the same
per-window exclusion as structured engine input. The framework window identity
and UIA root process/handle are revalidated immediately before mutation.

Success means the native provider accepted the operation (`dispatched:true`).
Application effects and provider accessibility updates can be asynchronous;
observe the resulting app state. A WebView2 fixture's Scroll pattern reports
0 percent even after its native `SetScrollPercent(100)` moves the actual region
to the bottom. Inspection preserves that provider result; verification records
it separately from the observed viewport change. Methods absent from a
provider are not reported as successful, and hidden providers are not expanded
by silently showing the application.

Password and unknown-password values remain redacted in inspection; text and
text-selection reads are rejected for those controls. Value writes never echo
their input. Native mutation failures report their HRESULT without copying a
provider error message that could contain the supplied value. UIA targets do
not include separate OS dialogs/popups outside the selected registered window;
those still require separate desktop/ownership verification.

Collection descriptors contain current provider runtime IDs and control
metadata, without text/value reads. Every returned element must prove raw UIA
ancestry inside the selected registered window. A different native window that
is not its Win32 child is rejected. Collections report `total`, returned
`elements`, `max_items` and `truncated`. Merged cells can return the same opaque
element for multiple grid coordinates; inspection includes row/column spans.
Inspection also reports Grid, GridItem, Table, Dock, MultipleView, Selection2,
Transform2 and LegacyIAccessible scalar states when supplied by the provider.
Legacy values obey the same password redaction as Value pattern values.

`relations` returns `{relation, related:{elements,total,max_items,truncated}}`
from the provider's current COM properties. Related elements use the same
bounded, window-scoped descriptors as collections, without text/value reads.
A successful null property is an empty collection; a failed HRESULT remains
an error. Labels use `CurrentLabeledBy`; arrays use `CurrentDescribedBy`,
`CurrentControllerFor`, `CurrentFlowsTo` and `IUIAutomationElement2::CurrentFlowsFrom`.
Runtime IDs are opaque current identities and must be resolved again after
navigation or element replacement.

Snapshots and inspection expose independent raw `help_text` and
`full_description` properties. The latter reads
`IUIAutomationElement6::CurrentFullDescription`; unavailable is `null` and an
available empty string stays empty. Do not infer absent descriptions from
`help_text` alone: the tested WebView2 email reports a complete correction in
`full_description`, but empty `help_text` and an empty `described-by` array.
The framework neither fills that array from DOM nor copies one native
description property into another.

`selection` defaults to `strategy:"provider"`, directly reading
`GetCurrentSelection`. The WebView2 fixture reports an empty grid collection
while the cell's native `CurrentIsSelected` is true. A caller can explicitly
choose `strategy:"items"` to traverse native SelectionItem providers, checking
both that flag and their current selection-container identity. This strategy
does not silently replace the provider collection. Its `scan_max_nodes`
(1..5000, default 1000), `scan_max_depth` (1..128, default 64) and five-second
budget bound the scan independently of the initial target search. Results
include `matched`, `visited` and `search_truncated`; an incomplete scan has
`total:null`. `truncated` also covers the returned item cap. These scans never
use DOM/AX properties to synthesize native selection.

Native table-wide and per-cell headers can differ: this fixture's table-wide
row-header array is empty, while its quantity cell reports `Apples` as its row
header. Both original provider results are retained. Tree walkers preserve
successful null COM results at tree edges, and propagate real provider errors
instead of treating failed branches as empty.

## What this proves

Hidden mode uses the same native WebView2 and Rust host as visible mode. It
still needs a Windows desktop session and is not a Linux/server-only renderer.
An application may explicitly show a hidden startup window later.

Windows may expose only its outer UIA providers while a window is hidden. The
test fixture exposes 102 UIA nodes visibly and seven while hidden; its real
engine AX tree retains 145 nodes in both modes. These are distinct reported
sources. The framework does not manufacture hidden OS accessibility nodes or
silently show a window to populate them. Screen-reader speech is a separate
manual acceptance check, not an inference from an AX snapshot or axe report.
Actual NVDA 2026.2 output, including a production EXE and the generated starter,
is recorded in [assistive technology acceptance](ASSISTIVE_TECHNOLOGY_VALIDATION.md).

`screenshot` contains WebView content; `native-screenshot` contains the selected
visible native window. Neither captures arbitrary other applications. Real
message-box and WebView2 file-picker inspection, native cancellation and
compositor frames have separate acceptance checks. Unsupported/unrelated OS
windows still fail explicitly. Engine input is not physical Windows input.
Desktop recordings capture the entire virtual desktop, not a hidden WebView.
External file-drop input, Narrator and untested screen-reader cases require
further acceptance; the linked NVDA matrix describes the observed cases.
Browser preview cannot prove those OS behaviors.

## Repeatable verification

From the repository root, with Node.js installed:

```powershell
cargo run -p rdesktop-cli -- dev --path examples/native_agent --port 1438
node scripts/verify-native-agent.mjs examples/native_agent/.rdesktop/agent-cli-PID.json .cache/native-visible
npm ci --ignore-scripts
node scripts/verify-native-accessibility.mjs examples/native_agent/.rdesktop/agent-cli-PID.json .cache/a11y-visible
node scripts/verify-native-uia-actions.mjs examples/native_agent/.rdesktop/agent-cli-PID.json .cache/uia-visible
node scripts/verify-native-uia-collections.mjs examples/native_agent/.rdesktop/agent-cli-PID.json .cache/collections-visible node_modules/axe-core/axe.min.js target/debug/rdesktop.exe
node scripts/verify-native-uia-relations.mjs examples/native_agent/.rdesktop/agent-cli-PID.json .cache/relations-visible
node scripts/verify-native-dialogs.mjs examples/native_agent/.rdesktop/agent-cli-PID.json .cache/dialogs-visible target/debug/rdesktop.exe
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

Starter checks save `native-initial.json` and, for visible windows,
`native-ready.json`. A completed document and engine AX tree can precede the
Windows provider: the generated-project run observed 18 native nodes without
the field, then 38 nodes with its real input after about 290 ms. The verifier
waits at most 10 seconds on UIA publication, preserves provider errors, and
still fails if the field never appears. Hidden runs do not wait for or show a
provider, and never substitute engine AX for UIA.

The UIA action fixture independently checks real native Value/Toggle/Range,
single/multiple SelectionItem, Invoke/Legacy, ExpandCollapse, Text selection,
Scroll, Transform and Window effects. It includes disabled/read-only controls,
ambiguous and removed targets, password redaction, search limits, cross-window
rejection and normal auxiliary-window close. Its hidden run records actual
provider availability and verifies that a request does not show the window.

The separate collections fixture verifies native grid coordinates, merged
cells, header/owner relations, selected-item collections and explicit item
scans, incomplete totals, password redaction, bounds, and cross-window/closed
window rejection. Its axe run keeps all rules enabled and preserves incomplete
contrast results for the rowspan and two native select controls. Separate
checks measure their actual opaque rendered foreground/background colors,
including all selected/unselected options, against WCAG AA's 4.5:1 threshold;
native visual review remains part of release acceptance. This is distinct from
the accessibility fixture's zero-incomplete axe result.

The native-dialog verifier keeps a real Windows modal open beyond 30 seconds,
checks parent state and responsive reads, invokes its real Cancel button,
verifies stale/cross-renderer rejection, and repeats with WebView2's actual
file picker. It decodes captured RGBA pixels, rejects blank frames, checks
stable DWM coordinates and CLI PNG output, and verifies minimized/hidden
capture rejection without changing visibility. File-picker screenshots can
contain local filenames and must remain ignored local artifacts.
