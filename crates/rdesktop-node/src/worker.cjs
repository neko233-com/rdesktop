"use strict";
// stdin/stdout are private framed RPC, never a renderer or an Electron process.
const fs = require("node:fs");
const path = require("node:path");
const MAX_FRAME = 4 * 1024 * 1024;
const MAX_DEPTH = 32;
const write = process.stdout.write.bind(process.stdout);
process.stdout.write = process.stderr.write.bind(process.stderr);
for (const name of ["log", "info", "warn", "error", "debug", "dir", "trace"]) {
  console[name] = () => {}; // A plugin's output cannot enter the RPC stream.
}
// This also covers dependencies using the shared built-in module exports.
if (process.platform === "win32") {
  const childProcess = require("node:child_process");
  for (const name of ["spawn", "spawnSync", "exec", "execSync", "execFile", "execFileSync", "fork"]) {
    const original = childProcess[name];
    childProcess[name] = function (...args) {
      const index = name === "exec" || name === "execSync" ? 1 : Array.isArray(args[1]) ? 2 : 1;
      if (args[index] && typeof args[index] === "object") args[index] = { ...args[index], windowsHide: true };
      else args.splice(index, 0, { windowsHide: true });
      return Reflect.apply(original, this, args);
    };
  }
  require("node:module").syncBuiltinESMExports();
}
const window = Object.create(null);
Object.defineProperty(globalThis, "window", { value: window });
let loaded = false, closing = false;
function nodePath(value) {
  // Rust canonicalize returns extended Windows paths. CommonJS resolution
  // needs a drive/UNC path; Node adds the extended prefix for filesystem I/O.
  if (process.platform === "win32" && typeof value === "string") {
    if (value.startsWith("\\\\?\\UNC\\")) return "\\\\" + value.slice(8);
    if (value.startsWith("\\\\?\\")) return value.slice(4);
  }
  return value;
}
function pack(value, depth = 0, seen = new Set()) {
  if (depth > MAX_DEPTH) throw new RangeError("Node result nesting exceeds its limit.");
  if (value === undefined) return { $rdesktop: "undefined" };
  if (typeof value === "bigint") return { $rdesktop: "bigint", value: String(value) };
  if (Buffer.isBuffer(value)) return { $rdesktop: "buffer", base64: value.toString("base64") };
  if (value === null || typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new TypeError("Node result must contain finite numbers.");
    return value;
  }
  if (typeof value !== "object") throw new TypeError("Node result type is not supported.");
  if (seen.has(value)) throw new TypeError("Cyclic Node results are not supported.");
  seen.add(value);
  try {
    if (Array.isArray(value)) return value.map(item => pack(item, depth + 1, seen));
    if (Object.getPrototypeOf(value) !== Object.prototype && Object.getPrototypeOf(value) !== null) {
      throw new TypeError("Node result must be a plain object, array, Buffer or scalar.");
    }
    // Wrap objects so a plugin's own $rdesktop property never becomes a tag.
    return { $rdesktop: "object", entries: Object.entries(value).map(([key, item]) => [key, pack(item, depth + 1, seen)]) };
  } finally { seen.delete(value); }
}
function reply(id, success, data) {
  let response = JSON.stringify({ version: 1, id, success, data });
  if (Buffer.byteLength(response) > MAX_FRAME) {
    response = JSON.stringify({ version: 1, id, success: false, data: { name: "RangeError", message: "Node response exceeds its size limit." } });
  }
  write(response + "\n");
}
async function request(message) {
  if (!message || message.version !== 1 || typeof message.id !== "string" || message.id.length > 64) {
    throw new TypeError("Invalid Node request envelope.");
  }
  if (message.operation === "shutdown") {
    closing = true;
    reply(message.id, true, null);
    // Preloads can own intervals. Shutdown is host-owned, not window.close().
    setImmediate(() => process.exit(0));
    return;
  }
  if (message.operation === "load") {
    if (loaded) throw new Error("A Node runtime can load only one preload.");
    const root = fs.realpathSync(nodePath(message.root));
    const preload = fs.realpathSync(nodePath(message.preload));
    const relative = path.relative(root, preload);
    if (!relative || relative === ".." || relative.startsWith(".." + path.sep) || path.isAbsolute(relative)) {
      throw new Error("Preload must be a file inside its package root.");
    }
    if (!fs.statSync(preload).isFile() || fs.statSync(preload).size > 1024 * 1024) {
      throw new Error("Preload is not a bounded regular file.");
    }
    process.chdir(root);
    loaded = true;
    require(preload);
    const names = Object.keys(window);
    if (names.length > 256 || names.some(name => name.length > 256)) throw new RangeError("Too many preload exports.");
    const functions = names.filter(name => typeof window[name] === "function");
    reply(message.id, true, {
      node: process.versions.node, electron: process.versions.electron ?? null,
      functions,
    });
    return;
  }
  if (message.operation === "call") {
    if (!loaded) throw new Error("Node preload is not loaded.");
    if (typeof message.name !== "string" || !Object.hasOwn(window, message.name) || typeof window[message.name] !== "function") {
      throw new TypeError("Unknown preload function.");
    }
    if (!Array.isArray(message.args) || message.args.length > 256) throw new TypeError("Invalid preload arguments.");
    reply(message.id, true, pack(await Reflect.apply(window[message.name], window, message.args)));
    return;
  }
  throw new TypeError("Unknown Node operation.");
}
// A single in-flight request keeps CommonJS shared state deterministic.
let busy = false;
function lineReceived(line) {
  if (closing) return;
  // The parent bounds writes; bound the worker too for corrupted transports.
  let message;
  try { message = JSON.parse(line); }
  catch { process.exit(1); }
  if (busy) { reply(message.id, false, { name: "Error", message: "Node runtime is busy." }); return; }
  busy = true;
  request(message).catch(error => reply(typeof message?.id === "string" ? message.id : "", false, {
    name: error?.name || "Error", message: String(error?.message || error),
    ...(typeof error?.code === "string" ? { code: error.code } : {}),
  })).finally(() => { busy = false; });
}
let input = Buffer.alloc(0);
process.stdin.on("data", chunk => {
  input = Buffer.concat([input, chunk]);
  for (;;) {
    const end = input.indexOf(10);
    if (end < 0) {
      if (input.length > MAX_FRAME) process.exit(1);
      return;
    }
    if (end > MAX_FRAME) process.exit(1);
    const line = input.subarray(0, end).toString("utf8");
    input = input.subarray(end + 1);
    lineReceived(line);
  }
});
// Closing the host's pipe ends the worker even when a plugin owns timers.
process.stdin.on("end", () => { if (!closing) process.exit(0); });
