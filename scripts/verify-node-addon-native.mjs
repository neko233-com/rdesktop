// Real Windows native addon acceptance, using an externally verified module set.
// No downloads, Electron, default payload changes or application installation.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { spawn, execFileSync } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';
const repo = path.resolve(import.meta.dirname, '..');
const [mode, modulesArgument, manifestArgument, outputArgument, ...flags] = process.argv.slice(2);
assert(['browser', 'worker', 'binding'].includes(mode) && outputArgument && flags.every(value => value === '--headless'), 'Expected browser|worker|binding MODULE_DIRECTORY FILE_MANIFEST OUTSIDE_OUTPUT [--headless]');
assert(process.platform === 'win32' && process.arch === 'x64');
const hidden = flags.includes('--headless'), output = path.resolve(outputArgument), relative = path.relative(repo, output);
assert(path.isAbsolute(relative) || relative === '..' || relative.startsWith('..' + path.sep), 'Output must be outside the repository');
await fs.mkdir(output, { recursive: true });
const run = path.join(output, `${mode}-${hidden ? 'hidden' : 'visible'}-${randomUUID()}`); await fs.mkdir(run);
const root = path.join(run, 'package'), source = await fs.realpath(modulesArgument);
const manifest = JSON.parse(await fs.readFile(manifestArgument, 'utf8'));
assert.equal(manifest.platform, 'win32-x64'); assert(Array.isArray(manifest.files) && manifest.files.length);
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
await fs.mkdir(root);
async function copyVerifiedFiles() {
  for (const file of manifest.files) {
    assert(/^[a-zA-Z0-9_@./-]+$/.test(file.path) && file.path.split('/').every(part => part && part !== '.' && part !== '..'));
    const original = await fs.realpath(path.join(source, file.path)), contained = path.relative(source, original);
    assert(contained && !path.isAbsolute(contained) && contained !== '..' && !contained.startsWith('..' + path.sep));
    const bytes = await fs.readFile(original); assert.equal(hash(bytes), file.sha256, file.path);
    const target = path.join(root, file.path); await fs.mkdir(path.dirname(target), { recursive: true }); await fs.writeFile(target, bytes, { flag: 'wx' });
  }
}
await copyVerifiedFiles();
await fs.copyFile(path.join(repo, 'examples/node_plugin/fixtures/addon-cases.cjs'), path.join(root, 'addon-cases.cjs'));
const node = process.env.RDESKTOP_TEST_NODE; assert(node && path.isAbsolute(node));
const nodeSha256 = hash(await fs.readFile(node)); assert.equal(nodeSha256, process.env.RDESKTOP_TEST_NODE_SHA256);
const expected = JSON.parse(execFileSync(node, [path.join(root, 'addon-cases.cjs')], { windowsHide: true, encoding: 'utf8', timeout: 20000 }));
await fs.writeFile(path.join(run, 'reference.json'), JSON.stringify(expected, null, 2));
const select = mode === 'binding' ? 'window.__rdesktopSharp()' : mode === 'worker' ? "require('node:module').createRequire(__filename)('sharp')" : "require('sharp')";
await fs.writeFile(path.join(root, 'preload.cjs'), `const cases=require('./addon-cases.cjs');window.runAddonCases=()=>cases(${select},Buffer,require,__dirname);`);
const moduleEntry = path.join(run, 'host-sharp.cjs'), moduleMarker = path.join(run, 'native-module-loaded.txt');
if (mode === 'binding') await fs.writeFile(moduleEntry, `require('node:fs').appendFileSync(${JSON.stringify(moduleMarker)},'loaded\\n');module.exports=require(${JSON.stringify(path.join(root, 'node_modules/sharp/lib/index.js'))});`);
await fs.writeFile(path.join(root, 'index.html'), `<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>Native addon acceptance</title><style>body{font:16px 'Segoe UI',sans-serif;margin:40px;background:#f5f6fa;color:#202331}button{font:inherit;padding:12px 20px;border:1px solid #4357d8;border-radius:8px;background:#fff;color:#263ba6}button:focus-visible{outline:3px solid #657cff;outline-offset:4px}img{image-rendering:pixelated;width:160px;height:100px}pre{white-space:pre-wrap}</style><h1>rdesktop · Node 原生扩展</h1><p>固定 Sharp 模块 · 原生窗口 · 真实 Node</p><button id="run">Run native addon cases</button><p role="status" id="status">准备运行</p><img hidden id="preview" alt="真实 Sharp 输出预览"><pre id="details"></pre><script>document.querySelector('#run').onclick=async()=>{try{const result=await window.runAddonCases();window.addonResult=result;document.querySelector('#status').textContent='已完成 '+result.checks.length+' 项原生扩展对照';document.querySelector('#details').textContent=JSON.stringify(result.checks,null,2);const image=document.querySelector('#preview');image.src='data:image/png;base64,'+result.preview;image.hidden=false;}catch(error){window.addonFailure={name:error.name,message:error.message,stack:error.stack};document.querySelector('#status').textContent=error.message;}};</script></html>`);
const sessionFile = path.join(run, 'session.json');
const child = spawn(path.join(repo, 'target/debug/node_plugin.exe'), [], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env, RDESKTOP_PLUGIN_ROOT: root, RDESKTOP_PLUGIN_PRELOAD: 'preload.cjs', RDESKTOP_PLUGIN_ENTRY: 'index.html', RDESKTOP_PLUGIN_NODE_GLOBALS: 'preload-only', RDESKTOP_PLUGIN_WORKER_MODULES: JSON.stringify(mode === 'binding' ? [{ getter: '__rdesktopSharp', module: moduleEntry }] : []), RDESKTOP_TEST_NODE: node, RDESKTOP_TEST_NODE_SHA256: nodeSha256, RDESKTOP_AGENT: '1', RDESKTOP_AGENT_SESSION: sessionFile, RDESKTOP_HEADLESS: hidden ? '1' : '0', RDESKTOP_PLUGIN_EMBEDDED: '0' } });
let log = '', session, main, proof, ownedProcesses = [];
const snapshot = () => JSON.parse(execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', 'Get-CimInstance Win32_Process | Select-Object Name,ProcessId,ParentProcessId | ConvertTo-Json -Compress'], { windowsHide: true, encoding: 'utf8' }));
child.stdout.on('data', bytes => log += bytes); child.stderr.on('data', bytes => log += bytes);
const exited = new Promise(resolve => child.once('exit', (code, signal) => resolve({ code, signal })));
async function call(command, params = {}) {
  const response = await fetch(session.url + '/__rdesktop__/agent/' + command, { method: 'POST', headers: { authorization: 'Bearer ' + session.token, 'content-type': 'application/json' }, body: JSON.stringify(params), signal: AbortSignal.timeout(20000) });
  const value = await response.json(); assert(response.ok, JSON.stringify(value)); return value;
}
async function until(read, label) {
  const end = Date.now() + 30000;
  while (Date.now() < end) { const value = await read(); if (value) return value; assert.equal(child.exitCode, null, log); await delay(50); }
  throw Error('Timed out: ' + label);
}
const req = (command, params = {}) => call(command, { window: main.id, ...params });
const evaluate = expression => req('evaluate', { expression });
try {
  await until(async () => { session = await fs.readFile(sessionFile, 'utf8').then(JSON.parse, () => null); return session; }, 'session');
  main = await until(async () => (await call('windows')).windows.find(value => value.title === 'rdesktop Native Node Plugin'), 'native document');
  await until(() => evaluate('document.readyState==="complete"&&typeof window.runAddonCases==="function"').catch(() => false), 'preload');
  assert.equal(await evaluate('window.__RDESKTOP_NODE_STATUS__.nodeGlobals'), 'preload-only');
  assert.deepEqual(await evaluate('[typeof require,typeof Buffer,typeof process]'), ['undefined', 'undefined', 'undefined']);
  if (mode === 'binding') {
    assert.equal(await fs.stat(moduleMarker).then(() => true, () => false), false, 'Binding must not eagerly execute native modules');
    assert.deepEqual(await evaluate('(()=>{const d=Object.getOwnPropertyDescriptor(window,"__rdesktopSharp");return [typeof d.value,d.writable,d.configurable,d.enumerable];})()'), ['function', false, false, false]);
  }
  const processes = snapshot(), owned = new Set([child.pid]);
  for (let pass = 0; pass < 16; pass++) for (const process of processes) if (owned.has(process.ParentProcessId)) owned.add(process.ProcessId);
  ownedProcesses = processes.filter(process => owned.has(process.ProcessId));
  assert(!ownedProcesses.some(process => /electron/i.test(process.Name))); assert(ownedProcesses.some(process => process.Name.toLowerCase() === 'node.exe')); assert(ownedProcesses.some(process => process.Name.toLowerCase() === 'msedgewebview2.exe'));
  await fs.writeFile(path.join(run, 'processes.json'), JSON.stringify(ownedProcesses, null, 2));
  if (hidden) await req('action', { action: 'click', selector: '#run' });
  else {
    const button = await until(async () => (await req('uia', { max_nodes: 1000 })).nodes.find(value => value.name === 'Run native addon cases' && value.control_type === 50000), 'native UIA button');
    await req('uia', { operation: 'invoke', target: { runtime_id: button.runtime_id } });
  }
  const result = await until(async () => { const failure = await evaluate('window.addonFailure??null'); assert(!failure, JSON.stringify(failure)); return evaluate('window.addonResult??null'); }, 'native addon cases');
  await fs.writeFile(path.join(run, 'native.json'), JSON.stringify(result, null, 2)); assert.deepEqual(result, expected);
  if (mode === 'binding') {
    assert.equal(await evaluate('window.__rdesktopSharp()===window.__rdesktopSharp()'), true);
    assert.equal(await fs.readFile(moduleMarker, 'utf8'), 'loaded\n');
  }
  if (!hidden) {
    const response = await fetch(session.url + '/__rdesktop__/agent/native-screenshot', { method: 'POST', headers: { authorization: 'Bearer ' + session.token, 'content-type': 'application/json' }, body: JSON.stringify({ window: main.id }), signal: AbortSignal.timeout(20000) });
    assert(response.ok); assert.equal(response.headers.get('x-rdesktop-capture-source'), 'windows-graphics-capture'); await fs.writeFile(path.join(run, 'native.png'), Buffer.from(await response.arrayBuffer()));
  }
  for (const file of manifest.files) assert.equal(hash(await fs.readFile(path.join(root, file.path))), file.sha256, file.path);
  proof = { mode, hidden, checks: result.checks.length, differences: 0, nodeSha256, moduleFiles: manifest.files.length, originalModuleHashesPreserved: true, ownedProcesses };
} catch (error) {
  await fs.writeFile(path.join(run, 'failure.json'), JSON.stringify({ name: error.name, message: error.message, stack: error.stack }, null, 2));
  if (session && main) await fs.writeFile(path.join(run, 'events.json'), JSON.stringify(await req('events'), null, 2));
  console.error('Native addon evidence:', run); throw error;
} finally {
  if (child.exitCode === null && session) await call('exit').catch(() => {});
  const ended = await Promise.race([exited, delay(15000, undefined, { ref: false }).then(() => { throw Error('Owned application has not exited normally; no process killed.'); })]);
  await fs.writeFile(path.join(run, 'host.log'), log); assert.equal(ended.code, 0, log);
  const remaining = snapshot();
  assert(!remaining.some(process => ownedProcesses.some(owned => owned.Name.toLowerCase() === 'node.exe' && owned.ProcessId === process.ProcessId && owned.ParentProcessId === process.ParentProcessId)), 'Owned Node worker did not retire normally');
}
await fs.writeFile(path.join(run, 'proof.json'), JSON.stringify({ ...proof, normalOwnedNodeExit: true }, null, 2));
console.log(JSON.stringify({ mode, hidden, checks: proof.checks, output: run }));
