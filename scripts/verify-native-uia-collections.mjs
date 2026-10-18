// Real Grid/Table/Selection COM reads. DOM is used only to observe app state.
// node scripts/verify-native-uia-collections.mjs SESSION OUTPUT AXE_SOURCE CLI_EXE
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {spawnSync} from 'node:child_process';
import {randomUUID} from 'node:crypto';
import {setTimeout as delay} from 'node:timers/promises';
const [sessionPath,directory,axeSource,cliPath]=process.argv.slice(2);assert(sessionPath&&directory&&axeSource&&cliPath);
const session=JSON.parse(await readFile(sessionPath,'utf8'));const output=resolve(directory);await mkdir(output,{recursive:true});
const headers={authorization:`Bearer ${session.token}`,'content-type':'application/json'};
async function call(command,params={},expected=200){const response=await fetch(`${session.url}/__rdesktop__/agent/${command}`,{method:'POST',headers,body:JSON.stringify(params)});const value=await response.json();assert.equal(response.status,expected,JSON.stringify(value));return value;}
const main=(await call('windows')).windows.find(window=>window.title==='rdesktop Native Agent');assert(main);
const information=await fetch(`${session.url}/__rdesktop__/info`,{headers});assert.equal(information.status,200);
const capabilities=(await information.json()).capabilities.uia;assert(capabilities.operations.includes('grid-item'));assert.equal(capabilities.collections.maximum_max_items,256);assert.deepEqual(capabilities.collections.selection_strategies,['provider','items']);
const window=main.id,req=(command,params={},expected=200)=>call(command,{window,...params},expected);
const evaluate=expression=>req('evaluate',{expression});
const uia=(operation,target,params={},expected=200)=>req('uia',{operation,target,...params},expected);
const target=(name,control_type)=>({name,control_type}),table=target('Inventory table',50036),grid=target('Project grid',50028),fruits=target('Selected fruits',50008);
async function until(expression){const end=Date.now()+8000;while(Date.now()<end){if(await evaluate(expression))return;await delay(30);}assert.fail(`timed out: ${expression}`);}
const base=new URL('.',main.url).href;await req('cdp',{method:'Page.navigate',params:{url:new URL('uia-collections.html',base).href}});
await until('document.readyState==="complete"&&!!window.fixture');
const initial=await req('uia');assert.equal(initial.source,'windows-uia');assert.equal(initial.truncated,false);
await writeFile(join(output,'initial-uia.json'),JSON.stringify(initial,null,2));
const checks=[],available=initial.nodes.some(node=>node.name===table.name&&node.control_type===table.control_type);
if(!main.visible&&!available){
 for(const operation of ['grid-item','table-headers','table-item-headers','selection','containing-grid','selection-container']){
  await uia(operation,table,operation==='grid-item'?{row:0,column:0}:{},422);
 }
 assert.equal((await call('windows')).windows.find(item=>item.id===window).visible,false);
 checks.push('hidden-unavailable-provider-reads-fail-without-showing-window');
 await writeFile(join(output,'proof.json'),JSON.stringify({headless:true,uiaNodes:initial.nodes.length,available,checks},null,2));
 console.log(JSON.stringify({headless:true,uiaNodes:initial.nodes.length,available,checks}));
 await req('cdp',{method:'Page.navigate',params:{url:base}});process.exit(0);
}
const tableState=(await uia('inspect',table)).element;assert.deepEqual(tableState.states.grid,{rows:3,columns:3});assert.equal(typeof tableState.states.table.row_or_column_major,'number');
const cell=(await uia('grid-item',table,{row:1,column:1})).element;assert.equal(cell.name,'12');assert(cell.runtime_id);
const cellTarget={runtime_id:cell.runtime_id};assert.deepEqual((await uia('inspect',cellTarget)).element.states.grid_item,{row:1,column:1,row_span:1,column_span:1});
assert.deepEqual((await uia('containing-grid',cellTarget)).element.runtime_id,tableState.runtime_id);checks.push('native-grid-coordinates-and-containing-grid');
const headersResult=await uia('table-headers',table);assert.deepEqual(headersResult.columns.elements.map(item=>item.name),['Item','Quantity','State']);assert.equal(headersResult.columns.truncated,false);
const partialHeaders=await uia('table-headers',table,{max_items:2});assert.equal(partialHeaders.columns.total,3);assert.equal(partialHeaders.columns.elements.length,2);assert.equal(partialHeaders.columns.truncated,true);checks.push('native-table-headers-and-bounded-results');
const cellHeaders=await uia('table-item-headers',cellTarget);assert.deepEqual(cellHeaders.columns.elements.map(item=>item.name),['Quantity']);assert.deepEqual(cellHeaders.rows.elements.map(item=>item.name),['Apples']);checks.push('native-cell-row-and-column-header-relations');
const merged=target('Merged cells table',50036);const wide=(await uia('grid-item',merged,{row:0,column:1})).element;
assert.equal(wide.name,'Wide cell');assert.deepEqual((await uia('grid-item',merged,{row:0,column:2})).element.runtime_id,wide.runtime_id);
assert.equal((await uia('inspect',{runtime_id:wide.runtime_id})).element.states.grid_item.column_span,2);
const shared=(await uia('grid-item',merged,{row:0,column:0})).element;assert.deepEqual((await uia('grid-item',merged,{row:1,column:0})).element.runtime_id,shared.runtime_id);
assert.equal((await uia('inspect',{runtime_id:shared.runtime_id})).element.states.grid_item.row_span,2);checks.push('native-merged-row-and-column-spans');
assert.deepEqual((await uia('inspect',grid)).element.states.grid,{rows:3,columns:3});assert.equal((await uia('grid-item',grid,{row:1,column:0})).element.name,'Alpha');
const gridProviderSelection=(await uia('selection',grid)).selection;
assert.equal((await uia('inspect',target('Alpha',50029))).element.states.selected,true);
const gridItemSelection=(await uia('selection',grid,{strategy:'items'})).selection;
assert.deepEqual(gridItemSelection.elements.map(item=>item.name),['Alpha']);assert.equal(gridItemSelection.search_truncated,false);
checks.push('native-aria-grid-and-explicit-item-selection-strategy');
const selected=await uia('selection',fruits);assert.deepEqual(selected.selection.elements.map(item=>item.name),['Apple','Banana','Cherry']);
const limited=await uia('selection',fruits,{max_items:2});assert.equal(limited.selection.total,3);assert.equal(limited.selection.elements.length,2);assert.equal(limited.selection.truncated,true);
assert.equal(await evaluate('document.querySelector("#choices").selectedOptions.length'),3);checks.push('native-selection-collection-and-truncation');
const scanned=await uia('selection',fruits,{strategy:'items'});assert.deepEqual(scanned.selection.elements.map(item=>item.name),['Apple','Banana','Cherry']);assert.equal(scanned.selection.total,3);
const fruitIdentity={runtime_id:(await uia('inspect',fruits)).element.runtime_id};
const truncatedScan=(await uia('selection',fruitIdentity,{strategy:'items',scan_max_nodes:1})).selection;assert.equal(truncatedScan.search_truncated,true);assert.equal(truncatedScan.total,null);checks.push('native-item-selection-scan-and-incomplete-total');
await uia('remove-from-selection',target('Cherry',50007));assert.deepEqual((await uia('selection',fruits)).selection.elements.map(item=>item.name),['Apple','Banana']);
await uia('add-to-selection',target('Cherry',50007));assert.equal((await uia('selection',fruits)).selection.total,3);checks.push('live-selection-collection-after-native-mutation');
const selectionContainer=(await uia('selection-container',target('Apple',50007))).element;assert.equal(selectionContainer.automation_id,'choices');
assert.deepEqual((await uia('selection',target('Empty selection',50008))).selection,{elements:[],total:0,truncated:false,max_items:32});checks.push('selection-container-and-empty-native-array');
const password=(await uia('inspect',target('Collection password',50004))).element;assert.equal(password.states.value.value,null);assert.equal(password.states.legacy.value,null);
assert(!JSON.stringify(password).includes('owned-collection-secret'));checks.push('legacy-password-value-redaction');
for(const params of [{row:-1,column:0},{row:0.5,column:0},{row:0,column:3},{row:3,column:0},{row:0,column:'0'},{row:0},{row:0,column:0,max_items:1}])await uia('grid-item',table,params,422);
for(const max_items of [0,257,'2',1.5])await uia('selection',fruits,{max_items},422);
await uia('selection',fruits,{strategy:'dom'},422);
await uia('grid-item',target('Collection password',50004),{row:0,column:0},422);await uia('selection',table,{},422);
await uia('selection-container',table,{},422);checks.push('invalid-coordinates-limits-and-unsupported-pattern-rejection');
await uia('grid-item',table,{row:0,column:0,view:'content'});checks.push('related-controls-validated-with-raw-window-ancestry');
const created=await req('ipc',{cmd:'second',payload:{}});const auxiliary=created.data.window;
try{
 await call('cdp',{window:auxiliary,method:'Page.navigate',params:{url:new URL('uia-collections.html',base).href}});
 const end=Date.now()+8000;let foreign;
 while(Date.now()<end){foreign=(await call('uia',{window:auxiliary})).nodes.find(node=>node.name===table.name&&node.control_type===table.control_type);if(foreign)break;await delay(30);}
 assert(foreign?.runtime_id);await uia('grid-item',{runtime_id:foreign.runtime_id},{row:0,column:0},422);
 assert.equal((await uia('grid-item',table,{row:1,column:1})).element.name,'12');
}finally{await call('window',{window:auxiliary,operation:'close'});}
await call('uia',{window:auxiliary,operation:'selection',target:fruits},422);checks.push('cross-window-and-closed-window-rejection');
await uia('set-scroll-percent',target('Native UIA collections fixture',50030),{vertical_percent:0});
await until('window.scrollY===0');await evaluate('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');
const auditFile=join(output,`axe-${randomUUID()}.json`);const audit=spawnSync(resolve(cliPath),['agent','--session',resolve(sessionPath),'audit','--window',window,'--axe-source',resolve(axeSource),'--output',auditFile],{encoding:'utf8',windowsHide:true});assert.equal(audit.status,0,audit.stderr);
const axe=JSON.parse(await readFile(auditFile,'utf8'));assert.equal(axe.violations.length,0,JSON.stringify(axe.violations.map(item=>({id:item.id,targets:item.nodes.map(node=>node.target)}))));
// Axe cannot resolve backgrounds for this rowspan/native-select rendering.
// Keep its incomplete results and independently verify the actual opaque
// rendered colors, including each selected/unselected option. No rule is off.
for(const item of axe.incomplete){assert.equal(item.id,'color-contrast');for(const node of item.nodes)assert(['th[rowspan="2"]','#choices','#empty'].includes(node.target[0]));}
const colors=await evaluate(`Array.from(document.querySelectorAll('th[rowspan="2"],#choices,#empty,#choices option,#empty option'),element=>{
 const style=getComputedStyle(element);let current=element,background;
 while(current){const value=getComputedStyle(current);if(value.opacity!=='1'||value.backgroundImage!=='none'||value.filter!=='none'||value.mixBlendMode!=='normal')throw Error('Complex contrast background requires manual review');if(!background&&value.backgroundColor!=='rgba(0, 0, 0, 0)')background=value.backgroundColor;current=current.parentElement;}
 return {element:element.tagName,text:element.textContent.trim(),foreground:style.color,background};})`);
