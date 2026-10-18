#!/usr/bin/env node
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
if(process.platform!=='win32'||process.arch!=='x64'){
    console.error('This rdesktop npm distribution supports Windows x64 only.');process.exit(1);
}
const binary=fileURLToPath(new URL('./rdesktop.exe',import.meta.url));
try{
    const metadata=JSON.parse(readFileSync(new URL('../package.json',import.meta.url),'utf8'));
    const digest=createHash('sha256').update(readFileSync(binary)).digest('hex');
    if(!metadata.rdesktopBinarySha256||digest!==metadata.rdesktopBinarySha256)throw Error('Native CLI SHA-256 verification failed');
}catch(error){console.error(`rdesktop: ${error.message}`);process.exit(1);}
const result=spawnSync(binary,process.argv.slice(2),{stdio:'inherit',windowsHide:true});
if(result.error){console.error(`rdesktop: ${result.error.message}`);process.exit(1);}
process.exit(result.status??1);
