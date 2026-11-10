#!/usr/bin/env node
// Verify the generated wasm-bindgen module without a model or a GUI.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { pathToFileURL } from "node:url";

const directory = resolve(process.argv[2] ?? "target/wasm-minishogi");
const engine = await import(pathToFileURL(join(directory, "haitaka_wasm.js")));
engine.initSync({ module: readFileSync(join(directory, "haitaka_wasm_bg.wasm")) });
const start = "rbsgk/4p/5/P4/KGSBR b - 1";
const perft = engine.perft(start, 5);
assert.equal(perft.nodes, 533203);
perft.free();
const result = engine.search(start, 2);
assert.match(result.bestMove, /^[1-5][a-e][1-5][a-e]\+?$/);
const best = result.bestMove;
result.free();
const session = new engine.UsiEngine();
assert(session.send("usi").includes("usiok"));
assert(session.send("isready").includes("readyok"));
assert.deepEqual(session.send(`position startpos moves ${best}`), []);
assert(session.send("go movetime 2").some(line => /^bestmove (?:[1-5][a-e][1-5][a-e]\+?|[PSGBR]\*[1-5][a-e])$/.test(line)));
assert.throws(() => engine.load_nnue(new Uint8Array()), /UNSUPPORTED_RULESET/);
assert.throws(() => engine.load_sfnn(new Uint8Array(), new Uint8Array(), "SFNN_ka2_4096_8_64_progress8"), /UNSUPPORTED_RULESET/);
assert.deepEqual(session.send("position sfen 4k/5/5/5/K4 b - 1 moves " + "5e5d 1a1b 5d5e 1b1a ".repeat(3).trim()), []);
assert(session.send("go depth 2").some(line => line.includes("repetition-draw rules minishogi-shogitter")));
session.free();
console.log(JSON.stringify({ variant: "minishogi", perftDepth5: 533203, bestmove: best, wasmBytes: readFileSync(join(directory, "haitaka_wasm_bg.wasm")).length }));
