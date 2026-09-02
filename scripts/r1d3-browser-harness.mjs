#!/usr/bin/env node
import { createServer } from "node:http";
import { readFileSync, writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

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
const sourceIdentityPath = requireArg(args, "source-identity");
const releaseExecutablePath = requireArg(args, "release-executable");
const chromePath = args.chrome ? resolve(args.chrome) : "/usr/bin/google-chrome";
const timeoutMs = Number(args["timeout-ms"] ?? 300000);
const harnessPath = fileURLToPath(import.meta.url);
const wasmPath = join(pkgDir, "haitaka_wasm_bg.wasm");
const gluePath = join(pkgDir, "haitaka_wasm.js");
const inputPaths = {
  browserHarness: harnessPath,
  browserWorker: workerPath,
  contract: contractPath,
  debugModel: modelPath,
  openings: openingsPath,
  releaseExecutable: releaseExecutablePath,
  sourceIdentity: sourceIdentityPath,
  wasm: wasmPath,
  wasmGlue: gluePath,
};
const inputBytes = Object.fromEntries(Object.entries(inputPaths).map(([name, path]) => [name, readFileSync(path)]));
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const identity = (name) => ({ path: inputPaths[name], bytes: inputBytes[name].byteLength, sha256: sha256(inputBytes[name]) });
const stable = (value) => Array.isArray(value) ? value.map(stable) : value && typeof value === "object"
  ? Object.fromEntries(Object.keys(value).sort().map((key) => [key, stable(value[key])])) : value;
const stableJson = (value) => JSON.stringify(stable(value));
const contract = JSON.parse(inputBytes.contract.toString("utf8"));
const sourceIdentity = JSON.parse(inputBytes.sourceIdentity.toString("utf8"));
const browserVersion = spawnSync(chromePath, ["--version"], { encoding: "utf8" }).stdout.trim();
const envelopeCore = {
  schema: contract.provenance.envelopeSchema,
  schemaVersion: 1,
  finalizedBeforeBrowserLaunch: true,
  files: Object.fromEntries(Object.keys(inputPaths).sort().map((name) => [name, identity(name)])),
  source: {
    schema: sourceIdentity.schema,
    workspaceCommit: sourceIdentity.workspace.commit,
    workspaceTree: sourceIdentity.workspace.tree,
    externalTrainerCommit: sourceIdentity.externalTrainer.commit,
    rebuildComplete: sourceIdentity.rebuildComplete,
  },
  execution: {
    browserExecutable: chromePath,
    browserVersion,
    hostClass: contract.production.hostClass,
    deviceClass: contract.production.deviceClass,
    workerCount: contract.production.workerCount,
    concurrentGames: contract.production.concurrentGames,
    clockControllerVersion: contract.production.clockControllerVersion,
    deadlinePollingNodes: contract.production.deadlinePollingNodes,
    coldWarmVersion: contract.production.coldWarmVersion,
    modelLoadVersion: contract.provenance.modelLoadVersion,
    historyRepetitionVersion: contract.provenance.historyRepetitionVersion,
    rootResultSchema: contract.provenance.rootResultSchema,
    nodeAccountingVersion: contract.provenance.nodeAccountingVersion,
    dfpnPolicy: contract.provenance.dfpnPolicy,
    adjudicationVersion: contract.provenance.adjudicationVersion,
    searchLimits: contract.schedule.lanes,
    maximumPlies: contract.schedule.maximumPlies,
    memoryConfiguration: contract.production.memoryConfiguration,
    wasmBuild: contract.production.wasmBuild,
  },
};
const provenanceEnvelope = Object.freeze({ ...envelopeCore, envelopeId: sha256(Buffer.from(stableJson(envelopeCore))) });
const config = { ...contract, openings: openings(openingsPath), provenanceEnvelope };
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
let provenanceAck = null;
let playEventObserved = false;
const mime = (path) => path.endsWith(".wasm") ? "application/wasm" : path.endsWith(".js") ? "text/javascript" : "application/octet-stream";
const server = createServer((request, response) => {
  const chunks = [];
  request.on("data", (chunk) => chunks.push(chunk));
  request.on("end", () => {
    const body = Buffer.concat(chunks);
    if (request.method === "POST" && request.url === "/provenance") {
      if (provenanceAck || playEventObserved) { rejectResult(new Error("duplicate or late provenance acknowledgement")); response.statusCode = 409; response.end("rejected"); return; }
      provenanceAck = JSON.parse(body.toString("utf8"));
      if (provenanceAck.envelopeId !== provenanceEnvelope.envelopeId) { rejectResult(new Error("provenance envelope id mismatch")); response.statusCode = 409; response.end("rejected"); return; }
      response.end("accepted-before-play");
      return;
    }
    if (request.method === "POST" && request.url === "/progress") {
      playEventObserved = true;
      if (!provenanceAck) { rejectResult(new Error("play progress preceded provenance acknowledgement")); response.statusCode = 409; response.end("rejected"); return; }
      const progress = JSON.parse(body.toString("utf8"));
      process.stdout.write(`R1-D3 browser ${progress.lane}: ${progress.completedPairs}/${contract.schedule.pairsPerLane} pairs\n`);
      response.end("ok");
      return;
    }
    if (request.method === "POST" && request.url === "/result") {
      playEventObserved = true;
      if (!provenanceAck) { rejectResult(new Error("result preceded provenance acknowledgement")); response.statusCode = 409; response.end("rejected"); return; }
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
      : request.url === "/worker.js" ? [inputBytes.browserWorker, "text/javascript"]
      : request.url === "/model.nnue" ? [inputBytes.debugModel, "application/octet-stream"]
      : request.url === "/contract.json" ? [inputBytes.contract, "application/json"]
      : request.url === "/openings.tsv" ? [inputBytes.openings, "text/plain"]
      : request.url === "/source-identity.json" ? [inputBytes.sourceIdentity, "application/json"]
      : request.url === "/pkg/haitaka_wasm.js" ? [inputBytes.wasmGlue, "text/javascript"]
      : request.url === "/pkg/haitaka_wasm_bg.wasm" ? [inputBytes.wasm, mime(".wasm")]
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
  result.chromeVersion = browserVersion;
  result.chromeStderrTail = chromeStderr.slice(-4000);
  result.producerEvents = { provenanceAcceptedBeforePlay: true, acknowledgement: provenanceAck };
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
