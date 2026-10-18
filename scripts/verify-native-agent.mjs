// Run `rdesktop dev --path examples/native_agent --port 1438` first, then:
// node scripts/verify-native-agent.mjs <session.json> <artifact-directory>
// Exercises the public native transport; never prints the session credential.
import assert from 'node:assert/strict';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, join} from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
const [sessionPath, outputPath] = process.argv.slice(2);
assert(sessionPath && outputPath, 'session and artifact directory required');
const session = JSON.parse(await readFile(sessionPath, 'utf8'));
const output = resolve(outputPath);
await mkdir(output, {recursive:true});
const headers = {authorization:`Bearer ${session.token}`, 'content-type':'application/json'};
async function request(command, params={}, expected=200) {
    const response=await fetch(`${session.url}/__rdesktop__/agent/${command}`,{method:'POST',headers,body:JSON.stringify(params)});
    const value=await response.json();
    assert.equal(response.status,expected,JSON.stringify(value));
    return value;
}
async function evaluate(expression, window) { return request('evaluate',{window,expression}); }
async function until(expression,window) {
    const deadline=Date.now()+5000;
    while(Date.now()<deadline) {if(await evaluate(expression,window))return;await delay(30);}
    assert.fail(`timed out: ${expression}`);
}
let response=await fetch(`${session.url}/__rdesktop__/health`);assert.equal(response.status,401);
response=await fetch(`${session.url}/__rdesktop__/health`,{headers:{...headers,origin:'http://untrusted.example'}});assert.equal(response.status,403);
response=await fetch(`${session.url}/__rdesktop__/agent/evaluate`,{method:'POST',headers,body:JSON.stringify({expression:'x'.repeat(256*1024)})});assert.equal(response.status,413);
const before=await request('windows');assert.equal(before.windows.length,1);
const main=before.windows[0].id;assert.equal(typeof main,'string');
await until('document.readyState === "complete" && !!document.querySelector("#greet")',main);
assert.match((await request('dom',{window:main})).html,/Real desktop/);
const ipc=await request('ipc',{window:main,cmd:'greet',payload:{name:'Rust transport'}});
assert.equal(ipc.data.message,'Hello, Rust transport!');assert.equal(ipc.data.pid,session.pid);
assert.equal(await evaluate('Promise.resolve(42)',main),42);
const concurrent=await Promise.all(Array.from({length:20},async(_,index)=>{
 const response=await fetch(`${session.url}/__rdesktop__/agent/evaluate`,{method:'POST',headers,body:JSON.stringify({window:main,expression:`new Promise(resolve=>setTimeout(()=>resolve(${index}),400))`})});
 const value=await response.json();if(response.status===200)assert.equal(value,index);return response.status;
}));
assert(concurrent.includes(429));assert(concurrent.includes(200));assert(concurrent.every(status=>status===200||status===429));
await request('evaluate',{window:main,expression:'(()=>{throw Error("owned-agent-test")})()'},422);
await request('cdp',{window:main,method:'Invalid.noSuchMethod'},422);
await evaluate('document.addEventListener("click",e=>window.__lastClickTrusted=e.isTrusted)',main);
await request('action',{window:main,action:'fill',selector:'#name',value:'Agent 中文'});
await request('action',{window:main,action:'click',selector:'#greet'});
await until('document.querySelector("#result").textContent === "Hello, Agent 中文!"',main);
assert.equal(await evaluate('window.__lastClickTrusted',main),true);
assert.equal((await request('state',{window:main})).state.greetings,1);
const elements=await request('elements',{window:main,selector:'#name'});assert.equal(elements[0].value,'Agent 中文');assert(elements[0].visible);
await request('action',{window:main,action:'fill',selector:'#name',value:'Keyboard'});
await request('action',{window:main,action:'press',value:'Enter'});
await until('document.querySelector("#result").textContent === "Hello, Keyboard!"',main);
await request('action',{window:main,action:'select',selector:'#choice',value:'Dark'});
assert.equal(await evaluate('document.querySelector("#choice-result").textContent',main),'Dark');
const second=await request('ipc',{window:main,cmd:'second',payload:{}});
const auxiliary=second.data.window;assert.equal(typeof auxiliary,'string');
assert(BigInt(auxiliary)>=2n**63n);
await until('document.readyState === "complete" && !!document.querySelector("#name")',auxiliary);
await request('dom',{},422); // No implicit target when there are multiple windows.
await request('action',{window:auxiliary,action:'fill',selector:'#name',value:'Secondary'});
assert.equal(await evaluate('document.querySelector("#name").value',auxiliary),'Secondary');
assert.equal(await evaluate('document.querySelector("#name").value',main),'Keyboard');
const windows=(await request('windows')).windows;assert.equal(windows.length,2);
await request('window',{window:auxiliary,operation:'resize',width:700,height:520});
await until('innerWidth===700 && innerHeight===520',auxiliary);
await request('action',{window:auxiliary,action:'drag',selector:'#drag',target_selector:'#drop'});
await until('document.querySelector("#drag-result").textContent === "rdesktop-native-drag trusted=true"',auxiliary);
await request('action',{window:main,action:'select',selector:'#choice',value:'missing'},422);
await evaluate('document.querySelector("#name").readOnly=true',main);
await request('action',{window:main,action:'fill',selector:'#name',value:'must reject'},422);
await evaluate('document.querySelector("#name").readOnly=false',main);
await request('action',{window:main,action:'fill',selector:'h1',value:'must reject'},422);
for(const [name,window] of [['main',main],['secondary',auxiliary]]) {
    const capture=await fetch(`${session.url}/__rdesktop__/agent/screenshot`,{method:'POST',headers,body:JSON.stringify({window})});
    assert.equal(capture.status,200);assert.equal(capture.headers.get('content-type'),'image/png');
    const png=Buffer.from(await capture.arrayBuffer());assert.equal(png.subarray(1,4).toString(),'PNG');
    assert(png.length>1000);await writeFile(join(output,`${name}.png`),png);
}
const events=await request('events');assert(events.events.some(event=>event.method==='Runtime.consoleAPICalled' && event.window===main));
// HMR: change an owned fixture, then restore the exact original bytes. Both
// WebViews reload the real frontend while retaining their native IDs and PID.
const frontend=resolve('examples/native_agent/frontend/index.html');
const original=await readFile(frontend);
try {
    await delay(350);
    await writeFile(frontend,original.toString().replace('Development fixture.','Hot reload proof.'));
    await until('document.querySelector("#version")?.textContent.includes("Hot reload proof.")',main);
    await until('document.querySelector("#version")?.textContent.includes("Hot reload proof.")',auxiliary);
    assert.equal((await request('ipc',{window:main,cmd:'greet',payload:{}})).data.pid,session.pid);
} finally {await writeFile(frontend,original);}
await until('document.querySelector("#version")?.textContent.includes("Development fixture.")',main);
await request('window',{window:auxiliary,operation:'close'});
await until('true',main);
assert.equal((await request('windows')).windows.length,1);
await request('dom',{window:auxiliary},422);
const proof={mode:session.mode,pid:session.pid,main,auxiliary,client:before.windows[0].client,
    headless:!before.windows[0].visible,checks:['authentication','request-size-limit','bounded-concurrency-and-correlated-replies','native-rust-ipc','async-evaluation','errors','trusted-engine-input','keyboard-submit','select','invalid-option-rejection','readonly-and-noneditable-rejection','trusted-html-drag-data','window-isolation','u64-string-identity','resize','png','console-events','hot-reload-same-pid','close-invalidates-id']};
await writeFile(join(output,'proof.json'),JSON.stringify(proof,null,2));
console.log(JSON.stringify(proof,null,2));
