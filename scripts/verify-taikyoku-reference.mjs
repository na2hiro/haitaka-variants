#!/usr/bin/env node
// Native must be built. Generate the complete corpus once, then compare source
// positions independently, preserving full adjudication and all transitions.
// Usage: node scripts/verify-taikyoku-reference.mjs target ../shogitter-3
// Set TAIKYOKU_REUSE_ORACLE=1 to reuse an existing corpus; every shard checks
// that its regenerated input is byte-identical to that corpus.
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,openSync,closeSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {spawn} from 'node:child_process';
import {resolve,join} from 'node:path';
const root=resolve(process.argv[2]??'target');
const frontend=resolve(process.argv[3]??'../shogitter-3');
const shards=Number(process.env.TAIKYOKU_ORACLE_SHARDS??4);
assert(Number.isInteger(shards)&&shards>=1&&shards<=8);
const env={...process.env,HAITAKA_ENGINE_DIR:resolve(root,'..'),TAIKYOKU_ORACLE:'1',TAIKYOKU_DATASET:'taikyoku',TAIKYOKU_POSITIONS:'128'};
async function run(label,extra) {
 const log=join(root,`taikyoku-reference-${label}.log`),fd=openSync(log,'w');
 const child=spawn('pnpm',['exec','vitest','run','app/bot/taikyokuReference.test.ts','--maxWorkers=1'],{cwd:frontend,env:{...env,...extra},stdio:['ignore',fd,fd]});
 try {await new Promise((resolve,reject)=>{child.on('error',reject);child.on('close',code=>code===0?resolve():reject(Error(`${label} exited ${code}; see ${log}`)));});}
 finally {closeSync(fd);}
 console.log('Taikyoku reference finished',label);
}
if(!process.env.TAIKYOKU_REUSE_ORACLE) await run('generate',{TAIKYOKU_GENERATE_ONLY:'1'});
const sha=name=>createHash('sha256').update(readFileSync(join(root,name))).digest('hex');
const inputSha256=sha('taikyoku-reference-input.jsonl'),nativeSha256=sha('haitaka-taikyoku');
await Promise.all(Array.from({length:shards},(_,shard)=>run(`shard-${shard}`,{TAIKYOKU_GENERATE_ONLY:'',TAIKYOKU_REUSE_ORACLE:'1',TAIKYOKU_ORACLE_SHARDS:String(shards),TAIKYOKU_ORACLE_SHARD:String(shard)})));
assert.equal(sha('taikyoku-reference-input.jsonl'),inputSha256);assert.equal(sha('haitaka-taikyoku'),nativeSha256);
const reports=Array.from({length:shards},(_,shard)=>{
 const prefix=shards===1?'taikyoku':`taikyoku-shard-${shard}`;
 const report=JSON.parse(readFileSync(join(root,`${prefix}-reference-summary.json`),'utf8'));
 assert.deepEqual(JSON.parse(readFileSync(join(root,`${prefix}-differences.json`),'utf8')),[]);
 assert.equal(report.shards,shards);assert.equal(report.shard,shard);assert.equal(report.randomPositions,128);
 assert.equal(report.seed,'0x64c02026');assert.equal(report.fixedPositions,report.positions-128);
 assert.equal(report.verifiedPositions,Array.from({length:report.positions},(_,i)=>i).filter(i=>i%shards===shard).length);
 return report;
});
const positions=reports.reduce((n,r)=>n+r.verifiedPositions,0),transitions=reports.reduce((n,r)=>n+r.transitions,0);
assert(reports.every(r=>r.positions===positions));
const report={seed:'0x64c02026',positions,transitions,randomPositions:128,fixedPositions:positions-128,inputSha256,nativeSha256,shards:reports};
writeFileSync(join(root,'taikyoku-reference-summary.json'),JSON.stringify(report,null,2)+'\n');
writeFileSync(join(root,'taikyoku-differences.json'),'[]\n');console.log(JSON.stringify(report));
