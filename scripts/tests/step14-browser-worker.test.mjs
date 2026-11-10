import assert from "node:assert/strict";
import { webcrypto, createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../strength_first_step14_browser_worker.js", import.meta.url), "utf8");
const deferred = () => {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return {promise, resolve};
};

function harness({reason = null, autoAck = true, invalidFlags = false} = {}) {
  const messages = [], engines = [], provenance = deferred(), firstGame = deferred();
  let initialized = 0, searches = 0;
  const bytes = new TextEncoder().encode("bound test artifact");
  const binding = {bytes: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex")};
  const rows = [0, 1].map(leg => ({
    globalGameIndex: leg, globalPairIndex: 0, block: 0, blockGameIndex: leg, leg,
    openingId: "fixture", startSfen: "fixture w - 1", initialSideToMove: "white",
    aColor: leg === 0 ? "black" : "white", bColor: leg === 0 ? "white" : "black",
    blackEngine: leg === 0 ? "A" : "B", whiteEngine: leg === 0 ? "B" : "A",
  }));
  const config = {
    schema: "haitaka-anhoku-strength-first-step14-browser-config-v1",
    fixtureOnly: true, fixtureMaxPlies: 2, maxPlies: 2, games: rows,
    provenance: {envelopeId: "fixture-envelope",
      routes: Object.fromEntries(["modelA", "wasm", "glue", "worker"].map(name => [name, `/${name}`])),
      files: Object.fromEntries(["modelA", "wasm", "glue", "worker"].map(name => [name, {...binding}]))},
  };
  const self = {postMessage(message) {
    messages.push(message);
    if (message.type === "provenance") provenance.resolve(message);
    if (message.type === "game") firstGame.resolve(message);
    if (autoAck && ["provenance", "game"].includes(message.type)) {
      queueMicrotask(() => acknowledge(message));
    }
  }};
  function acknowledge(message, ok = true) {
    context.api.receiveAcknowledgement({kind: message.type, index: message.index ?? 0, ok});
  }
  const wasmStub = {
    async default(actual) { assert.deepEqual(Array.from(actual), Array.from(bytes)); initialized++; },
    UsiEngine: class {
      constructor() { this.loaded = false; this.freed = false; this.moves = 0; engines.push(this); }
      load_nnue(actual) { assert.deepEqual(Array.from(actual), Array.from(bytes)); this.loaded = true; return "fixture-model"; }
      free() { this.freed = true; }
      send(command) {
        if (command === "usi") return ["usiok"];
        if (command === "isready") return ["readyok"];
        if (command.startsWith("position ")) {
          this.moves = command.includes(" moves ") ? command.split(" moves ")[1].split(" ").length : 0;
          return [];
        }
        if (command === "go movetime 1000") {
          searches++;
          if (reason && this.moves === 1) return [`info string gameover ${reason} rules anhoku`, "bestmove resign"];
          return [
            `info depth 1 rootResultSchema haitaka-search-root-result-v1 playMoveWasSearched ${invalidFlags ? 0 : 1} missingMove 0 emergencyFallbackUsed 0`,
            "bestmove 7g7f",
          ];
        }
        return [];
      }
    },
  };
  const context = vm.createContext({self, performance, crypto: webcrypto, wasmStub,
    fetch: async () => ({ok: true, arrayBuffer: async () => bytes.buffer})});
  // Replace only the module load: exercise production worker logic with a deterministic USI engine.
  assert.ok(source.includes('import("/pkg/haitaka_wasm.js")'));
  vm.runInContext(source.replace('import("/pkg/haitaka_wasm.js")', "Promise.resolve(wasmStub)") +
    "\nthis.api = {run, receiveAcknowledgement};", context);
  return {config, messages, engines, provenance, firstGame, acknowledge,
    run: () => context.api.run(config), stats: () => ({initialized, searches})};
}

test("Step 14 runs the rehearsal and scored config and preserves provisional caps", async () => {
  for (const fixtureOnly of [true, false]) {
    const h = harness();
    if (!fixtureOnly) Object.assign(h.config, {fixtureOnly: false, fixtureMaxPlies: null, maxPlies: 200});
    await h.run();
    const report = h.messages.at(-1).result;
    assert.equal(report.schema, "haitaka-anhoku-strength-first-step14-browser-report-v1");
    assert.equal(report.status, "passed");
    assert.equal(report.rawGames.length, 2);
    assert.equal(report.execution.fixtureOnly, fixtureOnly);
    assert.equal(report.execution.maxPlies, h.config.maxPlies);
    assert.equal(h.engines[0].loaded, true);
    assert.equal(h.engines[1].loaded, false, "opponent must stay handcrafted");
    assert.ok(h.engines.every(engine => engine.freed));
    for (const [index, game] of report.rawGames.entries()) {
      assert.equal(game.globalGameIndex, index);
      assert.equal(game.plies, h.config.maxPlies);
      assert.equal(game.searches.length, game.plies);
      assert.equal(game.rawEnd, "move-limit-reached");
      assert.equal(game.rawWinner, null);
      assert.equal(game.rawResult, "draw");
      assert.equal(game.provenanceEnvelopeId, "fixture-envelope");
      assert.equal(game.searches[0].color, "white");
      assert.equal(game.searches[0].engine, game.whiteEngine);
      assert.equal(game.searches[1].engine, game.blackEngine);
    }
  }
});

test("provenance and durable game acknowledgements gate search and the next game", async () => {
  const h = harness({autoAck: false});
  const finished = h.run();
  const provenance = await h.provenance.promise;
  assert.equal(h.stats().initialized, 0);
  assert.equal(h.stats().searches, 0);
  h.acknowledge(provenance);
  const game = await h.firstGame.promise;
  assert.equal(h.stats().searches, 2);
  assert.equal(h.messages.some(message => message.type === "result"), false);
  h.acknowledge(game);
  // Acknowledge subsequent games asynchronously, as the page does.
  const original = h.messages.push.bind(h.messages);
  h.messages.push = message => {
    if (message.type === "game") queueMicrotask(() => h.acknowledge(message));
    return original(message);
  };
  await finished;
  assert.equal(h.stats().searches, 4);
  assert.equal(h.messages.at(-1).type, "result");
});

test("terminal queries preserve the raw loser color and assign winners through swapped seats", async () => {
  for (const reason of ["no-legal-move", "no-legal-move winner black", "no-legal-move winner white", "king-captured", "repetition-draw", "jishogi-draw",
    "perpetual-check-loss:black", "perpetual-check-loss:white"]) {
    const h = harness({reason});
    await h.run();
    for (const game of h.messages.at(-1).result.rawGames) {
      assert.equal(game.plies, 1);
      assert.equal(game.searches.length, 2);
      const terminal = game.searches.at(-1);
      assert.equal(terminal.bestMove, "resign");
      assert.equal(terminal.gameoverReason, reason.split(/[\s:]/)[0]);
      assert.equal(terminal.gameoverReasonRaw, reason.split(/\s+/)[0]);
      assert.equal(terminal.terminalFlagsDerived, true);
      assert.equal(terminal.rootResultSchema, null);
      const winnerColor = reason.endsWith(":black") ? "white" : "black";
      const winner = ["repetition-draw", "jishogi-draw"].includes(reason) ? null
        : reason.startsWith("perpetual-check-loss:")
          ? (winnerColor === game.aColor ? "A" : "B") : reason.includes("winner black") ? game.blackEngine : game.whiteEngine;
      assert.equal(game.rawWinner, winner);
    }
  }
});

test("artifact mismatches and unsearched moves cannot produce a success report", async () => {
  const changed = harness();
  changed.config.provenance.files.modelA.sha256 = "0".repeat(64);
  await assert.rejects(changed.run(), /hash mismatch/);
  assert.equal(changed.stats().initialized, 0);
  assert.equal(changed.messages.length, 0);
  const unsearched = harness({invalidFlags: true});
  await assert.rejects(unsearched.run(), /invalid played search/);
  assert.equal(unsearched.messages.some(message => ["game", "result"].includes(message.type)), false);
});
