// Real OS modal lifetime, native UIA and compositor capture; no DOM substitute.
// node scripts/verify-native-dialogs.mjs SESSION OUTPUT CLI_EXE
import assert from 'node:assert/strict';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, join} from 'node:path';
import {spawnSync} from 'node:child_process';
import {randomUUID} from 'node:crypto';
import {inflateSync} from 'node:zlib';
import {setTimeout as delay} from 'node:timers/promises';
const [sessionPath, directory, executable]=process.argv.slice(2);
assert(sessionPath&&directory&&executable);
const session=JSON.parse(await readFile(sessionPath,'utf8'));
const output=resolve(directory);await mkdir(output,{recursive:true});
const headers={authorization:`Bearer ${session.token}`,'content-type':'application/json'};
async function raw(command,params={}) {
    return fetch(`${session.url}/__rdesktop__/agent/${command}`,{method:'POST',headers,body:JSON.stringify(params)});
}
async function call(command,params={},expected=200) {
    const response=await raw(command,params),value=await response.json();
    assert.equal(response.status,expected,JSON.stringify(value));return value;
}
const main=(await call('windows')).windows.find(item=>item.title==='rdesktop Native Agent');assert(main);
const window=main.id,req=(command,params={},expected=200)=>call(command,{window,...params},expected);
const base=new URL('.',main.url).href,checks=[];
const info=await fetch(`${session.url}/__rdesktop__/info`,{headers});assert.equal(info.status,200);
const capabilities=(await info.json()).capabilities;
assert.equal(capabilities.native_windows.modal_inspection,true);
assert.equal(capabilities.native_screenshot.source,'windows-graphics-capture');
assert.equal(capabilities.native_screenshot.maximum_pixels,16_777_216);
for(const max_windows of [0,65,-1,1.5,'4'])await req('native-windows',{max_windows},422);
for(const native_window of [null,2,{},'unobserved-window'])await req('uia',{native_window},422);
await req('native-screenshot',{hwnd:1},422);await req('state',{native_window:'native-1'},422);
checks.push('strict-native-targets-and-bounded-discovery');

