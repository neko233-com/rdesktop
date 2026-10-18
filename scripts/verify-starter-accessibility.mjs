// Actual generated-project / Hello World native host, including real Rust IPC.
// node scripts/verify-starter-accessibility.mjs SESSION OUTPUT AXE_SOURCE CLI_EXE FRONTEND_FILE
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {join,resolve} from 'node:path';
import {spawnSync} from 'node:child_process';
import {randomUUID} from 'node:crypto';
import {setTimeout as delay} from 'node:timers/promises';
const [sessionPath,directory,axeSource,cliPath,frontendPath]=process.argv.slice(2);
assert(sessionPath&&directory&&axeSource&&cliPath&&frontendPath);
const session=JSON.parse(await readFile(sessionPath,'utf8')),output=resolve(directory);
await mkdir(output,{recursive:true});
const headers={authorization:`Bearer ${session.token}`,'content-type':'application/json'};
async function call(command,params={},expected=200){
    const response=await fetch(`${session.url}/__rdesktop__/agent/${command}`,{method:'POST',headers,body:JSON.stringify(params)}),value=await response.json();
    assert.equal(response.status,expected,JSON.stringify(value));return value;
}
let inventory=(await call('windows')).windows;
const startupDeadline=Date.now()+10000;
while(inventory.length===0&&Date.now()<startupDeadline){await delay(30);inventory=(await call('windows')).windows;}
assert.equal(inventory.length,1);
const main=inventory[0],window=main.id,checks=[];
assert.equal(main.process_id,session.pid);assert.equal(typeof window,'string');
const req=(command,params={},expected=200)=>call(command,{window,...params},expected);
const evaluate=expression=>req('evaluate',{expression});
async function until(expression){const deadline=Date.now()+10000;while(Date.now()<deadline){if(await evaluate(expression))return;await delay(30);}assert.fail(`timed out: ${expression}`);}
await until('document.readyState==="complete"&&!!document.querySelector("#greet-form")');
const viewport=await evaluate('({width:innerWidth,height:innerHeight})');
async function capture(name){const response=await fetch(`${session.url}/__rdesktop__/agent/screenshot`,{method:'POST',headers,body:JSON.stringify({window})});assert.equal(response.status,200);const png=Buffer.from(await response.arrayBuffer());assert.equal(png.subarray(1,4).toString(),'PNG');await writeFile(join(output,`${name}.png`),png);}
async function audit(name){const file=join(output,`${name}-${randomUUID()}.json`),run=spawnSync(resolve(cliPath),['agent','--session',resolve(sessionPath),'audit','--window',window,'--axe-source',resolve(axeSource),'--output',file],{encoding:'utf8',windowsHide:true});assert.equal(run.status,0,run.stderr);const result=JSON.parse(await readFile(file,'utf8'));assert(result.passes.length);assert.equal(result.violations.length,0,JSON.stringify(result.violations.map(item=>({id:item.id,targets:item.nodes.map(node=>node.target)}))));assert.deepEqual(result.incomplete,[]);return{passes:result.passes.length,violations:0,incomplete:0};}
const ax=await req('accessibility');
assert(ax.result.nodes.some(node=>node.role?.value==='main'));
assert(ax.result.nodes.some(node=>node.role?.value==='textbox'&&node.name?.value==='Your name'));
const status=ax.result.nodes.find(node=>node.role?.value==='status');assert(status);
assert(status.properties.some(property=>property.name==='live'&&property.value?.value==='polite'));
assert.equal(await evaluate('document.querySelector("label[for=nameInput]").textContent'),'Your name');
checks.push('computed-persistent-label-main-landmark-and-live-status');
let native=await req('uia');assert.equal(native.source,'windows-uia');
await writeFile(join(output,'native-initial.json'),JSON.stringify(native,null,2));
let input=native.nodes.find(node=>node.name==='Your name'&&node.control_type===50004);
const providerObservations=[{nodes:native.nodes.length,inputAvailable:!!input}],providerStarted=Date.now();
if(main.visible){
    // Document readiness and engine AX do not prove that Windows has published
    // its cross-process provider yet. Wait only on real UIA, preserving the
    // first and final snapshots; provider errors still fail immediately.
    while(!input&&Date.now()-providerStarted<10000){
        await delay(50);native=await req('uia');assert.equal(native.source,'windows-uia');
        input=native.nodes.find(node=>node.name==='Your name'&&node.control_type===50004);
        providerObservations.push({elapsedMs:Date.now()-providerStarted,nodes:native.nodes.length,inputAvailable:!!input});
    }
    await writeFile(join(output,'native-ready.json'),JSON.stringify(native,null,2));
    assert(input?.runtime_id,JSON.stringify({providerObservations}));
    await req('uia',{operation:'set-value',target:{runtime_id:input.runtime_id},value:'Native label 中文'});await until('document.querySelector("#nameInput").value==="Native label 中文"');checks.push('native-uia-labelled-input-value');
}
else{assert.equal((await call('windows')).windows[0].visible,false);checks.push('hidden-provider-availability-reported-without-showing-window');}
await req('action',{action:'fill',selector:'#nameInput',value:'Keyboard 中文'});
await req('action',{action:'press',value:'Enter'});
await until('document.querySelector("#result").textContent==="Hello, Keyboard 中文!"');
checks.push('real-keyboard-enter-rust-greeting');
await req('action',{action:'fill',selector:'#nameInput',value:''});
await req('action',{action:'press',value:'Enter'});
await until('document.querySelector("#result").textContent==="Hello, World!"');
checks.push('blank-name-defaults-through-real-rust-handler');

