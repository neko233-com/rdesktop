# rdesktop

The real native Rust CLI for Windows 10/11 x64, distributed through npm. No
postinstall scripts, browser emulation or account login is required to use it.
The package checks its bundled executable's SHA-256 before starting it.

```powershell
npm install -g https://github.com/neko233-com/rdesktop/releases/download/v0.1.11/rdesktop-0.1.11.tgz
rdesktop init my-app
cd my-app
rdesktop dev
```

This release tarball works independently of npm registry publication. Once
version 0.1.11 is available on the registry, `npm install -g rdesktop@0.1.11`
installs the same distribution.

Native builds need the Rust MSVC toolchain, Windows SDK and WebView2 Runtime.
`--headless` starts the same native windows hidden. `--browser` selects the
separate frontend preview. Production applications do not include the opt-in
debug Agent server.

```powershell
rdesktop agent --session .rdesktop/agent-cli-PID.json windows
rdesktop agent --session .rdesktop/agent-cli-PID.json accessibility
rdesktop agent --session .rdesktop/agent-cli-PID.json uia
rdesktop agent --session .rdesktop/agent-cli-PID.json screenshot --output main.png
```

Read the [native Agent contract](https://github.com/neko233-com/rdesktop/blob/main/docs/NATIVE_AGENT.md)
for structured input, multi-window selection, WCAG audits, console events,
recording, release isolation and the distinction between hidden engine AX and
Windows UIA. macOS, Linux and Windows ARM64 npm binaries are not provided.

Source and reproducible verification: [neko233-com/rdesktop](https://github.com/neko233-com/rdesktop).
