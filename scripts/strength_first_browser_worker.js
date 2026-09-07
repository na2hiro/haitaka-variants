/*
 * Development-only strength-first browser worker.
 *
 * The worker deliberately keeps the two evaluators in separate UsiEngine
 * instances.  Engine A receives the supplied NNUE; engine B is left at the
 * runtime's handcrafted default.  A game is run only after the module and
 * model load have been measured and the browser has acknowledged all input
 * hashes to the Node harness.
 */

const ROOT_RESULT_SCHEMA = "haitaka-search-root-result-v1";
const TERMINAL_REASONS = new Set([
  "king-captured",
  "no-legal-move",
  "repetition-draw",
  "perpetual-check-loss",
  "jishogi-draw",
]);
const DRAW_REASONS = new Set(["repetition-draw", "jishogi-draw"]);
const LOSS_REASONS = new Set(["king-captured", "no-legal-move", "perpetual-check-loss"]);

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

function oppositeColor(color) {
  return color === "b" ? "w" : "b";
}

function colorName(color) {
  return color === "b" ? "black" : "white";
}

function scoreForA(winner) {
  return winner === "A" ? 1 : winner === "B" ? 0 : 0.5;
}

function percentile(values, quantile) {
  if (!values.length) return null;
  const ordered = [...values].sort((a, b) => a - b);
  const rank = Math.max(1, Math.ceil(quantile * ordered.length));
  return ordered[rank - 1];
}

function parseGameover(outputs) {
  const line = outputs.find((item) => item.startsWith("info string gameover ")) ?? null;
  if (!line) return { line: null, reason: null };
  const rest = line.slice("info string gameover ".length).trim();
  return { line, reason: rest.split(/\s+/)[0] || null };
}

function parseSearch(outputs, budget, engineName, color, dispatchAt, begunAt, endedAt, envelopeId) {
  const infoLines = outputs.filter((line) => line.startsWith("info "));
  const info = infoLines.at(-1) ?? "";
  const tokens = info.split(/\s+/);
  const bestLine = outputs.find((line) => line.startsWith("bestmove "));
  if (!bestLine) throw new Error(`missing bestmove output: ${outputs.join(" | ")}`);

  const bestMove = bestLine.slice("bestmove ".length).trim();
  const gameover = parseGameover(outputs);
  const requestedMs = budget.mode === "movetime" ? budget.movetimeMs : null;
  const elapsedMs = endedAt - begunAt;
  const deadlineLatenessMs = requestedMs === null ? 0 : Math.max(0, elapsedMs - requestedMs);
  const alphaBetaNodes = numberAfter(tokens, "alphaBetaNodes", "nodes") ?? 0;
  const qnodes = numberAfter(tokens, "qnodes") ?? 0;
  const totalNodes = numberAfter(tokens, "consumedBudgetNodes") ?? alphaBetaNodes + qnodes;
  const rootResultSchema = valueAfter(tokens, "rootResultSchema");
  const emergencyFallbackUsed = boolAfter(tokens, "emergencyFallbackUsed", "fallback");
  const missingMove = boolAfter(tokens, "missingMove") || (!gameover.reason && bestMove === "resign");
  const playMoveWasSearched = boolAfter(tokens, "playMoveWasSearched");

  return {
    schema: "haitaka-anhoku-strength-first-raw-search-v1",
    engine: engineName,
    color: colorName(color),
    command: `go ${budget.command}`,
    outputs,
    info,
    bestMove,
    gameover: gameover.line,
    gameoverReason: gameover.reason,
    requestedNodes: budget.mode === "nodes" ? budget.nodes : null,
    requestedMs,
    dispatchAt,
    begunAt,
    endedAt,
    schedulerDelayMs: begunAt - dispatchAt,
    elapsedMs,
    latenessMs: deadlineLatenessMs,
    deadlineLatenessMs,
    latenessDefined: requestedMs !== null,
    rootResultSchema,
    playMoveWasSearched,
    lastCompletedIterationValue: numberAfter(tokens, "lastCompletedIterationValue"),
    completedDepth: numberAfter(tokens, "completedDepth", "completedIterationDepth", "depth") ?? 0,
    completedIterationDepth: numberAfter(tokens, "completedIterationDepth"),
    completedRootMovesInInterruptedIteration: numberAfter(tokens, "completedRootMovesInInterruptedIteration") ?? 0,
    partialRootState: boolAfter(tokens, "partialRootState"),
    interruptionReason: valueAfter(tokens, "interruptionReason") ?? "terminal",
    emergencyFallbackUsed,
    missingMove,
    unsearchedMove: !gameover.reason && !playMoveWasSearched,
    alphaBetaNodes,
    qnodes,
    totalNodes,
    consumedBudgetNodes: numberAfter(tokens, "consumedBudgetNodes") ?? totalNodes,
    nodeBudgetCapHits: numberAfter(tokens, "nodeBudgetCapHits") ?? 0,
    engineReportedElapsedMs: numberAfter(tokens, "time"),
    provenanceEnvelopeId: envelopeId,
  };
}

