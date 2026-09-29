/* F1 two-network browser worker. One game at a time, with fresh engine
 * instances and per-instance network loads. Raw evidence is acknowledged
 * durable by the controller before the next game starts. */
const ROOT_SCHEMA = "haitaka-search-root-result-v1";
const TERMINAL = new Set(["king-captured", "no-legal-move", "repetition-draw", "perpetual-check-loss", "jishogi-draw"]);
let pending = null;

function waitAck(kind, index) {
  return new Promise((resolve, reject) => { pending = {kind, index, resolve, reject}; });
}
function ack(data) {
  if (!pending || data.kind !== pending.kind || data.index !== pending.index) throw new Error("unexpected acknowledgement");
  const current = pending; pending = null;
  if (data.ok !== true) current.reject(new Error("controller rejected evidence")); else current.resolve(data);
}
function send(engine, command) { return Array.from(engine.send(command), String); }
function one(tokens, name) {
  const positions = tokens.flatMap((value, index) => value === name ? [index] : []);
  if (positions.length !== 1 || positions[0] + 1 >= tokens.length) throw new Error(`missing/conflicting ${name}`);
  return tokens[positions[0] + 1];
}
function bool(tokens, name) {
  const value = one(tokens, name);
  if (value !== "0" && value !== "1") throw new Error(`invalid ${name}`);
  return value === "1";
}
function opposite(color) { return color === "black" ? "white" : "black"; }
function terminal(outputs) {
  const lines = outputs.filter(line => line.startsWith("info string gameover "));
  if (lines.length > 1) throw new Error("duplicate gameover");
  if (!lines.length) return null;
  const raw = lines[0].slice("info string gameover ".length).trim().split(/\s+/)[0];
  if (raw.startsWith("perpetual-check-loss:")) {
    const loserColor = raw.slice("perpetual-check-loss:".length);
    if (!["black", "white"].includes(loserColor)) throw new Error("invalid perpetual loser");
    return {reason: "perpetual-check-loss", raw, loserColor};
  }
  if (!TERMINAL.has(raw)) throw new Error(`unknown terminal ${raw}`);
  return {reason: raw, raw, loserColor: null};
}
function search(engine, row, moves, moveIndex, envelopeId) {
  const color = moveIndex % 2 === 0 ? row.initialSideToMove : opposite(row.initialSideToMove);
  const role = color === "black" ? row.blackEngine : row.whiteEngine;
  const position = `position sfen ${row.sfen}${moves.length ? ` moves ${moves.join(" ")}` : ""}`;
  if (send(engine, position).length) throw new Error("position command produced output");
  const started = performance.now();
  const outputs = send(engine, "go movetime 1000");
  const elapsedMs = performance.now() - started;
  const best = outputs.filter(line => line.startsWith("bestmove "));
  if (best.length !== 1) throw new Error("one bestmove required");
  const bestMove = best[0].slice("bestmove ".length).trim();
  const end = terminal(outputs);
  const info = outputs.filter(line => line.startsWith("info ") && !line.startsWith("info string gameover ")).at(-1);
  const tokens = info?.split(/\s+/).filter(Boolean) ?? [];
  const flags = end ? {rootResultSchema: null, playMoveWasSearched: false, missingMove: false, emergencyFallbackUsed: false, unsearchedMove: false} : {
    rootResultSchema: one(tokens, "rootResultSchema"), playMoveWasSearched: bool(tokens, "playMoveWasSearched"),
    missingMove: bool(tokens, "missingMove"), emergencyFallbackUsed: bool(tokens, "emergencyFallbackUsed"),
    unsearchedMove: false,
  };
  flags.unsearchedMove = !flags.playMoveWasSearched && !end;
  const reportedTimeMs = end ? null : Number(one(tokens, "time"));
  if (!end && (!Number.isSafeInteger(reportedTimeMs) || reportedTimeMs < 0)) throw new Error("invalid reported search time");
  if (end && bestMove !== "resign") throw new Error("terminal bestmove must resign");
  if (!end && (bestMove === "resign" || flags.rootResultSchema !== ROOT_SCHEMA || !flags.playMoveWasSearched || flags.missingMove || flags.emergencyFallbackUsed || flags.unsearchedMove)) throw new Error("invalid played search");
  return {schema: "haitaka-anhoku-strength-first-f1-raw-search-v1", gameId: row.gameId, moveIndex, ply: moveIndex+1,
    engine: role, color, positionCommand: position, command: "go movetime 1000", requestedMs: 1000,
    outputs, bestMove, elapsedMs, reportedTimeMs, terminalReason: end?.reason ?? null, terminalReasonRaw: end?.raw ?? null,
    terminalLoserColor: end?.loserColor ?? null, ...flags, provenanceEnvelopeId: envelopeId};
}
function rawWinner(end, trace) {
  if (["repetition-draw", "jishogi-draw"].includes(end)) return null;
  if (end === "perpetual-check-loss") return opposite(trace.terminalLoserColor);
  return opposite(trace.color);
}
async function digest(bytes) {
  return Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2,"0")).join("");
}
async function fetchBound(route, expected) {
  const response = await fetch(route, {cache: "no-store"});
  if (!response.ok) throw new Error(`fetch failed ${route}`);
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (bytes.length !== expected.bytes || await digest(bytes) !== expected.sha256) throw new Error(`hash mismatch ${route}`);
  return bytes;
}
async function play(wasm, modelT, modelOpponent, row, envelopeId, maxPlies) {
  const treatment = new wasm.UsiEngine();
  const opponent = new wasm.UsiEngine();
  try {
    const descriptionT = treatment.load_nnue(modelT);
    const descriptionOpponent = opponent.load_nnue(modelOpponent);
    const handshake = {T: {usi: send(treatment,"usi")}, opponent: {usi: send(opponent,"usi")}};
    if (!handshake.T.usi.includes("option name Hash type spin default 16 min 1 max 1024") || !handshake.opponent.usi.includes("option name Hash type spin default 16 min 1 max 1024")) throw new Error("Hash option contract differs");
    handshake.T.hash = send(treatment,"setoption name Hash value 16");
    handshake.opponent.hash = send(opponent,"setoption name Hash value 16");
    if (handshake.T.hash.length || handshake.opponent.hash.length) throw new Error("Hash option rejected");
    handshake.T.ready = send(treatment,"isready");
    handshake.opponent.ready = send(opponent,"isready");
    if (!handshake.T.ready.includes("readyok") || !handshake.opponent.ready.includes("readyok")) throw new Error("engine not ready");
    if (send(treatment,"usinewgame").length || send(opponent,"usinewgame").length) throw new Error("usinewgame output");
    const moves = [], searches = [];
    let rawEnd = "move-limit-reached", winnerColor = null;
    for (let moveIndex=0; moveIndex<maxPlies; moveIndex++) {
      const color = moveIndex % 2 === 0 ? row.initialSideToMove : opposite(row.initialSideToMove);
      const role = color === "black" ? row.blackEngine : row.whiteEngine;
      const trace = search(role === "T" ? treatment : opponent, row, moves, moveIndex, envelopeId);
      searches.push(trace);
      if (trace.terminalReason) {
        rawEnd = trace.terminalReason; winnerColor = rawWinner(rawEnd, trace); break;
      }
      moves.push(trace.bestMove);
    }
    const game = {schema: "haitaka-anhoku-strength-first-f1-raw-game-v1", ...row, moves, plies: moves.length,
      rawEnd, rawWinnerColor: winnerColor, searches, modelDescriptions: {T: descriptionT, opponent: descriptionOpponent},
      handshake, provenanceEnvelopeId: envelopeId, freshInstances: true, perInstanceNetworks: true};
    self.postMessage({type:"game", index:row.gameIndex, game});
    await waitAck("game", row.gameIndex);
    return game;
  } finally { treatment.free(); opponent.free(); }
}
async function run(config) {
  if (config?.schema !== "haitaka-anhoku-strength-first-f1-browser-config-v1" || ![2,200].includes(config.maxPlies)) throw new Error("F1 browser config differs");
  const bound = {};
  for (const name of ["modelT","modelOpponent","wasm","glue","worker"]) bound[name] = await fetchBound(config.provenance.routes[name], config.provenance.files[name]);
  self.postMessage({type:"provenance", acknowledgement:{envelopeId:config.provenance.envelopeId, verifiedBeforePlay:true, files:config.provenance.files}});
  await waitAck("provenance",0);
  const wasm = await import("/pkg/haitaka_wasm.js");
  await wasm.default(bound.wasm);
  const games = [];
  for (const row of config.games) {
    games.push(await play(wasm, bound.modelT, bound.modelOpponent, row, config.provenance.envelopeId, config.maxPlies));
    self.postMessage({type:"progress", completedGames:games.length, totalGames:config.games.length});
  }
  self.postMessage({type:"result", result:{schema:"haitaka-anhoku-strength-first-f1-browser-report-v1", status:"passed", rawGames:games,
    provenance:{envelopeId:config.provenance.envelopeId}, execution:{runtime:"desktop-google-chrome-headless-dedicated-module-worker",workerCount:1,concurrentGames:1,enginesPerGame:2,maxPlies:config.maxPlies,movetimeMs:1000}}});
}
self.onmessage = async ({data}) => {
  try {
    if (data?.type === "provenance-ack" || data?.type === "game-ack") { ack(data); return; }
    await run(data);
  } catch (_error) { self.postMessage({type:"error", errorCategory:"f1-browser-worker-error"}); }
};
