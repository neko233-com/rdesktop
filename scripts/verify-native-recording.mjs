// Explicit opt-in: records the entire Windows desktop locally for two seconds.
// Never upload the resulting video; it may include other applications.
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {setTimeout as delay} from 'node:timers/promises';
const [sessionPath,outputPath]=process.argv.slice(2);assert(sessionPath&&outputPath);
const session=JSON.parse(await readFile(sessionPath,'utf8'));const output=resolve(outputPath);await mkdir(output,{recursive:true});
const headers={authorization:`Bearer ${session.token}`,'content-type':'application/json'};
async function call(command,params={},expected=200){const response=await fetch(`${session.url}/__rdesktop__/agent/${command}`,{method:'POST',headers,body:JSON.stringify(params)});const value=await response.json();assert.equal(response.status,expected,JSON.stringify(value));return value;}
assert.equal((await call('recording')).status,'idle');
const started=await call('recording/start',{fps:5,max_duration_seconds:10});
const id=started.recording.session_id;assert(id);assert.equal(started.recording.native,true);assert.equal(started.reused,false);
let completed;
try{
 const reused=await call('recording/start',{fps:60,max_duration_seconds:3600});assert.equal(reused.reused,true);assert.equal(reused.recording.session_id,id);
 await call('recording/stop',{session_id:'not-the-owned-session'},409);
 const pending=await fetch(`${session.url}/__rdesktop__/agent/recording/file`,{headers});assert.equal(pending.status,409);await pending.arrayBuffer();
 await delay(2200);
 completed=await call('recording/stop',{session_id:id});
}finally{if(!completed)await call('recording/stop',{session_id:id});}
assert.equal(completed.status,'completed',JSON.stringify(completed));assert.equal(completed.error,null);assert(completed.bytes>1000);
const again=await call('recording/stop',{session_id:id});assert.equal(again.bytes,completed.bytes);assert.equal(again.session_id,id);
const response=await fetch(new URL(completed.download_url,session.url),{headers});assert.equal(response.status,200);assert.equal(response.headers.get('content-type'),'video/mp4');
const mp4=Buffer.from(await response.arrayBuffer());assert.equal(mp4.length,completed.bytes);assert.equal(mp4.subarray(4,8).toString(),'ftyp');assert(mp4.includes(Buffer.from('moov')));assert(mp4.includes(Buffer.from('mdat')));
await writeFile(join(output,'desktop-local-only.mp4'),mp4);
const proof={mode:session.mode,native:true,scope:'entire-virtual-desktop-local-only',bytes:mp4.length,sha256:createHash('sha256').update(mp4).digest('hex'),checks:['start','idempotent-start','session-mismatch-rejection','unfinished-file-rejection','finalize','idempotent-stop','authenticated-mp4-stream']};
await writeFile(join(output,'proof.json'),JSON.stringify(proof,null,2));console.log(JSON.stringify(proof,null,2));
