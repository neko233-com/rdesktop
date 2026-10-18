# Changelog

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
