// Windows native acceptance. No Electron/CDP replacement renderer is launched.
// node scripts/verify-node-plugin-native.mjs fixture OUTPUT [--headless]
// node scripts/verify-node-plugin-native.mjs public OUTPUT IMMUTABLE_PUBLIC_PACKAGE
import fs from 'node:fs/promises';import {spawn,execFileSync} from 'node:child_process';import {createHash,randomUUID} from 'node:crypto';import {join,resolve,relative,isAbsolute} from 'node:path';import {fileURLToPath} from 'node:url';import {setTimeout as delay} from 'node:timers/promises';import assert from 'node:assert/strict';
assert.equal(process.platform,'win32','Windows x64 is the current native acceptance platform');assert.equal(process.arch,'x64');
const repo=fileURLToPath(new URL('../',import.meta.url));
const [mode,outputArgument,publicPackage,...flags]=process.argv.slice(2),hidden=process.argv.includes('--headless');
assert(['fixture','public'].includes(mode)&&outputArgument,'Expected fixture|public and an external OUTPUT directory');
assert(!flags.some(flag=>flag!=='--headless'));assert(!publicPackage||publicPackage==='--headless'||mode==='public');
assert(!(mode==='public'&&hidden),'The public plugin UIA input/invoke acceptance requires a visible window');
const output=resolve(outputArgument),location=relative(repo,output);assert(isAbsolute(location)||location==='..'||location.startsWith('..\\')||location.startsWith('../'),'QA output must be outside the repository');await fs.mkdir(output,{recursive:true});
const runRoot=join(output,`${mode}-${hidden?'hidden':'visible'}-${randomUUID()}`);await fs.mkdir(runRoot);
const packageRoot=mode==='public'?resolve(publicPackage):join(runRoot,'package');
if(mode!=='public'){await fs.mkdir(packageRoot);for(const file of ['index.html','preload.cjs','helper.cjs','cycle-a.cjs','cycle-b.cjs'])await fs.copyFile(join(repo,'examples/node_plugin/fixtures',file),join(packageRoot,file));}
const nodeExe=process.env.RDESKTOP_TEST_NODE;assert(nodeExe);const nodeSha=createHash('sha256').update(await fs.readFile(nodeExe)).digest('hex');assert.equal(nodeSha,process.env.RDESKTOP_TEST_NODE_SHA256,'Explicitly pin the selected Node binary');
const sessionFile=join(runRoot,'session.json');const executable=process.env.RDESKTOP_PLUGIN_FIXTURE_EXE||join(repo,'target/debug/node_plugin.exe');
const snapshot=()=>JSON.parse(execFileSync('powershell.exe',['-NoProfile','-NonInteractive','-Command','Get-CimInstance Win32_Process | Select-Object Name,ProcessId,ParentProcessId | ConvertTo-Json -Compress'],{windowsHide:true,encoding:'utf8'}));
async function packageHashes(){const files=execFileSync('git',['-C',packageRoot,'ls-files','-z'],{windowsHide:true,encoding:'utf8'}).split('\0').filter(Boolean);return Object.fromEntries(await Promise.all(files.map(async file=>[file,createHash('sha256').update(await fs.readFile(join(packageRoot,file))).digest('hex')])));}
let originalHashes;if(mode==='public'){assert.equal(execFileSync('git',['-C',packageRoot,'rev-parse','HEAD'],{windowsHide:true,encoding:'utf8'}).trim(),'8a3f257591935cf4d2dd4baa599f3c20fc1b7e5d');originalHashes=await packageHashes();}
const child=spawn(executable,[],{windowsHide:true,stdio:['ignore','pipe','pipe'],env:{...process.env,RDESKTOP_TEST_NODE:nodeExe,RDESKTOP_TEST_NODE_SHA256:nodeSha,RDESKTOP_PLUGIN_ROOT:packageRoot,RDESKTOP_PLUGIN_PRELOAD:mode==='public'?'preload.js':'preload.cjs',RDESKTOP_PLUGIN_FIXTURE_FOLDER:runRoot,RDESKTOP_AGENT:'1',RDESKTOP_AGENT_SESSION:sessionFile,...(hidden?{RDESKTOP_HEADLESS:'1'}:{RDESKTOP_HEADLESS:'0'})}});
let log='';child.stdout.on('data',b=>log+=b);child.stderr.on('data',b=>log+=b);const exited=new Promise(done=>child.once('exit',(code,signal)=>done({code,signal})));let session,ownedProcesses=[];
async function call(command,params={}){const response=await fetch(session.url+'/__rdesktop__/agent/'+command,{method:'POST',headers:{authorization:'Bearer '+session.token,'content-type':'application/json'},body:JSON.stringify(params)});const value=await response.json();assert(response.ok,JSON.stringify(value));return value;}
async function until(read,label,timeout=20000){const end=Date.now()+timeout;while(Date.now()<end){const v=await read();if(v)return v;if(child.exitCode!==null)throw Error(log);await delay(50);}throw Error('Timed out: '+label+'\n'+log);}
try{
 await until(async()=>{session=await fs.readFile(sessionFile,'utf8').then(JSON.parse,()=>null);return session;},'session');
 const main=await until(async()=>(await call('windows')).windows.find(w=>w.title==='rdesktop Native Node Plugin'),'native window');const window=main.id;
 const req=(cmd,p={})=>call(cmd,{window,...p}),evaluate=expression=>req('evaluate',{expression});
 await until(()=>evaluate('document.readyState==="complete"').catch(()=>false),'document');
 const status=await evaluate('window.__RDESKTOP_NODE_STATUS__??null');assert(status,'renderer preload did not initialize: '+await evaluate('document.body?.innerText'));
 assert.equal(status.renderer,'native-webview');assert.equal(status.electron,false);assert.equal((await call('windows')).windows.find(w=>w.id===window).visible,!hidden);
 const processes=snapshot(),owned=new Set([child.pid]);for(let pass=0;pass<16;pass++)for(const process of processes)if(owned.has(process.ParentProcessId))owned.add(process.ProcessId);
 ownedProcesses=processes.filter(process=>owned.has(process.ProcessId));assert(!ownedProcesses.some(process=>/electron/i.test(process.Name)));assert(ownedProcesses.some(process=>process.Name.toLowerCase()==='node.exe'));assert(ownedProcesses.some(process=>process.Name.toLowerCase()==='msedgewebview2.exe'));assert.equal(main.process_id,child.pid);
 if(mode==='public'){
  assert.equal(createHash('sha256').update(await fs.readFile(join(packageRoot,'preload.js'))).digest('hex'),'87ff30f9890916209c73cf040da30771b4fb94e79bb39dfbbbf60ff3b45522a8');
  assert.equal(await evaluate('typeof window.createCurrentFolder'),'function');assert.equal(await evaluate('document.querySelector(".container")!==null'),true);
  const tree=await until(async()=>{const value=await req('uia',{max_nodes:1500});return value.nodes.some(n=>n.automation_id==='folderName'&&n.control_type===50004)?value:null;},'original native UIA input');const field=tree.nodes.find(n=>n.automation_id==='folderName'&&n.control_type===50004);assert(field?.runtime_id);
  const destination=join(runRoot,'原始插件创建的目录');await req('uia',{operation:'set-value',target:{runtime_id:field.runtime_id},value:destination});
  await until(()=>evaluate('!Array.from(document.querySelectorAll("button")).find(b=>b.textContent.trim()==="新建文件夹").disabled'),'original reactive input');
  if(!hidden){const response=await fetch(session.url+'/__rdesktop__/agent/native-screenshot',{method:'POST',headers:{authorization:'Bearer '+session.token,'content-type':'application/json'},body:JSON.stringify({window})});assert(response.ok);await fs.writeFile(join(runRoot,'native.png'),Buffer.from(await response.arrayBuffer()));}
  const relation=await req('uia',{operation:'relations',target:{runtime_id:field.runtime_id},relation:'labeled-by'});assert.equal(relation.related.total,1);assert.equal(relation.related.elements[0].name,'新建文件夹名：');
  const button=(await req('uia',{max_nodes:1500})).nodes.find(n=>n.name==='新建文件夹'&&n.control_type===50000);assert(button?.enabled);await req('uia',{operation:'invoke',target:{runtime_id:button.runtime_id}});
  await until(()=>fs.stat(destination).then(s=>s.isDirectory(),()=>false),'actual unchanged preload mkdir');
  await fs.writeFile(join(runRoot,'proof.json'),JSON.stringify({mode,hidden,status,ownedProcesses,fieldProviderPid:field.process_id,hostPid:main.process_id,destination,label:relation.related.elements[0].name,originalPreload:true,originalHashes},null,2));
 }else{
  await req('action',{action:'fill',selector:'#message',value:'真实 WebView2'});
  await req('action',{action:'click',selector:'#run'});
  const result=await until(async()=>{const failure=await evaluate('window.failure??null');if(failure)throw Error(failure);return evaluate('window.result?.checks?window.result:null');},'actual Node/CommonJS/DOM checks');
  assert(result.passed,JSON.stringify(result));
  if(!hidden){const response=await fetch(session.url+'/__rdesktop__/agent/native-screenshot',{method:'POST',headers:{authorization:'Bearer '+session.token,'content-type':'application/json'},body:JSON.stringify({window})});assert(response.ok);await fs.writeFile(join(runRoot,'native.png'),Buffer.from(await response.arrayBuffer()));}
  await fs.writeFile(join(runRoot,'proof.json'),JSON.stringify({mode,hidden,status,ownedProcesses,result},null,2));console.log(JSON.stringify(result));
 }
 console.log(JSON.stringify({output:runRoot,mode,hidden,passed:true}));
}catch(error){
 if(session)for(const window of (await call('windows')).windows)await fs.writeFile(join(runRoot,'events.json'),JSON.stringify(await call('events',{window:window.id}),null,2));
 throw error;
}finally{
 if(child.exitCode===null&&session)await call('exit').catch(()=>{});
 const ended=await Promise.race([exited,delay(15000,undefined,{ref:false}).then(()=>{throw Error('Owned application has not exited normally; no process killed.');})]);await fs.writeFile(join(runRoot,'host.log'),log);assert.equal(ended.code,0,log);
 if(originalHashes)assert.deepEqual(await packageHashes(),originalHashes,'The original public package must remain byte-for-byte unchanged');
 const remaining=snapshot();assert(!remaining.some(process=>ownedProcesses.some(owned=>owned.Name.toLowerCase()==='node.exe'&&owned.ProcessId===process.ProcessId&&owned.ParentProcessId===process.ParentProcessId)),'The owned Node worker must exit normally with its host');
}
