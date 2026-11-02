/* Step 14 browser match worker.  Raw games are acknowledged by the Python
 * controller before the next game is searched.  A 200-ply end is provisional;
 * the search-free replay owns final adjudication. */
const ROOT_RESULT_SCHEMA = "haitaka-search-root-result-v1";
const TERMINAL = new Set(["king-captured", "no-legal-move", "repetition-draw", "perpetual-check-loss", "jishogi-draw"]);

let pendingAcknowledgement = null;

function send(engine, command) { return Array.from(engine.send(command), String); }

function tokenValues(tokens, key) {
  const values = [];
  for (let index = 0; index < tokens.length; index += 1) {
    if (tokens[index] === key) {
      if (index + 1 >= tokens.length) throw new Error(`missing value for ${key}`);
      values.push(tokens[index + 1]);
    }
  }
  return values;
}

function requiredBool(tokens, key) {
  const values = tokenValues(tokens, key);
  if (values.length !== 1 || (values[0] !== "0" && values[0] !== "1")) throw new Error(`missing or conflicting boolean ${key}`);
  return values[0] === "1";
}

function requiredValue(tokens, key) {
  const values = tokenValues(tokens, key);
  if (values.length !== 1 || values[0] === "") throw new Error(`missing or conflicting value ${key}`);
  return values[0];
}

function colorName(value) { return value === "b" ? "black" : "white"; }

function terminalReason(outputs) {
  const lines = outputs.filter(line => line.startsWith("info string gameover "));
  if (lines.length > 1) throw new Error("duplicate gameover output");
  if (!lines.length) return null;
  const raw = lines[0].slice("info string gameover ".length).trim().split(/\s+/)[0];
  if (raw.startsWith("perpetual-check-loss:")) {
    const loserColor = raw.slice("perpetual-check-loss:".length);
    if (loserColor !== "black" && loserColor !== "white") throw new Error("invalid perpetual-check loser color");
    return { canonical: "perpetual-check-loss", raw, loserColor };
  }
  if (!TERMINAL.has(raw)) throw new Error(`unknown gameover reason ${raw}`);
  return { canonical: raw, raw, loserColor: null };
}

function waitForAcknowledgement(kind, index) {
  return new Promise((resolve, reject) => {
    pendingAcknowledgement = { kind, index, resolve, reject };
  });
}

function receiveAcknowledgement(data) {
  if (!pendingAcknowledgement || data.kind !== pendingAcknowledgement.kind || data.index !== pendingAcknowledgement.index) {
    throw new Error("unexpected browser acknowledgement");
  }
  const current = pendingAcknowledgement;
  pendingAcknowledgement = null;
  if (data.ok !== true) current.reject(new Error(`controller acknowledgement failed for ${data.kind}`));
  else current.resolve(data);
}

