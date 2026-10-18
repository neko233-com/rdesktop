// Native child creation and style/visibility regression in the real WebView2 host.
// Run against examples/native_agent before verify-native-agent.mjs.
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
const [sessionPath,outputPath]=process.argv.slice(2);assert(sessionPath&&outputPath);
const session=JSON.parse(await readFile(sessionPath,'utf8')),output=resolve(outputPath);
await mkdir(output,{recursive:true});
async function call(command,params={}){
 const response=await fetch(`${session.url}/__rdesktop__/agent/${command}`,{method:'POST',headers:{authorization:`Bearer ${session.token}`,'content-type':'application/json'},body:JSON.stringify(params)});
 const value=await response.json();assert(response.ok,JSON.stringify(value));return value;
}
async function until(fn){const deadline=Date.now()+10000;while(Date.now()<deadline){const result=await fn();if(result)return result;await delay(40);}assert.fail('native child condition timed out');}
const windows=()=>call('windows').then(value=>value.windows);
const before=await windows();assert.equal(before.length,1);const main=before[0].id;
const checks=[];
const denied=await call('ipc',{window:main,cmd:'child',payload:{parent:'999999999'}});assert.equal(denied.success,false);
assert.equal((await windows()).length,1);checks.push('foreign-or-expired-parent-denied-without-orphan-window');
const reply=await call('ipc',{window:main,cmd:'child',payload:{parent:main}});assert(reply.success);const child=reply.data.window;
await until(()=>call('evaluate',{window:child,expression:'document.readyState==="complete"&&!!document.querySelector("#name")'}));
await call('evaluate',{window:child,expression:'document.querySelector("#name").value="native-child-233";document.title="Native child acceptance"'});
async function nested(){const tree=await call('uia',{window:main,max_nodes:3000});return tree.nodes.find(node=>node.automation_id==='name'&&node.value==='native-child-233');}
const hidden=!before[0].visible;
const field=hidden ? null : await until(nested);
if(hidden){
 const tree=await call('accessibility',{window:child});assert(tree.result.nodes.some(node=>node.role?.value==='textbox'));
 checks.push('hidden-native-child-exposes-browser-accessibility-with-parent-still-hidden');
}else checks.push('parent-native-accessibility-tree-contains-the-child-document');
for(let cycle=0;cycle<3;cycle++){
 await call('window',{window:child,operation:'hide'});
 await call('window',{window:child,operation:'show'});
 await call('window',{window:child,operation:'resize',width:600+cycle,height:400+cycle});
 if(!hidden){const retained=await until(nested);assert.deepEqual(retained.runtime_id,field.runtime_id);}
 else assert.equal((await windows()).find(window=>window.id===main).visible,false);
 assert.equal(await call('evaluate',{window:child,expression:'document.querySelector("#name").value'}),'native-child-233');
}
checks.push(hidden?'hidden-parent-stays-hidden-and-child-dom-survives-show-hide-resize':'repeated-show-hide-resize-keeps-native-child-parent-provider-and-dom');
await call('window',{window:child,operation:'close'});await until(async()=>(await windows()).length===1);
checks.push('owned-child-closes-without-closing-parent');
const proof={headless:!before[0].visible,checks};await writeFile(join(output,'proof.json'),JSON.stringify(proof,null,2));console.log(JSON.stringify(proof));
