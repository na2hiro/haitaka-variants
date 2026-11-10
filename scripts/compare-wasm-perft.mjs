#!/usr/bin/env node
// Usage: node scripts/compare-wasm-perft.mjs BEFORE AFTER PLAYWRIGHT_MODULE OUTPUT
import {readFileSync,writeFileSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {pathToFileURL} from 'node:url';
const [beforePath,afterPath,playwrightModule,output]=process.argv.slice(2);
if(!beforePath||!afterPath||!playwrightModule||!output)throw new Error('Expected BEFORE AFTER PLAYWRIGHT_MODULE OUTPUT');
const {chromium}=await import(pathToFileURL(resolve(playwrightModule)));
const browser=await chromium.launch({executablePath:process.env.CHROME_BIN??'/usr/bin/google-chrome',args:['--no-sandbox']});
try{
 const page=await browser.newPage();
 const dirs={before:resolve(beforePath),after:resolve(afterPath)};
 await page.route('http://wasm.test/**',async route=>{
  const pathname=new URL(route.request().url()).pathname;
  if(pathname==='/')return route.fulfill({contentType:'text/html',body:'<!doctype html><title>WASM comparison</title>'});
  const [variant,file]=pathname.slice(1).split('/');
  if(!dirs[variant]||!['haitaka_wasm.js','haitaka_wasm_bg.wasm'].includes(file))return route.abort();
  return route.fulfill({contentType:file.endsWith('.wasm')?'application/wasm':'application/javascript',body:readFileSync(join(dirs[variant],file))});
 });
 await page.goto('http://wasm.test/');
 const result=await page.evaluate(async()=>{
  const before=await import('/before/haitaka_wasm.js');const after=await import('/after/haitaka_wasm.js');await before.default();await after.default();
  const cases=[['startpos','lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1',4],['endgame','ln1g5/1r4k2/p2pppn2/2ps2p2/1p7/2P6/PPSPPPPLP/2G2K1pr/LN4G1b b BG2SLPnp 61',3]];
  const median=values=>{const s=[...values].sort((a,b)=>a-b);return(s[s.length/2-1]+s[s.length/2])/2;};
  const run=(mod,sfen,depth)=>{const t=performance.now();const r=mod.perft(sfen,depth);const ms=performance.now()-t;const nodes=r.nodes;r.free();return{ms,nodes};};
  const reports=[];
  for(const[name,sfen,depth]of cases){
   for(let i=0;i<8;i++){run(before,sfen,depth);run(after,sfen,depth);}
   const pairs=[];
   for(let i=0;i<40;i++){
    let b,a;if(i%2){a=run(after,sfen,depth);b=run(before,sfen,depth);}else{b=run(before,sfen,depth);a=run(after,sfen,depth);}
    if(a.nodes!==b.nodes)throw new Error('perft mismatch');
    pairs.push({before:b.ms,after:a.ms,nodes:b.nodes,changePercent:100*(a.ms/b.ms-1)});
   }
   reports.push({name,depth,nodes:pairs[0].nodes,beforeMedianMs:median(pairs.map(p=>p.before)),afterMedianMs:median(pairs.map(p=>p.after)),medianPairedChangePercent:median(pairs.map(p=>p.changePercent)),pairs});
  }
  return{userAgent:navigator.userAgent,reports};
 });
 writeFileSync(output,JSON.stringify(result,null,2)+'\n');
 console.log(JSON.stringify(result.reports.map(({pairs,...summary})=>summary)));
}finally{await browser.close();}
