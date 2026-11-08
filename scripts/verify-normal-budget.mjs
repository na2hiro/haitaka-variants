#!/usr/bin/env node
// Actual frontend modules: a normal 5-second search, a legal complete move,
// initialization <5s and post-search linear memory <=64 MiB for every old rule.
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
const [vendor,output]=process.argv.slice(2);assert(vendor&&output);
const heian={
 'heian-8x8':'lnsgksnl/8/pppppppp/8/8/PPPPPPPP/8/LNSKGSNL b - 1',
 'heian-9x8':'lnsgkgsnl/9/ppppppppp/9/9/PPPPPPPPP/9/LNSGKGSNL b - 1',
 'heian-9x9':'lnsgkgsnl/9/ppppppppp/9/9/9/PPPPPPPPP/9/LNSGKGSNL b - 1',
};
const rules=[...Object.keys(heian),'shoshogi','chushogi','torishogi','washogi','heian-dai','daishogi','tenjiku-dai','dai-dai','maka-dai-dai','taishogi','taikyoku'];
const report={node:process.version,budgetMs:5000,scope:'Actual refreshed frontend WASM; normal search budget (soft deadline), legal complete move and post-search linear memory. Elapsed time is recorded without equating search states with perft NPS.',rules:[]};
for(const rule of rules){
 const dir=join(resolve(vendor),rule,'generated');const engine=await import(pathToFileURL(join(dir,'haitaka_engine.js')));
 const bytes=readFileSync(join(dir,'haitaka_engine_bg.wasm'));const started=performance.now();
 const instance=engine.initSync({module:bytes});const initializationMs=performance.now()-started;assert(initializationMs<5000);
 let result,wallElapsedMs;const callStarted=performance.now();
 if(rule in heian){
  const search=engine.search_iterative_deepening(heian[rule],100,5000);wallElapsedMs=performance.now()-callStarted;
  assert(search.bestMove);assert(search.timedOut);
  result={bestMove:search.bestMove,elapsedMs:search.elapsedMs,completedDepth:search.completedDepth,states:search.states};
  search.free();const session=new engine.UsiEngine();assert.deepEqual(session.send('position sfen '+heian[rule]+' moves '+result.bestMove),[]);session.free();
 }else{
  const input=engine.start_position_json();const searchStarted=performance.now();const raw=engine.search_json(input,100,5000,0);wallElapsedMs=performance.now()-searchStarted;result=JSON.parse(raw);
  assert(result.bestMove);assert(result.timedOut);
  const legal=JSON.parse(engine.legal_moves_json(input));assert(legal.some(m=>JSON.stringify(m)===JSON.stringify(result.bestMove)));
  const next=engine.apply_move_json(input,JSON.stringify(result.bestMove));assert(!JSON.parse(next).pending);
  result={bestMove:result.bestMove,elapsedMs:result.elapsedMs,completedDepth:result.completedDepth,states:result.states};
 }
 const linearMemoryBytes=instance.memory.buffer.byteLength;assert(linearMemoryBytes<=64*1024*1024,rule+' memory '+linearMemoryBytes);
 report.rules.push({rule,initializationMs,linearMemoryBytes,wallElapsedMs,wasmSha256:createHash('sha256').update(bytes).digest('hex'),...result});
 writeFileSync(output,JSON.stringify(report,null,2)+'\n');console.log(rule,result.elapsedMs.toFixed(3)+'ms',linearMemoryBytes,'bytes, legal complete move');
}
