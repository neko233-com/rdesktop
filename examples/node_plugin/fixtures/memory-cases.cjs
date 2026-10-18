// The exact same functions run against the pinned Node binary and the real
// native document. No fs/crypto-specific behavior is added to the bridge.
async function cases(Buffer, require, directory) {
  const fs = require('node:fs'), path = require('node:path'), vm = require('node:vm'), results = [];
  const bytes = value => Array.from(new Uint8Array(value.buffer, value.byteOffset, value.byteLength));
  const record = (name, value) => results.push({ name, value });
  const file = path.join(directory, 'binary-source.txt'); fs.writeFileSync(file, 'ABCDEFGHIJKL');
  const fd = fs.openSync(file, 'r');
  try {
    const backing = new ArrayBuffer(20), full = Buffer.from(backing); full.fill(46);
    const target = full.subarray(4, 12), alias = full.subarray(6, 10);
    record('readSync-offset', [fs.readSync(fd, target, 1, 4, 2), bytes(full), bytes(alias)]);
    record('readSync-options', [fs.readSync(fd, target, { offset: 2, length: 3, position: 0 }), bytes(target)]);
    record('readSync-repeat', [fs.readSync(fd, target, 0, 2, 9), bytes(full)]);
    const typed = new Uint16Array(backing, 4, 4);
    record('readSync-typed-view', [fs.readSync(fd, typed, 2, 3, 4), bytes(typed)]);
    const data = new DataView(backing, 4, 8);
    record('readSync-DataView', [fs.readSync(fd, data, 1, 2, 0), bytes(data)]);
    record('callback-read', await new Promise((resolve, reject) => fs.read(fd, target, 0, 4, 5, (error, count, returned) => error ? reject(error) : resolve([count, returned === target, bytes(target), bytes(alias)]))));
    const views = [full.subarray(0, 3), full.subarray(2, 6)];
    record('readvSync-shared', [fs.readvSync(fd, views, 0), bytes(full)]);
    record('callback-readv', await new Promise((resolve, reject) => fs.readv(fd, views, 3, (error, count, returned) => error ? reject(error) : resolve([count, returned[0] === views[0], returned[1] === views[1], bytes(full)]))));
    const handle = await fs.promises.open(file, 'r');
    try {
      const read = await handle.read(target, 0, 5, 2);
      record('FileHandle.read-result', [read.bytesRead, read.buffer === target, bytes(full)]);
      const readv = await handle.readv(views, 1);
      record('FileHandle.readv-result', [readv.bytesRead, readv.buffers[0] === views[0], readv.buffers[1] === views[1], bytes(full)]);
    } finally { await handle.close(); }
    const mutate = vm.runInNewContext('(a,b)=>{a[0]=61;b[0]=62;return a;}');
    record('generic-alias-and-return', [mutate(target, alias) === target, bytes(full)]);
    const slice = vm.runInNewContext('b=>{const view=b.subarray(1,3);return [view,view];}')(target);
    record('new-native-shared-view', [slice[0] === slice[1], slice[0].buffer === target.buffer, bytes(slice[0]), Buffer.isBuffer(slice[0])]);
    const stored = vm.runInNewContext('(()=>{let saved;return {set(b){saved=b;},write(){saved[1]=63;return saved;}};})()');
    stored.set(target); record('native-retained-view', [stored.write() === target, bytes(full)]);
    const fail = vm.runInNewContext('(b)=>{b[3]=64;throw new RangeError("after-write");}');
    let error; try { fail(target); } catch (cause) { error = [cause.name, cause.message]; }
    record('write-before-throw', [error, bytes(full)]);
    const types = require('node:util').types;
    record('native-view-types', [types.isUint16Array(typed), types.isDataView(data), types.isArrayBuffer(backing), types.isUint8Array(target)]);
    for (const type of ['Uint8Array','Uint8ClampedArray','Int8Array','Uint16Array','Int16Array','Uint32Array','Int32Array','Float32Array','Float64Array','BigInt64Array','BigUint64Array']) {
      const values = type.startsWith('Big') ? '[1n,2n]' : '[1,2]';
      const value = vm.runInNewContext('new '+type+'('+values+')');
      record('fresh-native-'+type, [Object.prototype.toString.call(value), bytes(value), Buffer.isBuffer(value)]);
    }
    const freshView = vm.runInNewContext('new DataView(new Uint8Array([1,2,3,4]).buffer)');
    record('fresh-native-DataView', [Object.prototype.toString.call(freshView), bytes(freshView)]);
    const freshArray = vm.runInNewContext('new Uint8Array([1,2,3,4]).buffer');
    record('fresh-native-ArrayBuffer', [Object.prototype.toString.call(freshArray), Array.from(new Uint8Array(freshArray))]);
    try { fs.readSync(-1,target,0,1,0); } catch (cause) { record('native-validation-error', [cause instanceof RangeError,cause.name,cause.code]); }
    const generic = vm.runInNewContext('(options)=>{options.parts[0][0]=71;options.parts[1].setUint8(2,72);return options.parts[0];}');
    record('nested-generic-view', [generic({ parts: [target, data] }) === target, bytes(full)]);
    const crypto = require('node:crypto');
    record('crypto-return-identity', crypto.randomFillSync(target, 0, 0) === target);
    const fresh = Buffer.alloc(8, 46);
    record('crypto-async-identity', await new Promise((resolve, reject) => crypto.randomFill(fresh, 0, 0, (error, returned) => error ? reject(error) : resolve([returned === fresh, bytes(fresh)]))));
    const emitter = new (require('node:events').EventEmitter)(); let observed;
    const callback = value => { observed = [value === target, bytes(value)]; };
    emitter.on('binary', callback); emitter.emit('binary', target);
    await new Promise(resolve => setTimeout(resolve, 30));
    emitter.off('binary', callback); target[0] = 90; emitter.emit('binary', target);
    await new Promise(resolve => setTimeout(resolve, 30));
    record('stable-callback-identity', observed);
    record('writeFile-reads-browser-edits', (() => { const output = path.join(directory, 'binary-written.txt'); fs.writeFileSync(output, target); return bytes(fs.readFileSync(output)); })());
    const big = new ArrayBuffer(8 * 1024 * 1024), small = Buffer.from(big, 4 * 1024 * 1024, 4);
    record('small-view-large-backing', [fs.readSync(fd, small, 0, 4, 0), bytes(small), small.byteOffset, small.buffer.byteLength]);
    const roundtrip = vm.runInNewContext('(a,b)=>[a===b,a.buffer===b.buffer]');
    record('repeated-view-native-identity', roundtrip(target, target));
  } finally { fs.closeSync(fd); }
  return results;
}
module.exports = cases;
if (require.main === module) {
  const root = require('node:fs').mkdtempSync(require('node:path').join(require('node:os').tmpdir(), 'rdesktop-memory-reference-'));
  cases(Buffer, require, root).then(value => console.log(JSON.stringify(value)), error => { console.error(error); process.exitCode = 1; });
}
