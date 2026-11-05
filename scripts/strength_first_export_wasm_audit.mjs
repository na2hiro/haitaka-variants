#!/usr/bin/env node
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';
const root = resolve('out/anhoku-strength-first/step6-browser-export-20260908-v1');
const config = JSON.parse(readFileSync('r0/anhoku-strength-first/step6-browser-export-v1.json'));
const mod = await import(pathToFileURL(`${root}/wasm-256/haitaka_wasm.js`));
await mod.default({module_or_path: readFileSync(`${root}/wasm-256/haitaka_wasm_bg.wasm`)});
const model = readFileSync(config.candidate.path);
if (createHash('sha256').update(model).digest('hex') !== config.candidate.sha256) throw Error('model identity');
mod.load_nnue(model);
const input = readFileSync(`${root}/static-audit/positions.jsonl`);
const rows = input.toString().trim().split('\n').map(JSON.parse);
const scores = rows.map(row => {
  const score = mod.evaluate_nnue(row.sfen);
  if (score !== row.nativeScore) throw Error(`WASM/native mismatch ${row.id}: ${score}/${row.nativeScore}`);
  return {id:row.id,score};
});
writeFileSync(`${root}/wasm-parity.json`,JSON.stringify({passed:true,positions:scores.length,modelSha256:config.candidate.sha256,inputSha256:createHash('sha256').update(input).digest('hex'),scores},null,2)+'\n');
console.log(`WASM/native parity passed on ${scores.length} positions`);
