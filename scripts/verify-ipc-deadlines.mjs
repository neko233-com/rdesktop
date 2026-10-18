// Execute the shipped bridge source, with a controlled clock and transport.
// This checks waits beyond the old 120-second limit without sleeping in tests.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {runInNewContext} from 'node:vm';
import {test} from 'node:test';

class Clock {
 now=0; next=0; timers=new Map();
 set=(callback,ms)=>{const id=++this.next;this.timers.set(id,{callback,ms,due:this.now+ms});return id;};
 clear=id=>this.timers.delete(id);
 async advance(ms){const end=this.now+ms;for(;;){const next=[...this.timers].filter(([,timer])=>timer.due<=end).sort((a,b)=>a[1].due-b[1].due)[0];if(!next)break;this.now=next[1].due;this.timers.delete(next[0]);next[1].callback();await Promise.resolve();}this.now=end;await Promise.resolve();}
}
async function nativeSource(backend){
 const source=await readFile(new URL(`../crates/rdesktop-${backend}/src/renderer.rs`,import.meta.url),'utf8');
 const method=source.indexOf('fn bridge_script()'),start=source.indexOf('r#"',method)+3,end=source.indexOf('"#',start);
 assert(method>=0&&start>=3&&end>start);const script=source.slice(start,end);
 const marker=backend==='webview'?'// ── Window Controls':'window.__RDESKTOP_WINDOW__ =';
 assert(script.includes(marker));return script.slice(0,script.indexOf(marker))+'\n})();';
}
const scripts=Object.fromEntries(await Promise.all(['webview','cef'].map(async backend=>[backend,await nativeSource(backend)])));
const browserSource=(await readFile(new URL('../crates/rdesktop-dev/assets/bridge.js',import.meta.url),'utf8')).split('// State reporter:')[0]+'\n})();';
function native(backend){
 const clock=new Clock(),sent=[],pushes=[],window={ipc:{postMessage:body=>sent.push(JSON.parse(body))}};let serial=0;
 runInNewContext(scripts[backend],{window,Math:{random:()=>++serial/1000},setTimeout:clock.set,clearTimeout:clock.clear,console});
 window.__RDESKTOP_PUSH__=value=>pushes.push(value);
 return{window,clock,pushes,take:()=>backend==='cef'?JSON.parse(window.__rdesktop_take__()):sent.splice(0),pending:()=>Object.keys(window.__RDESKTOP_RESOLVE__).length};
}
const reply=(host,request,data)=>host.window.__RDESKTOP_IPC__({id:request.id,success:true,data});
for(const backend of ['webview','cef']){
 test(`${backend}: explicit wait survives the old deadline, default still expires`,async()=>{
  const h=native(backend),standard=h.window.__RDESKTOP_INVOKE__('default'),extended=h.window.__RDESKTOP_INVOKE__('extended',{value:3},{timeoutMs:240000});
  const expired=assert.rejects(standard,error=>error.code==='RDESKTOP_IPC_TIMEOUT'&&error.timeoutMs===120000);
  const requests=h.take();assert.equal(requests.length,2);await h.clock.advance(120001);await expired;
  assert.equal(h.pending(),1);reply(h,requests[1],{complete:true});assert.equal((await extended).data.complete,true);
  assert.equal(h.pending(),0);assert.equal(h.clock.timers.size,0);
  reply(h,requests[0],{late:true});reply(h,requests[1],{duplicate:true});assert.equal(h.pushes.length,0);
  h.window.__RDESKTOP_IPC__({cmd:'example.push',payload:2});assert.equal(h.pushes.length,1);
 });
 test(`${backend}: successful, failed and out-of-order replies clear all timers`,async()=>{
  const h=native(backend),calls=Array.from({length:20},(_,value)=>h.window.__RDESKTOP_INVOKE__('echo',{value}));
  const requests=h.take();for(const request of [...requests].reverse())reply(h,request,request.payload);
  assert.deepEqual((await Promise.all(calls)).map(value=>value.data.value),Array.from({length:20},(_,value)=>value));
  const failed=h.window.__RDESKTOP_INVOKE__('failure');const request=h.take()[0];h.window.__RDESKTOP_IPC__({id:request.id,success:false,data:{message:'expected error'}});
  assert.equal((await failed).success,false);assert.equal(h.pending(),0);assert.equal(h.clock.timers.size,0);
 });
 test(`${backend}: invalid options never dispatch or create pending timers`,async()=>{
  const h=native(backend);
  for(const options of [null,false,0,[],...[-1,0,600001,1.5,NaN,Infinity,'10',null].map(timeoutMs=>({timeoutMs}))]){
   await assert.rejects(h.window.__RDESKTOP_INVOKE__('invalid',{},options),/IPC (options|timeoutMs)/);
  }
  assert.equal(h.take().length,0);assert.equal(h.pending(),0);assert.equal(h.clock.timers.size,0);
 });
 test(`${backend}: serialization and transport failures release their waiters`,async()=>{
  const h=native(backend),cyclic={};cyclic.self=cyclic;
  for(const value of [cyclic,{value:1n}])await assert.rejects(h.window.__RDESKTOP_INVOKE__('bad',value));
  if(backend==='webview')h.window.ipc.postMessage=()=>{throw Error('transport failed');};
  else h.window.__RDESKTOP_QUEUE__.push=()=>{throw Error('transport failed');};
  await assert.rejects(h.window.__RDESKTOP_INVOKE__('send'),/transport failed/);
  assert.equal(h.pending(),0);assert.equal(h.clock.timers.size,0);
 });
 test(`${backend}: both range endpoints and request snapshots are preserved`,async()=>{
  const h=native(backend),payload={value:'original'};
  const first=h.window.__RDESKTOP_INVOKE__('one',payload,{timeoutMs:1});payload.value='changed';
  const last=h.window.__RDESKTOP_INVOKE__('max',{}, {timeoutMs:600000});const requests=h.take();
  assert.equal(requests[0].payload.value,'original');assert.deepEqual([...h.clock.timers.values()].map(timer=>timer.ms),[1,600000]);
  reply(h,requests[0],null);reply(h,requests[1],null);await Promise.all([first,last]);assert.equal(h.clock.timers.size,0);
 });
}
test('webview: missing transport fails immediately with no leaked waiter',async()=>{
 const h=native('webview');delete h.window.ipc;
 await assert.rejects(h.window.__RDESKTOP_INVOKE__('unavailable'),error=>error.code==='RDESKTOP_IPC_UNAVAILABLE');
 assert.equal(h.pending(),0);assert.equal(h.clock.timers.size,0);
});
test('webview: an immediate response during postMessage also clears its timer',async()=>{
 const h=native('webview');h.window.ipc.postMessage=body=>reply(h,JSON.parse(body),{immediate:true});
 assert.equal((await h.window.__RDESKTOP_INVOKE__('immediate')).data.immediate,true);assert.equal(h.clock.timers.size,0);
});

