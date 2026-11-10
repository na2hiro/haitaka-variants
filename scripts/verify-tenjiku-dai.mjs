#!/usr/bin/env node
// Usage: node scripts/verify-tenjiku-dai.mjs target
// First generate Native/WASM artifacts and classicalReference.test.ts oracle data.
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,createReadStream} from 'node:fs';
import {createInterface} from 'node:readline';
import {resolve,join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {execFileSync} from 'node:child_process';
const root=resolve(process.argv[2]??'target');
const rule='tenjiku-dai';
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
const native=createInterface({input:createReadStream(join(root,rule+'-reference-output.jsonl')),crlfDelay:Infinity});
let transitions=0,i=0;
for await(const line of native) {
  const input=inputs[i];const expected=JSON.parse(line);
  const canonical=engine.validate_position_json(input);
  assert.equal(engine.validate_position_json(canonical),canonical);
  const inspect=JSON.parse(engine.inspect_json(input));
  assert.equal(inspect.positionHash,expected.hash);assert.deepEqual(inspect.outcome,expected.outcome);
  assert.deepEqual(JSON.parse(engine.legal_moves_json(input)),expected.moves);
  for(const t of expected.transitions) {
    const next=engine.apply_move_json(input,JSON.stringify(t.move));
    assert.deepEqual(JSON.parse(next).position,t.position);
    const info=JSON.parse(engine.inspect_json(next));assert.equal(info.positionHash,t.hash);assert.deepEqual(info.outcome,t.outcome);
    // Reloading canonical JSON rebuilds all cells and hashes, including promoted royals.
    assert.equal(engine.validate_position_json(next),next);transitions++;
  }
  i++;
}
assert.equal(i,inputs.length);
const start=engine.start_position_json();
const nativeCall=(action,args=[])=>JSON.parse(execFileSync(join(root,'haitaka-'+rule),['json',action,...args],{input:start,encoding:'utf8'}));
const nodes=JSON.parse(engine.perft_json(start,2)).nodes;
assert.equal(nodes,nativeCall('perft',['--depth','2']).nodes);
const reference=JSON.parse(readFileSync(join(root,rule+'-reference-perft.json'),'utf8'));
assert.equal(JSON.parse(engine.perft_json(start,1)).nodes,reference[0]);
assert.equal(nodes,reference[1]);
const nativeSearch=nativeCall('search',['--depth','8','--timeout-ms','30000','--nodes','1000']);
const wasmSearch=JSON.parse(engine.search_json(start,8,30000,1000));
for(const key of ['bestMove','completedDepth','nodeLimitReached','states','score','outcome']) assert.deepEqual(wasmSearch[key],nativeSearch[key],key);
const timed=JSON.parse(engine.search_json(start,100,1,0));assert(timed.bestMove);assert(timed.timedOut);
assert(JSON.parse(engine.legal_moves_json(start)).some(m=>JSON.stringify(m)===JSON.stringify(timed.bestMove)));
const report={schemaVersion:4,initializationMs,linearMemoryBytes:instance.memory.buffer.byteLength,lowBudgetElapsedMs:timed.elapsedMs,positions:inputs.length,transitions,perftDepth2:nodes,search:{bestMove:wasmSearch.bestMove,completedDepth:wasmSearch.completedDepth,states:wasmSearch.states,score:wasmSearch.score},wasmBytes:readFileSync(join(dir,'haitaka_wasm_bg.wasm')).length};
assert(instance.memory.buffer.byteLength<=64*1024*1024);
writeFileSync(join(root,rule+'-wasm-verification.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report));