// A negative control routes the UI's greeting through the real Rust handler's
// unknown-command response. It never substitutes a successful mock response.
await evaluate('window.__starterOriginalInvoke=window.__RDESKTOP_INVOKE__;window.__RDESKTOP_INVOKE__=(_command,payload)=>window.__starterOriginalInvoke("owned-starter-unknown-command",payload)');
try{await req('action',{action:'press',value:'Enter'});await until('document.querySelector("#result").textContent==="Error: Unknown command"');}
finally{await evaluate('window.__RDESKTOP_INVOKE__=window.__starterOriginalInvoke;delete window.__starterOriginalInvoke');}
checks.push('actual-rust-error-envelope-is-announced-as-error');
await evaluate('window.__starterOriginalInvoke=window.__RDESKTOP_INVOKE__;window.__RDESKTOP_INVOKE__=undefined');
try{await req('action',{action:'press',value:'Enter'});await until('document.querySelector("#result").textContent==="Error: rdesktop bridge unavailable"');}
finally{await evaluate('window.__RDESKTOP_INVOKE__=window.__starterOriginalInvoke;delete window.__starterOriginalInvoke');}
await req('action',{action:'press',value:'Enter'});await until('document.querySelector("#result").textContent==="Hello, World!"');
checks.push('missing-bridge-failure-and-exact-bridge-restoration');
const audits={desktop:await audit('desktop')};await capture('desktop');
checks.push('complete-default-axe-audit');

try{
    await req('window',{operation:'resize',width:360,height:640});
    await until('innerWidth===360&&innerHeight===640');
    assert.equal(await evaluate('document.documentElement.scrollWidth<=document.documentElement.clientWidth'),true);
    audits.narrow=await audit('narrow');await capture('narrow');
    checks.push('native-narrow-layout-without-horizontal-overflow');
    await req('window',{operation:'zoom',factor:2});
    await until('innerWidth===180');
    assert.equal(await evaluate('document.documentElement.scrollWidth<=document.documentElement.clientWidth'),true);
    audits.zoom=await audit('zoom');await capture('zoom');
    checks.push('native-200-percent-zoom-at-narrow-width');
}finally{await req('window',{operation:'zoom',factor:1});await req('window',{operation:'resize',...viewport});}
try{
    await req('cdp',{method:'Emulation.setEmulatedMedia',params:{features:[{name:'prefers-reduced-motion',value:'reduce'}]}});
    assert.equal(await evaluate('matchMedia("(prefers-reduced-motion:reduce)").matches&&getComputedStyle(document.querySelector("button")).transitionDuration==="0s"'),true);
    checks.push('actual-reduced-motion-style');
    await req('cdp',{method:'Emulation.setEmulatedMedia',params:{features:[{name:'forced-colors',value:'active'}]}});
    assert.equal(await evaluate('matchMedia("(forced-colors:active)").matches'),true);
    audits.forcedColors=await audit('forced-colors');await capture('forced-colors');
    checks.push('forced-colors-rendering-and-full-axe-audit');
}finally{await req('cdp',{method:'Emulation.setEmulatedMedia',params:{features:[]}});}

const original=await readFile(frontendPath),marker='<h1>Hello rdesktop!</h1>';
assert(original.includes(marker));
try{
    await writeFile(frontendPath,original.toString().replace(marker,'<h1>Hello rdesktop HMR!</h1>'));
    await until('document.querySelector("h1")?.textContent==="Hello rdesktop HMR!"');
    const updated=(await call('windows')).windows;assert.equal(updated.length,1);assert.equal(updated[0].id,window);assert.equal(updated[0].process_id,session.pid);
}finally{await writeFile(frontendPath,original);}
await until('document.querySelector("h1")?.textContent==="Hello rdesktop!"');
checks.push('actual-frontend-hot-reload-keeps-native-id-and-pid');
await req('action',{action:'press',value:'Tab'});
await until('document.activeElement.classList.contains("skip")');
assert.equal(await evaluate('document.activeElement.getBoundingClientRect().top>=0'),true);
await req('action',{action:'press',value:'Enter'});await until('document.activeElement.id==="main"');
checks.push('keyboard-visible-skip-link-and-main-focus');
const events=(await req('events')).events;
assert(!events.some(event=>event.method==='Runtime.exceptionThrown'),JSON.stringify(events.filter(event=>event.method==='Runtime.exceptionThrown')));
assert.equal((await call('windows')).windows[0].visible,main.visible);
checks.push('no-uncaught-errors-or-visibility-changes');
const proof={mode:session.mode,headless:!main.visible,uiaNodes:native.nodes.length,nativeInputAvailable:!!input,providerObservations,audits,checks};
await writeFile(join(output,'proof.json'),JSON.stringify(proof,null,2));console.log(JSON.stringify(proof,null,2));
