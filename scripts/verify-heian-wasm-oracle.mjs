#!/usr/bin/env node
// Compare every Native TSV transition against a verification-only WASM wrapper.
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
const root=resolve(process.argv[2]??'target');const report=[];
const sha=b=>createHash('sha256').update(b).digest('hex');
for(const rule of ['heian-8x8','heian-9x8','heian-9x9']){
 const dir=join(root,'heian-wasm-oracle',rule),engine=await import(pathToFileURL(join(dir,'oracle.js')));
 const bytes=readFileSync(join(dir,'oracle_bg.wasm'));const begin=performance.now();
 const instance=engine.initSync({module:bytes});const initializationMs=performance.now()-begin;assert(initializationMs<5000);
 const native=readFileSync(join(root,rule+'-transitions.tsv'),'utf8');
 const lines=native.trimEnd().split('\n');
 const roots=lines.filter(s=>s.split('\t')[1]==='=').map(s=>s.split('\t')[0]);
 assert(roots.length>=128);const wasm=engine.oracle(roots.join('\n'));
 assert.equal(wasm,native,rule+' all Native/WASM legal moves, resulting SFEN, side and outcomes');
 const linearMemoryBytes=instance.memory.buffer.byteLength;assert(linearMemoryBytes<=64*1024*1024);
 report.push({rule,positions:roots.length,transitions:lines.length-roots.length,byteIdentical:true,nativeAndWasmOutputSha256:sha(native),wasmSha256:sha(bytes),wasmBytes:bytes.length,initializationMs,linearMemoryBytes,scope:'Verification-only wrapper over the same core, compared to freshly generated Native TSV already independently checked with Shogitter. Production USI/perft/search exports tested separately.'});
 console.log(rule,roots.length,lines.length-roots.length,'all transitions match');
}
writeFileSync(process.argv[3],JSON.stringify(report,null,2)+'\n');
