// Deterministic protocol races that would be unreliable to provoke with real
// network timing. Native fs/callback/promise behavior has separate acceptance.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const rendererFactory=new Function('return ('+await readFile(new URL('../crates/rdesktop-node/src/memory.js',import.meta.url),'utf8')+');')();
const nodeFactory=new Function('return ('+await readFile(new URL('../crates/rdesktop-node/src/memory.cjs',import.meta.url),'utf8')+');')();
const renderer=rendererFactory(Buffer),node=nodeFactory(),input=Buffer.from([1,2,3,4]);
const tag=renderer.tag(input);
const send=()=>{const sent=renderer.prepare({args:[tag]});const retained=node.load(sent);return {sent,retained};};
const first=send(),native=node.unpack(tag);native[0]=5;
renderer.accept(first.sent,{memory:node.collect()});assert.deepEqual([...input],[5,2,3,4]);
assert.equal(node.unpack(tag),native);
native[1]=6;const older=node.collect();native[2]=7;const newer=node.collect();
renderer.accept(undefined,{memory:newer});renderer.accept(undefined,{memory:older});assert.deepEqual([...input],[5,6,7,4]);
native[0]=8;const overlapOld=node.collect();native[0]=9;const overlapNew=node.collect();
renderer.accept(undefined,{memory:overlapNew});renderer.accept(undefined,{memory:overlapOld});assert.equal(input[0],9);
input[0]=10;native[0]=11;renderer.accept(undefined,{memory:node.collect()});assert.equal(input[0],10);
const next=send();assert.equal(native[0],10);renderer.accept(next.sent,{memory:node.collect()});assert.equal(input[0],10);
const alias=input.subarray(1,3),aliasTag=renderer.tag(alias),aliasSent=renderer.prepare({args:[tag,aliasTag]});
const retained=node.load(aliasSent),nativeAlias=node.unpack(aliasTag);assert.equal(nativeAlias.buffer,native.buffer);
nativeAlias[0]=12;renderer.accept(aliasSent,{memory:node.collect()});assert.equal(input[1],12);
assert.equal(renderer.unpack(node.pack(nativeAlias)),alias);
nativeAlias[0]=13;const uncommitted=node.collect(false),stillPending=node.collect();
assert.deepEqual(stillPending.changes,uncommitted.changes);renderer.accept(undefined,{memory:stillPending});assert.equal(input[1],13);
for(const type of ['Uint8Array','Uint16Array','DataView','ArrayBuffer']) {
 const value=renderer.unpack({$rdesktop:'binary-view',type,length:type==='Uint16Array'?2:4,base64:'AQIDBA=='});
 assert.equal(Object.prototype.toString.call(value),'[object '+type+']');
 assert.deepEqual(Array.from(new Uint8Array(type==='ArrayBuffer'?value:value.buffer)),[1,2,3,4]);
}
assert.throws(()=>node.load([{id:'b999',byteLength:64*1024*1024+1,offset:0,base64:'',writes:[]}]),/binary backing/);
assert.throws(()=>node.unpack({...tag,length:100,byteLength:100}),{name:'TypeError',message:'Binary view identity changed.'});
assert.throws(()=>node.unpack({...tag,length:100000,byteLength:100000}),/binary view/);
assert.throws(()=>renderer.tag(new Uint8Array(new SharedArrayBuffer(4))),/Shared binary/);
console.log(JSON.stringify({passed:true,checks:['sync-writeback','stable-native-view','out-of-order-disjoint-polls','out-of-order-overlapping-polls','local-edits-survive-pending-poll','local-edits-return-to-native','shared-backing-alias','returned-original-view','fresh-native-binary-types','bounded-backing-and-view-validation','shared-memory-rejected']}));