const luminance=value=>{const match=/^rgb\((\d+), (\d+), (\d+)\)$/.exec(value);assert(match,`expected opaque sRGB: ${value}`);const rgb=match.slice(1).map(Number).map(value=>{value/=255;return value<=0.04045?value/12.92:((value+0.055)/1.055)**2.4;});return rgb[0]*0.2126+rgb[1]*0.7152+rgb[2]*0.0722;};
const contrast=colors.map(item=>{const a=luminance(item.foreground),b=luminance(item.background),ratio=(Math.max(a,b)+0.05)/(Math.min(a,b)+0.05);assert(ratio>=4.5,JSON.stringify({...item,ratio}));return{...item,ratio};});
await writeFile(join(output,'rendered-contrast.json'),JSON.stringify(contrast,null,2));checks.push('native-axe-audit-with-explicit-rendered-contrast-checks');
const capture=await fetch(`${session.url}/__rdesktop__/agent/screenshot`,{method:'POST',headers,body:JSON.stringify({window})});assert.equal(capture.status,200);const png=Buffer.from(await capture.arrayBuffer());assert.equal(png.subarray(1,4).toString(),'PNG');await writeFile(join(output,'collections.png'),png);
const proof={headless:!main.visible,uiaNodes:initial.nodes.length,tableWideRowHeaders:headersResult.rows.total,tableCellRowHeaders:cellHeaders.rows.total,gridProviderSelectionCount:gridProviderSelection.total,gridItemSelectionCount:gridItemSelection.total,axePasses:axe.passes.length,axeViolations:0,axeIncomplete:axe.incomplete.map(item=>({id:item.id,targets:item.nodes.map(node=>node.target)})),minimumVerifiedContrast:Math.min(...contrast.map(item=>item.ratio)),checks};await writeFile(join(output,'proof.json'),JSON.stringify(proof,null,2));console.log(JSON.stringify(proof,null,2));
await req('cdp',{method:'Page.navigate',params:{url:base}});