function search(engine, opening, moves, scheduled, moveIndex, envelopeId) {
  const suffix = moves.length ? ` moves ${moves.join(" ")}` : "";
  if (Array.from(engine.send(`position sfen ${opening.startSfen}${suffix}`), String).length) throw new Error("position command produced output");
  const begunAt = performance.now();
  const outputs = Array.from(engine.send("go movetime 1000"), String);
  const endedAt = performance.now();
  const bestLines = outputs.filter(line => line.startsWith("bestmove "));
  if (bestLines.length !== 1) throw new Error("search must emit exactly one bestmove");
  const bestMove = bestLines[0].slice("bestmove ".length).trim();
  const terminal = terminalReason(outputs);
  const infoLines = outputs.filter(line => line.startsWith("info ") && !line.startsWith("info string gameover "));
  const info = infoLines.at(-1) ?? "";
  const tokens = info.split(/\s+/).filter(Boolean);
  const engineName = moveIndex % 2 === 0
    ? (opening.initialSideToMove === "black" ? scheduled.blackEngine : scheduled.whiteEngine)
    : (opening.initialSideToMove === "black" ? scheduled.whiteEngine : scheduled.blackEngine);
  const color = moveIndex % 2 === 0 ? opening.initialSideToMove : (opening.initialSideToMove === "black" ? "white" : "black");
  if (terminal) {
    return {
      schema: "haitaka-anhoku-strength-first-step14-raw-search-v1",
      globalGameIndex: scheduled.globalGameIndex, globalPairIndex: scheduled.globalPairIndex,
      block: scheduled.block, blockGameIndex: scheduled.blockGameIndex,
      moveIndex, ply: moveIndex + 1, engine: engineName, color,
      command: "go movetime 1000", requestedMs: 1000, outputs, bestMove,
      gameoverReason: terminal.canonical, gameoverReasonRaw: terminal.raw,
      terminalLoserColor: terminal.loserColor,
      rootResultSchema: null, playMoveWasSearched: false,
      missingMove: false, emergencyFallbackUsed: false, unsearchedMove: false,
      terminalFlagsDerived: true, elapsedMs: endedAt - begunAt,
      provenanceEnvelopeId: envelopeId,
    };
  }
  if (!infoLines.length) throw new Error("nonterminal search has no info line");
  const rootResultSchema = requiredValue(tokens, "rootResultSchema");
  const searched = requiredBool(tokens, "playMoveWasSearched");
  const missing = requiredBool(tokens, "missingMove");
  const fallback = requiredBool(tokens, "emergencyFallbackUsed");
  return {
    schema: "haitaka-anhoku-strength-first-step14-raw-search-v1",
    globalGameIndex: scheduled.globalGameIndex, globalPairIndex: scheduled.globalPairIndex,
    block: scheduled.block, blockGameIndex: scheduled.blockGameIndex,
    moveIndex, ply: moveIndex + 1, engine: engineName, color,
    command: "go movetime 1000", requestedMs: 1000, outputs, bestMove,
    gameoverReason: null, gameoverReasonRaw: null, terminalLoserColor: null,
    rootResultSchema, playMoveWasSearched: searched, missingMove: missing,
    emergencyFallbackUsed: fallback, unsearchedMove: !searched,
    terminalFlagsDerived: false, elapsedMs: endedAt - begunAt,
    provenanceEnvelopeId: envelopeId,
  };
}

function winnerForTerminal(trace, scheduled) {
  if (trace.gameoverReason === "repetition-draw" || trace.gameoverReason === "jishogi-draw") return null;
  if (trace.gameoverReason === "perpetual-check-loss") {
    const winnerColor = trace.terminalLoserColor === "black" ? "white" : "black";
    return winnerColor === scheduled.aColor ? "A" : "B";
  }
  const winnerColor = trace.color === "black" ? "white" : "black";
  return winnerColor === scheduled.aColor ? "A" : "B";
}

async function playGame(engineA, engineB, opening, scheduled, envelopeId, maxPlies) {
  if (Array.from(engineA.send("usinewgame"), String).length || Array.from(engineB.send("usinewgame"), String).length) throw new Error("usinewgame produced output");
  const moves = [], searches = [];
  let rawEnd = "move-limit-reached", rawWinner = null;
  for (let moveIndex = 0; moveIndex < maxPlies; moveIndex += 1) {
    const color = moveIndex % 2 === 0 ? opening.initialSideToMove : (opening.initialSideToMove === "black" ? "white" : "black");
    const engineName = color === "black" ? scheduled.blackEngine : scheduled.whiteEngine;
    const trace = search(engineName === "A" ? engineA : engineB, opening, moves, scheduled, moveIndex, envelopeId);
    searches.push(trace);
    if (trace.gameoverReason) {
      if (trace.bestMove !== "resign" || trace.playMoveWasSearched !== false || trace.missingMove || trace.emergencyFallbackUsed || trace.unsearchedMove) throw new Error("invalid terminal query");
      rawEnd = trace.gameoverReason;
      rawWinner = winnerForTerminal(trace, scheduled);
      break;
    }
    if (trace.bestMove === "resign" || trace.rootResultSchema !== ROOT_RESULT_SCHEMA || trace.playMoveWasSearched !== true || trace.missingMove || trace.emergencyFallbackUsed || trace.unsearchedMove) throw new Error("invalid played search");
    moves.push(trace.bestMove);
  }
  const game = {
    schema: "haitaka-anhoku-strength-first-step14-raw-game-v1",
    ...Object.fromEntries(["globalGameIndex", "globalPairIndex", "block", "blockGameIndex", "leg", "openingId", "startSfen", "initialSideToMove", "aColor", "bColor", "blackEngine", "whiteEngine"].map(k => [k, scheduled[k]])),
    moves, plies: moves.length, rawEnd, rawWinner,
    rawResult: rawWinner === null ? "draw" : `${rawWinner.toLowerCase()}-win`,
    searches, provenanceEnvelopeId: envelopeId,
  };
  self.postMessage({ type: "game", index: scheduled.globalGameIndex, game });
  await waitForAcknowledgement("game", scheduled.globalGameIndex);
  return game;
}

