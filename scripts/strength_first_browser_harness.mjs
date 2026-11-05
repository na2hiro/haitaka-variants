#!/usr/bin/env node
import { createServer } from "node:http";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_WORKER = join(ROOT, "scripts/strength_first_browser_worker.js");
const DEFAULT_CHROME = "/usr/bin/google-chrome";
const MAX_PLIES = 200;
const CHROME_CLOSE_TIMEOUT_MS = 5000;
const PROFILE_CLEANUP_ATTEMPTS = 30;
const PROFILE_CLEANUP_DELAY_MS = 100;

function parseArgs(argv) {
  const result = {};
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (!token.startsWith("--")) throw new Error(`unexpected argument ${token}`);
    const name = token.slice(2);
    const next = argv[index + 1];
    if (next === undefined || next.startsWith("--")) result[name] = true;
    else {
      result[name] = next;
      index += 1;
    }
  }
  return result;
}

function firstArg(args, names) {
  for (const name of names) if (args[name] !== undefined) return args[name];
  return undefined;
}

function requireValue(args, names) {
  const value = firstArg(args, names);
  if (value === undefined || value === true || value === "") throw new Error(`missing --${names[0]}`);
  return value;
}

function numberArg(args, names, fallback) {
  const value = firstArg(args, names);
  if (value === undefined) return fallback;
  const number = Number(value);
  if (!Number.isInteger(number) || number <= 0) throw new Error(`--${names[0]} must be a positive integer`);
  return number;
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function stable(value) {
  if (Array.isArray(value)) return value.map(stable);
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.keys(value).sort().map((key) => [key, stable(value[key])]));
  }
  return value;
}

function stableJson(value) {
  return JSON.stringify(stable(value));
}

function gitValue(args) {
  const result = spawnSync("git", args, { cwd: ROOT, encoding: "utf8" });
  return result.status === 0 ? result.stdout.trim() : null;
}

function readJson(path, label) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    throw new Error(`cannot read ${label} ${path}: ${error.message}`);
  }
}

function parseOpenings(path) {
  const entries = [];
  const lines = readFileSync(path, "utf8").split(/\r?\n/);
  for (const raw of lines) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const fields = line.split("\t");
    const id = fields.length > 1 && fields[0].trim()
      ? fields[0].trim()
      : `anhoku-v3-${String(entries.length + 1).padStart(3, "0")}`;
    const sfen = (fields.length > 1 ? fields.slice(1).join("\t") : fields[0]).trim();
    const tokens = sfen.split(/\s+/);
    if (tokens.length < 4 || (tokens[1] !== "b" && tokens[1] !== "w")) {
      throw new Error(`malformed Anhoku SFEN in ${path}: ${line}`);
    }
    entries.push({ index: entries.length, id, sfen });
  }
  if (!entries.length) throw new Error(`opening suite is empty: ${path}`);
  return entries;
}

function artifact(path, bytes, route = null) {
  const item = { path, bytes: bytes.byteLength, sha256: sha256(bytes) };
  if (route) item.route = route;
  return item;
}

function makeInput(name, path, route = null) {
  const bytes = readFileSync(path);
  return { name, path, bytes, identity: artifact(path, bytes, route) };
}

