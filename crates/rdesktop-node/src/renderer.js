function rdesktopNodeRenderer(options) {
  'use strict';
  if (window.top !== window || !location.href.startsWith(options.base)) return;
  const documentIdentity = Array.from(crypto.getRandomValues(new Uint8Array(16)), byte => byte.toString(16).padStart(2, '0')).join('');
  const references = new WeakMap(), proxies = new Map(), callbacks = new Map(), callbackIds = new WeakMap();
  const promises = new Map(), released = new Set(), retained = new Map(), modules = new Map();
  let sequence = 0, closed = false, polling = false, terminalError = null;
  const finalizer = new FinalizationRegistry(({ id, key }) => {
    if (!proxies.get(key)?.deref()) proxies.delete(key);
    const count = (retained.get(id) || 1) - 1;
    if (count) retained.set(id, count); else { retained.delete(id); released.add(id); }
  });
  function retain(value, id, key) { retained.set(id, (retained.get(id) || 0) + 1); released.delete(id); finalizer.register(value, { id, key }); }
  function error(info) { const result = new Error(info?.message || 'Node operation failed.'); result.name = info?.name || 'Error'; if (info?.code) result.code = info.code; return result; }
  function report(cause) { console.error('rdesktop Node renderer:', cause); window.dispatchEvent(new CustomEvent('rdesktop-node-error', { detail: { name: cause.name, message: cause.message } })); }
  function events(items) {
    for (const event of items || []) {
      if (event.kind === 'callback') { const callback = callbacks.get(event.id); if (callback) queueMicrotask(() => { try { callback(...event.args.map(unpack)); } catch (cause) { report(cause); } }); }
      if (event.kind === 'promise') { const pending = promises.get(event.id); if (pending) { promises.delete(event.id); event.success ? pending.resolve(unpack(event.value)) : pending.reject(error(event.error)); } }
    }
  }
  function rpc(request) {
    if (closed) throw terminalError || new Error('The renderer Node lease is closed.');
    const end = Date.now() + 500;
    for (;;) {
      const xhr = new XMLHttpRequest(); xhr.open('POST', options.base + '__node', false);
      xhr.setRequestHeader('Content-Type', 'application/json'); xhr.setRequestHeader('X-Rdesktop-Node', options.token); xhr.setRequestHeader('X-Rdesktop-Document', documentIdentity);
      xhr.send(JSON.stringify(request));
      if (xhr.status !== 200) throw new Error('Renderer Node transport rejected the request (' + xhr.status + ').');
      const result = JSON.parse(xhr.responseText);
      // Busy means no operation began; never retry a timeout or an unknown result.
      if (!result.success) { if (result.error?.code === 'RDESKTOP_NODE_BUSY' && Date.now() < end) continue; throw error(result.error); }
      events(result.data.events); return result.data;
    }
  }
  function pack(value, seen = new Set()) {
    if (value === undefined) return { $rdesktop: 'undefined' };
    if (typeof value === 'bigint') return { $rdesktop: 'bigint', value: String(value) };
    if (typeof value === 'number' && !Number.isFinite(value)) return { $rdesktop: 'number', value: String(value) };
    if (value === null || ['string', 'number', 'boolean'].includes(typeof value)) return value;
    if (references.has(value)) return { $rdesktop: 'reference', id: references.get(value) };
    if (typeof value === 'function') {
      let id = callbackIds.get(value);
      if (!id) { if (callbacks.size >= 256) throw new RangeError('Renderer callback limit reached.'); id = String(++sequence); callbackIds.set(value, id); callbacks.set(id, value); }
      return { $rdesktop: 'callback', id };
    }
    if (value instanceof ArrayBuffer) value = new Uint8Array(value);
    if (ArrayBuffer.isView(value)) { let binary = ''; for (const byte of new Uint8Array(value.buffer, value.byteOffset, value.byteLength)) binary += String.fromCharCode(byte); return { $rdesktop: 'bytes', base64: btoa(binary) }; }
    if (seen.has(value) || seen.size > 32) throw new TypeError('Cyclic or excessively nested renderer values are unsupported.');
    seen.add(value);
    try { return Array.isArray(value) ? value.map(item => pack(item, seen)) : { $rdesktop: 'object', entries: Object.entries(value).map(([key, item]) => [key, pack(item, seen)]) }; }
    finally { seen.delete(value); }
  }
  class RendererBuffer extends Uint8Array {
    static from(value, encoding = 'utf8') {
      if (typeof value !== 'string') { if (typeof value === 'number' || value == null) throw new TypeError('Invalid Buffer input.'); return new RendererBuffer(value); }
      encoding = encoding.toLowerCase().replace('-', '');
      if (encoding === 'hex') { const bytes = []; for (let index = 0; index + 1 < value.length; index += 2) { const pair = value.slice(index, index + 2); if (!/^[0-9a-f]{2}$/i.test(pair)) break; bytes.push(parseInt(pair, 16)); } return new RendererBuffer(bytes); }
      if (encoding === 'base64' || encoding === 'base64url') return new RendererBuffer(Uint8Array.from(atob(value.replaceAll('-', '+').replaceAll('_', '/').replace(/\s/g, '')), char => char.charCodeAt(0)));
      if (encoding === 'utf8') return new RendererBuffer(new TextEncoder().encode(value));
      if (encoding === 'utf16le' || encoding === 'ucs2') { const result = new RendererBuffer(value.length * 2); for (let index = 0; index < value.length; index++) { result[index * 2] = value.charCodeAt(index) & 255; result[index * 2 + 1] = value.charCodeAt(index) >>> 8; } return result; }
      if (['ascii', 'latin1', 'binary'].includes(encoding)) return new RendererBuffer(Array.from(value, char => char.charCodeAt(0) & 255));
      throw new TypeError('Unsupported Buffer encoding: ' + encoding);
    }
    static alloc(length, fill = 0, encoding) { const result = new RendererBuffer(length); const bytes = typeof fill === 'string' ? RendererBuffer.from(fill, encoding) : typeof fill === 'number' ? [fill] : fill; if (bytes?.length) for (let i = 0; i < length; i++) result[i] = bytes[i % bytes.length]; return result; }
    static allocUnsafe(length) { return RendererBuffer.alloc(length); }
    static isBuffer(value) { return value instanceof RendererBuffer; }
    static byteLength(value, encoding) { return typeof value === 'string' ? this.from(value, encoding).length : value.byteLength; }
    static concat(values, length = values.reduce((total, value) => total + value.length, 0)) { const result = new RendererBuffer(length); let offset = 0; for (const value of values) { const part = value.subarray(0, Math.max(0, length - offset)); result.set(part, offset); offset += part.length; if (offset >= length) break; } return result; }
    toString(encoding = 'utf8', start = 0, end = this.length) {
      const bytes = this.subarray(Math.max(0, start), Math.min(this.length, end)); encoding = encoding.toLowerCase().replace('-', '');
      if (encoding === 'utf8') return new TextDecoder().decode(bytes);
      if (encoding === 'utf16le' || encoding === 'ucs2') return new TextDecoder('utf-16le').decode(bytes.subarray(0, bytes.length - bytes.length % 2));
      if (encoding === 'hex') return Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
      let binary = ''; for (const byte of bytes) binary += String.fromCharCode(encoding === 'ascii' ? byte & 127 : byte);
      if (encoding === 'base64' || encoding === 'base64url') { const result = btoa(binary); return encoding === 'base64url' ? result.replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '') : result; }
      if (['ascii', 'latin1', 'binary'].includes(encoding)) return binary;
      throw new TypeError('Unsupported Buffer encoding: ' + encoding);
    }
    slice(start, end) { return this.subarray(start, end); }
    toJSON() { return { type: 'Buffer', data: Array.from(this) }; }
    equals(other) { return this.length === other.length && this.every((byte, index) => byte === other[index]); }
  }
  function unpack(value, owner) {
    if (Array.isArray(value)) return value.map(item => unpack(item));
    if (value === null || typeof value !== 'object') return value;
    if (value.$rdesktop === 'undefined') return undefined;
    if (value.$rdesktop === 'bigint') return BigInt(value.value);
    if (value.$rdesktop === 'number') return Number(value.value);
    if (value.$rdesktop === 'bytes') return RendererBuffer.from(value.base64, 'base64');
    if (value.$rdesktop === 'error') return error(value);
    if (value.$rdesktop !== 'reference') throw new TypeError('Invalid Node value tag.');
    if (value.kind === 'promise') {
      const old = proxies.get(value.id)?.deref(); if (old) return old;
      const promise = new Promise((resolve, reject) => promises.set(value.id, { resolve, reject })); proxies.set(value.id, new WeakRef(promise)); retain(promise, value.id, value.id); return promise;
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
      getOwnPropertyDescriptor(_target, key) { const own = Reflect.getOwnPropertyDescriptor(target, key); if (own && !own.configurable) return own; const descriptor = rpc({ operation: 'descriptor', id: value.id, key }).value; return descriptor.exists ? { configurable: true, enumerable: descriptor.enumerable, writable: descriptor.writable, value: unpack(descriptor.value, proxy) } : undefined; },
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
        new Function('require', 'module', 'exports', '__filename', '__dirname', source + '\n//# sourceURL=' + resolved.filename.replace(/[\r\n]/g, '')).call(module.exports, require, module, module.exports, resolved.filename, dirname);
      }
      module.loaded = true; return module.exports;
    } catch (cause) { modules.delete(resolved.filename); throw cause; }
  }
  function makeRequire(parent) {
    const require = name => load(rpc({ operation: 'resolve', name, parent }));
    require.resolve = name => { const result = rpc({ operation: 'resolve', name, parent }); return result.filename || name; };
    return require;
  }
  rpc({ operation: 'attach' });
  const require = makeRequire(options.preload);
  Object.defineProperties(window, {
    require: { value: require, configurable: true },
    global: { value: window, configurable: true },
    Buffer: { value: RendererBuffer, configurable: true },
    process: { value: require('node:process'), configurable: true },
    __RDESKTOP_NODE_STATUS__: { value: Object.freeze({ node: options.node, renderer: 'native-webview', electron: false }), configurable: false },
  });
  const timer = setInterval(async () => {
    if (closed || polling) return; polling = true;
    try {
      const ids = [...released].slice(0, 4096); for (const id of ids) released.delete(id);
      if (ids.length) rpc({ operation: 'release', ids });
      const response = await fetch(options.base + '__node', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Rdesktop-Node': options.token, 'X-Rdesktop-Document': documentIdentity }, body: JSON.stringify({ operation: 'events' }) });
      if (!response.ok) throw new Error('Node event transport failed.');
      const result = await response.json(); if (!result.success) { if (result.error?.code !== 'RDESKTOP_NODE_BUSY') throw error(result.error); }
      else events(result.data.events);
    } catch (cause) { if (!closed) { closed = true; terminalError = cause; clearInterval(timer); for (const pending of promises.values()) pending.reject(cause); promises.clear(); report(cause); } }
    finally { polling = false; }
  }, 20);
  window.addEventListener('pagehide', () => { closed = true; clearInterval(timer); for (const pending of promises.values()) pending.reject(new Error('Renderer closed.')); promises.clear(); callbacks.clear(); }, { once: true });
  try { load(rpc({ operation: 'preload' })); }
  catch (cause) { report(cause); throw cause; }
}