async function digest(bytes) { return Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2, "0")).join(""); }

async function fetchBound(route, expected) {
  const response = await fetch(route, { cache: "no-store" });
  if (!response.ok) throw new Error(`fetch failed ${route}`);
  const bytes = new Uint8Array(await response.arrayBuffer());
  const actual = await digest(bytes);
  if (bytes.byteLength !== expected.bytes || actual !== expected.sha256) throw new Error(`hash mismatch ${route}`);
  return bytes;
}

async function run(config) {
  const fixtureOnly = config.fixtureOnly === true;
  if (config?.schema !== "haitaka-anhoku-strength-first-step14-browser-config-v1" || (fixtureOnly ? config.fixtureMaxPlies !== 2 || config.maxPlies !== 2 : config.maxPlies !== 200)) throw new Error("browser configuration differs");
  const fetched = {};
  for (const name of ["modelA", "wasm", "glue", "worker"]) fetched[name] = await fetchBound(config.provenance.routes[name], config.provenance.files[name]);
  self.postMessage({ type: "provenance", acknowledgement: { envelopeId: config.provenance.envelopeId, verifiedBeforePlay: true, files: config.provenance.files } });
  await waitForAcknowledgement("provenance", 0);
  const wasm = await import("/pkg/haitaka_wasm.js");
  await wasm.default(fetched.wasm);
  const engineA = new wasm.UsiEngine(), engineB = new wasm.UsiEngine();
  const modelLoadStarted = performance.now();
  const modelDescription = engineA.load_nnue(fetched.modelA);
  const modelLoadMs = performance.now() - modelLoadStarted;
  const handshake = { A: { usi: Array.from(engineA.send("usi"), String), ready: Array.from(engineA.send("isready"), String) }, B: { usi: Array.from(engineB.send("usi"), String), ready: Array.from(engineB.send("isready"), String) } };
  const started = performance.now();
  const completed = [];
  for (const scheduled of config.games) {
    const opening = { startSfen: scheduled.startSfen, initialSideToMove: scheduled.initialSideToMove };
    completed.push(await playGame(engineA, engineB, opening, scheduled, config.provenance.envelopeId, config.maxPlies));
    self.postMessage({ type: "progress", completedGames: completed.length, totalGames: config.games.length });
  }
  engineA.free(); engineB.free();
  self.postMessage({ type: "result", result: {
    schema: "haitaka-anhoku-strength-first-step14-browser-report-v1", status: "passed",
    provenance: { envelopeId: config.provenance.envelopeId }, rawGames: completed,
    execution: { runtime: "desktop-google-chrome-headless-dedicated-module-worker", workerCount: 1, concurrentGames: 1, engineCount: 2, maxPlies: config.maxPlies, fixtureOnly, openingCount: config.games.length, budget: { mode: "movetime", movetimeMs: 1000, nodes: null } },
    load: { browserStartedAt: started, modelDescription, modelLoadMs, handshake },
  } });
}

self.onmessage = async ({ data }) => {
  try {
    if (data?.type === "provenance-ack" || data?.type === "game-ack") { receiveAcknowledgement(data); return; }
    await run(data);
  } catch (_error) {
    self.postMessage({ type: "error", errorCategory: "browser-worker-error" });
  }
};
