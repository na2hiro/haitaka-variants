#!/usr/bin/env node
// Pin Node to a CPU. Compare actual old/new frontend bundles. Includes JSON
// parse/history replay in wall NPS; reports API-internal perft timing separately.
// Usage: taskset -c 7 node scripts/benchmark-wasm-perft.mjs BEFORE AFTER OUTPUT
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFileSync,writeFileSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {pathToFileURL} from 'node:url';
const [before,after,output]=process.argv.slice(2).map(p=>resolve(p));
const affinity=readFileSync('/proc/self/status','utf8').match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1];
assert.equal(affinity,'7','run with taskset -c 7');
const baseline=JSON.parse(readFileSync('plans/historical-shogi-performance/baseline/perft.json','utf8'));
const manifest=JSON.parse(readFileSync('plans/historical-shogi-performance/inputs/positions.json','utf8'));
const rules=['torishogi','washogi','heian-dai','daishogi','tenjiku-dai','dai-dai','maka-dai-dai','taishogi','taikyoku'];
const sha=b=>createHash('sha256').update(b).digest('hex');
async function load(root,rule){
 const dir=join(root,rule,root===after?'generated':'');
 const engine=await import(pathToFileURL(join(dir,'haitaka_engine.js')));
 const bytes=readFileSync(join(dir,'haitaka_engine_bg.wasm'));
 const start=performance.now();const instance=engine.initSync({module:bytes});
 const initializationMs=performance.now()-start;assert(initializationMs<5000);
 return {engine,instance,initializationMs,wasmSha256:sha(bytes),wasmBytes:bytes.length};
}
function batch(engine,input,depth,expected){
 const start=performance.now();let repetitions=0,nodes=0,perftMs=0;
 do{
  const r=JSON.parse(engine.perft_json(input,depth));assert.equal(r.nodes,expected);
  repetitions++;nodes+=r.nodes;perftMs+=r.elapsedMs;
 }while(performance.now()-start<3000);
 const seconds=(performance.now()-start)/1000;
 return {repetitions,totalLeafNodes:nodes,seconds,nps:nodes/seconds,apiInternalPerftSeconds:perftMs/1000,apiInternalPerftNps:perftMs>0?nodes*1000/perftMs:null};
}
function summarize(samples){
 const sorted=samples.map(s=>s.nps).sort((a,b)=>a-b);
 const spread=sorted[2]/sorted[0]-1;
 return {samples,medianNps:sorted[1],sampleRangePercent:spread*100,needsRemeasurement:spread>.05};
}
const report={node:process.version,logicalCpuAffinity:7,metric:'Leaf nodes / total JSON API wall seconds; parser/history replay and result JSON included. API-internal perft timers separately recorded. No TT/bulk/evaluation. One warmup, three alternating >=3s batches per version.',cases:[]};
for(const rule of rules){
 const old=await load(before,rule),current=await load(after,rule);
 const initial=baseline.rules.find(r=>r.name===rule).result;
 const input=old.engine.start_position_json();
 // JSON perft APIs intentionally cap depth at 4; Native library benchmarks
 // can use deeper roots. Freeze a capped control from old WASM before timing.
 const depth=Math.min(initial.depth,4);
 const expected=depth===initial.depth?initial.nodesPerPerft:JSON.parse(old.engine.perft_json(input,depth)).nodes;
 const cases=[{id:rule+'-initial',input,depth,expected}];
 if(rule==='tenjiku-dai'||rule==='taikyoku'){
  const p=manifest.positions.find(p=>p.id===(rule==='taikyoku'?'taikyoku-pending-three':'tenjiku-dai-pending-two'));
  cases.push({id:p.id,input:readFileSync(join('plans/historical-shogi-performance/inputs',p.file),'utf8'),depth:p.depth,expected:p.expectedNodes});
 }
 for(const c of cases){
  assert(Number.isInteger(c.expected)&&c.expected>0);
  for(const e of [old,current]) assert.equal(JSON.parse(e.engine.perft_json(c.input,c.depth)).nodes,c.expected);
  const oldSamples=[],newSamples=[];
  for(let i=0;i<3;i++){
   oldSamples.push(batch(old.engine,c.input,c.depth,c.expected));
   newSamples.push(batch(current.engine,c.input,c.depth,c.expected));
  }
  const previous=summarize(oldSamples),next=summarize(newSamples);
  const memory=current.instance.memory.buffer.byteLength;assert(memory<=64*1024*1024);
  report.cases.push({id:c.id,rule,depth:c.depth,nodesPerPerft:c.expected,inputSha256:sha(c.input),before:previous,after:next,speedup:next.medianNps/previous.medianNps,beforeWasmSha256:old.wasmSha256,afterWasmSha256:current.wasmSha256,beforeWasmBytes:old.wasmBytes,afterWasmBytes:current.wasmBytes,initializationMs:current.initializationMs,linearMemoryBytes:memory});
  writeFileSync(output,JSON.stringify(report,null,2)+'\n');
  console.log(c.id,Math.round(previous.medianNps),Math.round(next.medianNps),'x'+(next.medianNps/previous.medianNps).toFixed(2),previous.needsRemeasurement||next.needsRemeasurement?'remeasure':'');
 }
}