function position(engine, startSfen, moves) {
  const suffix = moves.length ? ` moves ${moves.join(" ")}` : "";
  const outputs = send(engine, `position sfen ${startSfen}${suffix}`);
  if (outputs.length) throw new Error(`position rejected: ${outputs.join(" | ")}`);
}

function search(engine, startSfen, moves, budget, engineName, color, envelopeId) {
  position(engine, startSfen, moves);
  const dispatchAt = performance.now();
  const begunAt = performance.now();
  const outputs = send(engine, `go ${budget.command}`);
  const endedAt = performance.now();
  return parseSearch(outputs, budget, engineName, color, dispatchAt, begunAt, endedAt, envelopeId);
}

function terminalFrom(trace, sideEngine) {
  if (!trace.gameoverReason) {
    if (trace.bestMove === "resign" || trace.missingMove || trace.emergencyFallbackUsed || !trace.playMoveWasSearched) {
      throw new Error(`unqualified ${sideEngine} move: ${JSON.stringify(trace)}`);
    }
    if (trace.rootResultSchema !== ROOT_RESULT_SCHEMA) {
      throw new Error(`wrong root result schema for ${sideEngine}: ${trace.rootResultSchema}`);
    }
    return null;
  }

  if (!TERMINAL_REASONS.has(trace.gameoverReason)) {
    throw new Error(`unknown gameover reason ${trace.gameoverReason}`);
  }
  if (trace.bestMove !== "resign") {
    throw new Error(`terminal ${trace.gameoverReason} returned ${trace.bestMove}`);
  }
  if (DRAW_REASONS.has(trace.gameoverReason)) return { reason: trace.gameoverReason, winner: null };
  if (LOSS_REASONS.has(trace.gameoverReason)) {
    return { reason: trace.gameoverReason, winner: sideEngine === "A" ? "B" : "A" };
  }
  throw new Error(`unhandled terminal reason ${trace.gameoverReason}`);
}

