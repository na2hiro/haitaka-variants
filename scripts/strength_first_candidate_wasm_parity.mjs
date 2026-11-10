#!/usr/bin/env node
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { pathToFileURL } from "node:url";

function argument(name, fallback = null) {
  const index = process.argv.indexOf(name);
  if (index < 0) return fallback;
  if (index + 1 >= process.argv.length) throw new Error(`${name} requires a value`);
  return process.argv[index + 1];
}

const pkgDir = resolve(argument("--pkg-dir"));
const networkPath = resolve(argument("--network"));
const corpusPath = resolve(argument("--corpus"));
const outputPath = resolve(argument("--output"));
const count = Number(argument("--count", "1024"));
if (!Number.isInteger(count) || count <= 0) throw new Error("--count must be positive");

const imported = await import(pathToFileURL(resolve(pkgDir, "haitaka_wasm.js")).href);
// wasm-pack's nodejs target initializes its sibling .wasm at require time and
// dynamic import exposes that module through `default`. Web/bundler targets
// instead expose an async default initializer, which Node must call explicitly.
let wasm;
if (typeof imported.default === "function") {
  const wasmBytes = await readFile(resolve(pkgDir, "haitaka_wasm_bg.wasm"));
  await imported.default(wasmBytes);
  wasm = imported;
} else {
  wasm = imported.default && typeof imported.default === "object" ? imported.default : imported;
}
const networkBytes = await readFile(networkPath);
const networkDescription = wasm.load_nnue(networkBytes);
const corpusLines = (await readFile(corpusPath, "utf8")).trim().split(/\r?\n/).slice(0, count);
if (corpusLines.length !== count) throw new Error(`corpus has fewer than ${count} rows`);
const positions = corpusLines.map((line, index) => {
  const row = JSON.parse(line);
  const expectedId = `r1a-${String(index).padStart(5, "0")}`;
  if (row.id !== expectedId) throw new Error(`unstable corpus order at ${index}`);
  return { id: row.id, sfen: row.sfen, score: wasm.evaluate_nnue(row.sfen) };
});
const report = {
  schema: "haitaka-anhoku-strength-first-wasm-parity-v1",
  networkDescription,
  positions,
  status: "passed",
};
await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(`wrote ${count} WASM parity scores to ${outputPath}`);
