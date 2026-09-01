#!/usr/bin/env node
import { createServer } from "node:http";
import { readFileSync, writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawn, spawnSync } from "node:child_process";

function argsOf(argv) {
  const result = {};
  for (let i = 0; i < argv.length; i += 2) result[argv[i].replace(/^--/, "")] = argv[i + 1];
  return result;
}

function requireArg(args, name) {
  if (!args[name]) throw new Error(`missing --${name}`);
  return resolve(args[name]);
}

function openings(path) {
  return readFileSync(path, "utf8").split(/\r?\n/).filter((line) => line && !line.startsWith("#")).map((line) => {
    const [id, sfen] = line.split("\t");
    if (!id || !sfen) throw new Error(`malformed opening line: ${line}`);
    return { id, sfen };
  });
}

const args = argsOf(process.argv.slice(2));
const pkgDir = requireArg(args, "pkg-dir");
const modelPath = requireArg(args, "model");
const contractPath = requireArg(args, "contract");
const openingsPath = requireArg(args, "openings");
const workerPath = requireArg(args, "worker");
const outputPath = requireArg(args, "output");
const chromePath = args.chrome ? resolve(args.chrome) : "/usr/bin/google-chrome";
const timeoutMs = Number(args["timeout-ms"] ?? 300000);
const contract = JSON.parse(readFileSync(contractPath, "utf8"));
const config = { ...contract, openings: openings(openingsPath) };
if (config.openings.length !== contract.schedule.openingGroups) throw new Error("opening count differs from contract");

const page = `<!doctype html><meta charset="utf-8"><title>R1-D3</title><script type="module">
const config = await (await fetch('/config.json')).json();
const worker = new Worker('/worker.js', { type: 'module' });
worker.onmessage = async ({data}) => {
  if (data.type === 'progress') await fetch('/progress', {method:'POST', body:JSON.stringify(data)});
  if (data.type === 'result') await fetch('/result', {method:'POST', body:JSON.stringify({...data.result, userAgent:navigator.userAgent})});
  if (data.type === 'error') await fetch('/error', {method:'POST', body:data.error});
};
worker.onerror = async (event) => fetch('/error', {method:'POST', body:event.message});
worker.postMessage(config);
</script><p>R1-D3 browser worker running</p>`;

let resolveResult;
let rejectResult;
const resultPromise = new Promise((resolvePromise, rejectPromise) => {
  resolveResult = resolvePromise;
  rejectResult = rejectPromise;
});
const mime = (path) => path.endsWith(".wasm") ? "application/wasm" : path.endsWith(".js") ? "text/javascript" : "application/octet-stream";
const server = createServer((request, response) => {
  const chunks = [];
  request.on("data", (chunk) => chunks.push(chunk));
  request.on("end", () => {
    const body = Buffer.concat(chunks);
    if (request.method === "POST" && request.url === "/progress") {
      const progress = JSON.parse(body.toString("utf8"));
      process.stdout.write(`R1-D3 browser ${progress.lane}: ${progress.completedPairs}/${contract.schedule.pairsPerLane} pairs\n`);
      response.end("ok");
      return;
    }
    if (request.method === "POST" && request.url === "/result") {
      resolveResult(JSON.parse(body.toString("utf8")));
      response.end("ok");
      return;
    }
    if (request.method === "POST" && request.url === "/error") {
      rejectResult(new Error(body.toString("utf8")));
      response.end("error recorded");
      return;
    }
    const route = request.url === "/" ? [Buffer.from(page), "text/html"]
      : request.url === "/config.json" ? [Buffer.from(JSON.stringify(config)), "application/json"]
      : request.url === "/worker.js" ? [readFileSync(workerPath), "text/javascript"]
      : request.url === "/model.nnue" ? [readFileSync(modelPath), "application/octet-stream"]
      : request.url === "/pkg/haitaka_wasm.js" ? [readFileSync(join(pkgDir, "haitaka_wasm.js")), "text/javascript"]
      : request.url === "/pkg/haitaka_wasm_bg.wasm" ? [readFileSync(join(pkgDir, "haitaka_wasm_bg.wasm")), mime(".wasm")]
      : null;
    if (!route) { response.statusCode = 404; response.end("not found"); return; }
    response.setHeader("Content-Type", route[1]);
    response.setHeader("Cache-Control", "no-store");
    response.end(route[0]);
  });
});

await new Promise((resolvePromise) => server.listen(0, "127.0.0.1", resolvePromise));
const address = server.address();
const profile = mkdtempSync(join(tmpdir(), "haitaka-r1d3-chrome-"));
const version = spawnSync(chromePath, ["--version"], { encoding: "utf8" }).stdout.trim();
const chrome = spawn(chromePath, [
  "--headless=new", "--disable-gpu", "--no-sandbox", "--no-first-run", "--no-default-browser-check",
  `--user-data-dir=${profile}`, `http://127.0.0.1:${address.port}/`,
], { stdio: ["ignore", "pipe", "pipe"] });
let chromeStderr = "";
chrome.stderr.on("data", (chunk) => { chromeStderr += chunk.toString("utf8"); });
const timeout = setTimeout(() => rejectResult(new Error(`browser harness timed out after ${timeoutMs} ms`)), timeoutMs);
try {
  const result = await resultPromise;
  clearTimeout(timeout);
  result.chromeVersion = version;
  result.chromeStderrTail = chromeStderr.slice(-4000);
  writeFileSync(outputPath, `${JSON.stringify(result, null, 2)}\n`);
  process.stdout.write(`R1-D3 browser trace written to ${outputPath}\n`);
} finally {
  if (chrome.exitCode === null) {
    chrome.kill("SIGTERM");
    await Promise.race([
      new Promise((resolvePromise) => chrome.once("exit", resolvePromise)),
      new Promise((resolvePromise) => setTimeout(resolvePromise, 2000)),
    ]);
  }
  server.close();
  rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
