# Changelog

## 0.1.11

Native UIA now reads grid cells, merged spans, table/cell headers, selected-item
collections and their owner controls. Returned native elements must prove
ancestry inside the selected registered window. Collection limits and truncation
are explicit; invalid coordinates and unsupported patterns fail. Inspection
adds Grid/GridItem/Table, Dock, MultipleView, Selection2, Transform2 and Legacy
states, with password values still redacted. Agent capabilities advertise the
new operations and bounded collection parameters.

An explicit native SelectionItem scan handles providers whose selected-item
collection disagrees with their item flags, while retaining the original
collection as a separate strategy. Scan limits are independent of target-search
limits; incomplete scans never claim a complete total. UIA tree walking now
preserves successful null COM results and propagates actual provider failures.

The new fixture verifies 14 groups of real Grid/Table/Selection behavior and
scope checks, alongside the existing visible/hidden native and accessibility
regressions. Its axe audit has no violations but retains incomplete contrast
results for merged/native-select rendering; separate rendered-color checks
measure at least 8.17:1, and actual native visual review confirms legibility.
These results do not constitute Narrator/NVDA speech or external-dialog
acceptance. Provider discrepancies and repeatable checks are documented in
[the native contract](docs/NATIVE_AGENT.md).

## 0.1.10

The native Agent now drives Windows UI Automation through real COM patterns:
Value, RangeValue, Toggle, ExpandCollapse, SelectionItem, Invoke, Scroll,
Text selection, Window and Transform, with explicit provider-dependent
VirtualizedItem, LegacyIAccessible and MultipleView requests. `agent info`
advertises the backend's operations and target criteria.

Targets are resolved again inside the selected registered native window using
exact criteria or opaque provider runtime IDs. Ambiguous, removed, disabled,
read-only, unsupported and truncated name searches fail explicitly. Runtime
IDs are temporary, not persistent identities. Mutations share per-window input
exclusion; password inspection/text is redacted and value writes never echo
their input. GET requests cannot perform UIA mutations.

A separate native fixture verifies 17 groups of actual UIA effects, including
Unicode targets, text selection, native movement/resizing, window state and
normal close. Visible/hidden engine and accessibility regression checks,
installer compilation, generated-project compilation and production interface
isolation also pass. Hidden Windows provider trees remain provider-dependent;
separate OS dialogs and Narrator/NVDA speech are not claimed as validated.
The WebView2 Scroll provider can report zero percent after a successful native
scroll; inspection preserves that OS result and verification records the real
viewport change separately. See [the full contract](docs/NATIVE_AGENT.md).

## 0.1.9

Windows development now starts the actual Rust desktop application by default.
`dev --headless` keeps the same WebView2 windows hidden; `dev --browser` selects
the separate preview. Existing applications opt in through the `native-agent`
feature and `enable_native_agent_from_env()`.

The authenticated loopback Agent CLI inspects native windows, DOM, computed
accessibility trees, Windows UI Automation, console/AX events and PNG frames.
It drives real engine input, role/name targets, native resize/zoom, Rust IPC and
frontend hot reload. It also runs a caller-supplied local axe-core engine.
Ordinary release applications compile out this integration, including when
the development feature is accidentally selected.

Fixed native IPC replies becoming stranded around nested WebView creation,
window IDs losing JavaScript integer precision, incomplete Enter key events,
focus accidentally clicking controls, failed/non-editable input reporting
success, inherited Windows startup hiding explicitly visible windows, and a
hot-reload generation race. Drag input uses the engine's intercepted data and
cleans up interception/button state on failure.

The CLI now generates a compilable native project, performs real release
builds, resolves Cargo's actual release binary for bundling, and accepts an
explicit binary path. Generated dependencies pin this immutable release tag.
Windows x64 npm distribution includes the native CLI and verifies its SHA-256.

Validation covers visible and hidden native runs, real Rust IPC, two-window
isolation, trusted input/drag data, backpressure, native 200% zoom, keyboard and
modal focus, Shadow DOM and iframe accessibility, positive/negative axe checks,
actual Windows installer compilation and release-interface isolation. Native
macOS/Linux Agent support and Narrator/NVDA speech are not claimed by these
Windows checks. See [the full contract](docs/NATIVE_AGENT.md).
