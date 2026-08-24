# rdesktop integration rules

This repository is the local Rust desktop framework source used by Terminal233. Keep the public
framework API and the browser Agent API observable so the same frontend can be inspected in a
browser and rendered by the native desktop path.

## Terminal233 contract

- Terminal233 pins rdesktop revision `01359a93b4337698ee4f43093f0c7bc78bb1b99a`; framework changes
  must be committed here first, then consumed by a new explicit revision in Terminal233.
- Desktop and browser launch are two renderers of one UI contract. Do not make browser-only UI
  behavior the source of truth for the native renderer.
- Native application IPC must remain structured, bounded, and testable through the Agent API.
- Keep screenshot/recording output bounded and idempotent; never persist credentials or test data.
- Keep the window decoration bridge observable and tested. `setDecorations(true)` means the OS
  title bar/borders are visible; `setDecorations(false)` enables a custom integrated title bar.
  Applications may switch this live, but must provide accessible custom controls for frameless
  windows and keep the system-decorated path usable.

## Platform and release rules

- Current local validation is Windows `x86_64-pc-windows-msvc` only.
- `aarch64-pc-windows-msvc` is reserved for future tag-only GitHub Actions after the user explicitly
  says “发布”. Do not build or publish ARM64 during ordinary development.
- 32-bit Windows targets are not supported.
- Do not add `.github/workflows`, tags, or release assets during normal development. The shared
  Terminal233 policy allows release automation only after an explicit publish instruction.

## Verification

Run `cargo fmt --all -- --check`, `cargo check --workspace`, and `cargo test --workspace` before
handing off framework changes. For renderer or Agent API changes, include a repeatable browser
request sequence and a native Windows visual check. Keep generated recordings, caches, credentials,
and installers out of Git.
