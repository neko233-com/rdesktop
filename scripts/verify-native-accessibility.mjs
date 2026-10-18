// Native WebView2 AX, Windows UIA, keyboard and WCAG audit integration.
// node scripts/verify-native-accessibility.mjs <session.json> <artifact-dir>
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {spawnSync} from 'node:child_process';
import {setTimeout as delay} from 'node:timers/promises';
const [sessionPath,outputPath]=process.argv.slice(2);assert(sessionPath&&outputPath);
const session=JSON.parse(await readFile(sessionPath,'utf8'));const output=resolve(outputPath);await mkdir(output,{recursive:true});
const headers={authorization:`Bearer ${session.token}`,'content-type':'application/json'};
async function call(command,params={},expected=200){const response=await fetch(`${session.url}/__rdesktop__/agent/${command}`,{method:'POST',headers,body:JSON.stringify(params)});const value=await response.json();assert.equal(response.status,expected,JSON.stringify(value));return value;}
const windows=await call('windows');assert.equal(windows.windows.length,1);const window=windows.windows[0].id;const headless=!windows.windows[0].visible;
const req=(command,params={},expected=200)=>call(command,{window,...params},expected);
const evaluate=expression=>req('evaluate',{expression});
const action=params=>req('action',params);
async function until(expression){const end=Date.now()+8000;while(Date.now()<end){if(await evaluate(expression))return;await delay(40);}assert.fail(`timed out: ${expression}`);}
const base=windows.windows[0].url;
await req('cdp',{method:'Page.navigate',params:{url:new URL('accessibility.html',base).href}});
await until('document.readyState==="complete"&&!!document.querySelector("#save")');
const tree=async()=> (await req('accessibility')).result.nodes;
const find=(nodes,role,name)=>nodes.find(node=>!node.ignored&&node.role?.value===role&&node.name?.value===name);
const property=(node,name)=>node.properties?.find(item=>item.name===name)?.value?.value;
let nodes=await tree();assert(find(nodes,'heading','Native accessibility'));
const textbox=find(nodes,'textbox','Display name');assert(textbox);assert.equal(property(textbox,'required'),true);
assert.equal(find(nodes,'textbox','Email').description?.value, 'Enter a valid email address.');
assert.equal(property(find(nodes,'textbox','Email'),'invalid'),'true');
assert(find(nodes,'checkbox','Receive notifications'));assert(find(nodes,'slider','Volume'));assert(find(nodes,'table','Compatibility examples'));
assert.equal(property(find(nodes,'button','Unavailable action'),'disabled'),true);
await req('action',{action:'click',role:'button',name:'Unavailable action'},422);
await req('action',{action:'click',role:'button',name:'Unavailable custom action'},422);
await req('action',{action:'click',role:'button',name:'Hidden action'},422);
// Focus must not click/toggle the control.
await action({action:'focus',role:'checkbox',name:'Receive notifications'});
assert.equal(await evaluate('document.querySelector("#notifications").checked'),false);
await action({action:'press',value:' '});assert.equal(await evaluate('document.querySelector("#notifications").checked'),true);
assert.equal(property(find(await tree(),'checkbox','Receive notifications'),'checked'),'true');
await action({action:'focus',role:'textbox',name:'Display name'});
await action({action:'fill',role:'textbox',name:'Display name',value:'Accessible agent'});
await action({action:'press',value:'Tab'});assert.equal(await evaluate('document.activeElement.id'),'email');
await action({action:'press',value:'Tab',modifiers:8});assert.equal(await evaluate('document.activeElement.id'),'display-name');
await action({action:'click',role:'button',name:'Save preferences'});
await until('document.querySelector("#status").textContent==="Preferences saved locally"');
assert.equal(property(find(await tree(),'status',''),'live'),'polite');
await action({action:'click',role:'button',name:'Show details'});
assert.equal(property(find(await tree(),'button','Show details'),'expanded'),true);
await action({action:'click',role:'button',name:'Open preferences dialog'});
await until('document.querySelector("#dialog").open');
assert.equal(property(find(await tree(),'dialog','Preferences dialog'),'modal'),true);
assert.equal(await evaluate('document.activeElement.id'),'close-dialog');
await action({action:'press',value:'Escape'});
await until('!document.querySelector("#dialog").open');
assert.equal(await evaluate('document.activeElement.id'),'open-dialog');
await action({action:'focus',role:'tab',name:'First view'});await action({action:'press',value:'ArrowRight'});
assert.equal(property(find(await tree(),'tab','Second view'),'selected'),true);assert.equal(await evaluate('document.activeElement.id'),'tab-two');
await action({action:'focus',role:'treeitem',name:'Projects'});await action({action:'press',value:'ArrowRight'});
assert.equal(property(find(await tree(),'treeitem','Projects'),'expanded'),true);
await action({action:'focus',role:'slider',name:'Volume'});await action({action:'press',value:'ArrowRight'});
assert.equal(find(await tree(),'slider','Volume').value?.value,51);
await action({action:'click',role:'button',name:'Shadow save'});await until('document.querySelector("#status").textContent==="Shadow action completed"');
// Complete Accessibility domain entry points, retaining relationships/sources.
nodes=await tree();const button=find(nodes,'button','Save preferences');assert(button.backendDOMNodeId);
assert((await req('accessibility',{method:'getRootAXNode'})).result.node);
assert((await req('accessibility',{method:'getPartialAXTree',params:{backendNodeId:button.backendDOMNodeId}})).result.nodes.length);
assert((await req('accessibility',{method:'getAXNodeAndAncestors',params:{backendNodeId:button.backendDOMNodeId}})).result.nodes.length);
const root=(await req('accessibility',{method:'getRootAXNode'})).result.node;
assert((await req('accessibility',{method:'getChildAXNodes',params:{id:root.nodeId}})).result.nodes.length);
const frames=await req('cdp',{method:'Page.getFrameTree'});const child=frames.frameTree.childFrames[0].frame.id;
assert(find((await req('accessibility',{params:{frameId:child}})).result.nodes,'button','Frame action'));
const dom=await req('cdp',{method:'DOM.getDocument',params:{depth:0}});
assert((await req('accessibility',{method:'queryAXTree',params:{nodeId:dom.root.nodeId,role:'button',accessibleName:'Save preferences'}})).result.nodes.length);
// OS accessibility is independently rooted in the same real native HWND.
const uia=await req('uia',{max_nodes:1000});assert.equal(uia.source,'windows-uia');assert(uia.nodes.length);
if(!headless){assert(uia.nodes.some(node=>node.name==='Native accessibility'));assert(uia.nodes.some(node=>node.name==='Save preferences'));assert.equal(uia.nodes.find(node=>node.automation_id==='email'&&node.control_type===50004)?.full_description,'Enter a valid email address.');}
for(const node of uia.nodes.filter(node=>node.password)){assert.equal(node.value,null);}
await writeFile(join(output,'windows-uia.json'),JSON.stringify(uia,null,2));
await writeFile(join(output,'engine-ax.json'),JSON.stringify(await req('accessibility'),null,2));
// Real WebView zoom, rather than a separate browser's device emulation.
await req('window',{operation:'zoom',factor:2});await until('innerWidth<500');
assert.equal(await evaluate('document.documentElement.scrollWidth<=innerWidth'),true);
await req('window',{operation:'zoom',factor:1});await until('innerWidth>=800');
await req('cdp',{method:'Emulation.setEmulatedMedia',params:{features:[{name:'prefers-reduced-motion',value:'reduce'},{name:'forced-colors',value:'active'}]}});
assert.equal(await evaluate('matchMedia("(prefers-reduced-motion: reduce)").matches && matchMedia("(forced-colors: active)").matches'),true);
await req('cdp',{method:'Emulation.setEmulatedMedia',params:{features:[]}});
// The CLI loads a pinned official axe engine locally; it sends no page data to
// an external audit service. All standard rules are enabled by default.
const runAudit=async(name)=>{
 const file=join(output,`${name}-${Date.now()}-${process.pid}.json`);
 const run=spawnSync(resolve('target/debug/rdesktop.exe'),['agent','--session',resolve(sessionPath),'audit','--window',window,'--axe-source',resolve('node_modules/axe-core/axe.min.js'),'--output',file],{encoding:'utf8',windowsHide:true});
 assert.equal(run.status,0,run.stderr);return JSON.parse(await readFile(file,'utf8'));
};
const audit=await runAudit('axe-positive');assert.equal(audit.engine.name,'axe-core');assert.equal(audit.engine.version,'4.14.0');
assert.equal(audit.violations.length,0,JSON.stringify(audit.violations.map(v=>({id:v.id,nodes:v.nodes.map(n=>n.target)}))));assert(audit.passes.length>10);assert.equal(audit.requiresManualReview,true);
// Negative controls prove that a quiet report is not a bypassed audit.
try{
 await evaluate('(()=>{const bad=document.createElement("button");bad.id="owned-bad-button";bad.style.cssText="width:40px;height:40px";document.querySelector("main").append(bad);})()');
 const negative=await runAudit('axe-negative');assert(negative.violations.some(v=>v.id==='button-name'));
}finally{await evaluate('document.querySelector("#owned-bad-button")?.remove()');}
const events=await req('events');assert(events.events.some(event=>event.method.startsWith('Accessibility.')));
const proof={mode:session.mode,pid:session.pid,headless,axNodes:(await tree()).length,uiaNodes:uia.nodes.length,uiaTruncated:uia.truncated,
 axeVersion:audit.engine.version,axePasses:audit.passes.length,axeViolations:audit.violations.length,manualReview:audit.incomplete.map(rule=>rule.id),
 checks:['computed-name-role-state','disabled-hidden-rejection','focus-without-click','checkbox-space','tab-shift-tab','live-region','expanded-state','modal-escape-focus-return','tab-arrows','tree-expansion','range-keyboard','shadow-role-target','all-accessibility-methods','frame-ax','windows-uia','password-redaction','native-200-percent-zoom','media-preferences','axe-positive-and-negative-controls','accessibility-events']};
await writeFile(join(output,'proof.json'),JSON.stringify(proof,null,2));console.log(JSON.stringify(proof,null,2));
await req('cdp',{method:'Page.navigate',params:{url:base}});
