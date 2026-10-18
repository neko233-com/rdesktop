# Native plugin runtime migration

The published rdesktop 0.1.15 provides native WebView2 windows, IPC and the
development Agent/UIA interfaces. It does **not** provide a Node preload or a
complete Electron compatibility layer. iHub currently fills that gap with an
Electron child renderer. That is an outstanding migration dependency, not proof
that the plugin already runs entirely on rdesktop.

The target is a plugin rendered in the same native rdesktop WebView2 path as the
application, with framework-owned Node execution and native API adapters. Node.js
is a language runtime here; Electron must not own the plugin's renderer, input,
accessibility tree or window lifecycle.

## Current source implementation

`rdesktop-node` is an unpublished execution primitive. An application explicitly
chooses a Node executable, pins its SHA-256 and supplies one package/preload root.
The framework verifies that binary before launching it. Windows creation starts
with `CREATE_NO_WINDOW`; child-process built-ins also set `windowsHide: true`.
`NODE_OPTIONS`, `NODE_PATH` and `ELECTRON_RUN_AS_NODE` are removed from the worker
environment. No binary is discovered from PATH or downloaded by the library.

The worker loads real CommonJS dependencies and exposes functions assigned to
`window`. Calls run in one private Node process and can await returned Promises.
Console/stdout projection is kept out of the private framed RPC stream. Frames
are limited to 4 MiB. Concurrent calls fail as busy; calls have explicit deadlines
of 1..120000 ms. A timed-out or malformed worker becomes unusable and is not
automatically retried. A separate I/O thread keeps a blocked pipe write inside
that deadline. Normal closure sends shutdown and waits for the owned worker;
unresponsive/faulted workers receive at most one second of termination grace.
This cleanup never terminates the application host or unrelated processes.

This is full-authority Node execution for explicitly trusted plugins, not a
filesystem or subprocess sandbox. Canonical preload-root checks select the entry
point; they do not restrict what a trusted `fs` call can access.

Bundle size is a migration gate. The crate is opt-in, is not a dependency of the
CLI/default scaffold and ships no Node binary. It reuses already resolved Rust
dependencies; its workspace lock change adds only this local crate. Production
integration should reuse the installed system WebView2 and keep Node, image and
FFmpeg runtimes in independently verified, demand-driven components. Measure
actual locked payload bytes and compressed installers before claiming savings.

Arguments currently contain JSON values. Results preserve arrays/scalars and use
these explicit representations:

| Result | Representation |
| --- | --- |
| `undefined` | `{ "$rdesktop": "undefined" }` |
| `bigint` | `{ "$rdesktop": "bigint", "value": "13" }` |
| `Buffer` | `{ "$rdesktop": "buffer", "base64": "..." }` |
| Plain object | `{ "$rdesktop": "object", "entries": [["key", value]] }` |

Wrapping objects preserves a plugin's own `$rdesktop` keys. Cycles, non-finite
numbers, excessive nesting and unsupported object types fail explicitly. Errors
preserve name/message/code without replacing them with successful empty values.

## Remaining integration gates

The current primitive does not project DOM objects, preload callbacks, Electron
APIs, lifecycle events or Node objects into a renderer. It does not yet replace
iHub's Electron runtime, ship a production Node executable, or establish native
addon ABI compatibility.

Before claiming that migration is complete:

1. Connect original plugin pages and preload exports to native WebView2 without
   giving remote frames or navigations the trusted Node binding.
2. Implement the framework's Node/Electron API adapters, callback/event/object
   lifetime rules, buffer transfers, dialogs, clipboard, native images, capture,
   subprocesses and declared native addon support.
3. Run an unchanged, immutable public plugin through real input, UIA, keyboard,
   screen-reader, screenshot, detach/adopt and filesystem behavior. Verify the
   process inventory contains no framework-launched Electron renderer.
4. Move iHub to those framework APIs and remove its Electron runtime, downloader,
   runtime lock and packaged Electron resources. Repeat production packaging,
   ordinary installed-app exit, update and development-isolation verification.

Neither a renamed process nor copied iHub adapters meet these gates. The same
renderer must supply the plugin's actual desktop and accessibility behavior.

## Repeating worker tests

Pure configuration tests run with `cargo test --workspace`. Tests requiring an
executable are marked ignored until the caller explicitly selects and pins it:

```powershell
$env:RDESKTOP_TEST_NODE = 'C:\path\to\node.exe'
$env:RDESKTOP_TEST_NODE_SHA256 = '<verified SHA-256>'
$env:RDESKTOP_TEST_PUBLIC_PLUGIN = 'C:\path\to\utools-file-to-folder'
cargo test -p rdesktop-node -- --include-ignored
```

Current runtime evidence is Windows x64 only. The worker contract tests cover
real CommonJS dependencies and filesystem calls, ordinary and failed startup
exit, typed errors, Promise/value behavior, console isolation, bounds, malformed
frames, concurrent calls and stalled input pipes. They do not replace the
renderer/plugin acceptance gates above.
The public-preload test additionally verifies the original `preload.js` and
`utils.js` hashes from ISC `fugary/utools-file-to-folder` commit
`8a3f257591935cf4d2dd4baa599f3c20fc1b7e5d`, then invokes its actual
`createCurrentFolder` with a fresh absolute temporary path. This is a preload
execution result; its original HTML and uTools lifecycle have not passed the
new runtime's native acceptance yet.
