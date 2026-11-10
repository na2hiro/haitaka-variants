#!/usr/bin/env node
// Usage: node scripts/verify-large-hashes.mjs target RULE BEFORE_WASM_DIR
// Supports JSON v3-v5 (four classical and five large rules).
// Reuse a corpus already independently checked against Shogitter. Before editing
// the engine, preserve its WASM .js/.wasm in BEFORE_WASM_DIR and native CLI as
// BEFORE_WASM_DIR/../before-RULE. Supports gzip oracle data without unpacking.
// LARGE_HASH_WORKERS=1..8 partitions the WASM pass (default 1).
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {createReadStream, existsSync, readFileSync, writeFileSync} from 'node:fs';
import {createInterface} from 'node:readline';
import {createGunzip} from 'node:zlib';
import {resolve, join, dirname} from 'node:path';
import {pathToFileURL} from 'node:url';
import {spawn, execFileSync} from 'node:child_process';
import {Worker, isMainThread, workerData, parentPort} from 'node:worker_threads';
const root=resolve(process.argv[2]??'target');
const rule=process.argv[3];
assert(['torishogi','washogi','heian-dai','daishogi','tenjiku-dai','dai-dai','maka-dai-dai','taishogi','taikyoku'].includes(rule));
const before=resolve(process.argv[4]);
const inputs=readFileSync(join(root,rule+'-reference-input.jsonl'),'utf8').trim().split('\n');
const plain=join(root,rule+'-reference-output.jsonl');
const oraclePath=existsSync(plain)?plain:plain+'.gz';
function oracle() {
  const stream=createReadStream(oraclePath);
  return createInterface({input:oraclePath.endsWith('.gz')?stream.pipe(createGunzip()):stream,crlfDelay:Infinity});
}
async function load(dir) {
  const basename=existsSync(join(dir,'haitaka_wasm.js'))?'haitaka_wasm':'haitaka_engine';
  const engine=await import(pathToFileURL(join(dir,basename+'.js')));
  const started=performance.now();
  const instance=engine.initSync({module:readFileSync(join(dir,basename+'_bg.wasm'))});
  const initializationMs=performance.now()-started;assert(initializationMs<5000);
  return {engine,instance,initializationMs};
}
async function nativeOracle() {
  const child=spawn(join(root,'haitaka-'+rule),['json','oracle'],{stdio:['pipe','pipe','inherit']});
  const done=new Promise((resolve,reject)=>{
    child.on('error',reject);
    child.on('close',code=>code===0?resolve():reject(Error('native exit '+code)));
  });
  createReadStream(join(root,rule+'-reference-input.jsonl')).pipe(child.stdin);
  const expected=oracle()[Symbol.asyncIterator]();
  let positions=0,transitions=0;
  for await(const line of createInterface({input:child.stdout,crlfDelay:Infinity})) {
    const old=await expected.next();
    assert(!old.done);
    assert(line===old.value,'native oracle mismatch at position '+positions);
    transitions+=JSON.parse(line).transitions.length;
    if(++positions%128===0) console.log(rule,'native',positions,'/',inputs.length);
  }
  await done;assert((await expected.next()).done);assert.equal(positions,inputs.length);
  return {positions,transitions,byteIdentical:true};
}
const mask=(1n<<64n)-1n;
const rotate=(n,bits)=>((n<<bits)|(n>>(64n-bits)))&mask;
function childContext(parent,hash) {
  // Original context: ordered fold of historical FNVs XOR terminal hash.
  // Recover the old fold from actual old WASM, then append the independently
  // verified child FNV. Pending affects the terminal term, not the history.
  const fold=BigInt('0x'+parent.contextHash)^rotate(BigInt('0x'+parent.positionHash),13n);
  const child=BigInt('0x'+hash);
  return (rotate(fold,7n)^((child*0x9e3779b97f4a7c15n)&mask)^rotate(child,13n)).toString(16).padStart(16,'0');
}
async function partition(index,count) {
  const {engine,instance,initializationMs}=await load(join(root,'wasm-'+rule));
  const old=(await load(before)).engine;
  let i=0,positions=0,transitions=0;
  for await(const line of oracle()) {
    const at=i++;
    if(at%count!==index) continue;
    const input=inputs[at],expected=JSON.parse(line);
    const canonical=engine.validate_position_json(input);
    assert.equal(engine.validate_position_json(canonical),canonical);
    const info=JSON.parse(engine.inspect_json(input));
    const oldInfo=JSON.parse(old.inspect_json(input));
    assert.deepEqual(info,oldInfo,'legacy inspect '+at);
    assert.equal(info.positionHash,expected.hash);
    assert.deepEqual(info.outcome,expected.outcome);
    assert.deepEqual(JSON.parse(engine.legal_moves_json(input)),expected.moves);
    if(rule==='tenjiku-dai'||rule==='taikyoku') {
      // perft applies every leaf through the position-owned generated cache;
      // public apply reparses the game and intentionally checks geometry again.
      assert.equal(JSON.parse(engine.perft_json(input,1)).nodes,expected.moves.length,'cached WASM perft(1) '+at);
    }
    for(const t of expected.transitions) {
      const next=engine.apply_move_json(input,JSON.stringify(t.move));
      assert.deepEqual(JSON.parse(next).position,t.position);
      const nextInfo=JSON.parse(engine.inspect_json(next));
      assert.equal(nextInfo.positionHash,t.hash);
      assert.deepEqual(nextInfo.outcome,t.outcome);
      assert.equal(nextInfo.contextHash,childContext(oldInfo,t.hash),'legacy child context '+at);
      // inspect_json has already parsed and replayed this entire child history.
      transitions++;
    }
    if(++positions%64===0) console.log(rule,'WASM partition',index,'positions',positions,'transitions',transitions);
  }
  assert.equal(i,inputs.length);
  const linearMemoryBytes=instance.memory.buffer.byteLength;
  assert(linearMemoryBytes<=64*1024*1024);
  return {index,count,positions,transitions,initializationMs,linearMemoryBytes};
}
if(!isMainThread) {
  parentPort.postMessage(await partition(workerData.index,workerData.count));
} else {
  const native=await nativeOracle();
  const count=Number(process.env.LARGE_HASH_WORKERS??1);
  assert(Number.isInteger(count)&&count>=1&&count<=8);
  const partitions=count===1?[await partition(0,1)]:await Promise.all(
    Array.from({length:count},(_,index)=>new Promise((resolve,reject)=>{
      const w=new Worker(new URL(import.meta.url),{argv:[root,rule,before],workerData:{index,count}});
      let report;
      w.on('message',r=>{report=r;});w.on('error',reject);
      w.on('exit',code=>code===0&&report?resolve(report):reject(Error('WASM worker exit '+code)));
    })));
  assert.equal(partitions.reduce((n,p)=>n+p.positions,0),native.positions);
  assert.equal(partitions.reduce((n,p)=>n+p.transitions,0),native.transitions);
  const {engine,instance}=await load(join(root,'wasm-'+rule));
  const old=(await load(before)).engine;
  assert.equal(engine.search,undefined);assert.equal(engine.UsiEngine,undefined);
  assert.throws(()=>engine.load_nnue(new Uint8Array()),/NNUE_UNSUPPORTED_RULESET/);
  assert.throws(()=>engine.load_sfnn(new Uint8Array(),new Uint8Array(),'mini'),/SFNN_UNSUPPORTED_RULESET/);
  const start=engine.start_position_json();
  const nativeCall=(action,args=[],input=start,cli=join(root,'haitaka-'+rule))=>
    JSON.parse(execFileSync(cli,['json',action,...args],{input,encoding:'utf8'}));
  const perft=[1,2].map(depth=>JSON.parse(engine.perft_json(start,depth)).nodes);
  const perftFile=join(root,rule+'-reference-perft.json');
  const referencePerft=existsSync(perftFile)?JSON.parse(readFileSync(perftFile,'utf8')):JSON.parse(readFileSync(join(root,rule+'-reference-summary.json'),'utf8')).perft;
  assert.deepEqual(perft,referencePerft);
  for(const depth of [1,2]) assert.equal(perft[depth-1],nativeCall('perft',['--depth',String(depth)]).nodes);
  const budget=['--depth','8','--timeout-ms','30000','--nodes','1000'];
  const nativeSearch=nativeCall('search',budget);
  const wasmSearch=JSON.parse(engine.search_json(start,8,30000,1000));
  const oldCli=join(dirname(before),'before-'+rule);
  const oldSearch=existsSync(oldCli)?nativeCall('search',budget,start,oldCli):JSON.parse(old.search_json(start,8,30000,1000));
  for(const key of ['bestMove','completedDepth','nodeLimitReached','states','score','outcome'])
    assert.deepEqual(wasmSearch[key],nativeSearch[key],key);
  if(process.env.LARGE_HASH_SEARCH_PARITY==='1') {
    for(const key of ['bestMove','completedDepth','nodeLimitReached','states','score','outcome'])
      assert.deepEqual(nativeSearch[key],oldSearch[key],'previous search '+key);
  }
  const timed=JSON.parse(engine.search_json(start,100,1,0));
  assert(timed.bestMove);assert(timed.timedOut);
  assert(JSON.parse(engine.legal_moves_json(start)).some(m=>JSON.stringify(m)===JSON.stringify(timed.bestMove)));
  const pendingFallbacks=[];
  for(const input of inputs.filter(s=>JSON.parse(s).pending)) {
    const q=JSON.parse(input), result=JSON.parse(engine.search_json(input,100,1,0));
    assert(result.bestMove);
    assert(JSON.parse(engine.legal_moves_json(input)).some(m=>JSON.stringify(m)===JSON.stringify(result.bestMove)));
    const prefix=q.pending.path??[q.pending.via];
    assert.deepEqual(result.bestMove.path.slice(0,prefix.length),prefix);
    pendingFallbacks.push({pendingHops:prefix.length,elapsedMs:result.elapsedMs});
  }
  // Sample Native's public contextHash directly; every corpus position/child
  // also checks old actual WASM inspect and the original FNV history fold.
  for(const i of [...new Set([0,Math.floor(inputs.length/2),inputs.length-1,
    ...inputs.flatMap((s,i)=>JSON.parse(s).pending?[i]:[])])]) {
    assert.deepEqual(nativeCall('inspect',[],inputs[i]),JSON.parse(old.inspect_json(inputs[i])));
  }
  const linearMemoryBytes=Math.max(instance.memory.buffer.byteLength,...partitions.map(p=>p.linearMemoryBytes));
  assert(linearMemoryBytes<=64*1024*1024);
  const sha=p=>createHash('sha256').update(readFileSync(p)).digest('hex');
  const searchSummary=s=>({bestMove:s.bestMove,completedDepth:s.completedDepth,states:s.states,score:s.score});
  const report={rule,positions:native.positions,transitions:native.transitions,native,
    legacyPositionAndContextHashDifferences:0,legacySearchParity:process.env.LARGE_HASH_SEARCH_PARITY==='1',partitions,perft,
    inputSha256:sha(join(root,rule+'-reference-input.jsonl')),
    beforeWasmSha256:sha(join(before,existsSync(join(before,'haitaka_wasm_bg.wasm'))?'haitaka_wasm_bg.wasm':'haitaka_engine_bg.wasm')),
    afterWasmSha256:sha(join(root,'wasm-'+rule,'haitaka_wasm_bg.wasm')),
    linearMemoryBytes,wasmBytes:readFileSync(join(root,'wasm-'+rule,'haitaka_wasm_bg.wasm')).length,
    lowBudgetElapsedMs:timed.elapsedMs,pendingFallbacks,
    search:searchSummary(wasmSearch),beforeSearch:searchSummary(oldSearch)};
  writeFileSync(join(root,'hash-stage2',rule+'-verification.json'),JSON.stringify(report,null,2)+'\n');
  console.log(JSON.stringify(report));
}