// Decode the actual RGBA PNG scanlines, including every standard PNG filter.
// Reject blank/transparent frames rather than merely recognizing a PNG header.
function pngPixels(png) {
    assert(png.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10])));
    const width=png.readUInt32BE(16),height=png.readUInt32BE(20);
    assert(width>0&&height>0&&width*height<=16_777_216);
    assert.equal(png[24],8);assert.equal(png[25],6);assert.equal(png[28],0);
    const compressed=[];let ended=false;
    for(let at=8;at<png.length;) {
        const length=png.readUInt32BE(at),type=png.toString('ascii',at+4,at+8);
        assert(at+length+12<=png.length);
        if(type==='IDAT')compressed.push(png.subarray(at+8,at+8+length));
        if(type==='IEND'){ended=true;assert.equal(at+12,png.length);break;}at+=length+12;
    }
    assert(ended);const packed=inflateSync(Buffer.concat(compressed)),stride=width*4;
    assert.equal(packed.length,(stride+1)*height);
    const decoded=Buffer.alloc(stride*height);
    const paeth=(a,b,c)=>{const p=a+b-c,pa=Math.abs(p-a),pb=Math.abs(p-b),pc=Math.abs(p-c);return pa<=pb&&pa<=pc?a:pb<=pc?b:c;};
    for(let y=0;y<height;y++) {
        const filter=packed[y*(stride+1)];assert(filter<=4);
        for(let x=0;x<stride;x++) {
            const at=y*stride+x,a=x>=4?decoded[at-4]:0,b=y?decoded[at-stride]:0,c=y&&x>=4?decoded[at-stride-4]:0;
            decoded[at]=(packed[y*(stride+1)+1+x]+[0,a,b,Math.floor((a+b)/2),paeth(a,b,c)][filter])&255;
        }
    }
    const colors=new Set();let opaque=0;
    for(let at=0;at<decoded.length;at+=4)if(decoded[at+3]>0){opaque++;colors.add(decoded.readUInt32BE(at));}
    assert(colors.size>16);assert(opaque>width*height/2);return{width,height,distinctColors:colors.size};
}
async function capture(native_window,name) {
    const response=await raw('native-screenshot',{window,...(native_window?{native_window}:{})});
    assert.equal(response.status,200,response.status===200?'':JSON.stringify(await response.json()));
    assert.equal(response.headers.get('content-type'),'image/png');
    assert.equal(response.headers.get('x-rdesktop-capture-source'),'windows-graphics-capture');
    const png=Buffer.from(await response.arrayBuffer()),pixels=pngPixels(png);
    assert.equal(Number(response.headers.get('x-rdesktop-capture-width')),pixels.width);
    assert.equal(Number(response.headers.get('x-rdesktop-capture-height')),pixels.height);
    const frame=Object.fromEntries(['x','y','width','height'].map(key=>{
        const value=response.headers.get(`x-rdesktop-frame-${key}`);assert.notEqual(value,null);assert(Number.isInteger(Number(value)));return[key,Number(value)];
    }));
    assert.equal(frame.width,pixels.width);assert.equal(frame.height,pixels.height);
    await writeFile(join(output,name),png);return{...pixels,frame};
}
if(!main.visible) {
    await req('native-screenshot',{},422);
    const content=await raw('screenshot',{window});assert.equal(content.status,200);
    assert.equal(content.headers.get('x-rdesktop-capture-source'),'webview-content');
    assert.equal((await call('windows')).windows.find(item=>item.id===window).visible,false);
    checks.push('hidden-compositor-rejection-preserves-hidden-window-and-content-capture');
    const proof={headless:true,checks};await writeFile(join(output,'proof.json'),JSON.stringify(proof,null,2));console.log(JSON.stringify(proof));process.exit(0);
}
await req('cdp',{method:'Page.navigate',params:{url:new URL('native-dialog.html',base).href}});
const ready=Date.now()+8000;while(!(await req('evaluate',{expression:'document.readyState==="complete"&&!!document.querySelector("#native-dialog")'}))){assert(Date.now()<ready);await delay(30);}
const rootCapture=await capture(undefined,'native-frame.png');
assert(rootCapture.height>main.client.height);assert(rootCapture.width>=main.client.width);
const content=await raw('screenshot',{window});const contentPng=Buffer.from(await content.arrayBuffer());
assert.equal(contentPng.readUInt32BE(20),main.client.height);
checks.push('actual-compositor-frame-and-webview-content-have-distinct-bounds');
await req('window',{operation:'minimize'});
try {await req('native-screenshot',{},422);assert.equal((await call('windows')).windows.find(item=>item.id===window).minimized,true);}
finally {await req('window',{operation:'restore'});}
checks.push('minimized-capture-fails-without-restoring-window');
const created=await req('ipc',{cmd:'second',payload:{}}),auxiliary=created.data.window;
let dialog,opening,dialogCapture,modalElapsed,filePicker;
try {
    // dispatch_window queues the modal without the synchronous UI-call deadline.
    opening=req('action',{action:'click',selector:'#native-dialog'}).then(value=>({value}),error=>({error:String(error)}));
    const deadline=Date.now()+10_000;
    while(Date.now()<deadline) {
        dialog=(await req('native-windows')).windows.find(item=>item.class_name==='#32770'&&item.title==='rdesktop native modal fixture');
        if(dialog)break;await delay(30);
    }
    assert(dialog,'real owned modal was not discovered');assert(dialog.visible);assert.match(dialog.id,/^native-\d+$/);
    const started=Date.now();
    const inventory=await req('native-windows');assert.equal(inventory.source,'windows-owned-windows');
    assert.equal(inventory.windows.find(item=>item.title===dialog.title).id,dialog.id);
    assert.equal((await call('windows')).windows.find(item=>item.id===window).enabled,false);
    assert(!(await call('native-windows',{window:auxiliary})).windows.some(item=>item.id===dialog.id));
    await call('uia',{window:auxiliary,native_window:dialog.id},422);
    await call('native-screenshot',{window:auxiliary,native_window:dialog.id},422);
    checks.push('stable-owned-modal-id-and-cross-renderer-isolation');
    const tree=await req('uia',{native_window:dialog.id});assert.equal(tree.source,'windows-uia');assert.equal(tree.truncated,false);
    const cancel=tree.nodes.find(node=>node.automation_id==='2'&&node.control_type===50000);assert(cancel?.runtime_id);
    assert(tree.nodes.some(node=>node.automation_id==='1'&&node.control_type===50000));
    // Windows exposes an owned modal under its owner's raw UIA ancestry too.
    assert.equal((await req('uia',{operation:'inspect',target:{runtime_id:cancel.runtime_id}})).element.automation_id,'2');
    assert.equal((await req('uia',{native_window:dialog.id,operation:'inspect',target:{runtime_id:cancel.runtime_id}})).element.automation_id,'2');
    checks.push('real-native-dialog-tree-and-owner-accessibility-ancestry');
    dialogCapture=await capture(dialog.id,'native-dialog.png');
    assert.equal(dialogCapture.frame.width,dialog.frame.width);assert.equal(dialogCapture.frame.height,dialog.frame.height);
    const paramsPath=join(output,'capture-params.json');await writeFile(paramsPath,JSON.stringify({window,native_window:dialog.id}));
    const cliCapture=join(output,`native-dialog-cli-${randomUUID()}.png`);
    const cli=spawnSync(resolve(executable),['agent','--session',resolve(sessionPath),'native-screenshot','--params-file',paramsPath,'--output',cliCapture],{encoding:'utf8',windowsHide:true});
    assert.equal(cli.status,0,cli.stderr);const cliPixels=pngPixels(await readFile(cliCapture));
    assert.equal(cliPixels.width,dialogCapture.width);assert.equal(cliPixels.height,dialogCapture.height);
    checks.push('actual-native-compositor-pixels-and-cli-png-output');
    // Exceed the old 30-second synchronous dispatch timeout. Read endpoints
    // must continue working and cancellation must still reach the real modal.
    while(Date.now()-started<31_000) {
        assert.equal((await call('windows')).windows.find(item=>item.id===window).enabled,false);
        assert.equal((await req('uia',{native_window:dialog.id,operation:'inspect',target:{runtime_id:cancel.runtime_id}})).element.automation_id,'2');
        await delay(1500);
    }
    modalElapsed=Date.now()-started;checks.push('modal-survives-synchronous-dispatch-timeout-with-live-inspection');
    await req('uia',{native_window:dialog.id,operation:'invoke',target:{runtime_id:cancel.runtime_id}});
    dialog.closed=true;
    const result=await opening;assert(!result.error,result.error);
    const status=await req('ipc',{cmd:'native_dialog_status',payload:{}});assert.equal(status.data.result,2);
    assert.equal((await call('windows')).windows.find(item=>item.id===window).enabled,true);
    checks.push('native-cancel-unblocks-parent-and-preserves-actual-dialog-result');
    await req('uia',{native_window:dialog.id},422);await req('native-screenshot',{native_window:dialog.id},422);
    assert(!(await req('native-windows')).windows.some(item=>item.id===dialog.id));
    checks.push('closed-native-target-rejected-for-inspection-and-capture');
    opening=req('action',{action:'click',selector:'#native-file'}).then(value=>({value}),error=>({error:String(error)}));
    const fileDeadline=Date.now()+10_000;dialog=undefined;
    while(Date.now()<fileDeadline) {
        dialog=(await req('native-windows')).windows.find(item=>item.visible&&item.class_name==='#32770');
        if(dialog)break;await delay(30);
    }
    assert(dialog,'WebView2 OS file picker was not discovered');
    const fileTree=await req('uia',{native_window:dialog.id}),fileCancel=fileTree.nodes.find(node=>node.automation_id==='2'&&node.control_type===50000);assert(fileCancel?.runtime_id);
    await call('uia',{window:auxiliary,native_window:dialog.id},422);
    await call('native-screenshot',{window:auxiliary,native_window:dialog.id},422);
    filePicker={separateProcess:dialog.process_id!==session.pid,capture:await capture(dialog.id,'native-file-picker.png'),uiaNodes:fileTree.nodes.length};
    await req('uia',{native_window:dialog.id,operation:'invoke',target:{runtime_id:fileCancel.runtime_id}});dialog.closed=true;
    const fileResult=await opening;assert(!fileResult.error,fileResult.error);
    const cancelDeadline=Date.now()+8000;
    while(!(await req('evaluate',{expression:'document.querySelector("#file-status").textContent==="Native file picker canceled"'}))){assert(Date.now()<cancelDeadline);await delay(30);}
    await req('uia',{native_window:dialog.id},422);await req('native-screenshot',{native_window:dialog.id},422);
    checks.push('actual-webview2-file-picker-native-ui-and-broker-owner-plus-uia-association');
    checks.push('file-picker-cross-renderer-rejection-cancel-and-stale-id');
} finally {
    if(dialog&&!dialog.closed) {
        const tree=await req('uia',{native_window:dialog.id});
        const cancel=tree.nodes.find(node=>node.automation_id==='2'&&node.control_type===50000);
        if(cancel)await req('uia',{native_window:dialog.id,operation:'invoke',target:{runtime_id:cancel.runtime_id}});
    } else if(!dialog) {
        // A discovery regression must still cancel the actual owner-scoped OS
        // modal, rather than leave it blocking the next verification run.
        const tree=await req('uia'),cancels=tree.nodes.filter(node=>node.automation_id==='2'&&node.control_type===50000);
        if(cancels.length===1)await req('uia',{operation:'invoke',target:{runtime_id:cancels[0].runtime_id}});
    }
    if(opening)await opening;
    await call('window',{window:auxiliary,operation:'close'});
    await req('cdp',{method:'Page.navigate',params:{url:base}});
}
const proof={headless:false,rootCapture,dialogCapture,filePicker,modalElapsedMilliseconds:modalElapsed,checks};
await writeFile(join(output,'proof.json'),JSON.stringify(proof,null,2));console.log(JSON.stringify(proof,null,2));
