/* Step 14 qualification-only terminal queries.  This worker uses the bound
 * WASM package but never plays a scored game or dispatches a nonterminal
 * search. */
const TERMINAL = new Set(["king-captured", "no-legal-move", "repetition-draw", "jishogi-draw"]);

function send(engine, command) { return Array.from(engine.send(command), String); }
async function digest(bytes) {
  return Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2, "0")).join("");
}
async function verify(config) {
  const fetched = {}, verified = {}, timings = {};
  for (const [name, route] of Object.entries(config.provenance.routes)) {
    const started = performance.now();
    const response = await fetch(route, {cache: "no-store"});
    if (!response.ok) throw new Error(`provenance fetch failed for ${name}`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    const sha = await digest(bytes);
    const expected = config.provenance.files[name];
    if (!expected || expected.bytes !== bytes.byteLength || expected.sha256 !== sha) throw new Error(`provenance mismatch for ${name}`);
    fetched[name] = bytes;
    verified[name] = {bytes: bytes.byteLength, sha256: sha, route};
    timings[name] = performance.now() - started;
  }
  return {fetched, verified, timings};
}
function parseTerminal(outputs) {
  const lines = outputs.filter(line => line.startsWith("info string gameover "));
  if (lines.length !== 1) throw new Error("terminal query did not emit exactly one gameover line");
  const raw = lines[0].slice("info string gameover ".length).trim().split(/\s+/)[0];
  let canonical = raw, loserColor = null;
  if (raw.startsWith("perpetual-check-loss:")) {
    loserColor = raw.slice("perpetual-check-loss:".length);
    if (loserColor !== "black" && loserColor !== "white") throw new Error("invalid perpetual-check loser color");
    canonical = "perpetual-check-loss";
  }
  if (!TERMINAL.has(canonical) && canonical !== "perpetual-check-loss") throw new Error("unsupported terminal reason");
  return {raw, canonical, loserColor};
}
function query(engine, record, engineName) {
  if (send(engine, "usinewgame").length) throw new Error("terminal usinewgame emitted output");
  const suffix = record.moves.length ? ` moves ${record.moves.join(" ")}` : "";
  if (send(engine, `position sfen ${record.startSfen}${suffix}`).length) throw new Error("terminal position rejected");
  const begunAt = performance.now();
  const outputs = send(engine, "go movetime 1000");
  const endedAt = performance.now();
  const best = outputs.filter(line => line.startsWith("bestmove "));
  if (best.length !== 1) throw new Error("terminal query bestmove count differs");
  if (best[0].slice("bestmove ".length).trim() !== "resign") throw new Error("terminal query did not resign");
  let terminal;
  try { terminal = parseTerminal(outputs); } catch (_) { throw new Error("terminal reason differs"); }
  return {engine: engineName, bestMove: "resign", outputs, gameoverReason: terminal.canonical,
          gameoverReasonRaw: terminal.raw, terminalLoserColor: terminal.loserColor,
          replayedPlies: record.moves.length, elapsedMs: endedAt - begunAt};
}
async function run(config) {
  if (config?.schema !== "haitaka-anhoku-strength-first-step14-terminal-config-v1" || !Array.isArray(config.records) || config.records.length === 0 || config.records.length > 8) throw new Error("terminal configuration differs");
  const started = performance.now();
  const verified = await verify(config);
  self.postMessage({type: "provenance", acknowledgement: {envelopeId: config.provenance.envelopeId, verifiedBeforePlay: true, verifiedFiles: verified.verified}});
  const wasm = await import("/pkg/haitaka_wasm.js");
  await wasm.default(verified.fetched.wasm);
  const engineA = new wasm.UsiEngine(), engineB = new wasm.UsiEngine();
  const modelDescription = engineA.load_nnue(verified.fetched.modelA);
  const handshake = {A: {usi: send(engineA, "usi"), ready: send(engineA, "isready")}, B: {usi: send(engineB, "usi"), ready: send(engineB, "isready")}};
  const replays = [];
  for (const record of config.records) {
    const observedA = query(engineA, record, "A");
    const observed = {A: observedA, B: query(engineB, record, "B")};
    for (const value of Object.values(observed)) {
      if (value.gameoverReason !== record.expectedReason) throw new Error("terminal reason differs from frozen fixture");
    }
    replays.push({...record, observed});
  }
  engineA.free(); engineB.free();
  self.postMessage({type: "result", result: {schema: "haitaka-anhoku-strength-first-step14-terminal-report-v1", status: "passed", provenance: {envelopeId: config.provenance.envelopeId}, execution: {runtime: "desktop-google-chrome-headless-dedicated-module-worker", workerCount: 1, concurrentGames: 1, engineCount: 2, scoredGames: 0, terminalQueries: replays.length * 2}, load: {browserStartedAt: started, modelDescription, handshake}, replays}});
}
self.onmessage = async ({data}) => {
  if (data?.type === "provenance-ack" || data?.type === "game-ack") return;
  try { await run(data); } catch (_) { self.postMessage({type: "error", errorCategory: "terminal-browser-worker-error"}); }
};
