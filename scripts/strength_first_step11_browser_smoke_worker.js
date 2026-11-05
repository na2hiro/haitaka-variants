const ROOT_RESULT_SCHEMA = "haitaka-search-root-result-v1";

function send(engine, command) {
  return Array.from(engine.send(command), String);
}

function valueAfter(tokens, name) {
  const index = tokens.indexOf(name);
  return index >= 0 && index + 1 < tokens.length ? tokens[index + 1] : null;
}

function boolAfter(tokens, ...names) {
  return names.some((name) => valueAfter(tokens, name) === "1");
}

function numberAfter(tokens, ...names) {
  for (const name of names) {
    const value = valueAfter(tokens, name);
    if (value !== null && value !== "null") {
      const number = Number(value);
      if (Number.isFinite(number)) return number;
    }
  }
  return null;
}

async function verifyProvenance(config) {
  const fetched = {};
  const verified = {};
  const fetchTimingsMs = {};
  for (const [name, route] of Object.entries(config.provenance.routes)) {
    const started = performance.now();
    const response = await fetch(route, { cache: "no-store" });
    if (!response.ok) throw new Error(`provenance fetch failed for ${name}: ${response.status}`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    const digestBytes = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
    const digest = Array.from(digestBytes, (value) => value.toString(16).padStart(2, "0")).join("");
    const expected = config.provenance.files[name];
    if (!expected || expected.bytes !== bytes.byteLength || expected.sha256 !== digest) throw new Error(`provenance mismatch for ${name}`);
    fetched[name] = bytes;
    verified[name] = { bytes: bytes.byteLength, sha256: digest, route };
    fetchTimingsMs[name] = performance.now() - started;
  }
  return { fetched, verified, fetchTimingsMs };
}

function parseSearch(outputs, engine, position, begunAt, endedAt) {
  const info = outputs.filter((line) => line.startsWith("info ")).at(-1) ?? "";
  const tokens = info.split(/\s+/);
  const best = outputs.find((line) => line.startsWith("bestmove "));
  if (!best) throw new Error(`smoke search missing bestmove for ${position.id}`);
  const bestMove = best.slice("bestmove ".length).trim();
  const gameoverLine = outputs.find((line) => line.startsWith("info string gameover ")) ?? null;
  return {
    schema: "haitaka-anhoku-strength-first-browser-smoke-search-v1",
    engine,
    positionId: position.id,
    sfen: position.sfen,
    command: "go movetime 1000",
    outputs,
    bestMove,
    gameover: gameoverLine,
    gameoverReason: gameoverLine ? gameoverLine.slice("info string gameover ".length).trim().split(/\s+/)[0] : null,
    requestedMs: 1000,
    begunAt,
    endedAt,
    elapsedMs: endedAt - begunAt,
    rootResultSchema: valueAfter(tokens, "rootResultSchema"),
    playMoveWasSearched: boolAfter(tokens, "playMoveWasSearched"),
    emergencyFallbackUsed: boolAfter(tokens, "emergencyFallbackUsed", "fallback"),
    missingMove: boolAfter(tokens, "missingMove"),
    unsearchedMove: !gameoverLine && !boolAfter(tokens, "playMoveWasSearched"),
    completedDepth: numberAfter(tokens, "completedDepth", "completedIterationDepth", "depth") ?? 0,
    provenanceEnvelopeId: null,
  };
}

async function run(config) {
  const started = performance.now();
  const verified = await verifyProvenance(config);
  const acknowledgement = {
    schema: "haitaka-anhoku-strength-first-browser-provenance-ack-v1",
    envelopeId: config.provenance.envelopeId,
    verifiedBeforePlay: true,
    verifiedFiles: verified.verified,
  };
  const ack = await fetch('/provenance', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(acknowledgement) });
  if (!ack.ok) throw new Error(`provenance acknowledgement rejected: ${ack.status}`);
  const importStarted = performance.now();
  const wasm = await import('/pkg/haitaka_wasm.js');
  const glueImportMs = performance.now() - importStarted;
  const wasmInitStarted = performance.now();
  await wasm.default('/pkg/haitaka_wasm_bg.wasm');
  const wasmInitMs = performance.now() - wasmInitStarted;
  const engineA = new wasm.UsiEngine();
  const engineB = new wasm.UsiEngine();
  const modelLoadStarted = performance.now();
  const modelDescription = engineA.load_nnue(verified.fetched.modelA);
  const modelLoadMs = performance.now() - modelLoadStarted;
  const handshake = {
    A: { usi: send(engineA, 'usi'), ready: send(engineA, 'isready') },
    B: { usi: send(engineB, 'usi'), ready: send(engineB, 'isready') },
  };
  const searches = [];
  for (const position of config.positions) {
    for (const [name, engine] of [['A', engineA], ['B', engineB]]) {
      if (send(engine, 'usinewgame').length) throw new Error(`smoke usinewgame output for ${name}`);
      const positionOutput = send(engine, `position sfen ${position.sfen}`);
      if (positionOutput.length) throw new Error(`smoke position rejected for ${position.id}`);
      const begunAt = performance.now();
      const outputs = send(engine, 'go movetime 1000');
      const endedAt = performance.now();
      const trace = parseSearch(outputs, name, position, begunAt, endedAt);
      trace.provenanceEnvelopeId = config.provenance.envelopeId;
      searches.push(trace);
    }
  }
  engineA.free();
  engineB.free();
  return {
    schema: 'haitaka-anhoku-strength-first-browser-smoke-v1',
    status: 'passed',
    provenance: config.provenance,
    execution: { runtime: 'desktop-google-chrome-headless-dedicated-module-worker', workerCount: 1, concurrentGames: 1, engineCount: 2, budget: config.budget, positions: config.positions.length, searches: searches.length },
    load: { workerStartedAt: started, fetchTimingsMs: verified.fetchTimingsMs, glueImportMs, wasmInitMs, modelDescription, modelLoadMs, coldLoadMs: performance.now() - started },
    handshake,
    searches,
  };
}

self.onmessage = async ({ data }) => {
  try {
    self.postMessage({ type: 'result', result: await run(data) });
  } catch (error) {
    self.postMessage({ type: 'error', error: error?.stack ?? String(error) });
  }
};
