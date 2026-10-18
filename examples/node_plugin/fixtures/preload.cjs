const fs=require('node:fs'),path=require('node:path');
const helper=require('./helper.cjs'),cycle=require('./cycle-a.cjs');
const loadedWithDocument=document.nodeType===9;
const preloadEnvironment={process:!!process.versions.node,buffer:Buffer.from('预载').toString()==='预载',global:global===window,args:arguments.length===5};
window.runChecks=async()=>{
  const checks={loadedWithDocument,commonjsCached:helper===require('./helper.cjs'),commonjsThis:helper.commonjsThis,cycle:cycle.value==='a:b:a'};
  checks.preloadGlobals=Object.values(preloadEnvironment).every(Boolean);
  checks.commonjsArguments=helper.commonjsArguments;
  checks.lazyModuleGlobals=require('./late.cjs')();
  checks.localGlobalShadowing=require('./shadow.cjs');
  checks.immediate=await new Promise(resolve=>setImmediate(()=>resolve(true)));
  const target=path.join(__dirname,'真实 中文.txt'),text=document.querySelector('#message').value;
  fs.writeFileSync(target,text+' 中文');
  checks.sync=fs.readFileSync(target,'utf8')===text+' 中文';
  checks.stat=fs.statSync(target).isFile()&&!fs.statSync(target).isDirectory();
  const bytes=fs.readFileSync(target);
  checks.buffer=bytes instanceof Uint8Array&&Buffer.isBuffer(bytes)&&bytes.toString('utf8')===text+' 中文'&&Buffer.from(bytes.toString('base64'),'base64').equals(bytes);
  checks.nodeBuffer=require('node:buffer').Buffer.isBuffer(bytes)&&require('node:buffer').Buffer.from('中文').toString()==='中文';
  checks.callback=await new Promise((resolve,reject)=>fs.readFile(target,(error,value)=>error?reject(error):resolve(value.toString()===text+' 中文')));
  checks.promise=await fs.promises.readFile(target,'utf8')===text+' 中文';
  checks.callbackError=await new Promise(resolve=>fs.readFile(path.join(__dirname,'missing'),error=>resolve(error instanceof Error&&error.code==='ENOENT')));
  try{fs.statSync(path.join(__dirname,'missing'));checks.typedError=false;}catch(error){checks.typedError=error.code==='ENOENT';}
  const {EventEmitter}=require('node:events'),emitter=new EventEmitter();let event;
  emitter.once('real-event',value=>{event=value;});emitter.emit('real-event','节点事件');
  emitter.custom={value:13};checks.properties='custom' in emitter&&Object.hasOwn(emitter,'custom')&&!Object.hasOwn(emitter,'notPresent')&&Object.keys(emitter).includes('custom')&&emitter.custom.value===13;delete emitter.custom;checks.delete=!('custom' in emitter);
  await new Promise(resolve=>setTimeout(resolve,50));checks.event=event==='节点事件';
  checks.child=await new Promise((resolve,reject)=>require('node:child_process').execFile(process.execPath,['-e','process.stdout.write("real-node-child")'],(error,stdout)=>error?reject(error):resolve(stdout==='real-node-child')));
  checks.realNode=!!process.versions.node&&process.versions.electron===undefined;
  checks.customErrorNames=['constructor','__proto__','CustomPluginError'].every(name=>{try{require('node:vm').runInNewContext(`(()=>{const e=new Error('owned-error');e.name=${JSON.stringify(name)};throw e;})()`);return false;}catch(error){return error instanceof Error&&error.name===name;}});
  checks.domCallback=document.querySelector('#message').value===text;
  return{checks,passed:Object.values(checks).every(value=>value===true),node:process.versions.node,electron:process.versions.electron??null};
};
window.runBufferCases=()=>require('./buffer-cases.cjs')(Buffer);
window.runMemoryCases=()=>require('./memory-cases.cjs')(Buffer,require,__dirname);
window.runPrivateTransportChecks=async()=>{
  const original=window.fetch;
  try{
    window.fetch=()=>{throw new Error('Owned plugin fetch override must not intercept private Node events.');};
    const immediate=await new Promise(resolve=>setImmediate(()=>resolve(true)));
    const callback=await new Promise((resolve,reject)=>fs.readFile(path.join(__dirname,'真实 中文.txt'),(error,value)=>error?reject(error):resolve(Buffer.isBuffer(value))));
    return {immediate,callback};
  }finally{window.fetch=original;}
};
window.runSlowTransportChecks=async()=>{
  const send=XMLHttpRequest.prototype.send,starts=[],ends=[];
  try{
    XMLHttpRequest.prototype.send=function(body){
      const events=JSON.parse(body).operation==='events';
      if(events)starts.push(performance.now());
      const result=send.call(this,body);
      if(events){const deadline=performance.now()+60;while(performance.now()<deadline){}ends.push(performance.now());}
      return result;
    };
    const deadline=performance.now()+4000;
    while(ends.length<3&&performance.now()<deadline)await new Promise(resolve=>setTimeout(resolve,10));
    const gaps=starts.slice(1).map((start,index)=>start-ends[index]);
    return {receivedPolls:ends.length>=3,yieldsAfterSlowReply:gaps.length>=2&&gaps.every(gap=>gap>=15)};
  }finally{XMLHttpRequest.prototype.send=send;}
};
window.runTransportChecks=async()=>{
  const checks={},send=XMLHttpRequest.prototype.send,append=fs.appendFileSync,read=fs.readFileSync;
  const target=path.join(__dirname,'transport-replay.txt');fs.writeFileSync(target,'');
  let lost=false,calls=0;
  try{
    const b=Buffer.from([1,2,3,4]);
    const pending=new Promise((resolve,reject)=>fs.readFile(target,(error,value)=>error?reject(error):resolve(Buffer.isBuffer(value))));
    XMLHttpRequest.prototype.send=function(body){const request=JSON.parse(body);if(request.operation==='buffer')calls++;const result=send.call(this,body);if(request.operation==='buffer'&&!lost){lost=true;throw new DOMException('Injected lost Buffer response after execution','NetworkError');}return result;};
    checks.bufferReplyLost=b.fill(7)===b&&Array.from(b).every(value=>value===7)&&lost&&calls===2;
    XMLHttpRequest.prototype.send=send;
    checks.bufferReplyPreservesPendingCallback=await pending;
    lost=false;calls=0;
    XMLHttpRequest.prototype.send=function(body){const request=JSON.parse(body);if(request.operation==='call')calls++;const result=send.call(this,body);if(request.operation==='call'&&!lost){lost=true;throw new DOMException('Injected lost append response after execution','NetworkError');}return result;};
    try{append(target,'x');checks.appendThrows=false;}catch(error){checks.appendThrows=error.name==='NetworkError';}
    XMLHttpRequest.prototype.send=send;
    checks.appendNotReplayed=calls===1&&read(target,'utf8')==='x';
  }finally{XMLHttpRequest.prototype.send=send;}
  return checks;
};
window.runFastBufferChecks=()=>{
  const b=Buffer.alloc(8),send=XMLHttpRequest.prototype.send;let requests=0,total=0;
  try{XMLHttpRequest.prototype.send=function(...args){requests++;return send.apply(this,args);};for(let index=0;index<100000;index++){b.writeUInt32LE(index,0);total+=b.readUInt32LE(0);}}
  finally{XMLHttpRequest.prototype.send=send;}
  return {numericResult:total===4999950000,noIpcInNumericLoop:requests===0};
};
