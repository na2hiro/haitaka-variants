import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

// Exercise the production worker functions with a deterministic USI transport.
const source = readFileSync(new URL("../r1d3-browser-worker.js", import.meta.url), "utf8")
  .replace(/^import .*from .*;\n/, "");
const context = vm.createContext({ self: {}, performance });
vm.runInContext(source + "\nthis.api = { parseSearch, terminalFrom, playGame };", context);
const { parseSearch, terminalFrom, playGame } = context.api;
const trace = (reason) => parseSearch([`info string gameover ${reason} rules anhoku`, "bestmove resign"], 10, 0, 0, 1);

test("perpetual-check loser maps through both engine color assignments", () => {
  for (const [black, white] of [["A", "B"], ["B", "A"]]) {
    for (const color of ["black", "white"]) {
      const loser = color === "black" ? black : white;
      for (const side of ["A", "B"]) {
        const result = terminalFrom(trace(`perpetual-check-loss:${color}`), side, black, white);
        assert.equal(result.winner, loser === "A" ? "B" : "A");
        assert.equal(result.reason, "perpetual-check-loss");
      }
    }
  }
  assert.throws(() => terminalFrom(trace("perpetual-check-loss"), "A", "A", "B"), /losing color/);
});

test("terminal notifications stay separate from genuine search traces", () => {
  for (const reason of ["no-legal-move", "repetition-draw", "perpetual-check-loss:black", "perpetual-check-loss:white"]) {
    for (const second of [false, true]) {
      let searches = 0;
      const engine = { send(command) {
        if (!command.startsWith("go ")) return [];
        searches += 1;
        if (searches === 1) return [
          "info depth 1 nodes 1 rootResultSchema haitaka-search-root-result-v1 playMoveWasSearched 1 interruptionReason none alphaBetaNodes 1",
          "bestmove 7g7f",
        ];
        return [`info string gameover ${reason} rules anhoku`, "bestmove resign"];
      } };
      const game = playGame(engine, { id: "unit", a: { go: "depth 1" }, b: { go: "depth 1" } },
        { id: "start", sfen: "start" }, 0, second, 4);
      assert.equal(game.searches.length, 1);
      assert.equal(game.moves.length, 1);
      assert.equal(game.terminalNotification.gameover, `info string gameover ${reason} rules anhoku`);
      assert.equal(game.terminalNotification.rootResultSchema, null);
      assert.equal(game.missingMoves, 0);
      assert.equal(game.emergencyFallbacks, 0);
      if (reason === "repetition-draw") assert.equal(game.winner, null);
      else if (reason === "no-legal-move") assert.equal(game.winner, game.blackEngine);
      else assert.equal(game.winner, reason.endsWith(":black") ? game.whiteEngine : game.blackEngine);
    }
  }
});

test("an unexplained resign remains a failure", () => {
  assert.throws(() => terminalFrom(parseSearch(["bestmove resign"], null, 0, 0, 1), "A", "A", "B"), /nonterminal missing move/);
});