function playGame(engineA, engineB, opening, pairIndex, secondGame, budget, maxPlies, envelopeId) {
  const newGameA = send(engineA, "usinewgame");
  const newGameB = send(engineB, "usinewgame");
  if (newGameA.length || newGameB.length) {
    throw new Error(`usinewgame produced output: A=${newGameA.join(" | ")} B=${newGameB.join(" | ")}`);
  }

  const sfenTokens = opening.sfen.trim().split(/\s+/);
  const startColor = sfenTokens[1];
  if (startColor !== "b" && startColor !== "w") throw new Error(`invalid SFEN side to move: ${opening.sfen}`);

  const blackEngine = secondGame ? "B" : "A";
  const whiteEngine = secondGame ? "A" : "B";
  const aColor = secondGame ? "white" : "black";
  const moves = [];
  const searches = [];
  let winner = null;
  let terminationReason = "maximum-ply-capped";

  for (let ply = 0; ply < maxPlies; ply += 1) {
    const color = ply % 2 === 0 ? startColor : oppositeColor(startColor);
    const sideEngine = color === "b" ? blackEngine : whiteEngine;
    const engine = sideEngine === "A" ? engineA : engineB;
    const trace = search(engine, opening.sfen, moves, budget, sideEngine, color, envelopeId);
    trace.ply = ply + 1;
    trace.moveIndex = moves.length;
    searches.push(trace);

    const terminal = terminalFrom(trace, sideEngine);
    if (terminal) {
      winner = terminal.winner;
      terminationReason = terminal.reason;
      break;
    }
    moves.push(trace.bestMove);
  }

  return {
    schema: "haitaka-anhoku-strength-first-raw-game-v1",
    gameIndex: pairIndex * 2 + (secondGame ? 1 : 0),
    pairIndex,
    openingIndex: opening.index,
    openingId: opening.id,
    startSfen: opening.sfen,
    initialSideToMove: colorName(startColor),
    aColor,
    blackEngine,
    whiteEngine,
    moves,
    plies: moves.length,
    result: winner === null ? "draw" : `${winner.toLowerCase()}-win`,
    winner,
    scoreA: scoreForA(winner),
    terminationReason,
    capped: terminationReason === "maximum-ply-capped",
    missingMoves: searches.filter((item) => item.missingMove).length,
    emergencyFallbacks: searches.filter((item) => item.emergencyFallbackUsed).length,
    unsearchedMoves: searches.filter((item) => item.unsearchedMove).length,
    searches,
  };
}

function aggregateSearches(games) {
  const make = () => ({
    searches: 0,
    alphaBetaNodes: 0,
    qnodes: 0,
    totalNodes: 0,
    completedDepthSum: 0,
    completedDepthMax: 0,
    elapsedMs: 0,
    missingMoves: 0,
    emergencyFallbacks: 0,
    unsearchedMoves: 0,
  });
  const totals = { all: make(), A: make(), B: make() };
  for (const game of games) {
    for (const item of game.searches) {
      for (const key of ["all", item.engine]) {
        const total = totals[key];
        total.searches += 1;
        total.alphaBetaNodes += item.alphaBetaNodes;
        total.qnodes += item.qnodes;
        total.totalNodes += item.totalNodes;
        total.completedDepthSum += item.completedDepth;
        total.completedDepthMax = Math.max(total.completedDepthMax, item.completedDepth);
        total.elapsedMs += item.elapsedMs;
        total.missingMoves += Number(item.missingMove);
        total.emergencyFallbacks += Number(item.emergencyFallbackUsed);
        total.unsearchedMoves += Number(item.unsearchedMove);
      }
    }
  }
  for (const total of Object.values(totals)) {
    total.completedDepthMean = total.searches ? total.completedDepthSum / total.searches : null;
    total.alphaBetaPlusQnodes = total.alphaBetaNodes + total.qnodes;
  }
  return totals;
}

