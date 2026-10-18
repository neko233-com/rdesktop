// Actual Windows system-menu commands; the fixture records OS GUI state before
// cleanup. No browser input or synthetic DOM event stands in for that state.
import assert from 'node:assert/strict';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {resolve,join,relative,isAbsolute} from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
const [sessionFile,destination]=process.argv.slice(2);assert(sessionFile&&destination);
const output=resolve(destination),outside=relative(process.cwd(),output);assert(isAbsolute(outside)||outside==='..'||outside.startsWith('..\\')||outside.startsWith('../'));await mkdir(output,{recursive:true});
const session=JSON.parse(await readFile(sessionFile,'utf8')),headers={authorization:'Bearer '+session.token,'content-type':'application/json'};
async function call(command,params={}){const response=await fetch(session.url+'/__rdesktop__/agent/'+command,{method:'POST',headers,body:JSON.stringify(params),signal:AbortSignal.timeout(12000)});const value=await response.json();assert.equal(response.status,200,JSON.stringify(value));return value;}
const main=(await call('windows')).windows.find(w=>w.title==='rdesktop Native Agent');assert(main);
const created=await call('ipc',{window:main.id,cmd:'second',payload:{decorations:false},timeout_ms:5000});assert(created.success);const window=created.data.window;
const req=(command,params={})=>call(command,{window,...params}),evaluate=expression=>req('evaluate',{expression});
try{
 const deadline=Date.now()+10000;while(!await evaluate('document.readyState==="complete"&&typeof window.__RDESKTOP_INVOKE__==="function"')){assert(Date.now()<deadline);await delay(30);}
 await req('window',{operation:'focus'});
 const probe=async kind=>{const r=await req('ipc',{cmd:'probe_key_menu',payload:{kind},timeout_ms:5000});assert(r.success,JSON.stringify(r));return r.data;};
 const bare=await probe('bare');await writeFile(join(output,'bare.json'),JSON.stringify(bare,null,2));assert(bare.menuClosed,'negative control cleanup must close its native menu');assert.equal(bare.enteredMenuMode,false,'bare Alt must not enter an invisible frameless menu loop');
 assert.equal(await evaluate('new Promise(resolve=>setTimeout(()=>resolve(42),40))'),42);
 const space=await probe('space');await writeFile(join(output,'space.json'),JSON.stringify(space,null,2));assert(space.enteredMenuMode,'Alt+Space system-menu command must remain available');assert(space.menuClosed);
 assert.equal(await evaluate('1+1'),2);
 const response=await fetch(session.url+'/__rdesktop__/agent/native-screenshot',{method:'POST',headers,body:JSON.stringify({window}),signal:AbortSignal.timeout(12000)});assert(response.ok);await writeFile(join(output,'native.png'),Buffer.from(await response.arrayBuffer()));
 const proof={platform:'windows-x64',bare,space,checks:['bare-alt-does-not-enter-empty-frameless-menu-loop','async-native-cdp-remains-responsive','alt-space-native-system-menu-command-preserved','native-menu-cancel-cleanup','windows-compositor-capture']};await writeFile(join(output,'proof.json'),JSON.stringify(proof,null,2));console.log(JSON.stringify(proof));
}finally{if((await call('windows')).windows.some(w=>w.id===window))await req('window',{operation:'close'});}
