import init, { UsiEngine } from "/pkg/haitaka_wasm.js";

const ROOT_RESULT_SCHEMA = "haitaka-search-root-result-v1";

function send(engine, command) {
  return Array.from(engine.send(command), String);
}

function valueAfter(tokens, name) {
  const index = tokens.indexOf(name);
  return index >= 0 && index + 1 < tokens.length ? tokens[index + 1] : null;
}

function boolAfter(tokens, name) {
  return valueAfter(tokens, name) === "1";
}

function numberAfter(tokens, name) {
  const value = valueAfter(tokens, name);
  return value === null || value === "null" ? null : Number(value);
}

function parseSearch(outputs, requestedMs, dispatchAt, begunAt, endedAt) {
  const info = outputs.find((line) => line.startsWith("info ")) ?? "";
  const bestLine = outputs.find((line) => line.startsWith("bestmove "));
  if (!bestLine) throw new Error(`missing bestmove output: ${outputs.join(" | ")}`);
  const tokens = info.split(/\s+/);
  const schema = valueAfter(tokens, "rootResultSchema");
  const bestMove = bestLine.slice("bestmove ".length).trim();
  const gameover = outputs.find((line) => line.startsWith("info string gameover "));
  const interruptionReason = valueAfter(tokens, "interruptionReason") ?? "terminal";
  const elapsedMs = endedAt - begunAt;
  return {
    outputs,
    info,
    bestMove,
    gameover: gameover ?? null,
    requestedMs,
    elapsedMs,
    deadlineLatenessMs: requestedMs === null ? 0 : Math.max(0, elapsedMs - requestedMs),
    schedulerDelayMs: begunAt - dispatchAt,
    rootResultSchema: schema,
    playMoveWasSearched: boolAfter(tokens, "playMoveWasSearched"),
    lastCompletedIterationValue: numberAfter(tokens, "lastCompletedIterationValue"),
    completedIterationDepth: numberAfter(tokens, "completedIterationDepth") ?? numberAfter(tokens, "depth") ?? 0,
    completedRootMovesInInterruptedIteration: numberAfter(tokens, "completedRootMovesInInterruptedIteration") ?? 0,
    partialRootState: boolAfter(tokens, "partialRootState"),
    interruptionReason,
    emergencyFallbackUsed: boolAfter(tokens, "emergencyFallbackUsed"),
    missingMove: boolAfter(tokens, "missingMove") || (!gameover && bestMove === "resign"),
    alphaBetaNodes: numberAfter(tokens, "alphaBetaNodes") ?? numberAfter(tokens, "nodes") ?? 0,
    qnodes: numberAfter(tokens, "qnodes") ?? 0,
    provenanceEnvelopeId: self.r1d3ProvenanceEnvelopeId,
  };
}

function position(engine, startSfen, moves) {
  const suffix = moves.length ? ` moves ${moves.join(" ")}` : "";
  const outputs = send(engine, `position sfen ${startSfen}${suffix}`);
  if (outputs.length) throw new Error(`position rejected: ${outputs.join(" | ")}`);
}

function search(engine, startSfen, moves, go, requestedMs) {
  position(engine, startSfen, moves);
  const dispatchAt = performance.now();
  const begunAt = performance.now();
  const outputs = send(engine, go);
  const endedAt = performance.now();
  return parseSearch(outputs, requestedMs, dispatchAt, begunAt, endedAt);
}

function terminalFrom(searchResult, sideEngine) {
  if (searchResult.bestMove !== "resign") return null;
  if (searchResult.gameover?.includes("repetition-draw")) {
    return { reason: "repetition-draw", winner: null };
  }
  if (searchResult.gameover?.includes("perpetual-check-loss")) {
    return { reason: "perpetual-check-loss", winner: sideEngine === "A" ? "B" : "A" };
  }
  if (searchResult.gameover?.includes("no-legal-move")) {
    return { reason: "no-legal-move", winner: sideEngine === "A" ? "B" : "A" };
  }
  throw new Error(`nonterminal missing move from engine ${sideEngine}: ${searchResult.outputs.join(" | ")}`);
}

function scoreForA(winner) {
  return winner === "A" ? 1 : winner === "B" ? 0 : 0.5;
}

function playGame(engine, lane, opening, pairIndex, secondGame, maxPlies) {
  send(engine, "usinewgame");
  const blackEngine = secondGame ? "B" : "A";
  const whiteEngine = secondGame ? "A" : "B";
  const moves = [];
  const searches = [];
  let winner = null;
  let terminationReason = "maximum-ply-capped";
  for (let ply = 0; ply < maxPlies; ply += 1) {
    const sideEngine = ply % 2 === 0 ? blackEngine : whiteEngine;
    const definition = lane[sideEngine.toLowerCase()];
    const requestedMs = definition.go.startsWith("movetime ") ? Number(definition.go.split(" ")[1]) : null;
    const trace = search(engine, opening.sfen, moves, `go ${definition.go}`, requestedMs);
    trace.ply = ply + 1;
    trace.engine = sideEngine;
    trace.color = ply % 2 === 0 ? "black" : "white";
    trace.coldWarmState = "warm";
    searches.push(trace);
    const terminal = terminalFrom(trace, sideEngine);
    if (terminal) {
      winner = terminal.winner;
      terminationReason = terminal.reason;
      break;
    }
    if (trace.rootResultSchema !== ROOT_RESULT_SCHEMA) {
      throw new Error(`wrong root result schema on ${lane.id} pair ${pairIndex}`);
    }
    if (trace.emergencyFallbackUsed || trace.missingMove || !trace.playMoveWasSearched) {
      throw new Error(`unqualified move on ${lane.id} pair ${pairIndex}: ${JSON.stringify(trace)}`);
    }
    moves.push(trace.bestMove);
  }
  return {
    schema: "haitaka-r1d3-raw-game-v1",
    lane: lane.id,
    gameIndex: pairIndex * 2 + (secondGame ? 1 : 0),
    pairIndex,
    openingId: opening.id,
    startSfen: opening.sfen,
    aColor: secondGame ? "white" : "black",
    blackEngine,
    whiteEngine,
    moves,
    result: winner === null ? "draw" : `${winner.toLowerCase()}-win`,
    winner,
    scoreA: scoreForA(winner),
    terminationReason,
    missingMoves: searches.filter((item) => item.missingMove).length,
    emergencyFallbacks: searches.filter((item) => item.emergencyFallbackUsed).length,
    searchedPartialRootMoves: searches.filter((item) => item.partialRootState && item.playMoveWasSearched).length,
    searches,
  };
}

