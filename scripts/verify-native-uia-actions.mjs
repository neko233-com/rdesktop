// Real UIA patterns only. DOM evaluation observes the app's resulting state;
// no CDP input, synthetic clicks, browser or mock provider performs actions.
// node scripts/verify-native-uia-actions.mjs SESSION.json ARTIFACT_DIRECTORY
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {join,resolve} from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
const [sessionPath,directory]=process.argv.slice(2);assert(sessionPath&&directory);
const session=JSON.parse(await readFile(sessionPath,'utf8'));const output=resolve(directory);await mkdir(output,{recursive:true});
const headers={authorization:`Bearer ${session.token}`,'content-type':'application/json'};
async function call(command,params={},expected=200){const response=await fetch(`${session.url}/__rdesktop__/agent/${command}`,{method:'POST',headers,body:JSON.stringify(params)});const value=await response.json();assert.equal(response.status,expected,JSON.stringify(value));return value;}
const main=(await call('windows')).windows.find(window=>window.title==='rdesktop Native Agent');assert(main);
const infoResponse=await fetch(`${session.url}/__rdesktop__/info`,{headers});assert.equal(infoResponse.status,200);
const info=await infoResponse.json();assert(info.capabilities.uia.operations.includes('text-select'));assert(info.capabilities.uia.operations.includes('resize'));
const deniedGet=await fetch(`${session.url}/__rdesktop__/agent/uia?window=${main.id}&operation=invoke`,{headers});assert.equal(deniedGet.status,405);
const window=main.id;const req=(command,params={},expected=200)=>call(command,{window,...params},expected);
const evaluate=expression=>req('evaluate',{expression});const uia=(operation,target,params={},expected=200)=>req('uia',{operation,target,...params},expected);
const target=(name,control_type)=>({name,control_type});const checks=[];
async function until(expression){const end=Date.now()+8000;while(Date.now()<end){if(await evaluate(expression))return;await delay(30);}assert.fail(`timed out: ${expression}`);}
async function nativeNode(predicate,present=true){const end=Date.now()+8000;while(Date.now()<end){const node=(await req('uia')).nodes.find(predicate);if(Boolean(node)===present)return node;await delay(30);}assert.fail('UIA provider did not publish the expected control lifetime');}
const base=main.url;await req('cdp',{method:'Page.navigate',params:{url:new URL('uia.html',base).href}});
await until('document.readyState==="complete"&&!!window.fixture');
const initial=await req('uia');assert.equal(initial.source,'windows-uia');assert.equal(initial.truncated,false);
assert(initial.nodes.every(node=>node.runtime_id===null||Array.isArray(node.runtime_id)));
await writeFile(join(output,'initial-uia.json'),JSON.stringify(initial,null,2));
if(!main.visible){
 const available=initial.nodes.some(node=>node.name==='Save preferences'&&node.control_type===50000);
 const response=await uia('invoke',target('Save preferences',50000),{},available?200:422);
 if(available){assert.equal(response.dispatched,true);await until('fixture.saves===1');}
 assert.equal((await call('windows')).windows.find(item=>item.id===window).visible,false);
 checks.push('hidden-provider-availability-reported-without-showing-window');
 await writeFile(join(output,'proof.json'),JSON.stringify({pid:session.pid,headless:true,uiaNodes:initial.nodes.length,available,checks},null,2));
 console.log(JSON.stringify({headless:true,uiaNodes:initial.nodes.length,available,checks}));
 await req('cdp',{method:'Page.navigate',params:{url:base}});process.exit(0);
}
const edit=target('Display name',50004),checkbox=target('Receive notifications',50002),volume=target('Volume',50015);
await uia('set-scroll-percent',target('Native UIA action fixture',50030),{vertical_percent:0});
await uia('focus',edit);await until('document.activeElement.id==="display-name"');
await uia('set-value',edit,{value:'UIA Agent 中文'});await until('document.querySelector("#display-name").value==="UIA Agent 中文"');
assert.equal((await uia('inspect',edit)).element.states.value.value,'UIA Agent 中文');checks.push('value-and-focus');
await uia('set-value',target('Read-only sample',50004),{value:'must remain fixed'},422);
assert.equal(await evaluate('document.querySelector("#readonly").value'),'Fixed value');checks.push('readonly-rejection');
await uia('focus',checkbox);assert.equal(await evaluate('document.querySelector("#notifications").checked'),false);
await uia('toggle',checkbox);await until('document.querySelector("#notifications").checked');
assert.equal((await uia('inspect',checkbox)).element.states.toggle,1);checks.push('focus-without-toggle-and-native-toggle');
await uia('set-range-value',volume,{value:73});await until('document.querySelector("#volume").value==="73"');
assert.equal((await uia('inspect',volume)).element.states.range.value,73);
await uia('set-range-value',volume,{value:101},422);await uia('set-range-value',volume,{value:'73'},422);
assert.equal(await evaluate('document.querySelector("#volume").value'),'73');checks.push('range-value-and-bounds');
await uia('select',target('Remote storage',50013));await until('document.querySelector("#remote").checked');
assert.equal((await uia('inspect',target('Remote storage',50013))).element.states.selected,true);
await uia('select',target('Apple',50007));await uia('add-to-selection',target('Banana',50007));
assert.deepEqual(await evaluate('Array.from(document.querySelector("#multi").selectedOptions,option=>option.textContent)'),['Apple','Banana']);
await uia('remove-from-selection',target('Apple',50007));
assert.deepEqual(await evaluate('Array.from(document.querySelector("#multi").selectedOptions,option=>option.textContent)'),['Banana']);checks.push('selection-item-add-remove');
await uia('expand',target('Show details',50000));await until('!document.querySelector("#details").hidden');
assert.equal((await uia('inspect',target('Show details',50000))).element.states.expand_collapse,1);
await uia('collapse',target('Show details',50000));await until('document.querySelector("#details").hidden');checks.push('expand-collapse');
await uia('invoke',target('Save preferences',50000));await until('fixture.saves===1');
await uia('legacy-default-action',target('Save preferences',50000));await until('fixture.saves===2');checks.push('invoke-and-legacy-default');
await uia('invoke',target('Unavailable action',50000),{},422);await uia('invoke',target('Unavailable custom action',50000),{},422);
await uia('scroll-into-view',target('Unavailable action',50000));
await uia('toggle',target('Save preferences',50000),{},422);assert.equal(await evaluate('fixture.saves'),2);checks.push('disabled-and-unsupported-pattern-rejection');
await uia('realize',edit,{},422);await uia('multiple-view-set',edit,{value:1},422);
await uia('invoke',target('Duplicate action',50000),{},422);assert.deepEqual(await evaluate('[fixture.duplicateA,fixture.duplicateB]'),[0,0]);
const duplicate=(await req('uia')).nodes.find(node=>node.automation_id==='duplicate-b'&&node.control_type===50000);assert(duplicate?.runtime_id);
await uia('invoke',{runtime_id:duplicate.runtime_id});await until('fixture.duplicateB===1');assert.equal(await evaluate('fixture.duplicateA'),0);
await uia('invoke',target('Unicode action 中文 🧪',50000));await until('fixture.unicodeCalls===1');
await uia('invoke',{id:duplicate.id},{},422);checks.push('ambiguity-and-runtime-identity');
await evaluate('(()=>{const el=document.createElement("button");el.id="ephemeral";el.textContent="Ephemeral action";document.querySelector("main").append(el);})()');
const ephemeral=await nativeNode(node=>node.automation_id==='ephemeral'&&node.control_type===50000);assert(ephemeral?.runtime_id);
await evaluate('document.querySelector("#ephemeral").remove()');
await nativeNode(node=>JSON.stringify(node.runtime_id)===JSON.stringify(ephemeral.runtime_id),false);
await uia('invoke',{runtime_id:ephemeral.runtime_id},{},422);checks.push('removed-control-rejection');
const doc=target('Native UIA action fixture',50030);
const text=await uia('text',doc,{max_chars:65536});assert.match(text.text,/Native UIA actions/);assert(!text.text.includes('owned-uia-secret'));
const password=target('Example password',50004);assert.equal((await uia('inspect',password)).element.states.value.value,null);
const secretWrite=await uia('set-value',password,{value:'owned-updated-uia-secret'});assert(!JSON.stringify(secretWrite).includes('owned-updated-uia-secret'));
assert.equal(await evaluate('document.querySelector("#secret").value'),'owned-updated-uia-secret');
assert.equal((await uia('inspect',password)).element.states.value.value,null);
await uia('text',password,{},422);await uia('text-selection',password,{},422);checks.push('native-text-and-password-redaction');
await req('window',{operation:'focus'});await uia('scroll-into-view',edit);await uia('focus',edit);
await uia('text-select',edit,{start:0,end:3});
await until('document.querySelector("#display-name").selectionStart===0&&document.querySelector("#display-name").selectionEnd===3');
assert((await uia('text-selection',edit)).selection.includes('UIA'));checks.push('native-text-selection');
await uia('text-select',edit,{start:1000,end:1001},422);await uia('text-select',password,{start:0,end:1},422);
assert.equal(await evaluate('document.querySelector("#display-name").selectionEnd'),3);
const scroll=target('Scrollable sample',50026);
await uia('scroll-into-view',scroll);await uia('set-scroll-percent',scroll,{vertical_percent:100});await until('document.querySelector("#scroll-box").scrollTop>100');
const reportedScrollPercent=(await uia('inspect',scroll)).element.states.scroll.vertical_percent;
assert(reportedScrollPercent>=0&&reportedScrollPercent<=100);
await uia('scroll',scroll,{vertical_amount:'large-decrement'});await until('document.querySelector("#scroll-box").scrollTop<400');
await uia('set-scroll-percent',scroll,{vertical_percent:-2},422);await uia('scroll',scroll,{vertical_amount:'invalid'},422);checks.push('scroll-pattern-and-validation');
await uia('invoke',target('Save preferences',50000),{max_nodes:1},422);assert.equal(await evaluate('fixture.saves'),2);checks.push('truncated-search-cannot-act');
const root={runtime_id:initial.nodes[0].runtime_id};
assert.equal((await uia('inspect',root,{max_nodes:1})).element.name,'rdesktop Native Agent');
const geometry=(await call('windows')).windows.find(item=>item.id===window).outer;
try {
 await uia('move',root,{x:geometry.x+20,y:geometry.y+20});
 const moved=(await call('windows')).windows.find(item=>item.id===window).outer;
 assert.equal(moved.x,geometry.x+20);assert.equal(moved.y,geometry.y+20);
 await uia('resize',root,{width:geometry.width-20,height:geometry.height-20});
 const resized=(await call('windows')).windows.find(item=>item.id===window).outer;
 assert.equal(resized.width,geometry.width-20);assert.equal(resized.height,geometry.height-20);
 await uia('rotate',root,{degrees:90},422);
}finally{await uia('move',root,{x:geometry.x,y:geometry.y});await uia('resize',root,{width:geometry.width,height:geometry.height});}
checks.push('native-transform-move-resize-and-rotation-rejection');
await uia('window-state',root,{state:'maximized'});
assert.equal((await call('windows')).windows.find(item=>item.id===window).maximized,true);
await uia('window-state',root,{state:'normal'});checks.push('native-window-pattern');
const second=(await req('ipc',{cmd:'second',payload:{}})).data.window;assert.equal(typeof second,'string');
await call('uia',{window:second,operation:'invoke',target:{runtime_id:duplicate.runtime_id}},422);
const secondary=(await call('uia',{window:second})).nodes[0];assert(secondary.runtime_id);
await call('uia',{window:second,operation:'window-close',target:{runtime_id:secondary.runtime_id}});
const deadline=Date.now()+8000;while((await call('windows')).windows.some(item=>item.id===second)){assert(Date.now()<deadline);await delay(30);}
await call('uia',{window:second},422);checks.push('cross-window-scope-and-normal-window-close');
const screenshot=await fetch(`${session.url}/__rdesktop__/agent/screenshot`,{method:'POST',headers,body:JSON.stringify({window})});assert.equal(screenshot.status,200);await writeFile(join(output,'uia-actions.png'),Buffer.from(await screenshot.arrayBuffer()));
const proof={pid:session.pid,headless:false,uiaNodes:initial.nodes.length,reportedScrollPercentAfterNativeSet100:reportedScrollPercent,checks};await writeFile(join(output,'proof.json'),JSON.stringify(proof,null,2));console.log(JSON.stringify(proof,null,2));
await req('cdp',{method:'Page.navigate',params:{url:base}});
