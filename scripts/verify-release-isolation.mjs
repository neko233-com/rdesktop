// Builds must be done first. Launches only the owned example, then waits for
// its normal window close. Never kills an application or runs an installer.
import {readFile,writeFile,mkdir,stat} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {spawn} from 'node:child_process';
import {createServer} from 'node:http';
import {resolve,join} from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
import assert from 'node:assert/strict';
const output=resolve(process.argv[2]??'.cache/release-isolation');await mkdir(output,{recursive:true});
const executable=resolve('target/release/native_agent.exe');const bytes=await readFile(executable);
// The fixture's own UI state reporter is ordinary app code; the framework
// environment switches, session transport and OS inspector must be absent.
const markers=['RDESKTOP_AGENT_SESSION','RDESKTOP_HEADLESS','RDESKTOP_DEV_URL','/__rdesktop__/agent/','windows-uia'];
for(const marker of markers)assert(!bytes.includes(Buffer.from(marker)),`release contains debug marker ${marker}`);
assert(!/RDESKTOP_AGENT(?!_STATE__)/.test(bytes.toString('latin1')),'release contains native Agent activation');
let requests=0;const probe=createServer((_req,res)=>{requests++;res.end('unexpected debug navigation');});
await new Promise(done=>probe.listen(0,'127.0.0.1',done));
const session=join(output,`forbidden-session-${Date.now()}.json`);
const child=spawn(executable,[],{windowsHide:true,stdio:'ignore',env:{...process.env,RDESKTOP_AGENT:'1',RDESKTOP_HEADLESS:'1',RDESKTOP_AGENT_SESSION:session,RDESKTOP_DEV_URL:`http://127.0.0.1:${probe.address().port}`}});
const exited=new Promise((done,reject)=>{child.once('error',reject);child.once('exit',(code,signal)=>done({code,signal}));});
await delay(2500);assert.equal(requests,0);assert.equal(await stat(session).then(()=>true,()=>false),false);
console.log(JSON.stringify({pid:child.pid,debugMarkers:0,debugNavigationRequests:requests,sessionCreated:false,action:'Close the visible owned native_agent example normally to finish this verification.'}));
const exit=await exited;probe.close();assert.equal(exit.code,0);assert.equal(requests,0);assert.equal(await stat(session).then(()=>true,()=>false),false);
const proof={pid:child.pid,executable,sha256:createHash('sha256').update(bytes).digest('hex'),bytes:bytes.length,debugMarkers:0,debugNavigationRequests:requests,sessionCreated:false,exit};
await writeFile(join(output,'proof.json'),JSON.stringify(proof,null,2));console.log(JSON.stringify(proof,null,2));