function parseSetup(args) {
  const pkgDir = resolve(requireValue(args, ["pkg-dir"]));
  const modelPath = resolve(requireValue(args, ["model", "model-a", "a-model", "a-nnue"]));
  const openingsPath = resolve(requireValue(args, ["openings", "opening-file"]));
  const outputPath = resolve(requireValue(args, ["output"]));
  const workerPath = resolve(firstArg(args, ["worker"]) ?? DEFAULT_WORKER);
  const chromePath = resolve(firstArg(args, ["chrome"]) ?? DEFAULT_CHROME);
  const contractValue = firstArg(args, ["contract"]);
  const sourceIdentityValue = firstArg(args, ["source-identity"]);
  const releaseExecutableValue = firstArg(args, ["release-executable", "native-executable"]);
  const contractPath = contractValue === undefined || contractValue === true ? null : resolve(contractValue);
  const sourceIdentityPath = sourceIdentityValue === undefined || sourceIdentityValue === true ? null : resolve(sourceIdentityValue);
  const releaseExecutablePath = releaseExecutableValue === undefined || releaseExecutableValue === true ? null : resolve(releaseExecutableValue);

  const nodesValue = firstArg(args, ["nodes-per-move", "nodes"]);
  const movetimeValue = firstArg(args, ["movetime-ms", "movetime"]);
  if ((nodesValue === undefined) === (movetimeValue === undefined)) {
    throw new Error("provide exactly one of --nodes-per-move or --movetime-ms");
  }
  const budgetValue = Number(nodesValue ?? movetimeValue);
  if (!Number.isInteger(budgetValue) || budgetValue <= 0) throw new Error("search budget must be a positive integer");
  const budget = nodesValue === undefined
    ? { mode: "movetime", movetimeMs: budgetValue, nodes: null, command: `movetime ${budgetValue}` }
    : { mode: "nodes", nodes: budgetValue, movetimeMs: null, command: `nodes ${budgetValue}` };

  const maxPlies = numberArg(args, ["max-plies"], MAX_PLIES);
  if (maxPlies > MAX_PLIES) throw new Error(`--max-plies cannot exceed ${MAX_PLIES}`);
  const openings = parseOpenings(openingsPath);
  const firstN = numberArg(args, ["first-n", "opening-count"], openings.length);
  if (firstN > openings.length) throw new Error(`requested ${firstN} openings but suite has ${openings.length}`);
  const selectedOpenings = openings.slice(0, firstN);
  const timeoutMs = numberArg(args, ["timeout-ms"], 300000);

  const wasmPath = join(pkgDir, "haitaka_wasm_bg.wasm");
  const gluePath = join(pkgDir, "haitaka_wasm.js");
  const inputs = [
    makeInput("browserHarness", fileURLToPath(import.meta.url)),
    makeInput("browserWorker", workerPath, "/worker.js"),
    makeInput("modelA", modelPath, "/model.nnue"),
    makeInput("openings", openingsPath, "/openings.tsv"),
    makeInput("wasm", wasmPath, "/pkg/haitaka_wasm_bg.wasm"),
    makeInput("wasmGlue", gluePath, "/pkg/haitaka_wasm.js"),
  ];
  if (contractPath) inputs.push(makeInput("contract", contractPath, "/contract.json"));
  if (sourceIdentityPath) inputs.push(makeInput("sourceIdentity", sourceIdentityPath, "/source-identity.json"));
  if (releaseExecutablePath) inputs.push(makeInput("releaseExecutable", releaseExecutablePath));

  const inputBytes = Object.fromEntries(inputs.map((item) => [item.name, item.bytes]));
  const inputIdentities = Object.fromEntries(inputs.map((item) => [item.name, item.identity]));
  const routes = Object.fromEntries(inputs.filter((item) => item.identity.route).map((item) => [item.name, item.identity.route]));
  const contract = contractPath ? readJson(contractPath, "contract") : null;
  const sourceIdentity = sourceIdentityPath ? readJson(sourceIdentityPath, "source identity") : null;
  const browserVersionResult = spawnSync(chromePath, ["--version"], { encoding: "utf8" });
  const browserVersion = (browserVersionResult.stdout || browserVersionResult.stderr || "").trim() || null;
  const source = {
    schema: "haitaka-anhoku-strength-first-source-provenance-v1",
    workspaceCommit: gitValue(["rev-parse", "HEAD"]),
    workspaceTree: gitValue(["rev-parse", "HEAD^{tree}"]),
    dirty: (gitValue(["status", "--porcelain", "--untracked-files=all"]) || "") !== "",
    dirtyStatus: gitValue(["status", "--porcelain", "--untracked-files=all"]),
    suppliedSourceIdentity: sourceIdentity,
    suppliedContract: contract,
  };
  const provenanceCore = {
    schema: "haitaka-anhoku-strength-first-browser-provenance-v1",
    schemaVersion: 1,
    finalizedBeforeBrowserLaunch: true,
    files: inputIdentities,
    source,
    execution: {
      command: process.argv,
      nodeVersion: process.version,
      platform: process.platform,
      arch: process.arch,
      browserExecutable: chromePath,
      browserVersion,
      workerCount: 1,
      concurrentGames: 1,
      wasmPackageDirectory: pkgDir,
    },
  };
  const provenance = {
    ...provenanceCore,
    envelopeId: sha256(Buffer.from(stableJson(provenanceCore))),
    routes,
  };
  return {
    pkgDir,
    modelPath,
    openingsPath,
    outputPath,
    workerPath,
    chromePath,
    timeoutMs,
    maxPlies,
    budget,
    openings: selectedOpenings,
    allOpeningCount: openings.length,
    inputBytes,
    contract,
    sourceIdentity,
    provenance,
    outputDirectory: dirname(outputPath),
  };
}

