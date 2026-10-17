# Windows integration validation

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
