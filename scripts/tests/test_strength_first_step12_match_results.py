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
import strength_first_step12_matches as matches


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


class CampaignResumeTests(unittest.TestCase):
    def campaign(self, interrupt=False):
        fixture = NativeResultTests()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        root = fixture.root
        engine = root / "fixture-cli"
        engine.write_bytes(b"offline-test-runtime")
        fixture.write(root / "preflight/match-gate.json", {"status": "passed"})
        fixture.write(root / "preflight/qualification-gate.json", {"status": "passed"})
        fixture.write(root / "contract.json", {})
        fixture.write(root / "authorization/cpu-match.json", {"granted": True})
        for row in fixture.rows:
            row.update({"matchSeed": matches.MATCH_SEED, "movetimeMs": matches.MOVETIME_MS,
                        "maxPlies": matches.MAX_PLIES, "threads": 1, "concurrentGames": 1,
                        "openingOrder": "sequential", "openingRandomPlies": 0})
        schedule_path = root / "openings/schedule.json"
        fixture.write(schedule_path, {"cells": list(matches.CELLS), "blocks": 4,
                                     "gamesPerCell": 256, "pairsPerCell": 128, "rows": fixture.rows})
        regression = root / "preflight/layout-regression.json"
        fixture.write(regression, {"status": "passed"})
        fixture.write(root / "preflight/schedule-layout-compatibility.json", {
            "schema": "haitaka-anhoku-strength-first-step12-schedule-layout-compatibility-v1",
            "status": "passed", "sourceLayout": "per-row", "rowsVerified": 512,
            "scheduleModified": False, "scheduleRegenerated": False,
            "schedule": {"sha256": matches.sha256(schedule_path), "bytes": schedule_path.stat().st_size},
            "patchedHelper": {"path": "scripts/strength_first_step12_matches.py", "sha256": matches.sha256(Path(matches.__file__))},
            "regressionTests": {"path": "preflight/layout-regression.json", "sha256": matches.sha256(regression)}})
        real_run_block = matches.run_block
        launched = []
        interrupted = False

        def block(repo, schedule, cell, index, runtime, network):
            nonlocal interrupted
            block_dir = root / f"matches/{cell}/block-{index:02d}"
            if matches.valid_accepted(block_dir) is not None:
                return real_run_block(repo, schedule, cell, index, runtime, network)
            if interrupt and len(launched) == 3 and not interrupted:
                interrupted = True
                raise matches.MatchError("simulated controller interruption")
            # Seal the real validator's offline fixture instead of launching games.
            attempt, _ = fixture.block(cell, index)
            validator.validate(root, cell, index, attempt)
            fixture.write(attempt / "accepted.json", {"status": "accepted"})
            launched.append((cell, index))
            return {"cell": cell, "block": index, "status": "accepted",
                    "attempt": attempt.name, "gamesLaunched": 64}

        return fixture, engine, launched, block

    def exercise(self, interrupt):
        fixture, engine, launched, block = self.campaign(interrupt)
        root = fixture.root
        with mock.patch.object(matches, "runtime", return_value=engine), \
             mock.patch.object(matches, "run_block", side_effect=block), \
             mock.patch.object(matches.subprocess, "run", side_effect=AssertionError("no actual matches")):
            if interrupt:
                with self.assertRaisesRegex(matches.MatchError, "simulated"):
                    matches.run(root, resume=False)
                interrupted_status = matches.load(root / "matches/status.json")
                self.assertEqual(interrupted_status["completedGames"], 192)
                self.assertEqual(interrupted_status["gamesLaunched"], 192)
            summary = matches.run(root, resume=interrupt)
            self.assertEqual(summary["completedBlocks"], 8)
            self.assertEqual(summary["completedGames"], 512)
            self.assertEqual(summary["gamesLaunched"], 320 if interrupt else 512)
            self.assertEqual(len(launched), 8)
            ledger = matches.load(root / "matches/resource-ledger.json")
            self.assertEqual(ledger["games"], 512)
            self.assertEqual(ledger["gamesLaunched"], summary["gamesLaunched"])
            with mock.patch.object(analysis, "BOOTSTRAP_REPLICATES", 16):
                result = analysis.analyze(root, root / "analysis.json")
            self.assertEqual(result["status"], "complete-sealed")
            repeated = matches.run(root, resume=True)
            self.assertEqual(repeated["completedGames"], 512)
            self.assertEqual(repeated["completedBlocks"], 8)
            self.assertEqual(repeated["gamesLaunched"], 0)
            self.assertEqual(len(launched), 8, "accepted blocks must not launch again")
            self.assertTrue(all(record["status"] == "already-accepted" for record in repeated["blocks"]))
            self.assertEqual(matches.load(root / "matches/resource-ledger.json")["games"], 512)

    def test_interrupted_campaign_resumes_and_remains_analyzable_without_relaunches(self):
        self.exercise(interrupt=True)

    def test_fresh_campaign_and_completed_rerun_keep_separate_counts(self):
        self.exercise(interrupt=False)


    def stale_gate_is_rejected(self, change):
        fixture, engine, _, _ = self.campaign()
        root = fixture.root
        with mock.patch.object(matches, "runtime", return_value=engine):
            original_gate = matches.gate(root)
            self.assertEqual(original_gate["status"], "passed")
            if change == "schedule":
                fixture.rows[0]["startSfen"] = "changed-opening b - 1"
                path = root / "openings/schedule.json"
                value = matches.load(path)
                value["rows"] = fixture.rows
                fixture.write(path, value)
                self.assertEqual(matches.schedule(root)["rows"][0]["startSfen"], "changed-opening b - 1")
                reason = "frozen schedule bytes"
            else:
                fixture.write(root / "authorization/cpu-match.json", {"granted": False})
                reason = "authorization is not granted"
            for resume in (False, True):
                with self.subTest(resume=resume):
                    fixture.write(root / "preflight/match-gate.json", original_gate)
                    with mock.patch.object(matches, "run_block", side_effect=AssertionError("must not dispatch")) as dispatch, \
                         mock.patch.object(matches.subprocess, "run", side_effect=AssertionError("must not launch")):
                        with self.assertRaisesRegex(matches.MatchError, "match gate is not passed"):
                            matches.run(root, resume=resume)
                        dispatch.assert_not_called()
                    rejected = matches.load(root / "preflight/match-gate.json")
                    self.assertEqual(rejected["status"], "blocked")
                    self.assertTrue(any(reason in item for item in rejected["reasons"]))

    def test_opening_changed_after_passed_gate_blocks_fresh_and_resumed_campaign(self):
        self.stale_gate_is_rejected("schedule")

    def test_revoked_authorization_blocks_fresh_and_resumed_campaign(self):
        self.stale_gate_is_rejected("authorization")


if __name__ == "__main__":
    unittest.main()
