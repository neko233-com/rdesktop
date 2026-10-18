// Compile the actual CLI-generated application against this candidate source.
import {mkdtemp,readFile,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {resolve,join} from 'node:path';
import {spawnSync} from 'node:child_process';
import assert from 'node:assert/strict';
const root=resolve(import.meta.dirname,'..');
const temp=await mkdtemp(join(tmpdir(),'rdesktop-scaffold-'));
const cli=join(root,'target/debug/rdesktop.exe');
const version=(await readFile(join(root,'Cargo.toml'),'utf8')).match(/\[workspace\.package\][\s\S]*?version = "([^"]+)"/)[1];
const reported=spawnSync(cli,['--version'],{encoding:'utf8',windowsHide:true});
assert.equal(reported.status,0,reported.stderr);assert.equal(reported.stdout.trim(),`rdesktop ${version}`);
const created=spawnSync(cli,['init','native-scaffold-proof'],{cwd:temp,encoding:'utf8',windowsHide:true});assert.equal(created.status,0,created.stderr);
const project=join(temp,'native-scaffold-proof');const manifestPath=join(project,'Cargo.toml');
let manifest=await readFile(manifestPath,'utf8');
assert(manifest.includes(`tag = "v${version}"`),'generated project must consume this exact framework version');
for(const crate of ['rdesktop-core','rdesktop-webview','rdesktop-cef']){
 const source=join(root,'crates',crate).replaceAll('\\','/');
 manifest=manifest.replace(new RegExp(`${crate} = \\{ git = "[^"]+", tag = "[^"]+"(, optional = true)? \\}`),(_match,optional)=>`${crate} = { path = "${source}"${optional??''} }`);
}
assert(!manifest.includes('git ='));
await writeFile(manifestPath,manifest);
for(const mode of [[],['--release']]){
 const checked=spawnSync('cargo',['check','--offline','--features','native-agent',...mode],{cwd:project,encoding:'utf8',windowsHide:true,env:{...process.env,CARGO_TARGET_DIR:join(root,'target')}});
 assert.equal(checked.status,0,checked.stderr);
}
const proof={project,debug:true,release:true,generatedGitTag:true};
await writeFile(join(root,'.cache/scaffold-proof.json'),JSON.stringify(proof,null,2));console.log(JSON.stringify(proof,null,2));