function contentType(path) {
  if (path.endsWith(".wasm")) return "application/wasm";
  if (path.endsWith(".js")) return "text/javascript";
  if (path.endsWith(".json")) return "application/json";
  if (path.endsWith(".tsv") || path.endsWith(".sfen")) return "text/plain";
  return "application/octet-stream";
}

function writeReport(path, report) {
  mkdirSync(dirname(path), { recursive: true });
  const temporaryPath = `${path}.tmp-${process.pid}`;
  writeFileSync(temporaryPath, `${JSON.stringify(report, null, 2)}\n`);
  renameSync(temporaryPath, path);
}

function waitForChromeClose(chrome, timeoutMs) {
  if (!chrome || chrome.exitCode !== null || chrome.signalCode !== null) return Promise.resolve(true);
  return new Promise((resolvePromise) => {
    let timer = null;
    const finish = (closed) => {
      if (timer) clearTimeout(timer);
      chrome.removeListener("close", onClose);
      chrome.removeListener("exit", onExit);
      resolvePromise(closed);
    };
    const onClose = () => finish(true);
    const onExit = () => finish(true);
    chrome.once("close", onClose);
    // A child with inherited/closed stdio can emit exit without close.  Treat
    // either event as completion, but always install the listeners before
    // sending a signal so a fast Chrome shutdown cannot be missed.
    chrome.once("exit", onExit);
    timer = setTimeout(() => finish(false), timeoutMs);
  });
}

function signalChrome(chrome, signal) {
  if (!chrome || chrome.exitCode !== null || chrome.signalCode !== null) return;
  // Chrome launches helper processes for the headless browser.  Spawn it in
  // its own process group so shutdown reaches those helpers as well; this is
  // what makes the profile cleanup below reliable on a busy machine.
  if (process.platform !== "win32" && chrome.pid) {
    try {
      process.kill(-chrome.pid, signal);
      return;
    } catch (error) {
      if (error?.code === "ESRCH") return;
    }
  }
  try { chrome.kill(signal); } catch (error) {
    if (error?.code !== "ESRCH") throw error;
  }
}

async function closeChrome(chrome) {
  if (!chrome || (chrome.exitCode !== null && chrome.signalCode !== null)) return;
  signalChrome(chrome, "SIGTERM");
  if (await waitForChromeClose(chrome, CHROME_CLOSE_TIMEOUT_MS)) return;
  // A browser that ignored SIGTERM must not keep writing the profile while
  // the harness tears it down.  Escalate and wait again before cleanup.
  signalChrome(chrome, "SIGKILL");
  if (!await waitForChromeClose(chrome, CHROME_CLOSE_TIMEOUT_MS)) {
    throw new Error("Chrome did not close after SIGTERM/SIGKILL");
  }
}

async function removeProfile(profile) {
  if (!profile) return;
  let lastError = null;
  for (let attempt = 0; attempt < PROFILE_CLEANUP_ATTEMPTS; attempt += 1) {
    try {
      rmSync(profile, { recursive: true, force: true, maxRetries: 0 });
      return;
    } catch (error) {
      lastError = error;
      if (attempt + 1 < PROFILE_CLEANUP_ATTEMPTS) {
        await new Promise((resolvePromise) => setTimeout(resolvePromise, PROFILE_CLEANUP_DELAY_MS));
      }
    }
  }
  throw lastError;
}

function closeServer(server) {
  if (!server.listening) return Promise.resolve();
  return new Promise((resolvePromise, rejectPromise) => {
    server.close((error) => error ? rejectPromise(error) : resolvePromise());
  });
}

