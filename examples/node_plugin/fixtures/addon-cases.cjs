// Identical functions run against the pinned Node reference and native WebView2.
module.exports = async function addonCases(sharp, Buffer, require, directory) {
  const fs = require('node:fs'), path = require('node:path');
  const { Readable, Writable } = require('node:stream');
  const { pipeline } = require('node:stream/promises');
  const digest = bytes => require('node:crypto').createHash('sha256').update(bytes).digest('hex');
  const checks = [], add = (name, value) => checks.push({ name, value });
  add('sharp-version', sharp.versions.sharp);
  add('vips-version', sharp.versions.vips);
  const input = await sharp({ create: { width: 16, height: 10, channels: 4, background: '#ff000080' } }).png().toBuffer();
  add('native-png-buffer', Buffer.isBuffer(input)); add('png-digest', digest(input));
  const metadata = await sharp(input).metadata();
  add('png-metadata', [metadata.format, metadata.width, metadata.height, metadata.channels]);
  const callback = await new Promise((resolve, reject) => sharp(input).png().toBuffer((error, bytes, info) => error ? reject(error) : resolve({ bytes, info })));
  add('native-callback', [Buffer.isBuffer(callback.bytes), callback.info.format, callback.info.width, callback.info.height, digest(callback.bytes)]);
  const callbackMetadata = await new Promise((resolve, reject) => sharp(input).metadata((error, info) => error ? reject(error) : resolve(info)));
  add('metadata-callback', [callbackMetadata.width, callbackMetadata.height]);
  const original = sharp(input), rotated = await original.clone().rotate(37, { background: '#00000000' }).tint('#00ff00').png().toBuffer({ resolveWithObject: true });
  add('arbitrary-rotation', [rotated.info.width, rotated.info.height, Buffer.isBuffer(rotated.data), digest(rotated.data)]);
  const unchanged = await original.png().toBuffer({ resolveWithObject: true });
  add('clone-isolation', [unchanged.info.width, unchanged.info.height, digest(unchanged.data)]);
  const chunks = [];
  await pipeline(Readable.from([input]), sharp().resize(5, 3).webp(), new Writable({ write(chunk, _encoding, done) { chunks.push(chunk); done(); } }));
  const streamed = await sharp(Buffer.concat(chunks)).metadata();
  add('native-stream-pipeline', [streamed.format, streamed.width, streamed.height, digest(Buffer.concat(chunks))]);
  const svg = Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="3" height="2"><rect width="3" height="2" fill="red"/></svg>');
  const raw = await sharp(svg).ensureAlpha().raw().toBuffer({ resolveWithObject: true });
  add('svg-raw-pixels', [raw.info.width, raw.info.height, raw.info.channels, Array.from(raw.data)]);
  const source = path.join(directory, 'addon-input.png'), target = path.join(directory, 'addon-output.png');
  fs.writeFileSync(source, input); await sharp(source).resize(4, 6).png().toFile(target);
  const saved = await sharp(fs.readFileSync(target)).metadata();
  add('real-file-pipeline', [saved.format, saved.width, saved.height, digest(fs.readFileSync(target))]);
  add('rejected-image', await sharp(Buffer.from('invalid-image')).metadata().then(() => false, error => error instanceof Error));
  add('rejected-callback', await new Promise(resolve => sharp(Buffer.from('invalid-image')).metadata(error => resolve(error instanceof Error))));
  return { checks, preview: input.toString('base64') };
};
if (require.main === module) module.exports(require('sharp'), Buffer, require, __dirname)
  .then(result => process.stdout.write(JSON.stringify(result)), error => { console.error(error); process.exitCode = 1; });