function summarizeGames(games, budget) {
  if (games.length % 2 !== 0) throw new Error("raw game count is not an even number of color-swapped games");
  const pairScoreBins = [0, 0, 0, 0, 0];
  const pairScores = [];
  const terminationCounts = {};
  let missingMoves = 0;
  let emergencyFallbacks = 0;
  let unsearchedMoves = 0;
  for (let index = 0; index < games.length; index += 2) {
    const first = games[index];
    const second = games[index + 1];
    if (first.pairIndex !== second.pairIndex || first.openingId !== second.openingId || first.startSfen !== second.startSfen) {
      throw new Error(`pair ${first.pairIndex} does not share opening identity`);
    }
    if (first.blackEngine !== second.whiteEngine || first.whiteEngine !== second.blackEngine) {
      throw new Error(`pair ${first.pairIndex} does not swap engine seats`);
    }
    const total = first.scoreA + second.scoreA;
    const bin = Math.round(total * 2);
    if (bin < 0 || bin >= pairScoreBins.length) throw new Error(`invalid pair score ${total}`);
    pairScoreBins[bin] += 1;
    pairScores.push({ pairIndex: first.pairIndex, openingId: first.openingId, scoreA: total / 2 });
  }
  for (const game of games) {
    terminationCounts[game.terminationReason] = (terminationCounts[game.terminationReason] ?? 0) + 1;
    missingMoves += game.missingMoves;
    emergencyFallbacks += game.emergencyFallbacks;
    unsearchedMoves += game.unsearchedMoves;
  }

  const searches = games.flatMap((game) => game.searches);
  const lateness = searches.map((item) => item.latenessMs).filter((value) => Number.isFinite(value));
  const elapsed = searches.map((item) => item.elapsedMs).filter((value) => Number.isFinite(value));
  const scoreA = games.reduce((sum, game) => sum + game.scoreA, 0);
  const score = {
    games: games.length,
    pairs: games.length / 2,
    aScore: scoreA,
    aScoreRate: games.length ? scoreA / games.length : null,
    aWins: games.filter((game) => game.winner === "A").length,
    bWins: games.filter((game) => game.winner === "B").length,
    draws: games.filter((game) => game.winner === null).length,
    pairScoreBins,
    pairScoreBinLabels: ["0", "0.5", "1", "1.5", "2"],
    pairScores,
  };
  return {
    score,
    timing: {
      latenessDefinition: budget.mode === "movetime"
        ? "max(0, measured wall elapsed around UsiEngine.send(go) minus requested movetime-ms)"
        : "no wall deadline in nodes-per-move mode; lateness is recorded as zero",
      latenessSampleCount: lateness.length,
      deadlineSampleCount: searches.filter((item) => item.latenessDefined).length,
      p95LatenessMs: percentile(lateness, 0.95),
      p99LatenessMs: percentile(lateness, 0.99),
      maxLatenessMs: lateness.length ? Math.max(...lateness) : null,
      p95ElapsedMs: percentile(elapsed, 0.95),
      p99ElapsedMs: percentile(elapsed, 0.99),
      maxElapsedMs: elapsed.length ? Math.max(...elapsed) : null,
    },
    nodes: aggregateSearches(games),
    missingMoves,
    emergencyFallbacks,
    unsearchedMoves,
    terminationCounts,
  };
}