function browser(fetch){
 const clock=new Clock(),requests=[],window={};
 runInNewContext(browserSource,{window,AbortController,setTimeout:clock.set,clearTimeout:clock.clear,fetch:(url,options)=>{requests.push({url,options});return fetch(url,options);}});
 return{clock,requests,window};
}
test('browser: successful responses clear the request deadline',async()=>{
 const h=browser(async()=>({ok:true,json:async()=>({success:true,data:7})}));
 assert.equal((await h.window.__RDESKTOP_INVOKE__('ok',{value:1},{timeoutMs:240000})).data,7);
 assert.equal(h.clock.timers.size,0);assert.equal(h.requests[0].options.signal.aborted,false);
 assert.deepEqual(JSON.parse(h.requests[0].options.body).payload,{value:1});
});
test('browser: the deadline rejects even if a transport ignores abort',async()=>{
 const h=browser(()=>new Promise(()=>{})),request=h.window.__RDESKTOP_INVOKE__('slow',{}, {timeoutMs:20});
 const expired=assert.rejects(request,error=>error.code==='RDESKTOP_IPC_TIMEOUT'&&error.timeoutMs===20);
 await h.clock.advance(20);await expired;assert(h.requests[0].options.signal.aborted);assert.equal(h.clock.timers.size,0);
});
test('browser: stalled response body cannot turn an abort into empty success',async()=>{
 const h=browser((_url,{signal})=>Promise.resolve({ok:true,json:()=>new Promise((_resolve,reject)=>signal.addEventListener('abort',()=>reject(Error('aborted'))))}));
 const request=h.window.__RDESKTOP_INVOKE__('body',{}, {timeoutMs:25}),expired=assert.rejects(request,error=>error.code==='RDESKTOP_IPC_TIMEOUT');
 await Promise.resolve();await h.clock.advance(25);await expired;assert.equal(h.clock.timers.size,0);
});
test('browser: invalid options, network and serialization errors leave no timer',async()=>{
 const h=browser(async()=>{throw Error('network failed');});
 for(const options of [null,[],{timeoutMs:0},{timeoutMs:600001},{timeoutMs:'20'}])await assert.rejects(h.window.__RDESKTOP_INVOKE__('bad',{},options),/IPC (options|timeoutMs)/);
 assert.equal(h.requests.length,0);await assert.rejects(h.window.__RDESKTOP_INVOKE__('network'),/network failed/);
 await assert.rejects(h.window.__RDESKTOP_INVOKE__('serialize',{value:1n}));assert.equal(h.clock.timers.size,0);
});