function processStatus(pid) {
  try {
    const status = readFileSync(`/proc/${pid}/status`, "utf8");
    const rss = status.match(/^VmRSS:\s+(\d+)\s+kB$/m);
    const highWater = status.match(/^VmHWM:\s+(\d+)\s+kB$/m);
    if (!rss) return null;
    return {
      pid,
      residentSetBytes: Number(rss[1]) * 1024,
      highWaterMarkBytes: highWater ? Number(highWater[1]) * 1024 : null,
    };
  } catch {
    return null;
  }
}

function processGroupMemory(processGroupId) {
  if (process.platform === "win32") return null;
  let entries;
  try {
    entries = readdirSync("/proc", { withFileTypes: true });
  } catch {
    return null;
  }
  const processes = [];
  for (const entry of entries) {
    if (!entry.isDirectory() || !/^\d+$/.test(entry.name)) continue;
    const pid = Number(entry.name);
    try {
      const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
      const closeParen = stat.lastIndexOf(")");
      if (closeParen < 0) continue;
      const fields = stat.slice(closeParen + 1).trim().split(/\s+/);
      // Fields after the comm field begin with state, ppid, pgrp.
      if (Number(fields[2]) !== processGroupId) continue;
    } catch {
      continue;
    }
    const status = processStatus(pid);
    if (status) processes.push(status);
  }
  if (!processes.length) return null;
  return {
    processCount: processes.length,
    residentSetBytes: processes.reduce((sum, item) => sum + item.residentSetBytes, 0),
    highWaterMarkBytes: processes.reduce((sum, item) => sum + (item.highWaterMarkBytes ?? item.residentSetBytes), 0),
  };
}

function startChromeMemorySampler(chrome) {
  const processGroupId = chrome?.pid ?? null;
  const samples = [];
  let stopped = false;
  const sample = () => {
    if (stopped || processGroupId === null) return;
    const memory = processGroupMemory(processGroupId);
    if (memory) samples.push({ sampledAtUnixMs: Date.now(), ...memory });
  };
  sample();
  const timer = setInterval(sample, 250);
  timer.unref?.();
  return {
    stop() {
      if (stopped) return summarizeChromeMemory(processGroupId, samples);
      clearInterval(timer);
      sample();
      stopped = true;
      return summarizeChromeMemory(processGroupId, samples);
    },
  };
}

function summarizeChromeMemory(processGroupId, samples) {
  if (!samples.length) {
    return {
      status: "unavailable",
      method: process.platform === "win32" ? "procfs-not-supported" : "procfs-no-live-process-samples",
      processGroupId,
      sampleIntervalMs: 250,
      sampleCount: 0,
    };
  }
  const peak = (field) => Math.max(...samples.map((sample) => sample[field]));
  return {
    status: "observed",
    method: "linux-procfs-process-group-sum",
    processGroupId,
    sampleIntervalMs: 250,
    sampleCount: samples.length,
    processCountPeak: peak("processCount"),
    peakResidentSetBytes: peak("residentSetBytes"),
    peakHighWaterMarkBytes: peak("highWaterMarkBytes"),
    firstSample: samples[0],
    lastSample: samples[samples.length - 1],
  };
}

