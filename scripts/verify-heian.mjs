#!/usr/bin/env node
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {pathToFileURL} from 'node:url';
const root=resolve(process.argv[2]??'target');
const cases=[
 ['heian-8x8',8,8,'lnsgksnl/8/pppppppp/8/8/PPPPPPPP/8/LNSKGSNL b - 1',[22,484,10296]],
 ['heian-9x8',9,8,'lnsgkgsnl/9/ppppppppp/9/9/PPPPPPPPP/9/LNSGKGSNL b - 1',[26,676,17108]],
 ['heian-9x9',9,9,'lnsgkgsnl/9/ppppppppp/9/9/9/PPPPPPPPP/9/LNSGKGSNL b - 1',[26,676,17108]],
];
function position(w,h,pieces){
 const rows=Array.from({length:h},()=>Array(w).fill(''));
 for(const [x,y,p]of pieces)rows[y-1][w-x]=p;
 return rows.map(row=>{let s='',n=0;for(const p of row){if(p){if(n)s+=n;n=0;s+=p;}else n++;}if(n)s+=n;return s;}).join('/')+' b - 1';
}
for(const [rule,w,h,start,counts]of cases){
 const dir=join(root,'wasm-'+rule);const engine=await import(pathToFileURL(join(dir,'haitaka_wasm.js')));
 const wasm=readFileSync(join(dir,'haitaka_wasm_bg.wasm'));
 const initStarted=performance.now();const instance=engine.initSync({module:wasm});
 const initializationMs=performance.now()-initStarted;assert(initializationMs<5000);
 for(let d=1;d<=3;d++){const result=engine.perft(start,d);assert.equal(result.nodes,counts[d-1]);result.free();}
 const result=engine.search_iterative_deepening(start,2,1000);const best=result.bestMove;assert(best);assert.equal(result.completedDepth,2);result.free();
 const session=new engine.UsiEngine();assert(session.send('usi').includes('usiok'));assert(session.send('isready').includes('readyok'));
 assert.deepEqual(session.send('position startpos moves '+best),[]);
 const out=session.send('go nodes 1');const reply=out.find(s=>s.startsWith('bestmove '))?.slice(9);assert(reply&&reply!=='resign');
 assert.deepEqual(session.send(`position startpos moves ${best} ${reply}`),[]);
 const promotion=position(w,h,[[1,1,'k'],[w,h,'K'],[3,4,'S'],[3,2,'g']]);
 assert.deepEqual(session.send(`position sfen ${promotion} moves 3d3c+ 3b3c`),[]);
 const draw=position(w,h,[[1,1,'k'],[4,4,'K'],[3,3,'p']]);
 assert.deepEqual(session.send(`position sfen ${draw} moves 4d3c`),[]);
 const terminal=session.send('go nodes 1');assert(terminal.includes('bestmove resign'));assert(terminal.some(line=>line.includes('only-kings-draw')));
 assert.throws(()=>engine.load_nnue(new Uint8Array()),/UNSUPPORTED_RULESET/);
 assert.throws(()=>engine.load_sfnn(new Uint8Array(),new Uint8Array(),'SFNN_ka2_4096_8_64_progress8'),/UNSUPPORTED_RULESET/);
 assert.throws(()=>engine.dfpn(start,100,10,1,4),/DFPN_UNSUPPORTED_RULESET/);
 const timed=engine.search_iterative_deepening(start,100,1);
 const lowBudgetElapsedMs=timed.elapsedMs;assert(timed.bestMove);assert(timed.timedOut);
 assert.deepEqual(session.send('position startpos moves '+timed.bestMove),[]);timed.free();
 const linearMemoryBytes=instance.memory.buffer.byteLength;assert(linearMemoryBytes<=64*1024*1024);
 session.free();console.log(JSON.stringify({rule,perft:counts,best,reply,wasmBytes:wasm.length,initializationMs,linearMemoryBytes,lowBudgetElapsedMs}));
}
