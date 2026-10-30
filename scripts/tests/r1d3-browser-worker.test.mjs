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

test("the last allowed move is adjudicated before recording a capped draw", () => {
  for (const reason of [null, "no-legal-move", "repetition-draw", "perpetual-check-loss:black", "perpetual-check-loss:white"]) {
    for (const second of [false, true]) {
      let searches = 0;
      let adjudications = 0;
      let finalPosition = "";
      const engine = {
        send(command) {
          if (command.startsWith("position ")) { finalPosition = command; return []; }
          if (!command.startsWith("go ")) return [];
          searches += 1;
          return ["info depth 1 rootResultSchema haitaka-search-root-result-v1 playMoveWasSearched 1 alphaBetaNodes 1 interruptionReason none", "bestmove 7g7f"];
        },
        terminal_notifications() {
          adjudications += 1;
          assert.equal(finalPosition, "position sfen start moves 7g7f");
          return reason ? [`info string gameover ${reason} rules anhoku`, "bestmove resign"] : [];
        },
      };
      const game = playGame(engine, { id: "unit", a: { go: "depth 1" }, b: { go: "depth 1" } },
        { id: "start", sfen: "start" }, 0, second, 1);
      assert.equal(searches, 1);
      assert.equal(adjudications, 1);
      assert.equal(game.searches.length, 1);
      assert.equal(game.moves.length, 1);
      if (reason === null) {
        assert.equal(game.terminationReason, "maximum-ply-capped");
        assert.equal(game.winner, null);
        assert.equal(game.terminalNotification, null);
      } else {
        assert.equal(game.terminationReason, reason.split(":")[0]);
        assert.equal(game.terminalNotification.rootResultSchema, null);
        const winner = reason === "repetition-draw" ? null : reason.endsWith(":black") ? game.whiteEngine : game.blackEngine;
        assert.equal(game.winner, winner);
      }
    }
  }
});

test("DFPN-selected moves retain their independent proof/node evidence", () => {
  const result = parseSearch([
    "info depth 0 nodes 0 qnodes 0 rootResultSchema haitaka-search-root-result-v1 playMoveWasSearched 1 dfpnSelected 1 dfpnNodes 5 dfpnStatus mate dfpnBestMove R*1b",
    "bestmove R*1b",
  ], 100, 0, 0, 1);
  assert.equal(result.dfpnSelected, true);
  assert.equal(result.dfpnNodes, 5);
  assert.equal(result.dfpnStatus, "mate");
  assert.equal(result.dfpnBestMove, result.bestMove);
  assert.equal(result.alphaBetaNodes, 0);
  assert.equal(result.qnodes, 0);
});
