// This factory runs inside the explicitly trusted, pinned Node worker.
function createModuleRuntime(root, preload) {
  const fs = require('node:fs'), path = require('node:path'), Module = require('node:module');
  const nativeTypes = require('node:util').types;
  const references = new Map(), identities = new WeakMap();
  const events = [];
  const memory = createNodeMemory();
  const callbacks = createNodeCallbacks(pack, queue, rdesktopCollectGarbage);
  let sequence = 0, overflow = false;
  const maxReferences = 4096, maxEvents = 256;
  function queue(event) {
    if (events.length >= maxEvents) { overflow = true; return; }
    events.push(event);
  }
  function reference(value, receiver) {
    let id = identities.get(value);
    if (!id || !references.has(id)) {
      if (references.size >= maxReferences) throw new RangeError('Node reference limit reached. Release unused objects.');
      id = String(++sequence); identities.set(value, id); references.set(id, value);
    }
    if (value instanceof Promise && !references.has('promise:' + id)) {
      references.set('promise:' + id, true);
      value.then(result => queue({ kind: 'promise', id, success: true, value: pack(result) }),
        error => queue({ kind: 'promise', id, success: false, error: diagnostic(error) })).catch(error => { overflow = true; });
    }
    return { $rdesktop: 'reference', id, kind: value instanceof Promise ? 'promise' : typeof value, ...(receiver ? { receiver } : {}) };
  }
  function pack(value, depth = 0, seen = new Set(), receiver) {
    if (depth > 32) throw new RangeError('Node value nesting limit reached.');
    if (value === undefined) return { $rdesktop: 'undefined' };
    if (typeof value === 'bigint') return { $rdesktop: 'bigint', value: String(value) };
    if (typeof value === 'number' && !Number.isFinite(value)) return { $rdesktop: 'number', value: String(value) };
    if (value === null || ['string', 'number', 'boolean'].includes(typeof value)) return value;
    if (nativeTypes.isArrayBuffer(value) || ArrayBuffer.isView(value)) {
      const mapped = memory.pack(value); if (mapped) return mapped;
      if (Buffer.isBuffer(value)) return { $rdesktop: 'bytes', base64: value.toString('base64') };
      const array = nativeTypes.isArrayBuffer(value), bytes = array ? Buffer.from(value) : Buffer.from(value.buffer, value.byteOffset, value.byteLength);
      const type = array ? 'ArrayBuffer' : Object.prototype.toString.call(value).slice(8, -1);
      return { $rdesktop: 'binary-view', type, base64: bytes.toString('base64'), length: array || type === 'DataView' ? value.byteLength : value.length };
    }
    if (value instanceof Error) return { $rdesktop: 'error', ...diagnostic(value) };
    if (typeof value === 'function') { const callback = callbacks.tag(value); if (callback) return callback; }
    if (Array.isArray(value)) {
      if (seen.has(value)) return reference(value, receiver);
      seen.add(value); try { return value.map(item => pack(item, depth + 1, seen)); } finally { seen.delete(value); }
    }
    if (typeof value === 'object' || typeof value === 'function') return reference(value, receiver);
    throw new TypeError('Unsupported Node value type.');
  }
  function get(id) { if (!references.has(id)) throw new ReferenceError('Node object has been released.'); return references.get(id); }
  function diagnostic(error) { return { name: error?.name || 'Error', message: String(error?.message || error), ...(typeof error?.code === 'string' ? { code: error.code } : {}) }; }
  function unpack(value, depth = 0) {
    if (depth > 32) throw new RangeError('Renderer value nesting limit reached.');
    if (Array.isArray(value)) return value.map(item => unpack(item, depth + 1));
    if (value === null || typeof value !== 'object') return value;
    switch (value.$rdesktop) {
      case 'undefined': return undefined;
      case 'bigint': return BigInt(value.value);
      case 'number': if (!['NaN', 'Infinity', '-Infinity'].includes(value.value)) throw new TypeError('Invalid number tag.'); return Number(value.value);
      case 'bytes': return Buffer.from(value.base64, 'base64');
      case 'memory-view': return memory.unpack(value);
      case 'reference': return get(value.id);
      case 'callback': return callbacks.unpack(value);
      case 'object': {
        if (!Array.isArray(value.entries) || value.entries.length > 4096) throw new TypeError('Invalid renderer object.');
        const result = Object.create(null);
        for (const [key, item] of value.entries) {
          if (typeof key !== 'string') throw new TypeError('Invalid object key.');
          Object.defineProperty(result, key, { value: unpack(item, depth + 1), writable: true, enumerable: true, configurable: true });
        }
        return result;
      }
      default: throw new TypeError('Renderer object must use an explicit protocol tag.');
    }
  }
  function inside(filename) {
    const resolved = fs.realpathSync(filename), relative = path.relative(root, resolved);
    if (!relative || relative === '..' || relative.startsWith('..' + path.sep) || path.isAbsolute(relative)) throw new Error('CommonJS source is outside its package root.');
    return resolved;
  }
  function resolve(name, parent) {
    if (typeof name !== 'string' || name.length > 1024 || typeof parent !== 'string') throw new TypeError('Invalid CommonJS request.');
    const local = Module.createRequire(inside(parent));
    if (Module.isBuiltin(name)) return { kind: 'native', value: reference(local(name)) };
    const filename = inside(local.resolve(name));
    if (path.extname(filename) === '.node') return { kind: 'native', value: reference(local(filename)) };
    if (!['.js', '.cjs', '.json', ''].includes(path.extname(filename))) throw new TypeError('This renderer loader currently supports CommonJS/JSON/native modules only.');
    if (fs.statSync(filename).size > 1024 * 1024) throw new RangeError('CommonJS source exceeds 1 MiB.');
    return { kind: path.extname(filename) === '.json' ? 'json' : 'source', filename, source: fs.readFileSync(filename, 'utf8') };
  }
  function bufferCatalog(target) {
    const functions = new Map();
    return Object.getOwnPropertyNames(target).flatMap(name => {
      const descriptor = Object.getOwnPropertyDescriptor(target, name);
      if (name === 'constructor' || typeof descriptor.value !== 'function') return [];
      const alias = functions.get(descriptor.value); functions.set(descriptor.value, alias || name);
      return [{ name, functionName: descriptor.value.name, arity: descriptor.value.length, enumerable: descriptor.enumerable, ...(alias ? { alias } : {}) }];
    });
  }
  const bufferApi = { statics: bufferCatalog(Buffer), methods: bufferCatalog(Buffer.prototype), poolSize: Buffer.poolSize };
  const bufferFunctions = new Map(['static', 'method'].flatMap(kind => {
    const target = kind === 'static' ? Buffer : Buffer.prototype;
    return Object.getOwnPropertyNames(target).flatMap(name => {
      const value = Object.getOwnPropertyDescriptor(target, name).value;
      return name !== 'constructor' && typeof value === 'function' ? [[kind + ':' + name, value]] : [];
    });
  }));
  if (typeof require('node:buffer').SlowBuffer === 'function') bufferFunctions.set('module:SlowBuffer', require('node:buffer').SlowBuffer);
  function bufferOperation(message) {
    const operation = bufferFunctions.get(message.kind + ':' + message.name);
    if (!operation
      || !Array.isArray(message.backings) || message.backings.length > 4096
      || !Array.isArray(message.args) || message.args.length > 256) throw new TypeError('Invalid Buffer operation.');
    const buffers = message.backings.map(base64 => {
      if (typeof base64 !== 'string') throw new TypeError('Invalid Buffer backing store.');
      const bytes = Buffer.from(base64, 'base64'), store = Buffer.from(new ArrayBuffer(bytes.length));
      store.set(bytes); return store;
    });
    const originals = buffers.map(value => Buffer.from(value)), objects = new Map(), identities = new WeakMap();
    const types = { Uint8Array, Uint8ClampedArray, Int8Array, Uint16Array, Int16Array, Uint32Array, Int32Array, Float32Array, Float64Array, BigInt64Array, BigUint64Array, DataView };
    function decode(value, depth = 0) {
      if (depth > 32) throw new RangeError('Buffer argument nesting limit reached.');
      if (Array.isArray(value)) return value.map(item => decode(item, depth + 1));
      if (!value || typeof value !== 'object') return value;
      if (value.$buffer === 'view' || value.$buffer === 'array-buffer') {
        const source = buffers[value.backing]; if (!source) throw new RangeError('Unknown Buffer backing store.');
        let result;
        if (value.$buffer === 'array-buffer') result = source.buffer;
        else if (value.type === 'Buffer') result = Buffer.from(source.buffer, source.byteOffset + value.offset, value.length);
        else { const Type = types[value.type]; if (!Type) throw new TypeError('Unsupported Buffer argument view.'); result = new Type(source.buffer, source.byteOffset + value.offset, value.length); }
        if (objects.has(value.id)) return objects.get(value.id);
        objects.set(value.id, result); identities.set(result, value.id); return result;
      }
      if (value.$buffer === 'object') {
        const result = value.nullPrototype ? Object.create(null) : {};
        if (!Array.isArray(value.entries) || value.entries.length > 4096) throw new RangeError('Buffer object exceeds its limit.');
        for (const [key, item] of value.entries) Object.defineProperty(result, key, { value: decode(item, depth + 1), enumerable: true });
        return result;
      }
      if (value.$buffer === 'symbol') return Symbol(value.description);
      if (!['undefined', 'bigint', 'number'].includes(value.$rdesktop)) throw new TypeError('Buffer commands accept copied data only.');
      return unpack(value);
    }
    const receiver = message.kind === 'method' ? decode(message.receiver) : Buffer;
    const args = message.args.map(value => decode(value));
    const result = Reflect.apply(operation, receiver, args);
    let value;
    if (result && typeof result === 'object' && identities.has(result)) value = { $buffer: 'same', id: identities.get(result) };
    else if (Buffer.isBuffer(result)) {
      const backing = buffers.findIndex(store => store.buffer === result.buffer && result.byteOffset >= store.byteOffset && result.byteOffset + result.length <= store.byteOffset + store.length);
      value = backing < 0 ? pack(result) : { $buffer: 'result-view', backing, offset: result.byteOffset - buffers[backing].byteOffset, length: result.length };
    } else if (result && typeof result === 'object') value = { $buffer: 'json', value: JSON.parse(JSON.stringify(result)) };
    else value = pack(result);
    return { value, changes: buffers.flatMap((store, backing) => store.equals(originals[backing]) ? [] : [{ backing, base64: store.toString('base64') }]) };
  }
  function execute(message) {
    if (!message || typeof message.operation !== 'string') throw new TypeError('Invalid module operation.');
    let value;
    switch (message.operation) {
      case 'bootstrap': return { preload, root, node: process.versions.node, buffer: bufferApi, token: require('node:crypto').randomBytes(32).toString('hex') };
      // Copy-only commands are replayable after a lost HTTP reply. Leave async
      // events for the normal event transport instead of consuming them here.
      case 'buffer': {
        if (overflow) throw new RangeError('Renderer event queue overflowed; close this runtime.');
        return { ...bufferOperation(message), events: [] };
      }
      case 'resolve': return resolve(message.name, message.parent);
      case 'preload': return resolve('./' + path.basename(preload), preload);
      case 'get': value = pack(Reflect.get(get(message.id), message.key), 0, new Set(), message.id); break;
      case 'keys': value = Reflect.ownKeys(get(message.id)).filter(key => typeof key === 'string'); break;
      case 'has': value = Reflect.has(get(message.id), message.key); break;
      case 'delete': value = Reflect.deleteProperty(get(message.id), message.key); break;
      case 'descriptor': {
        const descriptor = Reflect.getOwnPropertyDescriptor(get(message.id), message.key);
        value = descriptor ? { exists: true, enumerable: !!descriptor.enumerable, writable: !!descriptor.writable, value: pack(Reflect.get(get(message.id), message.key), 0, new Set(), message.id) } : { exists: false }; break;
      }
      case 'set': value = Reflect.set(get(message.id), message.key, unpack(message.value)); break;
      case 'call': {
        if (!Array.isArray(message.args) || message.args.length > 256) throw new TypeError('Invalid Node call arguments.');
        const fn = get(message.id), args = message.args.map(arg => unpack(arg));
        value = pack(message.construct ? Reflect.construct(fn, args) : Reflect.apply(fn, message.receiver ? get(message.receiver) : undefined, args)); break;
      }
      case 'release': {
        if (!Array.isArray(message.ids) || message.ids.length > 4096) throw new TypeError('Invalid reference release.');
        for (const id of message.ids) { references.delete(id); references.delete('promise:' + id); } value = null; break;
      }
      case 'events': value = null; break;
      case 'collect-callbacks': callbacks.collect(true); value = null; break;
      default: throw new TypeError('Unknown module operation.');
    }
    if (overflow) throw new RangeError('Renderer event queue overflowed; close this runtime.');
    return { value, events: events.splice(0) };
  }
  return { request(message) {
    // Copy-only retries must neither consume events nor change callback leases.
    if (message?.operation === 'buffer') return execute(message);
    callbacks.acknowledge(message?.callbackAcks);
    const reconcile = callbacks.claims(message?.callbackClaims);
    callbacks.collect();
    if (!['events', 'collect-callbacks'].includes(message?.operation)) callbacks.touch();
    if (!message?.rendererMemory) { const result = execute(message); reconcile(); return { ...result, callbackRetirements: callbacks.retirements() }; }
    // Keep input views alive throughout the operation, including typed errors.
    // Native objects and pending callbacks retain their own views afterwards;
    // weak backing registries do not turn every call into a permanent copy.
    let retained, result;
    try { retained = memory.load(message.memory); result = execute(message); }
    catch (error) { result = { thrown: diagnostic(error), events: events.splice(0) }; }
    reconcile();
    try {
      const changes = memory.collect(false), response = { ...result, memory: changes, callbackRetirements: callbacks.retirements() }; void retained;
      // The worker adds its own envelope. Do not acknowledge changed bytes before
      // knowing that this entire reply fits its frame, including returned views.
      if (Buffer.byteLength(JSON.stringify(response)) > 4 * 1024 * 1024 - 1024) throw new RangeError('Node memory response exceeds its frame limit.');
      changes.commit(); return response;
    } catch (error) { events.unshift(...(result.events || [])); throw error; }
  } };
}
