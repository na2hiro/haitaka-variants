"""Validate terminal metadata against synthetic, already accepted replay receipts."""

import copy
import itertools
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import strength_first_step14_browser as browser
import strength_first_step14_contract as contract


class PerpetualCheckTraceTests(unittest.TestCase):
    def block(self):
        rows, games, replays = [], [], []
        envelope = "terminal-contract-fixture"
        for index, (loser, leg, initial, plies) in enumerate(itertools.product(
                ("black", "white"), (0, 1), ("black", "white"), (0, 1))):
            row = {"globalGameIndex": index, "globalPairIndex": index // 2,
                   "block": 0, "blockGameIndex": index, "leg": leg, "openingId": f"fixture-{index}",
                   "startSfen": f"fixture {'b' if initial == 'black' else 'w'} - 1",
                   "initialSideToMove": initial, "aColor": "black" if leg == 0 else "white",
                   "bColor": "white" if leg == 0 else "black",
                   "blackEngine": "A" if leg == 0 else "B", "whiteEngine": "B" if leg == 0 else "A"}
            trace = browser.make_fixture_trace(row, plies, True, envelope)
            reason = f"perpetual-check-loss:{loser}"
            trace.update(gameoverReason="perpetual-check-loss", gameoverReasonRaw=reason,
                         terminalLoserColor=loser,
                         outputs=[f"info string gameover {reason} rules anhoku", "bestmove resign"])
            winner = row["whiteEngine"] if loser == "black" else row["blackEngine"]
            searches = [browser.make_fixture_trace(row, move, False, envelope) for move in range(plies)]
            games.append({**row, "schema": "haitaka-anhoku-strength-first-step14-raw-game-v1",
                          "moves": ["7g7f"] * plies, "plies": plies, "rawEnd": "perpetual-check-loss",
                          "rawWinner": winner, "rawResult": f"{winner.lower()}-win",
                          "searches": searches + [trace], "provenanceEnvelopeId": envelope})
            replays.append({**row, "schema": "haitaka-anhoku-strength-first-step14-replay-v1",
                            "status": "passed", "replayedPlies": plies, "maxPlies": 200,
                            "terminationReason": "perpetual-check-loss", "winner": winner,
                            "boardStatus": "ongoing", "historyAdjudication": reason,
                            "checks": dict.fromkeys(("schema", "identity", "uniqueGameIdentity", "sfen",
                                "sideToMove", "seats", "plies", "moveSyntax", "moveLegality",
                                "noMoveAfterTerminal", "history", "terminalPrecedence", "capSemantics",
                                "disposition", "winner"), True)})
            rows.append(row)
        report = {"schema": "haitaka-anhoku-strength-first-step14-browser-report-v1", "status": "passed",
                  "provenance": {"envelopeId": envelope}, "rawGames": games}
        return report, {"records": replays}, {"rows": rows}, envelope

    def test_block_accepts_both_loser_colors_across_seats_sides_and_move_parity(self):
        report, replay, schedule, envelope = self.block()
        accepted = contract.validate_block(report, replay, schedule, 0, envelope)
        self.assertEqual(accepted["status"], "passed")
        self.assertEqual(len(accepted["games"]), 16)
        for game, raw in zip(accepted["games"], report["rawGames"]):
            self.assertEqual(game["terminationReason"], "perpetual-check-loss")
            self.assertEqual(game["winner"], raw["rawWinner"])
            self.assertFalse(game["capped"])

    def test_conflicting_raw_canonical_and_loser_metadata_still_rejects_block(self):
        report, replay, schedule, envelope = self.block()
        for mutation in (
                {"gameoverReasonRaw": "perpetual-check-loss:white"},
                {"terminalLoserColor": "white"},
                {"terminalLoserColor": None},
                {"gameoverReason": "perpetual-check-loss:black"},
                {"gameoverReason": "no-legal-move"},
                {"gameoverReasonRaw": "perpetual-check-loss", "terminalLoserColor": None,
                 "outputs": ["info string gameover perpetual-check-loss", "bestmove resign"]},
                {"gameoverReasonRaw": "perpetual-check-loss:red", "terminalLoserColor": "red",
                 "outputs": ["info string gameover perpetual-check-loss:red", "bestmove resign"]},
                {"outputs": ["info string gameover perpetual-check-loss:black",
                             "info string gameover perpetual-check-loss:white", "bestmove resign"]}):
            with self.subTest(mutation=mutation):
                changed = copy.deepcopy(report)
                changed["rawGames"][0]["searches"][-1].update(mutation)
                with self.assertRaises(contract.ContractError):
                    contract.validate_block(changed, replay, schedule, 0, envelope)


if __name__ == "__main__":
    unittest.main()
