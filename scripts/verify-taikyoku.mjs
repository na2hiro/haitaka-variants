#!/usr/bin/env node
// Usage: node scripts/verify-taikyoku.mjs target
// Generate Native/WASM artifacts and taikyokuReference.test.ts oracle data first.
// Each worker verifies independent positions, including every transition's full history.
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,createReadStream} from 'node:fs';
import {createInterface} from 'node:readline';
import {resolve,join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {execFileSync} from 'node:child_process';
import {Worker,isMainThread,workerData,parentPort} from 'node:worker_threads';
const root=resolve(process.argv[2]??'target');
const rule='taikyoku';
const initStart=performance.now();
const dir=join(root,'wasm-'+rule);
const engine=await import(pathToFileURL(join(dir,'haitaka_wasm.js')));
const instance=engine.initSync({module:readFileSync(join(dir,'haitaka_wasm_bg.wasm'))});
const initializationMs=performance.now()-initStart;
assert(initializationMs<5000);
assert.equal(engine.search,undefined);assert.equal(engine.UsiEngine,undefined);
assert.throws(()=>engine.load_nnue(new Uint8Array()),/NNUE_UNSUPPORTED_RULESET/);
assert.throws(()=>engine.load_sfnn(new Uint8Array(),new Uint8Array(),'mini'),/SFNN_UNSUPPORTED_RULESET/);
const inputs=readFileSync(join(root,rule+'-reference-input.jsonl'),'utf8').trim().split('\n');
async function verifyPartition(index,count) {
  const native=createInterface({input:createReadStream(join(root,rule+'-reference-output.jsonl')),crlfDelay:Infinity});
  let transitions=0,positions=0,i=0;
  for await(const line of native) {
    const at=i++;
    if(at%count!==index) continue;
    if(positions%16===0) console.log('Taikyoku WASM worker',index,'position',at,'/',inputs.length,'transitions',transitions);
    const input=inputs[at];const expected=JSON.parse(line);
    const canonical=engine.validate_position_json(input);
    assert.equal(engine.validate_position_json(canonical),canonical);
    const inspect=JSON.parse(engine.inspect_json(input));
    assert.equal(inspect.positionHash,expected.hash);assert.deepEqual(inspect.outcome,expected.outcome);
    assert.deepEqual(JSON.parse(engine.legal_moves_json(input)),expected.moves);
    for(const t of expected.transitions) {
      const next=engine.apply_move_json(input,JSON.stringify(t.move));
      assert.deepEqual(JSON.parse(next).position,t.position);
      const info=JSON.parse(engine.inspect_json(next));assert.equal(info.positionHash,t.hash);assert.deepEqual(info.outcome,t.outcome);
      // Full history replay and independent canonical reload for every transition.
      assert.equal(engine.validate_position_json(next),next);transitions++;
    }
    positions++;
  }
  assert.equal(i,inputs.length);
  assert(instance.memory.buffer.byteLength<=64*1024*1024);
  return {index,count,positions,transitions,initializationMs,linearMemoryBytes:instance.memory.buffer.byteLength};
}
if(!isMainThread) {
  parentPort.postMessage(await verifyPartition(workerData.index,workerData.count));
} else {
  const count=Number(process.env.TAIKYOKU_WASM_WORKERS??4);
  assert(Number.isInteger(count)&&count>=1&&count<=8);
  const partitions=count===1?[await verifyPartition(0,1)]:await Promise.all(Array.from({length:count},(_,index)=>new Promise((resolve,reject)=>{
    const worker=new Worker(new URL(import.meta.url),{workerData:{index,count},argv:[root]});
    let result;
    worker.on('message',value=>{result=value;});worker.on('error',reject);
    worker.on('exit',code=>code===0&&result?resolve(result):reject(Error(`WASM worker ${index} exited ${code}`)));
  })));
  const transitions=partitions.reduce((sum,p)=>sum+p.transitions,0);
  assert.equal(partitions.reduce((sum,p)=>sum+p.positions,0),inputs.length);
  const coverage=JSON.parse(readFileSync(join(root,rule+'-piece-coverage.json'),'utf8'));
  assert.equal(transitions,coverage.transitions);
  const start=engine.start_position_json();
  const nativeCall=(action,args=[])=>JSON.parse(execFileSync(join(root,'haitaka-'+rule),['json',action,...args],{input:start,encoding:'utf8'}));
  const nodes=JSON.parse(engine.perft_json(start,2)).nodes;
  assert.equal(nodes,nativeCall('perft',['--depth','2']).nodes);
  const reference=JSON.parse(readFileSync(join(root,rule+'-reference-perft.json'),'utf8'));
  assert.equal(JSON.parse(engine.perft_json(start,1)).nodes,reference[0]);assert.equal(nodes,reference[1]);
  const nativeSearch=nativeCall('search',['--depth','8','--timeout-ms','30000','--nodes','1000']);
  const wasmSearch=JSON.parse(engine.search_json(start,8,30000,1000));
  for(const key of ['bestMove','completedDepth','nodeLimitReached','states','score','outcome']) assert.deepEqual(wasmSearch[key],nativeSearch[key],key);
  const timed=JSON.parse(engine.search_json(start,100,1,0));assert(timed.bestMove);assert(timed.timedOut);
  assert(JSON.parse(engine.legal_moves_json(start)).some(m=>JSON.stringify(m)===JSON.stringify(timed.bestMove)));
  const pendingFallbacks=[];
  for(const input of inputs.filter(s=>JSON.parse(s).pending)) {
    const q=JSON.parse(input),result=JSON.parse(engine.search_json(input,100,1,0));assert(result.bestMove);
    assert(JSON.parse(engine.legal_moves_json(input)).some(m=>JSON.stringify(m)===JSON.stringify(result.bestMove)));
    assert.deepEqual(result.bestMove.path.slice(0,q.pending.path.length),q.pending.path);
    pendingFallbacks.push({pendingHops:q.pending.path.length,bestMove:result.bestMove,elapsedMs:result.elapsedMs});
  }
  const linearMemoryBytes=Math.max(instance.memory.buffer.byteLength,...partitions.map(p=>p.linearMemoryBytes));
  assert(linearMemoryBytes<=64*1024*1024);
  const report={schemaVersion:5,initializationMs,linearMemoryBytes,lowBudgetElapsedMs:timed.elapsedMs,positions:inputs.length,transitions,partitions,pendingFallbacks,perftDepth2:nodes,search:{bestMove:wasmSearch.bestMove,completedDepth:wasmSearch.completedDepth,states:wasmSearch.states,score:wasmSearch.score},wasmBytes:readFileSync(join(dir,'haitaka_wasm_bg.wasm')).length};
  writeFileSync(join(root,rule+'-wasm-verification.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report));
}
