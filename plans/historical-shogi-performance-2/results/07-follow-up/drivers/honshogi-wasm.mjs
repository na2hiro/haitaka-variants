import assert from 'node:assert/strict';
import {readFileSync,writeFileSync} from 'node:fs';
import {join} from 'node:path';
import {chromium} from '/home/na2hiro/proj/shogitter-3/node_modules/@playwright/test/index.mjs';
const root='/home/na2hiro/proj/haitaka-variants/target/honshogi-historical-impact';
assert.equal(readFileSync('/proc/self/status','utf8').match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1],'7');
const browser=await chromium.launch({executablePath:'/usr/bin/google-chrome',args:['--no-sandbox']});
try{
 const page=await browser.newPage();
 await page.route('http://wasm.test/**',async route=>{
  const p=new URL(route.request().url()).pathname;
  if(p==='/')return route.fulfill({contentType:'text/html',body:'<!doctype html><title>Honshogi historical impact benchmark</title>'});
  const [side,variant,file]=p.slice(1).split('/');
  if(!['before','after'].includes(side)||!['scalar','simd'].includes(variant)||!['haitaka_wasm.js','haitaka_wasm_bg.wasm'].includes(file))return route.abort();
  return route.fulfill({contentType:file.endsWith('.wasm')?'application/wasm':'application/javascript',body:readFileSync(join(root,side,variant,file))});
 });
 await page.goto('http://wasm.test/');
 const report={browserVersion:browser.version(),metric:'Actual production WASM perft API wall NPS, including SFEN parse and result access/free; standard API uses leaf bulk counting in both versions, no evaluation/pruning/TT. Five alternating >=5s samples, 3s warmup, CPU7. Distinct from leaf-applied Native metric.',cases:[]};
 for(const variant of ['scalar','simd']){
  await page.evaluate(async variant=>{for(const side of ['before','after']){const m=await import(`/${side}/${variant}/haitaka_wasm.js`);await m.default();(window.engines??={})[side+'-'+variant]=m;}},variant);
  for(const input of [
   {label:'初期局面',sfen:'lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1',depth:5,expected:19861490},
   {label:'持駒の多い終盤',sfen:'ln1g5/1r4k2/p2pppn2/2ps2p2/1p7/2P6/PPSPPPPLP/2G2K1pr/LN4G1b b BG2SLPnp 61',depth:3,expected:3107043}
  ]){
   const row=await page.evaluate(({variant,input})=>{
    const run=side=>{const r=window.engines[side+'-'+variant].perft(input.sfen,input.depth);const nodes=r.nodes;r.free();if(nodes!==input.expected)throw Error('perft mismatch '+nodes);return nodes;};
    for(const side of ['before','after']){const start=performance.now();do{run(side);}while(performance.now()-start<3000);}
    const samples={before:[],after:[]};
    for(let i=0;i<5;i++)for(const side of(i%2?['after','before']:['before','after'])){
     const start=performance.now();let nodes=0,repetitions=0;
     do{nodes+=run(side);repetitions++;}while(performance.now()-start<5000);
     const seconds=(performance.now()-start)/1000;samples[side].push({seconds,repetitions,nodesPerPerft:input.expected,totalLeafNodes:nodes,nps:nodes/seconds});
    }
    const out={variant,label:input.label,sfen:input.sfen,depth:input.depth,nodesPerPerft:input.expected};
    for(const side of ['before','after']){const a=samples[side],n=a.map(s=>s.nps).sort((a,b)=>a-b);out[side]={samples:a,medianNps:n[2],sampleRangePercent:(n.at(-1)/n[0]-1)*100};}
    out.npsChangePercent=(out.after.medianNps/out.before.medianNps-1)*100;return out;
   },{variant,input});
   report.cases.push(row);writeFileSync(join(root,'wasm-results.json'),JSON.stringify(report,null,2)+'\n');console.log(variant,input.label,Math.round(row.before.medianNps),Math.round(row.after.medianNps),row.npsChangePercent.toFixed(3));
  }
 }
}finally{await browser.close();}
