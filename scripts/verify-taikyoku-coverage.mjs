#!/usr/bin/env node
// Verify coverage of the full oracle corpus produced by taikyokuReference.test.ts.
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,createReadStream} from 'node:fs';
import {createInterface} from 'node:readline';
import {resolve,join} from 'node:path';
const root=resolve(process.argv[2]??'target');
const defs=JSON.parse(readFileSync(join(root,'taikyoku-protocol-config.json'),'utf8'));
const inputs=readFileSync(join(root,'taikyoku-reference-input.jsonl'),'utf8').trim().split('\n').map(JSON.parse);
const pieces=new Set(),promotions=new Set(),origins=new Set();
const pathLengths={},pendingLengths={};let i=0,transitions=0;
for await(const line of createInterface({input:createReadStream(join(root,'taikyoku-reference-output.jsonl')),crlfDelay:Infinity})) {
 const q=inputs[i++];assert.equal(q.schemaVersion,5);assert.equal(q.ruleId,64);
 const board=new Map(q.position.pieces.map(p=>[p.at.join(','),p]));
 for(const p of q.position.pieces) {
  pieces.add(`${p.piece}:${p.color}`);
  if(p.origin) origins.add(`${p.piece}:${p.origin}:${p.color}`);
 }
 const pending=q.pending?.path.length??0;pendingLengths[pending]=(pendingLengths[pending]??0)+1;
 const result=JSON.parse(line);
 for(const t of result.transitions) {
  const m=t.move;assert.equal(m.schemaVersion,5);assert(m.path.length>=1&&m.path.length<=4);
  pathLengths[m.path.length]=(pathLengths[m.path.length]??0)+1;
  const mover=board.get(m.from.join(','));assert(mover);
  if(m.promotion) {
   assert(defs.promotion[mover.piece]);promotions.add(`${mover.piece}:${mover.color}`);
   const p=t.position.pieces.find(p=>p.at.join(',')===m.path.at(-1).join(','));
   assert.equal(p.piece,defs.promotion[mover.piece]);
   if(defs.origins[p.piece]) assert.equal(p.origin,mover.piece);
  }
  transitions++;
 }
}
assert.equal(i,inputs.length);
const missingPieces=[],missingPromotions=[],missingOrigins=[];
for(const color of [0,1]) {
 for(const id of defs.ids) if(!pieces.has(`${id}:${color}`)) missingPieces.push(`${id}:${color}`);
 for(const [id,to] of Object.entries(defs.promotion)) if(to&&!promotions.has(`${id}:${color}`)) missingPromotions.push(`${id}:${color}`);
 for(const [id,fronts] of Object.entries(defs.origins)) for(const origin of fronts) if(!origins.has(`${id}:${origin}:${color}`)) missingOrigins.push(`${id}:${origin}:${color}`);
}
assert.deepEqual(missingPieces,[]);assert.deepEqual(missingPromotions,[]);assert.deepEqual(missingOrigins,[]);
assert(pathLengths[4]>0);assert(pendingLengths[3]>0);
const report={positions:i,transitions,pieceColorPairs:pieces.size,promotionColorPairs:promotions.size,originColorPairs:origins.size,pathLengths,pendingLengths,pieces:[...pieces].sort(),promotions:[...promotions].sort(),origins:[...origins].sort(),missingPieces,missingPromotions,missingOrigins};
writeFileSync(join(root,'taikyoku-piece-coverage.json'),JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({...report,pieces:undefined,promotions:undefined,origins:undefined}));
