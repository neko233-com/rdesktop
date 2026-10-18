function createRdesktopBuffer(options) {
  'use strict';
  const { rpc, pack, unpack, metadata } = options;
  const encodeBase64 = bytes => { let text = ''; for (const byte of bytes) text += String.fromCharCode(byte); return btoa(text); };
  const decodeBase64 = text => Uint8Array.from(atob(text), char => char.charCodeAt(0));
  const wrap = view => Object.setPrototypeOf(view, Buffer.prototype);
  function Buffer(value, encodingOrOffset, length) {
    return typeof value === 'number' ? Buffer.allocUnsafe(value) : Buffer.from(value, encodingOrOffset, length);
  }
  Object.setPrototypeOf(Buffer, Uint8Array);
  Buffer.prototype = Object.create(Uint8Array.prototype, { constructor: { value: Buffer, writable: true, configurable: true } });
  Object.defineProperty(Buffer, 'poolSize', { value: metadata.poolSize, writable: true, enumerable: true, configurable: true });
  function relay(kind, name, receiver, args) {
    const backings = [], backingIds = new Map(), objects = [], objectIds = new Map(), active = new Set();
    function backing(value, offset, length) {
      if (!backingIds.has(value)) { backingIds.set(value, backings.length); backings.push({ buffer: value, min: offset, max: offset + length }); }
      const id = backingIds.get(value), row = backings[id]; row.min = Math.min(row.min, offset); row.max = Math.max(row.max, offset + length); return id;
    }
    const encodedViews = [];
    function encode(value, depth = 0) {
      if (depth > 32) throw new RangeError('Buffer argument nesting limit reached.');
      if (typeof value === 'function') throw new TypeError('Buffer commands accept copied data, not callbacks or Node functions.');
      if (typeof value === 'symbol') return { $buffer: 'symbol', description: value.description };
      if (value instanceof ArrayBuffer || ArrayBuffer.isView(value)) {
        if (!objectIds.has(value)) { objectIds.set(value, objects.length); objects.push(value); }
        const id = objectIds.get(value);
        if (value instanceof ArrayBuffer) return { $buffer: 'array-buffer', backing: backing(value, 0, value.byteLength), id };
        const view = { $buffer: 'view', backing: backing(value.buffer, value.byteOffset, value.byteLength), id, offset: value.byteOffset, length: value instanceof DataView ? value.byteLength : value.length, type: Buffer.isBuffer(value) ? 'Buffer' : Object.prototype.toString.call(value).slice(8, -1) };
        encodedViews.push(view); return view;
      }
      if (value && typeof value === 'object' && !Array.isArray(value)) {
        if (active.has(value)) throw new TypeError('Cyclic Buffer arguments are unsupported.');
        active.add(value);
        try { return { $buffer: 'object', nullPrototype: Object.getPrototypeOf(value) === null, entries: Object.entries(value).map(([key, item]) => [key, encode(item, depth + 1)]) }; }
        finally { active.delete(value); }
      }
      if (Array.isArray(value)) return value.map(item => encode(item, depth + 1));
      return pack(value);
    }
    const encodedReceiver = kind === 'method' ? encode(receiver) : undefined, encodedArgs = args.map(value => encode(value));
    for (const row of backings) row.start = Math.floor(row.min / 8) * 8;
    for (const view of encodedViews) view.offset -= backings[view.backing].start;
    const response = rpc({ operation: 'buffer', kind, name, receiver: encodedReceiver, args: encodedArgs, backings: backings.map(row => encodeBase64(new Uint8Array(row.buffer, row.start, row.max - row.start))) });
    for (const change of response.changes) { const row = backings[change.backing]; new Uint8Array(row.buffer, row.start, row.max - row.start).set(decodeBase64(change.base64)); }
    if (response.value?.$buffer === 'same') return objects[response.value.id];
    if (response.value?.$buffer === 'json') return response.value.value;
    if (response.value?.$buffer === 'result-view') { const row = backings[response.value.backing]; return wrap(new Uint8Array(row.buffer, row.start + response.value.offset, response.value.length)); }
    return unpack(response.value);
  }
  // Normal binary parsing stays in the document. Invalid arguments use the
  // selected Node version's actual validation, names and error codes.
  function numeric(name) {
    const match = /^(read|write)(BigUInt|BigUint|BigInt|UInt|Uint|Int|Float|Double)(8|16|32|64)?(LE|BE)?$/.exec(name);
    if (!match) return null;
    const write = match[1] === 'write', type = match[2], fixed = match[3] ? Number(match[3]) / 8 : type === 'Float' ? 4 : type === 'Double' ? 8 : null, little = match[4] !== 'BE';
    return function (...args) {
      const rawOffset = args[write ? 1 : 0], offset = fixed && rawOffset === undefined ? 0 : rawOffset, width = fixed || args[write ? 2 : 1], value = args[0];
      const validOffset = typeof offset === 'number' && Number.isInteger(offset) && offset >= 0;
      const validWidth = Number.isInteger(width) && width >= 1 && width <= (fixed || 6);
      const big = type.startsWith('Big'), signed = type === 'Int' || type === 'BigInt', float = type === 'Float' || type === 'Double';
      let validValue = true;
      if (write) {
        if (big) { const bits = BigInt(width * 8); validValue = typeof value === 'bigint' && value >= (signed ? -(1n << (bits - 1n)) : 0n) && value <= (signed ? (1n << (bits - 1n)) - 1n : (1n << bits) - 1n); }
        else validValue = typeof value === 'number' && (float || (value >= (signed ? -(2 ** (width * 8 - 1)) : 0) && value <= (signed ? 2 ** (width * 8 - 1) - 1 : 2 ** (width * 8) - 1)));
      }
      if (!(this instanceof Uint8Array) || !validOffset || !validWidth || offset + width > this.length || !validValue) return relay('method', name, this, args);
      const view = new DataView(this.buffer, this.byteOffset, this.byteLength);
      if (float) {
        const method = (write ? 'set' : 'get') + (width === 4 ? 'Float32' : 'Float64');
        if (write) { view[method](offset, value, little); return offset + width; }
        return view[method](offset, little);
      }
      if (big) {
        const method = (write ? 'setBig' : 'getBig') + (signed ? 'Int64' : 'Uint64');
        if (write) { view[method](offset, value, little); return offset + width; }
        return view[method](offset, little);
      }
      if (!write) {
        let result = 0; for (let index = 0; index < width; index++) result += this[offset + index] * 2 ** (8 * (little ? index : width - index - 1));
        return signed && result >= 2 ** (width * 8 - 1) ? result - 2 ** (width * 8) : result;
      }
      let result = Math.trunc(value); if (result < 0) result += 2 ** (width * 8);
      for (let index = 0; index < width; index++) this[offset + index] = Math.floor(result / 2 ** (8 * (little ? index : width - index - 1))) & 255;
      return offset + width;
    };
  }
  const methods = new Map();
  for (const entry of metadata.methods) {
    const implementation = entry.alias && methods.get(entry.alias) || numeric(entry.name) || function (...args) { return relay('method', entry.name, this, args); };
    methods.set(entry.name, implementation);
    Object.defineProperty(Buffer.prototype, entry.name, { value: implementation, writable: true, configurable: true, enumerable: entry.enumerable });
  }
  for (const entry of metadata.statics) Object.defineProperty(Buffer, entry.name, { value: function (...args) { return relay('static', entry.name, null, args); }, writable: true, configurable: true, enumerable: entry.enumerable });
  Buffer.isBuffer = value => value instanceof Buffer;
  const from = Buffer.from;
  const alloc = Buffer.alloc;
  Buffer.alloc = function(size, fill, encoding) {
    if (typeof size === 'number' && Number.isInteger(size) && size >= 0 && size < 2 ** 32 && (fill === undefined || typeof fill === 'number')) {
      const result = wrap(new Uint8Array(size)); if (fill !== undefined) Uint8Array.prototype.fill.call(result, fill); return result;
    }
    return alloc(size, fill, encoding);
  };
  Buffer.from = function (value, ...args) {
    if (value instanceof ArrayBuffer) {
      const offset = args[0] === undefined ? 0 : args[0], length = args[1] === undefined && typeof offset === 'number' ? value.byteLength - offset : args[1];
      if (Number.isInteger(offset) && offset >= 0 && Number.isInteger(length) && length >= 0 && offset + length <= value.byteLength) return wrap(new Uint8Array(value, offset, length));
    }
    if (ArrayBuffer.isView(value) && !(value instanceof DataView) && !(value instanceof BigInt64Array) && !(value instanceof BigUint64Array)) return wrap(new Uint8Array(value));
    if (value && typeof value === 'object' && !(value instanceof ArrayBuffer) && !ArrayBuffer.isView(value)) {
      const primitive = typeof value.valueOf === 'function' ? value.valueOf() : value;
      if (primitive != null && primitive !== value && (typeof primitive === 'string' || typeof primitive === 'object')) return Buffer.from(primitive, ...args);
      if (Array.isArray(value)) value = value.map(item => item & 255);
      else if (typeof value.length === 'number' && value.length >= 0 && value.length <= 3 * 1024 * 1024) value = Array.from({ length: value.length }, (_, index) => value[index] & 255);
      else if (value.length === undefined && value.type !== 'Buffer' && typeof value[Symbol.toPrimitive] === 'function') {
        const converted = value[Symbol.toPrimitive]('string'); if (typeof converted === 'string') return from(converted, ...args);
      }
    }
    if (Array.isArray(value)) return wrap(new Uint8Array(value));
    return from(value, ...args);
  };
  Buffer.prototype.subarray = function(start, end) { return Uint8Array.prototype.subarray.call(this, start, end); };
  Buffer.prototype.slice = function(start, end) { return Buffer.prototype.subarray.call(this, start, end); };
  Buffer.prototype.toJSON = function () { return { type: 'Buffer', data: Array.from(this) }; };
  Object.defineProperties(Buffer.prototype, {
    parent: { get() { return Buffer.isBuffer(this) ? this.buffer : undefined; }, configurable: true, enumerable: true },
    offset: { get() { return Buffer.isBuffer(this) ? this.byteOffset : undefined; }, configurable: true, enumerable: true },
    [Symbol.for('nodejs.util.inspect.custom')]: { value: Buffer.prototype.inspect, writable: true, configurable: true },
  });
  for (const [target, entries] of [[Buffer, metadata.statics], [Buffer.prototype, metadata.methods]]) for (const entry of entries) {
    Object.defineProperties(target[entry.name], { name: { value: entry.functionName, configurable: true }, length: { value: entry.arity, configurable: true } });
  }
  return { Buffer, decode: base64 => wrap(decodeBase64(base64)), SlowBuffer: function SlowBuffer(...args) { return relay('module', 'SlowBuffer', null, args); } };
}
