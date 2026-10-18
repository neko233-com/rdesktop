// This function needs only a host's Sharp factory and standard web globals.
// The identical function body also runs independently in the pinned Node reference.
module.exports = async function noPreloadCases(sharp) {
  const checks = [], add = (name, value) => checks.push({ name, value });
  add('sharp-version', sharp.versions.sharp);
  add('vips-version', sharp.versions.vips);
  const image = sharp({ create: { width: 7, height: 5, channels: 4, background: '#12ab34' } }).png();
  const bytes = await image.clone().toBuffer();
  add('typed-bytes', bytes instanceof Uint8Array);
  add('png-header', Array.from(bytes.subarray(0, 8)));
  const digest = await crypto.subtle.digest('SHA-256', bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength));
  add('png-digest', Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0')).join(''));
  const metadata = await sharp(bytes).metadata();
  add('metadata', [metadata.format, metadata.width, metadata.height, metadata.channels]);
  const copied = JSON.parse(JSON.stringify(metadata));
  add('metadata-json', [copied.format, copied.width, copied.height, copied.channels]);
  const output = await sharp(bytes).resize(3, 2).ensureAlpha().raw().toBuffer({ resolveWithObject: true });
  add('raw-pixels', [output.info.width, output.info.height, output.info.channels, Array.from(output.data)]);
  add('callback-metadata', await new Promise((resolve, reject) => sharp(bytes).metadata((error, info) => error ? reject(error) : resolve([info.width, info.height]))));
  add('immediate', await new Promise(resolve => setImmediate(() => resolve('delivered'))));
  add('invalid-image', await sharp(new Uint8Array([0, 1, 2])).metadata().then(() => false, error => error instanceof Error));
  return { checks, preview: bytes.toString('base64') };
};
