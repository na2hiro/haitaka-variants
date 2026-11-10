#!/usr/bin/env node
// Run on generated wasm-bindgen output: node scripts/verify-small-shogi.mjs [target].
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { pathToFileURL } from "node:url";

const root = resolve(process.argv[2] ?? "target");
const cases = [
  ["minishogi", "rbsgk/4p/5/P4/KGSBR b - 1", 2512],
  ["judkins", "rbnsgk/5p/6/6/P5/KGSNBR b - 1", 6183],
  ["gorogoro", "sgkgs/5/1ppp1/1PPP1/5/SGKGS b - 1", 4166],
  ["gorogoro-plus", "sgkgs/5/1ppp1/1PPP1/5/SGKGS b NLnl 1", 44436],
  ["minishogi-kingin", "sgkgs/5/5/5/SGKGS b RBrb 1", 46127],
];
for (const [variant, start, nodes] of cases) {
  const dir = join(root, `wasm-${variant}`);
  const engine = await import(pathToFileURL(join(dir, "haitaka_wasm.js")));
  const wasm = readFileSync(join(dir, "haitaka_wasm_bg.wasm"));
  engine.initSync({ module: wasm });
  const perft = engine.perft(start, 3);
  assert.equal(perft.nodes, nodes, variant);
  perft.free();
  const result = engine.search(start, 2);
  const best = result.bestMove;
  assert(best, variant);
  result.free();
  const session = new engine.UsiEngine();
  assert(session.send("usi").includes("usiok"));
  assert(session.send("isready").includes("readyok"));
  assert.deepEqual(session.send(`position startpos moves ${best}`), []);
  const output = session.send("go nodes 100");
  const reply = output.find(line => line.startsWith("bestmove "))?.slice(9);
  assert(reply && reply !== "resign", variant);
  assert.deepEqual(session.send(`position startpos moves ${best} ${reply}`), []);
  assert.throws(() => engine.load_nnue(new Uint8Array()), /UNSUPPORTED_RULESET/);
  assert.throws(() => engine.load_sfnn(new Uint8Array(), new Uint8Array(), "SFNN_ka2_4096_8_64_progress8"), /UNSUPPORTED_RULESET/);
  session.free();
  console.log(JSON.stringify({ variant, perftDepth3: nodes, bestmove: best, reply, wasmBytes: wasm.length }));
}
