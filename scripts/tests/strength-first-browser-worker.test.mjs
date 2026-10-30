import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../strength_first_browser_worker.js", import.meta.url), "utf8");
const context = vm.createContext({ self: {}, performance });
vm.runInContext(source + "\nthis.api = { parseSearch, terminalFrom, playGame, summarizeGames };", context);
const { parseSearch, terminalFrom, playGame, summarizeGames } = context.api;
const budget = { mode: "movetime", command: "movetime 10", movetimeMs: 10 };
const notification = (reason) => [`info string gameover ${reason} rules anhoku`, "bestmove resign"];
const parsed = (reason) => parseSearch(notification(reason), budget, "A", "b", 0, 0, 1, "unit");
const move = (usi) => [
  "info depth 1 rootResultSchema haitaka-search-root-result-v1 playMoveWasSearched 1 alphaBetaNodes 1 qnodes 0 interruptionReason none",
  `bestmove ${usi}`,
];

test("perpetual-check notifications retain loser color across seats and queried sides", () => {
  for (const [black, white] of [["A", "B"], ["B", "A"]]) {
    for (const color of ["black", "white"]) {
      const trace = parsed(`perpetual-check-loss:${color}`);
      assert.equal(trace.gameoverReason, "perpetual-check-loss");
      assert.equal(trace.gameoverLoserColor, color);
      assert.equal(trace.gameover, notification(`perpetual-check-loss:${color}`)[0]);
      const loser = color === "black" ? black : white;
      for (const side of ["A", "B"]) {
        const terminal = terminalFrom(trace, side, black, white);
        assert.equal(terminal.reason, "perpetual-check-loss");
        assert.equal(terminal.winner, loser === "A" ? "B" : "A");
      }
    }
  }
  for (const reason of ["perpetual-check-loss", "perpetual-check-loss:red", "perpetual-check-loss:black:extra"]) {
    assert.throws(() => terminalFrom(parsed(reason), "A", "A", "B"), /losing color/);
  }
});

test("unknown terminals and unqualified nonterminal moves still fail", () => {
  assert.throws(() => terminalFrom(parsed("unknown"), "A", "A", "B"), /unknown gameover/);
  for (const output of [["bestmove resign"], ["bestmove 7g7f"]]) {
    const trace = parseSearch(output, budget, "A", "b", 0, 0, 1, "unit");
    assert.throws(() => terminalFrom(trace, "A", "A", "B"), /unqualified/);
  }
});

test("final-ply terminal adjudication honors seats, initial side and move parity without extra search", () => {
  const reasons = [null, "king-captured", "no-legal-move", "repetition-draw", "jishogi-draw",
    "perpetual-check-loss:black", "perpetual-check-loss:white"];
  for (const reason of reasons) {
    for (const second of [false, true]) {
      for (const start of ["b", "w"]) {
        for (const maxPlies of [1, 2]) {
          let goCount = 0;
          let terminalCount = 0;
          const moves = ["7g7f", "3c3d"].slice(0, maxPlies);
          const opening = { id: "unit", index: 0, sfen: `fixture ${start} - 1` };
          const black = second ? "B" : "A";
          const white = second ? "A" : "B";
          const nextColor = maxPlies % 2 === 0 ? start : start === "b" ? "w" : "b";
          const nextSide = nextColor === "b" ? black : white;
          const makeEngine = (name) => {
            let position = null;
            return {
              send(command) {
                if (command.startsWith("position ")) { position = command; return []; }
                if (!command.startsWith("go ")) return [];
                assert.ok(goCount < maxPlies, "adjudication must not search an extra move");
                const color = goCount % 2 === 0 ? start : start === "b" ? "w" : "b";
                assert.equal(name, color === "b" ? black : white);
                return move(moves[goCount++]);
              },
              terminal_notifications() {
                terminalCount += 1;
                assert.equal(name, nextSide);
                assert.equal(position, `position sfen ${opening.sfen} moves ${moves.join(" ")}`);
                return reason ? notification(reason) : [];
              },
            };
          };
          const game = playGame(makeEngine("A"), makeEngine("B"), opening, 0, second, budget, maxPlies, "unit");
          assert.equal(goCount, maxPlies);
          assert.equal(terminalCount, 1);
          assert.deepEqual(Array.from(game.moves), moves);
          assert.equal(game.searches.length, maxPlies);
          assert.equal(game.missingMoves + game.emergencyFallbacks + game.unsearchedMoves, 0);
          assert.equal(game.capped, reason === null);
          assert.equal(game.terminationReason, reason?.split(":")[0] ?? "maximum-ply-capped");
          let winner = null;
          if (reason?.startsWith("perpetual-check-loss:")) winner = reason.endsWith(":black") ? white : black;
          else if (reason === "king-captured" || reason === "no-legal-move") winner = nextSide === "A" ? "B" : "A";
          assert.equal(game.winner, winner);
          assert.equal(game.scoreA, winner === "A" ? 1 : winner === "B" ? 0 : 0.5);
          if (reason) {
            assert.equal(game.terminalNotification.command, null);
            assert.equal(game.terminalNotification.color, nextColor === "b" ? "black" : "white");
            assert.equal(game.terminalNotification.moveIndex, maxPlies);
            assert.equal(game.terminalNotification.gameover, notification(reason)[0]);
          } else assert.equal(game.terminalNotification, null);
          // Terminal probes must not bias timing, depth, node or deadline samples.
          const summary = summarizeGames([game, { ...game, blackEngine: white, whiteEngine: black }], budget);
          assert.equal(summary.nodes.all.searches, 2 * maxPlies);
          assert.equal(summary.timing.deadlineSampleCount, 2 * maxPlies);
          assert.equal(summary.nodes.all.totalNodes, 2 * maxPlies);
        }
      }
    }
  }
});

test("terminals before the cap keep raw notification outside search metrics", () => {
  for (const reason of ["no-legal-move", "repetition-draw", "perpetual-check-loss:black", "perpetual-check-loss:white"]) {
    for (const second of [false, true]) {
      let calls = 0;
      const engine = { send(command) {
        if (!command.startsWith("go ")) return [];
        return calls++ === 0 ? move("7g7f") : notification(reason);
      }, terminal_notifications() { assert.fail("already adjudicated terminal must not be probed again"); } };
      const game = playGame(engine, engine, { id: "unit", index: 0, sfen: "fixture b - 1" }, 0, second, budget, 4, "unit");
      assert.equal(game.searches.length, 1);
      assert.equal(game.plies, 1);
      assert.equal(game.capped, false);
      assert.equal(game.terminalNotification.gameover, notification(reason)[0]);
      const winner = reason === "repetition-draw" ? null
        : reason.endsWith(":black") ? game.whiteEngine : game.blackEngine;
      assert.equal(game.winner, winner);
    }
  }
});
