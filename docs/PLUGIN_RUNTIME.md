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

## Experimental native renderer module mode

Unreleased `main` also provides `NodeRuntime::start_modules` and
`rdesktop_node::renderer::RendererBridge`. The original CommonJS preload and its
JavaScript dependencies execute in the actual WebView2 document. Their `window`,
DOM, closures, synchronous callbacks between JavaScript modules, module cache and
CommonJS cycles therefore remain in that document. Node built-ins and native
objects use a separate, explicitly selected Node worker. Electron supplies none
of the renderer, input, UIA provider or window lifecycle in this mode.

`RendererBridge::start(config, "index.html")` returns `url()` and
`initialization_script()`. Give these to `NativeWindowOptions` on a worker through
`NativeRuntime::create_window`. Keep the bridge alive for that one document and
drop it when the document retires. A reload needs a new bridge; a second document
cannot attach to the first document's module/callback table. The example
`examples/node_plugin` shows this wiring without adding it to the CLI or starters.

Applications with an existing private per-plugin asset server can use
`RendererModules::start(config, base_url)` instead. Inject its
`initialization_script()` into the native document and route POST `__node` under
that exact base to `respond(origin, capability, document, request)`. The response
is an HTTP status and JSON envelope. The application remains responsible for
body/header limits, HTTP method checks and retiring the session with its lease;
the framework reuses the same origin/capability/document checks as its standalone
server. This keeps the application's existing database and host API endpoints
on the same private origin without granting cross-origin access or adding a
second asset server.

The loopback service binds only `127.0.0.1`, uses a random 256-bit capability,
requires the exact Host and Origin plus a custom capability header, and binds
Node calls to the first attached document identity. It supplies no CORS grant.
Remote frames do not receive the initialization script. The default local CSP
also blocks remote page networking; this is not an Electron network-compatibility
claim. Node modules retain their explicitly trusted Node network authority.

The renderer issues this browser request sequence against its private `__node`
endpoint: `attach`, `resolve`, `get`/`call`/`descriptor`, and `events`; released
object identities use `release`. RPC is POST only. The private worker envelope is
not an application IPC command and unrelated origins/methods/leases are rejected.
WinSock accepted sockets explicitly return to blocking mode before bounded reads;
a split-header regression reproduces the former inherited-FIONBIO failure.

Current bounds: 4 MiB RPC frames, 16 MiB assets, 1 MiB CommonJS sources, 4096 Node
references, 256 renderer callbacks and 256 queued events; renderer worker calls
must use deadlines of at most five seconds. HTTP headers/body have a five-second
total read deadline. Node references use browser GC release hints, callbacks and
Promises return through a scoped event queue, and errors preserve name/code.
Buffer results are browser `Uint8Array` values with a Buffer facade shared by
the lexical `Buffer` and `require('buffer').Buffer`. The selected Node executable
supplies the method catalog, signatures, encoding/argument validation and slow
operations. Common allocation, byte views and valid integer/float/BigInt reads
and writes execute in the document. Slice/subarray preserve shared backing
stores; copy/fill/swap operations preserve overlapping aliases and update the
original document buffers. Small views transfer their required backing region,
including alignment, instead of the entire allocation. TypeError and RangeError
preserve their native JavaScript classes and Node error codes.

Buffer RPC accepts copied data only, with no callbacks or Node references. It
uses captured built-in functions, leaves pending async events for polling, and
publishes byte changes only after a complete reply. A single NetworkError retry
is therefore allowed for these copy-only commands. Other Node operations retain
the no-replay contract, including after a file write's response is lost.
Fault-injection checks verify both cases and preservation of a pending callback.

This is measured compatibility, not a claim of every Buffer/Node behavior. Node
allocation-pool identity/layout, arbitrary prototype overrides, SharedArrayBuffer
and mutation of Buffer arguments by other native Node APIs remain migration
gates. RPC frames still have the 4 MiB bound. A full authority plugin is not a
filesystem or subprocess sandbox.

`RendererModules::start_with_globals(config, base_url, NodeGlobals::PreloadOnly)`
and `RendererBridge::start_with_globals` select a preload-only loading policy.
Preloads and their CommonJS dependencies retain lexical `process`, `Buffer`,
`global`, `setImmediate` and `clearImmediate`; page scripts do not implicitly
receive `require`, `process`, `Buffer` or `global`. This prevents browser bundles
from taking a Node branch merely because a preload needs Node. Explicit preload
exports remain available. It is not a sandbox or a separate JavaScript world.
The existing `start` methods keep the page-global policy for compatibility.
Both policies preserve the CommonJS five-argument wrapper, top-level `this`,
local declarations shadowing Node globals, and modules loaded after navigation
has finished. Page-level `setImmediate` and `clearImmediate` remain available.

Native verification is repeatable, with all evidence outside the checkout:

