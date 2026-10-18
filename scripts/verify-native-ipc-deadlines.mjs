// Uses the actual native_agent Rust fixture, not the HTTP browser simulator.
// node scripts/verify-native-ipc-deadlines.mjs SESSION OUTPUT RDESKTOP_EXE
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {spawn} from 'node:child_process';
import {setTimeout as delay} from 'node:timers/promises';
const [sessionPath,outputPath,cliPath]=process.argv.slice(2);assert(sessionPath&&outputPath&&cliPath);
const output=resolve(outputPath);await mkdir(output,{recursive:true});
const session=JSON.parse(await readFile(sessionPath,'utf8')),headers={authorization:`Bearer ${session.token}`,'content-type':'application/json'};
async function call(command,params={},expected=200){const response=await fetch(`${session.url}/__rdesktop__/agent/${command}`,{method:'POST',headers,body:JSON.stringify(params)});const value=await response.json();assert.equal(response.status,expected,JSON.stringify(value));return value;}
async function until(read,label){const end=Date.now()+12000;while(Date.now()<end){const value=await read();if(value)return value;await delay(30);}assert.fail(`timed out: ${label}`);}
const main=(await call('windows')).windows.find(window=>window.title==='rdesktop Native Agent');assert(main);
const window=main.id,req=(command,params={},expected=200)=>call(command,{window,...params},expected),evaluate=expression=>req('evaluate',{expression});
const info=await fetch(`${session.url}/__rdesktop__/info`,{headers});assert.equal(info.status,200);
const information=await info.json(),capability=information.capabilities.ipc;
assert.deepEqual(capability,{timeout_parameter:'timeout_ms',default_timeout_ms:120000,maximum_timeout_ms:600000,completion_grace_ms:2000,cancels_backend:false});
await until(()=>evaluate('document.readyState==="complete"&&!!document.querySelector("#greet")'),'main fixture document');
await until(()=>evaluate('Object.keys(window.__RDESKTOP_RESOLVE__).length===0'),'idle IPC');
const checks=['native-ipc-capabilities-and-bounded-deadline-contract'];
for(const timeout_ms of [null,0,-1,600001,1.5,'25',false])await req('ipc',{cmd:'delay',payload:{milliseconds:0},timeout_ms},422);
checks.push('invalid-agent-timeout-values-rejected-before-dispatch');
const invalid=await evaluate(`(async()=>{const results=[];for(const options of [null,[],{timeoutMs:0},{timeoutMs:600001},{timeoutMs:'25'}]){try{await window.__RDESKTOP_INVOKE__('delay',{milliseconds:0},options);results.push('unexpected success');}catch(error){results.push(error.message);}}return{results,pending:Object.keys(window.__RDESKTOP_RESOLVE__).length};})()`);
assert(invalid.results.every(message=>message.startsWith('IPC ')));assert.equal(invalid.pending,0);checks.push('invalid-frontend-options-leave-no-pending-resolvers');
const fast=await req('ipc',{cmd:'greet',payload:{name:'Deadline default'}});assert(fast.success);assert.equal(fast.data.pid,main.process_id);assert.equal(fast.data.message,'Hello, Deadline default!');checks.push('default-invocation-reaches-real-rust-handler');
try{
 await evaluate(`(()=>{window.__ipcDeadlineQa={late:0,push:0,ipc:window.__RDESKTOP_IPC__,previousPush:window.__RDESKTOP_PUSH__};window.__RDESKTOP_IPC__=message=>{const data=typeof message==='string'?JSON.parse(message):message;if(data.data?.milliseconds===250)window.__ipcDeadlineQa.late++;window.__ipcDeadlineQa.ipc(message);};window.__RDESKTOP_PUSH__=message=>{if(message.id&&typeof message.success==='boolean')window.__ipcDeadlineQa.push++;window.__ipcDeadlineQa.previousPush?.(message);};})()`);
 const timed=await evaluate(`window.__RDESKTOP_INVOKE__('delay',{milliseconds:250},{timeoutMs:10}).then(()=>({unexpected:true}),error=>({code:error.code,timeoutMs:error.timeoutMs,pending:Object.keys(window.__RDESKTOP_RESOLVE__).length}))`);
 assert.deepEqual(timed,{code:'RDESKTOP_IPC_TIMEOUT',timeoutMs:10,pending:0});checks.push('actual-short-deadline-rejects-with-typed-error-and-no-waiter');
 await until(()=>evaluate('window.__ipcDeadlineQa.late===1'),'actual late Rust reply');
 assert.equal(await evaluate('window.__ipcDeadlineQa.push'),0);assert.equal(await evaluate('Object.keys(window.__RDESKTOP_RESOLVE__).length'),0);
 checks.push('actual-late-rust-reply-is-not-reclassified-as-a-push');
 const serialization=await evaluate(`(()=>{const value={};value.self=value;return window.__RDESKTOP_INVOKE__('delay',value).then(()=>({unexpected:true}),error=>({failed:!!error.message,pending:Object.keys(window.__RDESKTOP_RESOLVE__).length}));})()`);
 assert.deepEqual(serialization,{failed:true,pending:0});checks.push('native-serialization-failure-immediately-cleans-resolver');
 const params=join(output,'long-ipc-params.json');await writeFile(params,JSON.stringify({window,cmd:'delay',payload:{milliseconds:16000},timeout_ms:25000}));
 const started=Date.now(),child=spawn(resolve(cliPath),['agent','--session',resolve(sessionPath),'ipc','--params-file',params],{windowsHide:true});let stdout='',stderr='';
 child.stdout.on('data',bytes=>stdout+=bytes);child.stderr.on('data',bytes=>stderr+=bytes);
 const done=new Promise((complete,reject)=>{child.once('error',reject);child.once('exit',(code,signal)=>complete({code,signal}));});
 await until(()=>evaluate('Object.keys(window.__RDESKTOP_RESOLVE__).length===1'),'CLI command pending in the real frontend');
 const responsiveStart=Date.now();await req('action',{action:'fill',selector:'#name',value:'IPC responsive'});await req('action',{action:'click',selector:'#greet'});
 await until(()=>evaluate('document.querySelector("#result").textContent.includes("Hello, IPC responsive!")'),'UI result while a Rust command is pending');
 const responsiveMs=Date.now()-responsiveStart;assert(responsiveMs<8000);assert.equal(child.exitCode,null);
 const captureCommand=main.visible?'native-screenshot':'screenshot';
 const capture=await fetch(`${session.url}/__rdesktop__/agent/${captureCommand}`,{method:'POST',headers,body:JSON.stringify({window})});assert.equal(capture.status,200);
 if(main.visible)assert.equal(capture.headers.get('x-rdesktop-capture-source'),'windows-graphics-capture');
 await writeFile(join(output,'ipc-responsive.png'),Buffer.from(await capture.arrayBuffer()));checks.push('real-ui-and-rust-greet-stay-responsive-during-long-command');
 const end=await done;assert.equal(end.code,0,stderr);const completed=JSON.parse(stdout);assert(completed.success);assert.equal(completed.data.milliseconds,16000);assert.equal(completed.data.pid,main.process_id);
 const elapsedMs=Date.now()-started;assert(elapsedMs>=16000&&elapsedMs<30000);checks.push('actual-cli-rust-roundtrip-exceeds-the-old-fifteen-second-cdp-limit');
 await until(()=>evaluate('Object.keys(window.__RDESKTOP_RESOLVE__).length===0'),'completed IPC cleanup');
 const current=(await call('windows')).windows.find(item=>item.id===window);assert.equal(current.process_id,main.process_id);assert.equal(current.visible,main.visible);
 checks.push('same-native-process-visibility-and-empty-pending-table');
 const proof={frameworkVersion:information.version,headless:!main.visible,capability,elapsedMs,responsiveMs,checks};await writeFile(join(output,'proof.json'),JSON.stringify(proof,null,2));console.log(JSON.stringify(proof,null,2));
}finally{
 await evaluate('(()=>{const saved=window.__ipcDeadlineQa;if(saved){window.__RDESKTOP_IPC__=saved.ipc;if(saved.previousPush===undefined)delete window.__RDESKTOP_PUSH__;else window.__RDESKTOP_PUSH__=saved.previousPush;delete window.__ipcDeadlineQa;}})()');
}
