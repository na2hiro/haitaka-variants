import assert from "node:assert/strict";
import { createHash, webcrypto } from "node:crypto";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../strength_first_step14_terminal_worker.js", import.meta.url), "utf8");

function harness(recordCount = 4, immediateAck = false) {
  const messages = [], engines = [];
  let initialized = 0, queries = 0, announced;
  const provenance = new Promise(resolve => { announced = resolve; });
  const bytes = new TextEncoder().encode("bound terminal fixture");
  const binding = {bytes: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex")};
  const config = {
    schema: "haitaka-anhoku-strength-first-step14-terminal-config-v1",
    records: Array.from({length: recordCount}, (_, gameIndex) => ({
      gameIndex, startSfen: "fixture b - 1", moves: ["7g7f"], expectedReason: "no-legal-move",
    })),
    provenance: {envelopeId: "terminal-fixture-envelope",
      routes: Object.fromEntries(["modelA", "wasm", "glue", "worker"].map(name => [name, `/${name}`])),
      files: Object.fromEntries(["modelA", "wasm", "glue", "worker"].map(name => [name, {...binding}]))},
  };
  const self = {postMessage(message) {
    messages.push(message);
    if (message.type === "provenance") {
      announced(message);
      if (immediateAck) void acknowledge();
    }
  }};
  const wasmStub = {
    async default() { initialized++; },
    UsiEngine: class {
      constructor() { this.freed = false; engines.push(this); }
      load_nnue() { return "terminal-fixture-model"; }
      free() { this.freed = true; }
      send(command) {
        if (command === "usi") return ["usiok"];
        if (command === "isready") return ["readyok"];
        if (command === "go movetime 1000") {
          queries++;
          return ["info string gameover no-legal-move rules anhoku", "bestmove resign"];
        }
        return [];
      }
    },
  };
  const context = vm.createContext({self, performance, crypto: webcrypto, wasmStub,
    fetch: async () => ({ok: true, arrayBuffer: async () => bytes.buffer})});
  assert.ok(source.includes('import("/pkg/haitaka_wasm.js")'));
  // Run the production message handler, replacing the WASM import with a deterministic USI engine.
  vm.runInContext(source.replace('import("/pkg/haitaka_wasm.js")', "Promise.resolve(wasmStub)"), context);
  function acknowledge(overrides = {}) {
    return self.onmessage({data: {type: "provenance-ack", kind: "provenance", index: 0, ok: true, ...overrides}});
  }
  return {messages, engines, provenance, acknowledge,
    run: () => self.onmessage({data: config}), stats: () => ({initialized, queries})};
}

test("Step 14 and F1 terminal qualification wait for delayed page acknowledgement", async () => {
  for (const recordCount of [4, 6]) {
    const h = harness(recordCount);
    const finished = h.run();
    const announcement = await h.provenance;
    assert.equal(announcement.acknowledgement.envelopeId, "terminal-fixture-envelope");
    // Let the worker finish anything it could do while the page is still verifying provenance.
    await new Promise(setImmediate);
    assert.deepEqual(h.stats(), {initialized: 0, queries: 0});
    assert.deepEqual(h.messages.map(message => message.type), ["provenance"]);
    await h.acknowledge();
    await finished;
    assert.deepEqual(h.stats(), {initialized: 1, queries: recordCount * 2});
    assert.deepEqual(h.messages.map(message => message.type), ["provenance", "result"]);
    const report = h.messages.at(-1).result;
    assert.equal(report.status, "passed");
    assert.equal(report.execution.terminalQueries, recordCount * 2);
    assert.equal(report.execution.scoredGames, 0);
    assert.ok(h.engines.every(engine => engine.freed));
  }
});

test("rejected and mismatched acknowledgements cannot launch terminal queries", async () => {
  for (const ack of [{ok: false}, {kind: "game"}, {index: 1}]) {
    const h = harness();
    const finished = h.run();
    await h.provenance;
    await h.acknowledge(ack);
    await finished;
    assert.deepEqual(h.stats(), {initialized: 0, queries: 0});
    assert.deepEqual(h.messages.map(message => message.type), ["provenance", "error"]);
    assert.equal(h.messages.at(-1).errorCategory, "terminal-browser-worker-error");
  }
});

test("an immediate provenance acknowledgement is registered before publication", async () => {
  const h = harness(4, true);
  await h.run();
  assert.deepEqual(h.messages.map(message => message.type), ["provenance", "result"]);
  assert.equal(h.stats().queries, 8);
});