async function run(setup) {
  const inputBytes = setup.inputBytes;
  const config = {
    schema: "haitaka-anhoku-strength-first-browser-config-v1",
    budget: setup.budget,
    maxPlies: setup.maxPlies,
    openings: setup.openings,
    wasmBuild: setup.contract?.production?.wasmBuild ?? null,
    provenance: setup.provenance,
  };
  const page = `<!doctype html><meta charset="utf-8"><title>Anhoku strength-first</title><script type="module">
const config = await (await fetch('/config.json', {cache:'no-store'})).json();
const worker = new Worker('/worker.js', {type:'module'});
worker.onmessage = async ({data}) => {
  if (data.type === 'progress') await fetch('/progress', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(data)});
  if (data.type === 'result') await fetch('/result', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({...data.result, userAgent:navigator.userAgent})});
  if (data.type === 'error') await fetch('/error', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(data)});
};
worker.onerror = async (event) => fetch('/error', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({type:'error', error:event.message})});
worker.postMessage(config);
</script><p>Anhoku strength-first browser worker running</p>`;

  let resolveResult;
  let rejectResult;
  const resultPromise = new Promise((resolvePromise, rejectPromise) => {
    resolveResult = resolvePromise;
    rejectResult = rejectPromise;
  });
  let provenanceAck = null;
  let playEventObserved = false;
  let settled = false;
  const fail = (error) => {
    if (!settled) {
      settled = true;
      rejectResult(error);
    }
  };
  const finish = (result) => {
    if (!settled) {
      settled = true;
      resolveResult(result);
    }
  };
  const server = createServer((request, response) => {
    const chunks = [];
    request.on("data", (chunk) => chunks.push(chunk));
    request.on("error", (error) => fail(error));
    request.on("end", () => {
      const body = Buffer.concat(chunks);
      if (request.method === "POST" && request.url === "/provenance") {
        if (provenanceAck || playEventObserved) {
          response.statusCode = 409;
          response.end("duplicate or late provenance acknowledgement");
          fail(new Error("duplicate or late provenance acknowledgement"));
          return;
        }
        try {
          provenanceAck = JSON.parse(body.toString("utf8"));
        } catch (error) {
          response.statusCode = 400;
          response.end("invalid provenance acknowledgement");
          fail(error);
          return;
        }
        if (provenanceAck.envelopeId !== setup.provenance.envelopeId || provenanceAck.verifiedBeforePlay !== true) {
          response.statusCode = 409;
          response.end("provenance envelope mismatch");
          fail(new Error("provenance envelope mismatch"));
          return;
        }
        response.end("accepted-before-play");
        return;
      }
      if (request.method === "POST" && request.url === "/progress") {
        playEventObserved = true;
        if (!provenanceAck) {
          response.statusCode = 409;
          response.end("play preceded provenance acknowledgement");
          fail(new Error("play progress preceded provenance acknowledgement"));
          return;
        }
        try {
          const progress = JSON.parse(body.toString("utf8"));
          process.stdout.write(`strength-first browser: ${progress.completedPairs}/${progress.totalPairs} pairs\n`);
        } catch (error) {
          fail(error);
        }
        response.end("ok");
        return;
      }
      if (request.method === "POST" && request.url === "/result") {
        playEventObserved = true;
        if (!provenanceAck) {
          response.statusCode = 409;
          response.end("result preceded provenance acknowledgement");
          fail(new Error("result preceded provenance acknowledgement"));
          return;
        }
        try {
          finish(JSON.parse(body.toString("utf8")));
          response.end("ok");
        } catch (error) {
          fail(error);
          response.statusCode = 400;
          response.end("invalid result");
        }
        return;
      }
      if (request.method === "POST" && request.url === "/error") {
        let value;
        try { value = JSON.parse(body.toString("utf8")); } catch { value = { error: body.toString("utf8") }; }
        const error = new Error(value.error || "browser worker failed");
        error.partial = value.partial ?? null;
        fail(error);
        response.end("error recorded");
        return;
      }

      const route = request.url === "/" ? [Buffer.from(page), "text/html"]
        : request.url === "/config.json" ? [Buffer.from(JSON.stringify(config)), "application/json"]
        : request.url === "/worker.js" ? [inputBytes.browserWorker, "text/javascript"]
        : request.url === "/model.nnue" ? [inputBytes.modelA, "application/octet-stream"]
        : request.url === "/openings.tsv" ? [inputBytes.openings, contentType(setup.openingsPath)]
        : request.url === "/pkg/haitaka_wasm.js" ? [inputBytes.wasmGlue, "text/javascript"]
        : request.url === "/pkg/haitaka_wasm_bg.wasm" ? [inputBytes.wasm, "application/wasm"]
        : request.url === "/contract.json" && inputBytes.contract ? [inputBytes.contract, "application/json"]
        : request.url === "/source-identity.json" && inputBytes.sourceIdentity ? [inputBytes.sourceIdentity, "application/json"]
        : null;
      if (!route) {
        response.statusCode = 404;
        response.end("not found");
        return;
      }
      response.setHeader("Content-Type", route[1]);
      response.setHeader("Cache-Control", "no-store");
      response.end(route[0]);
    });
  });

  let profile = null;
  let chrome = null;
  let chromeMemorySampler = null;
  let timeout = null;
  let reportCommitted = false;
  let runError = null;
  try {
    await new Promise((resolvePromise, rejectPromise) => {
      server.once("error", rejectPromise);
      server.listen(0, "127.0.0.1", resolvePromise);
    });
    const address = server.address();
    profile = mkdtempSync(join(tmpdir(), "haitaka-strength-first-chrome-"));
    chrome = spawn(setup.chromePath, [
      "--headless=new", "--disable-gpu", "--no-sandbox", "--no-first-run", "--no-default-browser-check",
      `--user-data-dir=${profile}`, `http://127.0.0.1:${address.port}/`,
    ], { detached: true, stdio: ["ignore", "pipe", "pipe"] });
    chromeMemorySampler = startChromeMemorySampler(chrome);
    let chromeStderr = "";
    chrome.stderr.on("data", (chunk) => { chromeStderr += chunk.toString("utf8"); });
    chrome.once("error", (error) => fail(error));
    timeout = setTimeout(() => fail(new Error(`browser harness timed out after ${setup.timeoutMs} ms`)), setup.timeoutMs);
    timeout.unref?.();
    const result = await resultPromise;
    clearTimeout(timeout);
    result.chromeVersion = setup.provenance.execution.browserVersion;
    result.chromeStderrTail = chromeStderr.slice(-4000);
    result.hostTelemetry = {
      schema: "haitaka-anhoku-strength-first-browser-host-telemetry-v1",
      memory: { status: "pending", method: "optional-post-result-sampling" },
    };
    result.producerEvents = {
      provenanceAcceptedBeforePlay: true,
      acknowledgement: provenanceAck,
      progressObserved: playEventObserved,
    };
    // Persist the qualified game result before collecting optional host
    // telemetry. A telemetry or cleanup fault must not discard completed
    // games or replace the report with a failure record.
    writeReport(setup.outputPath, result);
    reportCommitted = true;
    try {
      result.hostTelemetry.memory = chromeMemorySampler?.stop() ?? null;
      chromeMemorySampler = null;
      writeReport(setup.outputPath, result);
    } catch (error) {
      chromeMemorySampler = null;
      process.stderr.write(`strength-first browser telemetry warning: ${error?.message ?? String(error)}\n`);
    }
    process.stdout.write(`strength-first browser report written to ${setup.outputPath}\n`);
  } catch (error) {
    runError = error;
    throw error;
  } finally {
    if (timeout) clearTimeout(timeout);
    const cleanupErrors = [];
    if (chromeMemorySampler) {
      try { chromeMemorySampler.stop(); } catch (error) { cleanupErrors.push(error); }
    }
    try { await closeChrome(chrome); } catch (error) { cleanupErrors.push(error); }
    try { await closeServer(server); } catch (error) { cleanupErrors.push(error); }
    try { await removeProfile(profile); } catch (error) { cleanupErrors.push(error); }
    if (cleanupErrors.length) {
      const detail = cleanupErrors.map((error) => error?.message ?? String(error)).join("; ");
      if (reportCommitted) {
        process.stderr.write(`strength-first browser cleanup warning: ${detail}\n`);
      } else if (!runError) {
        throw cleanupErrors[0];
      }
    }
  }
}

let setup = null;
try {
  setup = parseSetup(parseArgs(process.argv.slice(2)));
  await run(setup);
} catch (error) {
  if (setup) {
    const failure = {
      schema: "haitaka-anhoku-strength-first-browser-report-v1",
      status: "failed",
      provenance: setup.provenance,
      execution: {
        workerCount: 1,
        concurrentGames: 1,
        maxPlies: setup.maxPlies,
        openingCount: setup.openings.length,
        budget: setup.budget,
      },
      failure: { message: error?.message ?? String(error), stack: error?.stack ?? null },
      partial: error?.partial ?? null,
    };
    try {
      writeReport(setup.outputPath, failure);
    } catch (writeError) {
      process.stderr.write(`could not write failure report: ${writeError.message}\n`);
    }
  }
  process.stderr.write(`${error?.stack ?? String(error)}\n`);
  process.exitCode = 1;
}
