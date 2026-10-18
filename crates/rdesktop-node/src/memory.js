function createRendererMemory(Buffer) {
  'use strict';
  const backings = new WeakMap(), views = new WeakMap(), targets = new Map();
  let sequence = 0;
  const cleanup = new FinalizationRegistry(id => targets.delete(id));
  const encode = bytes => { let text = ''; for (const byte of bytes) text += String.fromCharCode(byte); return btoa(text); };
  const decode = text => Uint8Array.from(atob(text), char => char.charCodeAt(0));
  function register(value, id) {
    if (targets.size >= 8192) {
      for (const [id, weak] of targets) if (!weak.deref()) targets.delete(id);
      if (targets.size >= 8192) throw new RangeError('Live renderer binary identities exceed their limit.');
    }
    targets.set(id, new WeakRef(value)); cleanup.register(value, id); return id;
  }
  const remember = (value, prefix) => register(value, prefix + (++sequence));
  function state(buffer) {
    if (!(buffer instanceof ArrayBuffer)) throw new TypeError('Shared binary memory is not supported by the Node transport.');
    let result = backings.get(buffer);
    if (!result) { result = { id: remember(buffer, 'b'), ranges: [] }; backings.set(buffer, result); }
    return result;
  }
  function range(buffer, row, start, length) {
    const end = start + length;
    const existing = row.ranges.find(item => item.start <= start && item.end >= end); if (existing) return existing;
    const overlapping = row.ranges.filter(item => item.start <= end && item.end >= start);
    const min = Math.min(start, ...overlapping.map(item => item.start)), max = Math.max(end, ...overlapping.map(item => item.end));
    const result = { start: min, end: max, bytes: new Uint8Array(max - min), known: new Uint8Array(max - min), epochs: new Float64Array(max - min) };
    for (const item of overlapping) { const offset = item.start - min; result.bytes.set(item.bytes, offset); result.known.set(item.known, offset); result.epochs.set(item.epochs, offset); }
    row.ranges = row.ranges.filter(item => !overlapping.includes(item)); row.ranges.push(result); return result;
  }
  function tag(value) {
    const array = value instanceof ArrayBuffer, buffer = array ? value : value.buffer, row = state(buffer);
    let id = views.get(value); if (!id) { id = remember(value, 'v'); views.set(value, id); }
    return { $rdesktop: 'memory-view', id, backing: row.id, offset: array ? 0 : value.byteOffset,
      length: array || value instanceof DataView ? value.byteLength : value.length,
      type: array ? 'ArrayBuffer' : Buffer.isBuffer(value) ? 'Buffer' : Object.prototype.toString.call(value).slice(8, -1),
      byteLength: value.byteLength };
  }
  function prepare(request) {
    const grouped = new Map();
    function visit(value) {
      if (!value || typeof value !== 'object') return;
      if (value.$rdesktop === 'memory-view') {
        const buffer = targets.get(value.backing)?.deref(); if (!buffer) throw new ReferenceError('Binary argument was released.');
        const row = state(buffer), old = grouped.get(row.id);
        if (old) { old.start = Math.min(old.start, value.offset); old.end = Math.max(old.end, value.offset + value.byteLength); }
        else grouped.set(row.id, { row, buffer, start: value.offset, end: value.offset + value.byteLength });
      } else for (const item of Object.values(value)) visit(item);
    }
    visit(request.args); visit(request.value);
    return [...grouped.values()].map(({ row, buffer, start, end }) => {
      const stored = range(buffer, row, start, end - start), bytes = new Uint8Array(buffer, start, end - start), writes = [];
      for (let index = 0; index < bytes.length;) {
        const at = index + start - stored.start;
        if (stored.known[at] && bytes[index] === stored.bytes[at]) { index++; continue; }
        const first = index++;
        while (index < bytes.length && (!stored.known[index + start - stored.start] || bytes[index] !== stored.bytes[index + start - stored.start])) index++;
        writes.push([first, index - first]);
      }
      return { id: row.id, byteLength: buffer.byteLength, offset: start, base64: encode(bytes), writes };
    });
  }
  function accept(sent, reply) {
    if (!reply?.memory) return;
    const epoch = reply.memory.sequence;
    // A synchronous call can finish ahead of an older in-flight event poll.
    // Per-byte epochs preserve disjoint writes while rejecting stale overlaps.
    for (const row of sent || []) {
      const buffer = targets.get(row.id)?.deref(); if (!buffer) continue;
      const stored = range(buffer, state(buffer), row.offset, decode(row.base64).length), bytes = decode(row.base64);
      for (const [offset, length] of row.writes) for (let index = offset; index < offset + length; index++) {
        const at = row.offset + index - stored.start;
        if (epoch >= stored.epochs[at]) { stored.bytes[at] = bytes[index]; stored.known[at] = 1; stored.epochs[at] = epoch; }
      }
    }
    for (const change of reply.memory.changes) {
      const buffer = targets.get(change.id)?.deref(); if (!buffer) continue;
      const bytes = decode(change.base64), stored = range(buffer, state(buffer), change.offset, bytes.length), current = new Uint8Array(buffer);
      for (let index = 0; index < bytes.length; index++) {
        const at = change.offset + index - stored.start;
        if (epoch < stored.epochs[at]) continue;
        // Preserve a browser write made while an asynchronous poll was pending;
        // the next explicit binary argument sends that local change to Node.
        if (!stored.known[at] || current[change.offset + index] === stored.bytes[at]) current[change.offset + index] = bytes[index];
        stored.bytes[at] = bytes[index]; stored.known[at] = 1; stored.epochs[at] = epoch;
      }
    }
  }
  function unpack(value) {
    if (value.$rdesktop === 'binary-view') {
      const bytes = decode(value.base64);
      return construct(bytes.buffer, { ...value, offset: 0 });
    }
    const original = targets.get(value.id)?.deref(); if (original) return original;
    let buffer = targets.get(value.backing)?.deref();
    if (!buffer) {
      buffer = new ArrayBuffer(value.storeLength); const row = { id: value.backing, ranges: [] };
      backings.set(buffer, row); targets.set(row.id, new WeakRef(buffer)); cleanup.register(buffer, row.id);
      new Uint8Array(buffer, value.offset, value.byteLength).set(decode(value.base64));
    }
    const stored = range(buffer, state(buffer), value.offset, value.byteLength), bytes = decode(value.base64), current = new Uint8Array(buffer);
    for (let index = 0; index < bytes.length; index++) {
      const at = value.offset + index - stored.start;
      if (!stored.known[at]) { current[value.offset + index] = bytes[index]; stored.bytes[at] = bytes[index]; stored.known[at] = 1; }
    }
    const result = construct(buffer, value);
    if (value.id) { register(result, value.id); views.set(result, value.id); }
    return result;
  }
  function construct(buffer, value) {
    if (value.type === 'ArrayBuffer') return buffer;
    if (value.type === 'Buffer') return Buffer.from(buffer, value.offset, value.length);
    const types = { Uint8Array, Uint8ClampedArray, Int8Array, Uint16Array, Int16Array, Uint32Array, Int32Array, Float32Array, Float64Array, BigInt64Array, BigUint64Array, DataView };
    const Type = types[value.type]; if (!Type) throw new TypeError('Invalid native binary type.');
    return new Type(buffer, value.offset, value.length);
  }
  return { tag, prepare, accept, unpack };
}
