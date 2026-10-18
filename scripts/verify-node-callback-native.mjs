import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { spawn, execFileSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { join, resolve, relative, isAbsolute } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
const repo = resolve(import.meta.dirname, '..'), [destination, ...flags] = process.argv.slice(2), hidden = flags.includes('--headless');
assert(destination && flags.every(flag => flag === '--headless'));
const output = resolve(destination), location = relative(repo, output); assert(isAbsolute(location) || location === '..' || location.startsWith('..\\') || location.startsWith('../'));
await fs.mkdir(output, { recursive: true });
const run = join(output, `${hidden ? 'hidden' : 'visible'}-${randomUUID()}`), root = join(run, 'package'); await fs.mkdir(root, { recursive: true });
await fs.copyFile(join(repo, 'examples/node_plugin/fixtures/callback-cases.cjs'), join(root, 'callback-cases.cjs'));
const node = process.env.RDESKTOP_TEST_NODE; assert(node);
const nodeSha256 = createHash('sha256').update(await fs.readFile(node)).digest('hex'); assert.equal(nodeSha256, process.env.RDESKTOP_TEST_NODE_SHA256);
const expected = JSON.parse(execFileSync(node, [join(root, 'callback-cases.cjs')], { windowsHide: true, encoding: 'utf8', timeout: 30000 }));
await fs.writeFile(join(run, 'reference.json'), JSON.stringify(expected, null, 2));
await fs.writeFile(join(root, 'preload.cjs'), `const send=XMLHttpRequest.prototype.send;window.callbackRequests=0;XMLHttpRequest.prototype.send=function(...args){window.callbackRequests++;return Reflect.apply(send,this,args);};
window.runCallbackCases=async()=>{
 const result=await require('./callback-cases.cjs')(require,__dirname,value=>window.callbackProgress=value,emitter=>{let caught;try{emitter.on('budget',()=>{});}catch(error){caught=error;}if(!(caught instanceof RangeError)||emitter.listenerCount('budget')!==256)throw Error('Live callback budget was bypassed');window.callbackLimitEnforced=true;});
 const stat=require('node:fs').statSync,cyclic={};cyclic.self=cyclic;
 for(let batch=0;batch<4;batch++){for(let index=0;index<80;index++){let caught;try{stat(()=>{},cyclic);}catch(error){caught=error;}if(!(caught instanceof TypeError))throw Error('Undispatched callback retained a slot');}await new Promise(resolve=>setTimeout(resolve,100));}
 window.callbackOrphansRetired=320;await new Promise(resolve=>setTimeout(resolve,2500));const requests=window.callbackRequests;await new Promise(resolve=>setTimeout(resolve,500));if(window.callbackRequests!==requests)throw Error('Callback polling did not stop while idle');window.callbackIdleRequests=0;return result;
};`);
await fs.writeFile(join(root, 'index.html'), `<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>Native callback lifecycle</title><style>body{font:16px system-ui;margin:40px;color:#202331;background:#f5f6fa}button{font:inherit;padding:12px 20px}pre{white-space:pre-wrap}</style><h1>rdesktop · 回调生命周期</h1><button id="run">Run callback lifetime cases</button><p id="status" role="status">等待运行</p><pre id="details"></pre><script>document.querySelector('#run').onclick=async()=>{try{window.callbackResult=await window.runCallbackCases();document.querySelector('#status').textContent='已完成 600 次短期回调及长期监听对照';document.querySelector('#details').textContent=JSON.stringify(window.callbackResult,null,2);}catch(error){window.callbackFailure={name:error.name,message:error.message,stack:error.stack};document.querySelector('#status').textContent=error.message;}};</script></html>`);
const sessionFile = join(run, 'session.json');
const child = spawn(join(repo, 'target/debug/node_plugin.exe'), [], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env, RDESKTOP_TEST_NODE: node, RDESKTOP_TEST_NODE_SHA256: nodeSha256, RDESKTOP_PLUGIN_ROOT: root, RDESKTOP_PLUGIN_PRELOAD: 'preload.cjs', RDESKTOP_PLUGIN_ENTRY: 'index.html', RDESKTOP_PLUGIN_WORKER_MODULES: '[]', RDESKTOP_PLUGIN_NODE_GLOBALS: 'preload-only', RDESKTOP_PLUGIN_EMBEDDED: '0', RDESKTOP_AGENT: '1', RDESKTOP_AGENT_SESSION: sessionFile, RDESKTOP_HEADLESS: hidden ? '1' : '0' } });
let session, main, log = '', proof, ownedNode;
const snapshot = () => JSON.parse(execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', 'Get-CimInstance Win32_Process | Select-Object Name,ProcessId,ParentProcessId | ConvertTo-Json -Compress'], { windowsHide: true, encoding: 'utf8' }));
child.stdout.on('data', bytes => log += bytes); child.stderr.on('data', bytes => log += bytes);
const exited = new Promise(resolve => child.once('exit', (code, signal) => resolve({ code, signal })));
async function call(command, params = {}) { const response = await fetch(session.url + '/__rdesktop__/agent/' + command, { method: 'POST', headers: { authorization: 'Bearer ' + session.token, 'content-type': 'application/json' }, body: JSON.stringify(params), signal: AbortSignal.timeout(25000) }); const value = await response.json(); assert(response.ok, JSON.stringify(value)); return value; }
async function until(read, label) { const end = Date.now() + 90000; while (Date.now() < end) { const value = await read(); if (value) return value; assert.equal(child.exitCode, null, log); await delay(50); } throw Error('Timed out: ' + label); }
const req = (command, params = {}) => call(command, { window: main.id, ...params }), evaluate = expression => req('evaluate', { expression });
try {
  await until(async () => { session = await fs.readFile(sessionFile, 'utf8').then(JSON.parse, () => null); return session; }, 'session');
  main = await until(async () => (await call('windows')).windows.find(value => value.title === 'rdesktop Native Node Plugin'), 'window');
  await until(() => evaluate('document.readyState==="complete"&&typeof window.runCallbackCases==="function"'), 'preload');
  const processes = snapshot(); ownedNode = processes.find(value => value.ParentProcessId === child.pid && value.Name.toLowerCase() === 'node.exe'); assert(ownedNode);
  if (hidden) await req('action', { action: 'click', selector: '#run' });
  else { const button = await until(async () => (await req('uia', { max_nodes: 1000 })).nodes.find(value => value.name === 'Run callback lifetime cases' && value.control_type === 50000), 'native UIA button'); await req('uia', { operation: 'invoke', target: { runtime_id: button.runtime_id } }); }
  const actual = await until(async () => { const failure = await evaluate('window.callbackFailure??null'); assert(!failure, JSON.stringify(failure)); return evaluate('window.callbackResult??null'); }, 'callback contract');
  await fs.writeFile(join(run, 'native.json'), JSON.stringify(actual, null, 2)); assert.deepEqual(actual, expected);
  if (!hidden) { const response = await fetch(session.url + '/__rdesktop__/agent/native-screenshot', { method: 'POST', headers: { authorization: 'Bearer ' + session.token, 'content-type': 'application/json' }, body: JSON.stringify({ window: main.id }) }); assert(response.ok); await fs.writeFile(join(run, 'native.png'), Buffer.from(await response.arrayBuffer())); }
  const lifecycle = await evaluate('({limitEnforced:window.callbackLimitEnforced,undispatchedCallbacks:window.callbackOrphansRetired,idleRequests:window.callbackIdleRequests})');
  assert.deepEqual(lifecycle, { limitEnforced: true, undispatchedCallbacks: 320, idleRequests: 0 });
  proof = { hidden, nodeSha256, sequentialCallbacks: actual.sequentialCallbacks, checks: actual, lifecycle, differences: 0 };
} catch (error) { await fs.writeFile(join(run, 'failure.json'), JSON.stringify({ name: error.name, message: error.message, progress: session && main ? await evaluate('window.callbackProgress??0').catch(() => null) : null }, null, 2)); console.error('Callback evidence:', run); throw error; }
finally { if (session && child.exitCode === null) await call('exit').catch(() => {}); const result = await Promise.race([exited, delay(15000, undefined, { ref: false }).then(() => { throw Error('Owned host did not exit normally; no process killed.'); })]); await fs.writeFile(join(run, 'host.log'), log); assert.equal(result.code, 0, log); }
assert(ownedNode && !snapshot().some(value => value.ProcessId === ownedNode.ProcessId));
await fs.writeFile(join(run, 'proof.json'), JSON.stringify({ ...proof, hostExitCode: 0, ownedNodeRetired: true }, null, 2)); console.log(JSON.stringify({ output: run, ...proof }));