async function verifyProvenance(config) {
  const verifiedFiles = {};
  const fetchedBytes = {};
  const fetchTimings = {};
  for (const [name, route] of Object.entries(config.provenance.routes)) {
    const started = performance.now();
    const response = await fetch(route, { cache: "no-store" });
    if (!response.ok) throw new Error(`provenance fetch failed for ${name}: ${response.status}`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    const digestBytes = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
    const digest = Array.from(digestBytes, (value) => value.toString(16).padStart(2, "0")).join("");
    const expected = config.provenance.files[name];
    if (!expected || expected.bytes !== bytes.byteLength || expected.sha256 !== digest) {
      throw new Error(`provenance mismatch for ${name}`);
    }
    fetchedBytes[name] = bytes;
    verifiedFiles[name] = { bytes: bytes.byteLength, sha256: digest, route };
    fetchTimings[name] = performance.now() - started;
  }
  return { fetchedBytes, verifiedFiles, fetchTimings };
}

async function run(config, state) {
  const workerStartedAt = performance.now();
  const provenanceVerificationStartedAt = performance.now();
  const verified = await verifyProvenance(config);
  const provenanceVerificationMs = performance.now() - provenanceVerificationStartedAt;
  const acknowledgement = {
    schema: "haitaka-anhoku-strength-first-browser-provenance-ack-v1",
    envelopeId: config.provenance.envelopeId,
    verifiedBeforePlay: true,
    verifiedFiles: verified.verifiedFiles,
  };
  const ackResponse = await fetch("/provenance", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(acknowledgement),
  });
  if (!ackResponse.ok) throw new Error(`provenance acknowledgement rejected: ${ackResponse.status}`);

  const moduleImportStartedAt = performance.now();
  const wasmModule = await import("/pkg/haitaka_wasm.js");
  const glueImportMs = performance.now() - moduleImportStartedAt;
  const wasmInitStartedAt = performance.now();
  await wasmModule.default("/pkg/haitaka_wasm_bg.wasm");
  const wasmInitMs = performance.now() - wasmInitStartedAt;

  const engineA = new wasmModule.UsiEngine();
  const engineB = new wasmModule.UsiEngine();
  const modelBytes = verified.fetchedBytes.modelA;
  if (!modelBytes) throw new Error("verified modelA bytes are missing");
  const modelLoadStartedAt = performance.now();
  const modelDescription = engineA.load_nnue(modelBytes);
  const modelLoadMs = performance.now() - modelLoadStartedAt;
  const warmStartedAt = performance.now();

  const handshake = {
    A: { usi: send(engineA, "usi"), ready: send(engineA, "isready") },
    B: { usi: send(engineB, "usi"), ready: send(engineB, "isready") },
  };
  const games = state.games;
  for (let pairIndex = 0; pairIndex < config.openings.length; pairIndex += 1) {
    const opening = config.openings[pairIndex];
    games.push(playGame(engineA, engineB, opening, pairIndex, false, config.budget, config.maxPlies, config.provenance.envelopeId));
    games.push(playGame(engineA, engineB, opening, pairIndex, true, config.budget, config.maxPlies, config.provenance.envelopeId));
    self.postMessage({ type: "progress", completedPairs: pairIndex + 1, totalPairs: config.openings.length });
  }
  const summary = summarizeGames(games, config.budget);
  if (summary.missingMoves || summary.emergencyFallbacks || summary.unsearchedMoves) {
    throw new Error(`raw match contains unqualified moves: ${JSON.stringify({
      missingMoves: summary.missingMoves,
      emergencyFallbacks: summary.emergencyFallbacks,
      unsearchedMoves: summary.unsearchedMoves,
    })}`);
  }

  const finishedAt = performance.now();
  const result = {
    schema: "haitaka-anhoku-strength-first-browser-report-v1",
    status: "passed",
    provenance: config.provenance,
    execution: {
      runtime: "desktop-google-chrome-headless-dedicated-module-worker",
      workerCount: 1,
      concurrentGames: 1,
      engineCount: 2,
      evaluatorA: "supplied-nnue",
      evaluatorB: "handcrafted-runtime-default",
      wasmBuild: config.wasmBuild ?? null,
      coldWarmVersion: "fresh-worker-module-and-two-engine-load-then-warm-games-v1",
      maxPlies: config.maxPlies,
      openingCount: config.openings.length,
      openingSelection: "first-N-anhoku-v3-SFEN-records-v1",
      budget: config.budget,
    },
    load: {
      workerStartedAt,
      provenanceVerificationMs,
      fetchTimingsMs: verified.fetchTimings,
      glueImportMs,
      wasmInitMs,
      moduleLoadMs: glueImportMs + wasmInitMs,
      modelBytes: modelBytes.byteLength,
      modelDescription,
      modelFetchMs: verified.fetchTimings.modelA ?? null,
      modelLoadMs,
      coldLoadMs: warmStartedAt - workerStartedAt,
      warmStartedAt,
      finishedAt,
      warmElapsedMs: finishedAt - warmStartedAt,
    },
    handshake,
    summary,
    rawGames: games,
    games,
  };
  engineA.free();
  engineB.free();
  return result;
}

self.onmessage = async (event) => {
  const state = { games: [] };
  try {
    const config = event.data;
    if (!config || !config.provenance || config.provenance.finalizedBeforeBrowserLaunch !== true) {
      throw new Error("missing or unsupported strength-first provenance envelope");
    }
    if (config.budget?.mode !== "nodes" && config.budget?.mode !== "movetime") {
      throw new Error("unsupported strength-first search budget");
    }
    const result = await run(config, state);
    self.postMessage({ type: "result", result });
  } catch (error) {
    self.postMessage({
      type: "error",
      error: error?.stack ?? String(error),
      partial: { games: state.games },
    });
  }
};
