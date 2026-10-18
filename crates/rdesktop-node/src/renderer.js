function rdesktopNodeRenderer(options) {
  'use strict';
  if (window.top !== window || !location.href.startsWith(options.base)) return;
  const documentIdentity = Array.from(crypto.getRandomValues(new Uint8Array(16)), byte => byte.toString(16).padStart(2, '0')).join('');
  const references = new WeakMap(), proxies = new Map(), callbacks = new Map(), callbackIds = new WeakMap();
  const callbackAcks = new Map();
  const promises = new Map(), released = new Set(), retained = new Map(), modules = new Map();
  let nodeProcess, nodeSetImmediate, nodeClearImmediate, memory, idleRequest = null;
  const startTimer = window.setTimeout.bind(window), stopTimer = window.clearTimeout.bind(window);
  let sequence = 0, callbackEpoch = 0, transportSequence = 0, closed = false, polling = false, terminalError = null, timer = null;
  const finalizer = new FinalizationRegistry(({ id, key }) => {
    if (!proxies.get(key)?.deref()) proxies.delete(key);
    const count = (retained.get(id) || 1) - 1;
    if (count) retained.set(id, count); else { retained.delete(id); released.add(id); ensureEvents(); }
  });
  function retain(value, id, key) { retained.set(id, (retained.get(id) || 0) + 1); released.delete(id); finalizer.register(value, { id, key }); }
  const errorTypes = new Map(Object.entries({ Error, TypeError, RangeError, ReferenceError, SyntaxError, EvalError, URIError }));
  function error(info) { const Type = errorTypes.get(info?.name) || Error; const result = new Type(info?.message || 'Node operation failed.'); result.name = info?.name || 'Error'; if (info?.code) result.code = info.code; return result; }
  function report(cause) { console.error('rdesktop Node renderer:', cause); window.dispatchEvent(new CustomEvent('rdesktop-node-error', { detail: { name: cause.name, message: cause.message } })); }
  function events(items) {
    for (const event of items || []) {
      if (event.kind === 'callback') { const callback = callbacks.get(event.id)?.fn; if (callback) queueMicrotask(() => { try { callback.apply(unpack(event.receiver), event.args.map(unpack)); } catch (cause) { report(cause); } }); }
      if (event.kind === 'promise') { const pending = promises.get(event.id); if (pending) { promises.delete(event.id); event.success ? pending.resolve(unpack(event.value)) : pending.reject(error(event.error)); } }
    }
  }
  function retireCallbacks(items = []) {
    for (const { id, epoch } of items) { if (callbacks.get(id)?.epoch === epoch) callbacks.delete(id); callbackAcks.set(id, epoch); }
    if (callbackAcks.size) ensureEvents();
  }
  function rpc(request) {
    if (closed) throw terminalError || new Error('The renderer Node lease is closed.');
    request = { ...request, rendererMemory: true };
    if (request.operation !== 'buffer' && callbackAcks.size) request.callbackAcks = [...callbackAcks].map(([id, epoch]) => ({ id, epoch }));
    if (!['buffer', 'collect-callbacks'].includes(request.operation) && callbacks.size) request.callbackClaims = [...callbacks].map(([id, entry]) => ({ id, epoch: entry.epoch }));
    if (memory && request.operation !== 'buffer') request.memory = memory.prepare(request);
    request.transportId = ++transportSequence;
    const end = Date.now() + 500;
    let replayedBuffer = false;
    // Reuse one completed request. A synchronous burst cannot yield to browser
    // cleanup between property reads; allocating an XHR for each read can exhaust
    // native request buffers. A reentrant call gets a separate active request.
    const xhr = idleRequest || new XMLHttpRequest(); idleRequest = null;
    try { for (;;) {
      xhr.open('POST', options.base + '__node', false);
      xhr.setRequestHeader('Content-Type', 'application/json'); xhr.setRequestHeader('X-Rdesktop-Node', options.token); xhr.setRequestHeader('X-Rdesktop-Document', documentIdentity);
      try { xhr.send(JSON.stringify(request)); }
      catch (cause) {
        // Buffer commands accept copied data only, mutate private copies, and
        // publish changes after a complete reply. Replaying a lost reply is safe.
        // Ordinary Node calls may have external effects and are never replayed.
        if (request.operation === 'buffer' && !replayedBuffer && cause.name === 'NetworkError') { replayedBuffer = true; continue; }
        throw cause;
      }
      if (xhr.status !== 200) throw new Error('Renderer Node transport rejected the request (' + xhr.status + ').');
      const result = JSON.parse(xhr.responseText);
      // Busy means no operation began; never retry a timeout or an unknown result.
      if (!result.success) { if (result.error?.code === 'RDESKTOP_NODE_BUSY' && Date.now() < end) continue; throw error(result.error); }
      memory?.accept(request.memory, result.data);
      events(result.data.events);
      for (const { id, epoch } of request.callbackAcks || []) if (callbackAcks.get(id) === epoch) callbackAcks.delete(id);
      retireCallbacks(result.data.callbackRetirements);
      if (result.data.thrown) throw error(result.data.thrown);
      return result.data;
    } } finally { if (!idleRequest && !closed) idleRequest = xhr; }
  }
  function pack(value, seen = new Set()) {
    if (value === undefined) return { $rdesktop: 'undefined' };
    if (typeof value === 'bigint') return { $rdesktop: 'bigint', value: String(value) };
    if (typeof value === 'number' && !Number.isFinite(value)) return { $rdesktop: 'number', value: String(value) };
    if (value === null || ['string', 'number', 'boolean'].includes(typeof value)) return value;
    if (references.has(value)) return { $rdesktop: 'reference', id: references.get(value) };
    if (typeof value === 'function') {
      let id = callbackIds.get(value);
      if (!id || !callbacks.has(id)) {
        if (callbacks.size >= 256) rpc({ operation: 'collect-callbacks' });
        if (callbacks.size >= 256) throw new RangeError('Renderer callback limit reached.');
        if (!id) { id = String(++sequence); callbackIds.set(value, id); }
        callbacks.set(id, { fn: value, epoch: 0 }); ensureEvents();
      }
      const entry = callbacks.get(id); entry.epoch = ++callbackEpoch;
      return { $rdesktop: 'callback', id, epoch: entry.epoch };
    }
    if (value instanceof ArrayBuffer || ArrayBuffer.isView(value)) return memory.tag(value);
    if (seen.has(value) || seen.size > 32) throw new TypeError('Cyclic or excessively nested renderer values are unsupported.');
    seen.add(value);
    try { return Array.isArray(value) ? value.map(item => pack(item, seen)) : { $rdesktop: 'object', entries: Object.entries(value).map(([key, item]) => [key, pack(item, seen)]) }; }
    finally { seen.delete(value); }
  }
  const bufferFacade = (__RDESKTOP_BUFFER_FACTORY__)({ rpc, pack, unpack, metadata: options.buffer });
  const RendererBuffer = bufferFacade.Buffer;
  memory = (__RDESKTOP_MEMORY_FACTORY__)(RendererBuffer);
  let nativeBufferModule;
  function unpack(value, owner) {
    if (Array.isArray(value)) return value.map(item => unpack(item));
    if (value === null || typeof value !== 'object') return value;
    if (value.$rdesktop === 'undefined') return undefined;
    if (value.$rdesktop === 'bigint') return BigInt(value.value);
    if (value.$rdesktop === 'number') return Number(value.value);
    if (value.$rdesktop === 'bytes') return bufferFacade.decode(value.base64);
    if (value.$rdesktop === 'memory-view' || value.$rdesktop === 'binary-view') return memory.unpack(value);
    if (value.$rdesktop === 'error') return error(value);
    if (value.$rdesktop === 'callback-reference') { const callback = callbacks.get(value.id)?.fn; if (!callback) throw new ReferenceError('Node returned a retired renderer callback.'); return callback; }
    if (value.$rdesktop !== 'reference') throw new TypeError('Invalid Node value tag.');
    if (value.kind === 'promise') {
      const old = proxies.get(value.id)?.deref(); if (old) return old;
      const promise = new Promise((resolve, reject) => promises.set(value.id, { resolve, reject })); proxies.set(value.id, new WeakRef(promise)); retain(promise, value.id, value.id); ensureEvents(); return promise;
    }
    const key = value.id + ':' + (value.receiver || '');
    const old = proxies.get(key)?.deref(); if (old) return old;
    const target = value.kind === 'function' ? function () {} : {};
    const proxy = new Proxy(target, {
      get(_target, key) {
        const own = Reflect.getOwnPropertyDescriptor(target, key); if (own && !own.configurable && !own.writable) return own.value;
        if (key === Symbol.toStringTag) return 'RdesktopNodeObject';
        if (typeof key === 'symbol') return undefined;
        return unpack(rpc({ operation: 'get', id: value.id, key }).value, proxy);
      },
      set(_target, key, item) { return rpc({ operation: 'set', id: value.id, key, value: pack(item) }).value; },
      has(_target, key) { return Reflect.has(target, key) || rpc({ operation: 'has', id: value.id, key }).value; },
      deleteProperty(_target, key) { const own = Reflect.getOwnPropertyDescriptor(target, key); if (own && !own.configurable) return false; return rpc({ operation: 'delete', id: value.id, key }).value; },
      ownKeys() { return [...new Set([...Reflect.ownKeys(target), ...rpc({ operation: 'keys', id: value.id }).value])]; },
      getOwnPropertyDescriptor(_target, key) { const own = Reflect.getOwnPropertyDescriptor(target, key); if (own && !own.configurable) return own; const descriptor = rpc({ operation: 'descriptor', id: value.id, key }).value; return !descriptor.exists ? undefined : { configurable: true, enumerable: descriptor.enumerable, ...(descriptor.accessor ? { get: unpack(descriptor.get), set: unpack(descriptor.set) } : { writable: descriptor.writable, value: unpack(descriptor.value, proxy) }) }; },
      apply(_target, thisArg, args) { void owner; return unpack(rpc({ operation: 'call', id: value.id, receiver: value.receiver || references.get(thisArg), args: args.map(arg => pack(arg)) }).value); },
      construct(_target, args) { return unpack(rpc({ operation: 'call', id: value.id, construct: true, args: args.map(arg => pack(arg)) }).value); },
    });
    references.set(proxy, value.id); proxies.set(key, new WeakRef(proxy)); retain(proxy, value.id, key); return proxy;
  }
  function load(resolved) {
    if (resolved.kind === 'native') return unpack(resolved.value);
    if (modules.has(resolved.filename)) return modules.get(resolved.filename).exports;
    const module = { id: resolved.filename, filename: resolved.filename, exports: {}, loaded: false };
    modules.set(resolved.filename, module);
    try {
      if (resolved.kind === 'json') module.exports = JSON.parse(resolved.source);
      else {
        const require = makeRequire(resolved.filename);
        const dirname = resolved.filename.slice(0, Math.max(resolved.filename.lastIndexOf('/'), resolved.filename.lastIndexOf('\\')));
        const source = resolved.source.replace(/^#![^\n]*\n/, '');
        // Keep Node globals in an outer lexical scope: adding wrapper arguments
        // would change CommonJS arguments.length and reject a local `const process`.
        const factory = new Function('process', 'Buffer', 'global', 'setImmediate', 'clearImmediate',
          'return function(require,module,exports,__filename,__dirname){\n' + source + '\n};\n//# sourceURL=' + resolved.filename.replace(/[\r\n]/g, ''));
        factory(nodeProcess, RendererBuffer, window, nodeSetImmediate, nodeClearImmediate)
          .call(module.exports, require, module, module.exports, resolved.filename, dirname);
      }
      module.loaded = true; return module.exports;
    } catch (cause) { modules.delete(resolved.filename); throw cause; }
  }
  function makeRequire(parent) {
    const require = name => {
      if (name === 'buffer' || name === 'node:buffer') {
        if (!nativeBufferModule) {
          const native = load(rpc({ operation: 'resolve', name, parent }));
          nativeBufferModule = new Proxy(native, { get(target, key) { return key === 'Buffer' ? RendererBuffer : key === 'SlowBuffer' ? bufferFacade.SlowBuffer : Reflect.get(target, key); } });
        }
        return nativeBufferModule;
      }
      return load(rpc({ operation: 'resolve', name, parent }));
    };
    require.resolve = name => { const result = rpc({ operation: 'resolve', name, parent }); return result.filename || name; };
    return require;
  }
  rpc({ operation: 'attach' });
  const require = makeRequire(options.preload);
  nodeProcess = require('node:process');
  const timers = require('node:timers');
  nodeSetImmediate = timers.setImmediate;
  nodeClearImmediate = timers.clearImmediate;
  if (options.exposeNodeGlobals !== false) Object.defineProperties(window, {
    require: { value: require, configurable: true, writable: true },
    global: { value: window, configurable: true, writable: true },
    Buffer: { value: RendererBuffer, configurable: true, writable: true },
    process: { value: nodeProcess, configurable: true, writable: true },
  });
  Object.defineProperties(window, {
    setImmediate: { value: nodeSetImmediate, configurable: true, writable: true },
    clearImmediate: { value: nodeClearImmediate, configurable: true, writable: true },
    __RDESKTOP_NODE_STATUS__: { value: Object.freeze({ node: options.node, renderer: 'native-webview', electron: false, nodeGlobals: options.exposeNodeGlobals === false ? 'preload-only' : 'page', preload: options.hasPreload ? 'package' : 'none' }), configurable: false },
  });
  // Native packages can keep their entire JS/stream/native-addon graph in Node
  // while the original preload retains its real document and DOM closures.
  // The host selects exact entries; these bindings grant full Node authority.
  function installWorkerBindings() {
    for (const binding of options.workerModules || []) {
      if (Object.hasOwn(window, binding.getter)) throw new TypeError('Worker module getter collides with an existing global: ' + binding.getter);
      let loaded = false, exports;
      Object.defineProperty(window, binding.getter, { value() {
        if (!loaded) { exports = require('node:module').createRequire(options.preload)(binding.module); loaded = true; }
        return exports;
      } });
    }
  }
  function ensureEvents() { if (!closed && !polling && timer === null) timer = startTimer(pollEvents, 20); }
  function pollEvents() {
    timer = null;
    if (closed || polling) return; polling = true;
    try {
      const ids = [...released].slice(0, 4096); for (const id of ids) released.delete(id);
      if (ids.length) rpc({ operation: 'release', ids });
      // Serialize event reads with calls in this document. An outstanding async
      // fetch/XHR can be starved by a run of synchronous CommonJS requests in a
      // native child. The existing bounded RPC applies memory before callbacks.
      if (callbacks.size || promises.size || callbackAcks.size) rpc({ operation: 'events' });
    } catch (cause) { if (!closed) { closed = true; terminalError = cause; for (const pending of promises.values()) pending.reject(cause); promises.clear(); report(cause); } }
    // Start the delay after the previous bounded call returns. A fixed interval
    // can already be overdue after a slow synchronous request and monopolize a
    // native child's event loop. Leave time for painting, input and CDP work.
    finally { polling = false; if (!closed && (callbacks.size || promises.size || released.size || callbackAcks.size)) ensureEvents(); }
  }
  window.addEventListener('pagehide', () => { closed = true; idleRequest = null; stopTimer(timer); timer = null; for (const pending of promises.values()) pending.reject(new Error('Renderer closed.')); promises.clear(); callbacks.clear(); callbackAcks.clear(); }, { once: true });
  try { installWorkerBindings(); if (options.hasPreload) load(rpc({ operation: 'preload' })); }
  catch (cause) { report(cause); throw cause; }
}
