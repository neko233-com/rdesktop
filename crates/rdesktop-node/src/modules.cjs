// This factory runs inside the explicitly trusted, pinned Node worker.
function createModuleRuntime(root, preload) {
  const fs = require('node:fs'), path = require('node:path'), Module = require('node:module');
  const references = new Map(), identities = new WeakMap(), callbacks = new Map();
  const events = [];
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
    if (Buffer.isBuffer(value) || value instanceof Uint8Array) return { $rdesktop: 'bytes', base64: Buffer.from(value).toString('base64') };
    if (value instanceof Error) return { $rdesktop: 'error', ...diagnostic(value) };
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
      case 'reference': return get(value.id);
      case 'callback': {
        if (typeof value.id !== 'string' || value.id.length > 64) throw new TypeError('Invalid renderer callback.');
        if (!callbacks.has(value.id)) {
          if (callbacks.size >= 256) throw new RangeError('Renderer callback limit reached.');
          callbacks.set(value.id, (...args) => { queue({ kind: 'callback', id: value.id, args: args.map(arg => pack(arg)) }); });
        }
        return callbacks.get(value.id);
      }
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
  return { request(message) {
    if (!message || typeof message.operation !== 'string') throw new TypeError('Invalid module operation.');
    let value;
    switch (message.operation) {
      case 'bootstrap': return { preload, root, node: process.versions.node, token: require('node:crypto').randomBytes(32).toString('hex') };
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
      default: throw new TypeError('Unknown module operation.');
    }
    if (overflow) throw new RangeError('Renderer event queue overflowed; close this runtime.');
    return { value, events: events.splice(0) };
  } };
}
