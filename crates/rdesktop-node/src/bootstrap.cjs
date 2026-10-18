// Read exactly the bounded source prefix, leaving the first JSON request in stdin.
// Keeping source out of --eval avoids Windows' 32767-character command-line limit.
const bootFs = require('node:fs');
function bootRead(size) {
  const bytes = Buffer.alloc(size); let offset = 0;
  while (offset < size) { const count = bootFs.readSync(0, bytes, offset, size - offset); if (!count) throw new Error('Truncated Node worker bootstrap.'); offset += count; }
  return bytes;
}
const bootSize = bootRead(4).readUInt32LE();
if (!bootSize || bootSize > 256 * 1024) throw new RangeError('Node worker bootstrap exceeds its limit.');
new Function('require', 'module', 'exports', bootRead(bootSize).toString('utf8'))(require, module, module.exports);
