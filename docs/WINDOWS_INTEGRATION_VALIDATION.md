# Windows integration validation

## Modal UI dispatch regression (2026-10-18)

`NativeRuntime::with_window` is a synchronous query with a 30-second reply
deadline. A consumer must not use it as a scheduler for an operation that opens
a native modal picker: after 30 seconds the caller receives an error even while
the user is still selecting a file. Raising the HTTP timeout alone does not fix
that native scheduling failure.

`NativeRuntime::dispatch` queues a UI-thread closure and reports acceptance.
It uses the same 256-request bound and rejects work after shutdown; shutdown
drops unexecuted closures. It has no operation-completion deadline. Callers
that need results provide their own completion channel and must not wait on
that channel from the UI thread. Existing synchronous query deadlines remain.

Queue tests exercise immediate acceptance from the registered UI thread,
independent completion, saturation and shutdown without opening user windows.
For a native consumer check, use the local iHub Cargo command-line patch
overrides for rdesktop-core and rdesktop-webview; do not commit local dependency
paths. Launch its debug executable, open the SD image parameter fixture, keep
the Windows picker open for more than 30 seconds, then select only a test-owned
PNG. The plugin must display its parameters and the original copy button must
write the exact owned text. Preserve the pre-test clipboard formats in memory
and restore them after observing that text. Normal application exit must still
work. This check is separate from the synthetic HTTP dialog-delay test.

The browser request sequence is unchanged: open the consumer preview at
`http://127.0.0.1:1420/`, open plugin center, open settings, then return to the
launcher and inspect warning/error logs. Dispatch is a native Rust host API;
browser preview does not stand in for the modal Windows check.

The local Windows check passed against the patched framework: after a measured
64 seconds in the real picker, iHub displayed the owned PNG and its parameters.
The public plugin's original copy button produced the exact text; the test
restored prior clipboard formats in memory. The tested local-patch consumer EXE
SHA-256 was `0044ce0dd615ff7225624269863a78c8cd889942ee78046e42d4ebb4da879ad0`.
Framework fmt/check/clippy and 40 tests passed (the installer compiler opt-in
remained ignored). The browser plugin-center/settings/launcher sequence also
passed with no warning/error logs. Version numbers remain unchanged.
The consumer then exited through its visible settings control; no iHub process
remained. No process was forcibly terminated or installed executable replaced.

Validated locally on Windows x64 on 2026-10-17. No macOS or ARM64 runtime
validation is implied. The source version remains 0.1.8 during this work.

## Framework checks

Run from the workspace root:

```powershell
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The native runtime tests cover queue bounds, missing windows, shutdown, and
rejecting synchronous calls from the event-loop thread. Renderer tests cover
embedded asset resolution and window-scoped file-drop responses. Hotkey tests
exercise actual Windows registration ownership and conflict reporting.

## Real installer compiler check

Set `RDESKTOP_NSIS` to makensis.exe, `RDESKTOP_CANDLE` and `RDESKTOP_LIGHT` to
WiX 3 compiler executables, and `RDESKTOP_TEST_INSTALLER_ICON` to an ICO produced
by `rdesktop icons`. Then run:

```powershell
cargo test -p rdesktop-bundler windows_compilers_produce_real_installers -- --ignored --nocapture
```

This packages the test executable into real NSIS and MSI files without running
either installer. The check uses spaces in the product name and an ampersand in
publisher metadata. It caught and fixed invalid NSIS instruction placement,
NSIS output-name quoting, WiX 3 rejecting canonical verbatim path prefixes,
and BOM handling for compiler objects. Local tools were NSIS 3.11 and WiX
3.14.1. ICE91 reports that these packages install only in the current user's
profile, which is the intended per-user scope.

Installers create a Start menu shortcut only when an ICO is configured; that
ICO is installed beside the executable and used as the shortcut icon. WebView2
must already be installed. The bundler does not currently embed its runtime
bootstrapper, sign the publisher, or embed application frontend assets.

## Native consumer check

The local iHub consumer uses `NativeRuntime` for its resident main window and
native windows, and embeds its production frontend with `set_embedded_assets`.
The tested development binary was built from the current sources, with no
installer or installed executable replacement:

1. Build the iHub frontend and its debug Rust executable.
2. Launch the executable. Confirm the native window renders the launcher at
   `http://rdesktop.localhost/index.html` and native commands load index status.
3. Press Escape. Confirm the launcher becomes hidden while its process remains.
4. Launch the same executable again. Confirm the existing window returns and
   the original PID remains the only iHub process.
5. Open the plugin center. Confirm the catalog renders, then open preferences.
6. Use the visible **退出 iHub** control. Confirm no iHub process remains.

These steps passed on the local Windows desktop. They validate resident-window
operation, IPC, embedded content, single-instance activation in the consumer,
and normal exit. They do not establish complete plugin API compatibility.

For the equivalent browser UI flow, run `pnpm dev --host 127.0.0.1` in the iHub
consumer, open `http://127.0.0.1:1420/`, open local search, select PDF, and check
that the list and inspector both change. At an 800×602 viewport all three panes
must remain visible; console errors and framework overlays must be absent.
Browser mode uses preview data and cannot validate native file icons or plugin
Node/Electron execution. Those require separate desktop checks.

## Native CSS title-bar interaction (2026-10-18)

Wry 0.56.1 already enables WebView2 non-client regions. Mark title regions with
`app-region: drag` plus the Chromium-prefixed form, and explicitly mark controls
`no-drag`. Moving from an asynchronous application command to CSS regions made
quick native dragging work in iHub's settings, launcher strip and plugin center.

CSS dragging did not emit Tao's `DecorationsClick` in this local run. The
framework now latches `WM_ENTERSIZEMOVE` on the owning window and reports one
`rdesktop:window-move-resize-start` event on the following move/resize/focus
dispatch. The latch survives a quick completed loop and is consumed once;
configured minimum/maximum tracking flags remain intact. Programmatic layout
does not enter that loop. The hidden native-window regression test exercises
those message sequences without moving another application's windows.

The full workspace checks above passed, including 38 tests and one separately
opted-in installer compiler test remaining ignored. The consumer was first
built against the local framework using command-line Cargo patch overrides;
these overrides are not checked-in dependency paths.

Repeat the browser inspection in the consumer at `http://127.0.0.1:1420/`:

1. Focus search, press Ctrl+comma, and inspect the settings header.
2. Read `getComputedStyle(header).getPropertyValue('app-region')`; it is `drag`.
   Both header buttons read `no-drag`.
3. Click the back button, then the local-settings button in the plugin center.
4. Confirm settings opens and console warnings/errors remain empty.

Those browser requests passed. The equivalent native Windows sequence also
passed with the actual window bounds captured after every input:

| Step | Visible window origin | Visible screenshot size |
| --- | --- | --- |
| Settings before drag | (559,210) | 802×604 |
| Drag title by (+100,+40) | (659,250) | 802×604 |
| Escape to launcher | (659,250) | 802×382 |
| Reopen settings | (659,250) | 802×604 |
| Back to plugin center, drag strip by (-50,+30) | (609,280) | 802×604 |
| Open settings, click close | (609,280) | 802×382 |
| Drag launcher strip by (+60,+40) | (669,320) | 802×382 |

The plugin-center input accepted and filtered `JSON`, then was cleared. Dragging
that input did not move the window. Header buttons were tested with actual
coordinate clicks, so an accessibility invocation did not bypass hit testing.
Hiding and re-launching the existing process retained its PID and reset the
visible session to the centered launcher. This is Windows x64 evidence only;
other platforms and touch/pen hardware were not runtime-tested here.