```powershell
$env:RDESKTOP_TEST_NODE = (Get-Command node).Source
$env:RDESKTOP_TEST_NODE_SHA256 = (Get-FileHash -LiteralPath $env:RDESKTOP_TEST_NODE -Algorithm SHA256).Hash.ToLowerInvariant()
cargo build -p node_plugin --features native-agent
node scripts/verify-node-plugin-native.mjs fixture "$env:TEMP/rdesktop-node-acceptance"
node scripts/verify-node-plugin-native.mjs fixture "$env:TEMP/rdesktop-node-acceptance" --headless
node scripts/verify-node-plugin-native.mjs fixture "$env:TEMP/rdesktop-node-acceptance" --preload-only
node scripts/verify-node-plugin-native.mjs fixture "$env:TEMP/rdesktop-node-acceptance" --preload-only --headless
node scripts/verify-node-plugin-native.mjs public "$env:TEMP/rdesktop-node-acceptance" C:\path\to\immutable\utools-file-to-folder
```

The public case requires the original ISC repository at commit
`8a3f257591935cf4d2dd4baa599f3c20fc1b7e5d`. It hashes every tracked file before
and after, uses real UIA Value/Invoke and label relations, creates only a fresh
absolute temporary directory through the original preload, and normally exits.
Its example supplies a small fixture uTools host adapter (folder context,
notification and exit), not a complete uTools SDK. Do not mistake that adapter
for iHub's finished lifecycle, file picker, theme or detach implementation.

The fixture case covers synchronous fs/Stats methods, module identity/cycles and
top-level `this`, callbacks, errors, Promise results, EventEmitter, property
descriptors/mutation, byte buffers, DOM closures and a real hidden Node child.
Both modes inspect the actual owned process tree for Node and WebView2, reject
Electron descendants, and require the owned Node worker to exit normally.

Windows x64 acceptance on 2026-10-19 used Node 24.19.0. The current fixture passed
24 checks, including lazy modules, lexical global boundaries and custom error
names, in visible and
hidden native windows with the preload-only policy, plus the default page-global
policy. The original public package also passed the preload-only policy and
passed UIA Value/Invoke, label relation, byte-preservation and real mkdir checks.
The accepted-socket fix also passed eight consecutive native fixture runs. The
22 explicit Node tests (including the externally hosted session and copy-only
Buffer gates) and the full
workspace's native-agent build/test/clippy checks passed.

The identical Buffer case functions also ran against the explicitly pinned Node
executable and the real native WebView2 facade: 414 cases matched, including
numeric boundaries, encoding edge cases, method aliases/signatures, coercion,
overlapping copy/fill, shared views, legacy constructors, and a small view on an
8 MiB backing store. The visible, hidden and page-global modes passed. A
100000-iteration numeric read/write loop returned the expected sum with zero
IPC requests. Full reference/native results and transport-fault proofs are saved
by `verify-node-plugin-native.mjs` outside the repository.

The release example additionally passed the same 18 checks through real Windows
accessibility input. It contained no Agent/session/headless/UIA debug markers,
ignored injected debug environment variables and created no Agent session or
debug navigation. It exited normally with code zero. That standalone EXE was
1971200 bytes (SHA-256
`4e7889015e486f88ca4d70bfce8435d701a99d10a73182f6d4288550205cc196`),
excluding the separately selected Node executable and installed system WebView2.
This is an example measurement, not an iHub installer size or Authenticode claim.
The workspace lock adds only the local example package; no new external resolved
dependency, Node binary or second browser was added to the default CLI payload.

## Remaining integration gates

The direct worker mode has no DOM. The renderer mode is experimental: it does
not yet replace iHub's Electron runtime, ship a production Node executable, or
establish native addon ABI compatibility. ESM, arbitrary synchronous return
values from callbacks into Node, full Buffer/prototype semantics, complete
callback disposal and application-owned subprocess lifecycle remain incomplete.
Loading a native addon is delegated to the selected Node ABI and has not been
accepted as universal addon compatibility. Electron API adapters, uTools host
lifecycle, native dialogs, drag/drop/file paths and detach/adopt are still gates.

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

## Native embedded documents on Windows

`NativeWindowOptions.parent_window` accepts an existing window ID owned by the
same `NativeRuntime`. An unknown or expired parent is rejected before creation.
The window is created with Tao's child flag, preserving `WS_CHILD` across native
show/hide and style updates. Converting an existing top-level window with
`SetParent` alone does not establish that flag and can lose its native parent on
the next show operation. Applications still own child bounds, document IPC,
navigation policy and lifetime. Other platforms return an explicit unsupported
error for this option until they have native acceptance coverage.

Run `scripts/verify-native-child.mjs SESSION OUTPUT` against the `native_agent`
example to exercise parent rejection, nested native accessibility, repeated
show/hide/resize and child-only teardown in a real WebView2 window.
