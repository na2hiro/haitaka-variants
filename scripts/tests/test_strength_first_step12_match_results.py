"""Native CLI terminal/cap records must survive Step 12 validation and analysis."""

from collections import Counter
import copy
from fractions import Fraction
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import strength_first_step12_match_validation as validator
import strength_first_step12_analysis as analysis


class NativeResultTests(unittest.TestCase):
    OUTCOMES = (
        ("king-captured", "a-win"), ("king-captured", "b-win"),
        ("no-legal-move", "a-win"), ("perpetual-check-loss", "b-win"),
        ("repetition-draw", "draw"), ("jishogi-draw", "draw"),
        ("maximum-ply-capped", "capped"), ("maximum-plies", "draw"),
    )

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.write(self.root / "training/selection.json", {"selectedUpdate": 128})
        self.networks = {}
        for cell, update in (("early-selected", 128), ("u2688-control", 2688)):
            network = self.root / f"training/run/exports/u{update:06d}.nnue"
            network.parent.mkdir(parents=True, exist_ok=True)
            network.write_bytes(f"fixture-network-{update}".encode())
            self.networks[cell] = network
        self.rows = []
        for cell in validator.CELLS:
            for block in range(validator.BLOCKS):
                for index in range(validator.GAMES_PER_BLOCK):
                    self.rows.append({"cell": cell, "block": block,
                        "gameIndexInCell": block * 64 + index,
                        "pairIndex": block * 32 + index // 2,
                        "aColor": "black" if index % 2 == 0 else "white",
                        "bColor": "white" if index % 2 == 0 else "black",
                        "startSfen": f"fixture-{block}-{index // 2} b - 1"})
        self.write(self.root / "openings/schedule.json", {"rows": self.rows})

    @staticmethod
    def write(path, value):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value))

    def block(self, cell="early-selected", block=0):
        attempt = self.root / f"matches/{cell}/block-{block:02d}/attempt-00"
        rows = [row for row in self.rows if row["cell"] == cell and row["block"] == block]
        games = []
        for index, row in enumerate(rows):
            reason, result = self.OUTCOMES[index % len(self.OUTCOMES)]
            plies = 200 if reason in ("maximum-ply-capped", "maximum-plies") else 1
            games.append({"gameIndex": index + 1, "pairIndex": index // 2,
                "aColor": row["aColor"], "bColor": row["bColor"], "startSfen": row["startSfen"],
                "result": result, "terminationReason": reason, "failureState": None,
                "fallbacks": 0, "plies": plies, "moves": ["7g7f"] * plies})
        self.save_block(attempt, cell, games)
        return attempt, games

    def save_block(self, attempt, cell, games):
        counts = Counter(game["result"] for game in games)
        report = {"schema": "haitaka-self-play-report", "schemaVersion": 3, "ruleset": "anhoku",
            "command": {"games": 64, "threads": 1, "movetimeMs": 100, "openingOrder": "sequential",
                "openingUnit": "game", "openingRandomPlies": 0, "seed": 2026091607, "maxPlies": 200},
            "engines": [{"label": "A", "kind": "nnue", "nnueSha256": validator.sha256(self.networks[cell])},
                {"label": "B", "kind": "handcrafted"}],
            # MatchStats counts all winner=None games, including caps, as draws.
            "summary": {"games": 64, "fallbacks": 0, "aWins": counts["a-win"],
                "bWins": counts["b-win"], "draws": counts["draw"] + counts["capped"]}}
        self.write(attempt / "raw/self-play-report.json", report)
        (attempt / "raw/self-play-games.jsonl").write_text("".join(json.dumps(game) + "\n" for game in games))

    def test_native_terminal_vocabulary_and_legacy_caps_validate_without_rewriting(self):
        attempt, games = self.block()
        raw = attempt / "raw/self-play-games.jsonl"
        original = raw.read_bytes()
        result = validator.validate(self.root, "early-selected", 0, attempt)
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["cappedGames"], 16)
        self.assertEqual(result["wdl"], {"aWins": 16, "bWins": 16, "draws": 32})
        self.assertEqual(sum(result["wdl"].values()), 64)
        for pair in result["pairScores"]:
            self.assertEqual(pair["score"], 0.5)
        cap_pair = result["pairScores"][3]["games"]
        self.assertEqual([game["result"] for game in cap_pair], ["capped", "draw"])
        self.assertTrue(all(game["capped"] and game["score"] == 0.5 for game in cap_pair))
        self.assertEqual(raw.read_bytes(), original)

    def test_invalid_cap_lengths_and_inconsistent_terminal_results_fail(self):
        attempt, games = self.block()
        original = copy.deepcopy(games)
        changes = [
            (6, {"result": "a-win"}), (6, {"result": "draw"}),
            (6, {"plies": 199, "moves": ["7g7f"] * 199}),
            (7, {"plies": 199, "moves": ["7g7f"] * 199}),
            (0, {"result": "capped"}), (0, {"result": "draw"}),
            (4, {"result": "a-win"}), (0, {"terminationReason": "unfinished", "result": "unfinished"}),
            (0, {"terminationReason": "unknown"}),
        ]
        for index, change in changes:
            with self.subTest(index=index, change=change):
                altered = copy.deepcopy(original)
                altered[index].update(change)
                self.save_block(attempt, "early-selected", altered)
                with self.assertRaises(validator.ValidationError):
                    validator.validate(self.root, "early-selected", 0, attempt)

    def test_genuine_terminal_on_last_allowed_ply_is_not_counted_as_cap(self):
        attempt, games = self.block()
        games[0].update({"plies": 200, "moves": ["7g7f"] * 200})
        self.save_block(attempt, "early-selected", games)
        result = validator.validate(self.root, "early-selected", 0, attempt)
        self.assertEqual(result["cappedGames"], 16)
        self.assertFalse(result["pairScores"][0]["games"][0]["capped"])
        self.assertEqual(result["pairScores"][0]["games"][0]["score"], 1)

    def test_report_draw_summary_includes_caps_and_is_still_checked(self):
        attempt, _ = self.block()
        path = attempt / "raw/self-play-report.json"
        report = json.loads(path.read_text())
        report["summary"]["draws"] = 24  # Omit the eight native capped records.
        self.write(path, report)
        with self.assertRaisesRegex(validator.ValidationError, "WDL summary"):
            validator.validate(self.root, "early-selected", 0, attempt)

    def test_all_blocks_feed_nominal_scores_and_cap_sensitivity_bounds(self):
        for cell in validator.CELLS:
            for block in range(validator.BLOCKS):
                attempt, _ = self.block(cell, block)
                validator.validate(self.root, cell, block, attempt)
                self.write(attempt / "accepted.json", {"status": "accepted"})
        scores, pairs, bounds, refs = analysis.collect(self.root)
        self.assertEqual(len(refs), 8)
        for cell in validator.CELLS:
            self.assertEqual(scores[cell], [Fraction(1, 2)] * 128)
            self.assertEqual(sum(pair["cappedGames"] for pair in pairs[cell]), 64)
            self.assertEqual(sum(bounds[cell]["lower"]) / 128, Fraction(3, 8))
            self.assertEqual(sum(bounds[cell]["upper"]) / 128, Fraction(5, 8))
            self.assertEqual(analysis.score("capped"), Fraction(1, 2))
        sensitivity = analysis.capped_summary(scores, bounds)
        self.assertEqual(sensitivity["contrastBounds"]["lower"]["exact"], "-1/4")
        self.assertEqual(sensitivity["contrastBounds"]["upper"]["exact"], "1/4")
        self.write(self.root / "preflight/qualification-gate.json", {"status": "passed"})
        self.write(self.root / "matches/status.json", {"status": "complete-sealed", "completedBlocks": 8, "completedGames": 512})
        # Exercise the report path without spending CPU on 10,000 identical draws.
        with mock.patch.object(analysis, "BOOTSTRAP_REPLICATES", 16):
            result = analysis.analyze(self.root, self.root / "analysis.json")
        for model in result["models"].values():
            self.assertEqual(model["wdl"], {"aWins": 64, "bWins": 64, "draws": 128})
            self.assertEqual(model["cappedGames"], 64)
            self.assertEqual(model["meanPairScore"]["exact"], "1/2")


if __name__ == "__main__":
    unittest.main()
