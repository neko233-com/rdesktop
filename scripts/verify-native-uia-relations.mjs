// Real UIA relationships and description properties, independently of engine AX.
// node scripts/verify-native-uia-relations.mjs SESSION OUTPUT
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
const [sessionPath,directory]=process.argv.slice(2);assert(sessionPath&&directory);
const session=JSON.parse(await readFile(sessionPath,'utf8')),output=resolve(directory);await mkdir(output,{recursive:true});
const headers={authorization:`Bearer ${session.token}`,'content-type':'application/json'};
async function call(command,params={},expected=200){const response=await fetch(`${session.url}/__rdesktop__/agent/${command}`,{method:'POST',headers,body:JSON.stringify(params)});const value=await response.json();assert.equal(response.status,expected,JSON.stringify(value));return value;}
const main=(await call('windows')).windows.find(item=>item.title==='rdesktop Native Agent');assert(main);
const window=main.id,req=(command,params={},expected=200)=>call(command,{window,...params},expected);
const evaluate=expression=>req('evaluate',{expression});
const target=(name,control_type=50004)=>({name,control_type});
const uia=(operation,target,params={},expected=200)=>req('uia',{operation,target,...params},expected);
const related=(target,relation,params={},expected=200)=>uia('relations',target,{relation,...params},expected);
async function until(read,accept){const end=Date.now()+8000;while(Date.now()<end){const value=await read();if(accept(value))return value;await delay(40);}assert.fail('provider did not publish expected current state');}
const info=await fetch(`${session.url}/__rdesktop__/info`,{headers});assert.equal(info.status,200);
const capability=(await info.json()).capabilities.uia;assert(capability.operations.includes('relations'));
assert.deepEqual(capability.relations,['described-by','labeled-by','controller-for','flows-to','flows-from']);
assert(capability.description_properties.includes('full_description'));
const base=new URL('.',main.url).href,fixture=new URL('accessibility.html',base).href;
await req('cdp',{method:'Page.navigate',params:{url:fixture}});
await until(()=>evaluate('document.readyState==="complete"&&!!document.querySelector("#email")'),Boolean);
const initial=await req('uia',{max_nodes:1500}),checks=[],evidence={};
const email=initial.nodes.find(node=>node.automation_id==='email'&&node.control_type===50004);
const badTarget={runtime_id:[42,-2147483647,777]};
for(const relation of [null,'','desktop',1])await related(badTarget,relation,{},422);
for(const max_items of [0,257,1.5,'2'])await related(badTarget,'flows-to',{max_items},422);
await uia('relations',badTarget,{},422);
await related(badTarget,'flows-to',{value:'must not be ignored'},422);
checks.push('invalid-relations-and-bounds-rejected');
if(!main.visible&&!email){
 for(const relation of capability.relations)await related(target('Email'),relation,{},422);
 assert.equal((await call('windows')).windows.find(item=>item.id===window).visible,false);
 checks.push('hidden-provider-unavailable-without-showing-or-dom-fallback');
}else{
 assert(email?.runtime_id);assert.equal(email.full_description,'Enter a valid email address.');
 const inspected=(await uia('inspect',{runtime_id:email.runtime_id})).element;
 assert.equal(inspected.full_description,email.full_description);assert.equal(inspected.help_text,email.help_text);
 evidence.description={help_text:email.help_text,full_description:email.full_description};
 checks.push('raw-native-full-description-in-snapshot-and-inspect');
 // Prove describedby actually contributes to the native property. Using only
 // this association avoids duplicate descriptions in the tested NVDA build.
 try{
  await evaluate('document.querySelector("#email").removeAttribute("aria-describedby")');
  await until(()=>uia('inspect',target('Email')),value=>value.element.full_description==='');
 }finally{await evaluate('document.querySelector("#email").setAttribute("aria-describedby","email-error")');}
 await until(()=>uia('inspect',target('Email')),value=>value.element.full_description==='Enter a valid email address.');
 checks.push('description-negative-control-and-live-restoration');
 const label=await related(target('Email'),'labeled-by');assert.equal(label.related.total,1);assert.equal(label.related.elements[0].name,'Email');assert.equal(label.related.elements[0].control_type,50020);
 assert.equal((await uia('inspect',{runtime_id:label.related.elements[0].runtime_id})).element.name,'Email');
 evidence.label=label;checks.push('actual-native-label-relationship-and-returned-identity');
 const described=await related(target('Email'),'described-by');assert.equal(described.source,'windows-uia');
 // Chromium may publish descriptions only as FullDescription, leaving its
 // DescribedBy element array empty. Preserve that exact provider distinction.
 evidence.describedBy=described;checks.push('raw-described-by-provider-result');
 try{
  await evaluate('document.querySelector("#email").setAttribute("aria-flowto","display-name volume secret");document.querySelector("#disclosure").click()');
  const forward=await until(()=>related(target('Email'),'flows-to'),value=>value.related.total===3);
  assert.deepEqual(forward.related.elements.map(item=>item.automation_id),['display-name','volume','secret']);
  const bounded=await related(target('Email'),'flows-to',{max_items:1});assert.equal(bounded.related.total,3);assert.equal(bounded.related.elements.length,1);assert.equal(bounded.related.truncated,true);
  const reverse=await related(target('Display name'),'flows-from');assert.deepEqual(reverse.related.elements.map(item=>item.automation_id),['email']);
  const controlled=await related(target('Show details',50000),'controller-for');assert.deepEqual(controlled.related.elements.map(item=>item.automation_id),['details']);
  const secret=forward.related.elements.find(item=>item.automation_id==='secret');assert.equal(secret.password,true);assert(!Object.hasOwn(secret,'value'));assert(!Object.hasOwn(secret,'text'));
  assert(!JSON.stringify(forward).includes('owned-test-secret'));
  evidence.flow={forward,bounded,reverse,controlled};
  checks.push('native-forward-and-reverse-flow','bounded-relation-arrays-with-total','native-controller-relationship','password-related-descriptor-without-value-or-text');
 }finally{await evaluate('document.querySelector("#email").removeAttribute("aria-flowto");if(document.querySelector("#disclosure").getAttribute("aria-expanded")==="true")document.querySelector("#disclosure").click()');}
 await until(()=>related(target('Email'),'flows-to'),value=>value.related.total===0);
 for(const relation of ['labeled-by','described-by','controller-for','flows-to','flows-from'])assert.deepEqual((await related(target('Save preferences',50000),relation)).related,{elements:[],max_items:32,total:0,truncated:false});
 checks.push('successful-null-and-empty-relations-remain-successful');
 const created=await req('ipc',{cmd:'second',payload:{}}),auxiliary=created.data.window;
 try{
  await call('cdp',{window:auxiliary,method:'Page.navigate',params:{url:fixture}});
  const foreign=await until(async()=> (await call('uia',{window:auxiliary})).nodes.find(node=>node.automation_id==='email'&&node.control_type===50004),node=>!!node?.runtime_id);
  await related({runtime_id:foreign.runtime_id},'labeled-by',{},422);
  assert.equal((await related(target('Email'),'labeled-by')).related.total,1);
 }finally{await call('window',{window:auxiliary,operation:'close'});}
 await call('uia',{window:auxiliary,operation:'relations',target:target('Email'),relation:'labeled-by'},422);
 await evaluate('document.querySelector("#email").remove()');
 await related({runtime_id:email.runtime_id},'labeled-by',{},422);
 checks.push('foreign-closed-window-and-stale-element-rejection');
}
await writeFile(join(output,'native-relations.json'),JSON.stringify(evidence,null,2));
const proof={headless:!main.visible,providerAvailable:!!email,uiaNodes:initial.nodes.length,checks};
await writeFile(join(output,'proof.json'),JSON.stringify(proof,null,2));console.log(JSON.stringify(proof,null,2));
await req('cdp',{method:'Page.navigate',params:{url:base}});
