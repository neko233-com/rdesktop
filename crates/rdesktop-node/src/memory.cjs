function createNodeMemory() {
  const backings = new Map(), views = new Map(), identities = new WeakMap(), owners = new WeakMap();
  let sequence = 0, viewSequence = 0;
  const types = { Uint8Array, Uint8ClampedArray, Int8Array, Uint16Array, Int16Array, Uint32Array, Int32Array, Float32Array, Float64Array, BigInt64Array, BigUint64Array, DataView };
  function bounded(value, max) { return Number.isSafeInteger(value) && value >= 0 && value <= max; }
  function id(value) { if (typeof value !== 'string' || !/^[bvn][1-9][0-9]{0,15}$/.test(value)) throw new TypeError('Invalid binary identity.'); return value; }
  function liveBytes() {
    let bytes = 0;
    for (const [key, weak] of backings) { const row = weak.deref(); if (row) bytes += row.data.length; else backings.delete(key); }
    for (const [key, weak] of views) if (!weak.deref()) views.delete(key);
    return bytes;
  }
  function range(row, start, length) {
    const existing = row.ranges.find(item => item.start <= start && item.end >= start + length); if (existing) return existing;
    const end = start + length, overlapping = row.ranges.filter(item => item.start <= end && item.end >= start);
    const min = Math.min(start, ...overlapping.map(item => item.start)), max = Math.max(end, ...overlapping.map(item => item.end));
    const result = { start: min, end: max, bytes: Buffer.from(row.data.subarray(min, max)) };
    for (const item of overlapping) result.bytes.set(item.bytes, item.start - min);
    row.ranges = row.ranges.filter(item => !overlapping.includes(item)); row.ranges.push(result); return result;
  }
  function load(rows = []) {
    if (!Array.isArray(rows) || rows.length > 4096) throw new RangeError('Invalid binary backing list.');
    const retained = [];
    for (const input of rows) {
      id(input.id);
      if (!bounded(input.byteLength, 64 * 1024 * 1024) || !bounded(input.offset, input.byteLength)
        || typeof input.base64 !== 'string' || input.base64.length > 4 * 1024 * 1024
        || !Array.isArray(input.writes) || input.writes.length > 65536) throw new TypeError('Invalid binary backing.');
      const bytes = Buffer.from(input.base64, 'base64');
      if (input.offset + bytes.length > input.byteLength) throw new RangeError('Binary view exceeds its backing.');
      let row = backings.get(input.id)?.deref();
      if (!row) {
        if (liveBytes() + input.byteLength > 64 * 1024 * 1024 || backings.size >= 4096) throw new RangeError('Live binary memory exceeds its limit.');
        row = { id: input.id, data: Buffer.from(new ArrayBuffer(input.byteLength)), ranges: [] };
        owners.set(row.data.buffer, row); backings.set(row.id, new WeakRef(row));
        row.data.set(bytes, input.offset);
      } else if (row.data.length !== input.byteLength) throw new TypeError('Binary backing identity changed size.');
      range(row, input.offset, bytes.length);
      for (const pair of input.writes) {
        if (!Array.isArray(pair) || pair.length !== 2 || !bounded(pair[0], bytes.length) || !bounded(pair[1], bytes.length - pair[0])) throw new RangeError('Invalid binary write range.');
        row.data.set(bytes.subarray(pair[0], pair[0] + pair[1]), input.offset + pair[0]);
      }
      retained.push(row);
    }
    return retained;
  }
  function unpack(value) {
    const row = backings.get(id(value.backing))?.deref(); if (!row) throw new ReferenceError('Unknown binary backing.');
    id(value.id);
    if (!bounded(value.offset, row.data.length) || !bounded(value.byteLength, row.data.length - value.offset)
      || !bounded(value.length, row.data.length)) throw new RangeError('Invalid binary view.');
    const old = views.get(value.id)?.deref();
    if (old) {
      const info = identities.get(old);
      if (info.backing !== value.backing || info.offset !== value.offset || info.type !== value.type || info.length !== value.length) throw new TypeError('Binary view identity changed.');
      return old;
    }
    if (views.size >= 4096) { liveBytes(); if (views.size >= 4096) throw new RangeError('Live binary views exceed their limit.'); }
    let result;
    if (value.type === 'ArrayBuffer') {
      if (value.offset !== 0 || value.byteLength !== row.data.length) throw new TypeError('Invalid ArrayBuffer view.');
      result = row.data.buffer;
    } else if (value.type === 'Buffer') result = Buffer.from(row.data.buffer, value.offset, value.length);
    else { const Type = types[value.type]; if (!Type) throw new TypeError('Invalid binary view type.'); result = new Type(row.data.buffer, value.offset, value.length); }
    if (result.byteLength !== value.byteLength) throw new TypeError('Binary view byte length mismatch.');
    identities.set(result, { ...value, owner: row }); views.set(value.id, new WeakRef(result)); return result;
  }
  function pack(value) {
    const array = value instanceof ArrayBuffer, buffer = array ? value : value.buffer, row = owners.get(buffer);
    if (!row) return null;
    let info = identities.get(value);
    const offset = array ? 0 : value.byteOffset, byteLength = value.byteLength;
    const type = array ? 'ArrayBuffer' : Buffer.isBuffer(value) ? 'Buffer' : Object.prototype.toString.call(value).slice(8, -1);
    if (!info) {
      if (views.size >= 4096) { liveBytes(); if (views.size >= 4096) throw new RangeError('Live binary views exceed their limit.'); }
      info = { id: 'n' + (++viewSequence), backing: row.id, offset, length: array || type === 'DataView' ? byteLength : value.length, type, owner: row };
      identities.set(value, info); views.set(info.id, new WeakRef(value));
    }
    range(row, offset, byteLength);
    return { $rdesktop: 'memory-view', id: info.id, backing: row.id, storeLength: row.data.length, offset, byteLength,
      length: array || type === 'DataView' ? byteLength : value.length, type,
      base64: row.data.subarray(offset, offset + byteLength).toString('base64') };
  }
  function collect(commitImmediately = true) {
    const changes = [], pending = []; let bytesToSend = 0;
    for (const weak of backings.values()) {
      const row = weak.deref(); if (!row) continue;
      for (const region of row.ranges) {
        let start = region.start, end = region.end;
        while (start < end && row.data[start] === region.bytes[start - region.start]) start++;
        while (end > start && row.data[end - 1] === region.bytes[end - 1 - region.start]) end--;
        if (start === end) continue;
        bytesToSend += end - start;
        if (bytesToSend > 2 * 1024 * 1024 || changes.length >= 4096) throw new RangeError('Binary writeback exceeds the bounded transport.');
        const bytes = Buffer.from(row.data.subarray(start, end));
        changes.push({ id: row.id, offset: start, base64: bytes.toString('base64') }); pending.push({ region, start, bytes });
      }
    }
    if (Buffer.byteLength(JSON.stringify(changes)) > 3 * 1024 * 1024) throw new RangeError('Binary writeback exceeds the bounded transport.');
    const commit = () => { for (const { region, start, bytes } of pending) region.bytes.set(bytes, start - region.start); };
    const result = { sequence: ++sequence, changes };
    Object.defineProperty(result, 'commit', { value: commit });
    if (commitImmediately) commit();
    return result;
  }
  return { load, unpack, pack, collect };
}