function fixedDepthDiagnostic(engine, fixture) {
  send(engine, "usinewgame");
  const outputs = send(engine, fixture.position);
  if (outputs.length) throw new Error(`diagnostic position rejected: ${outputs.join(" | ")}`);
  const dispatchAt = performance.now();
  const begunAt = performance.now();
  const searchOutputs = send(engine, fixture.go);
  const endedAt = performance.now();
  return {
    id: fixture.id,
    position: fixture.position,
    go: fixture.go,
    trace: parseSearch(searchOutputs, null, dispatchAt, begunAt, endedAt),
  };
}

function summarizeGames(games) {
  const bins = [0, 0, 0, 0, 0];
  for (let index = 0; index < games.length; index += 2) {
    const total = games[index].scoreA + games[index + 1].scoreA;
    bins[Math.round(total * 2)] += 1;
  }
  return {
    games: games.length,
    pairs: games.length / 2,
    pairScoreBins: bins,
    aScore: games.reduce((sum, game) => sum + game.scoreA, 0),
  };
}

self.onmessage = async (event) => {
  try {
    const config = event.data;
    const envelope = config.provenanceEnvelope;
    if (!envelope || envelope.schema !== config.provenance.envelopeSchema || envelope.schemaVersion !== 1 || !envelope.finalizedBeforeBrowserLaunch) {
      throw new Error("missing or unsupported provenance envelope");
    }
    self.r1d3ProvenanceEnvelopeId = envelope.envelopeId;
    const routes = {
      browserWorker: "/worker.js",
      contract: "/contract.json",
      debugModel: "/model.nnue",
      openings: "/openings.tsv",
      sourceIdentity: "/source-identity.json",
      wasm: "/pkg/haitaka_wasm_bg.wasm",
      wasmGlue: "/pkg/haitaka_wasm.js",
    };
    const verifiedFiles = {};
    for (const [name, route] of Object.entries(routes)) {
      const response = await fetch(route, { cache: "no-store" });
      if (!response.ok) throw new Error(`provenance fetch failed for ${name}: ${response.status}`);
      const bytes = new Uint8Array(await response.arrayBuffer());
      const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), (value) => value.toString(16).padStart(2, "0")).join("");
      const expected = envelope.files[name];
      if (!expected || expected.bytes !== bytes.byteLength || expected.sha256 !== digest) throw new Error(`provenance mismatch for ${name}`);
      verifiedFiles[name] = { bytes: bytes.byteLength, sha256: digest };
    }
    const acknowledgement = {
      schema: config.provenance.acknowledgementSchema,
      envelopeId: envelope.envelopeId,
      verifiedBeforePlay: true,
      verifiedFiles,
    };
    const ackResponse = await fetch("/provenance", { method: "POST", body: JSON.stringify(acknowledgement) });
    if (!ackResponse.ok) throw new Error(`provenance acknowledgement rejected: ${ackResponse.status}`);
    const coldStarted = performance.now();
    await init("/pkg/haitaka_wasm_bg.wasm");
    const modelResponse = await fetch("/model.nnue");
    if (!modelResponse.ok) throw new Error(`model fetch failed: ${modelResponse.status}`);
    let modelBytes = new Uint8Array(await modelResponse.arrayBuffer());
    const modelByteLength = modelBytes.byteLength;
    const engine = new UsiEngine();
    const modelDescription = engine.load_nnue(modelBytes);
    modelBytes = null;
    const coldLoadMs = performance.now() - coldStarted;
    const handshake = {
      usi: send(engine, "usi"),
      ready: send(engine, "isready"),
    };
    const diagnostics = config.nativeEquivalence.positions.map((fixture) => fixedDepthDiagnostic(engine, fixture));
    const lanes = [];
    for (const lane of config.schedule.lanes) {
      const games = [];
      for (let pairIndex = 0; pairIndex < config.schedule.pairsPerLane; pairIndex += 1) {
        const opening = config.openings[pairIndex];
        games.push(playGame(engine, lane, opening, pairIndex, false, config.schedule.maximumPlies));
        games.push(playGame(engine, lane, opening, pairIndex, true, config.schedule.maximumPlies));
        self.postMessage({ type: "progress", lane: lane.id, completedPairs: pairIndex + 1 });
      }
      lanes.push({ id: lane.id, a: lane.a, b: lane.b, summary: summarizeGames(games), games });
    }
    self.postMessage({
      type: "result",
      result: {
        schema: "haitaka-r1d3-browser-trace-v2",
        provenanceEnvelope: envelope,
        clockControllerVersion: config.production.clockControllerVersion,
        coldWarmVersion: config.production.coldWarmVersion,
        workerCount: 1,
        concurrentGames: 1,
        modelBytes: modelByteLength,
        modelDescription,
        coldLoadMs,
        handshake,
        diagnostics,
        lanes,
      },
    });
  } catch (error) {
    self.postMessage({ type: "error", error: error?.stack ?? String(error) });
  }
};
