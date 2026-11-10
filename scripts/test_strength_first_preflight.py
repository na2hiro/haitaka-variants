#!/usr/bin/env python3
"""Focused tests for the strength-first Step 1 inventory and audit helpers."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("strength_first_preflight.py")
SPEC = importlib.util.spec_from_file_location("strength_first_preflight", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class StrengthFirstPreflightTests(unittest.TestCase):
    def test_schedule_freezes_64_openings_twice(self) -> None:
        schedule = MODULE.freeze_schedule(MODULE.ROOT / "haitaka_learn/openings/anhoku-v3.tsv")
        self.assertEqual(schedule["status"], "valid")
        self.assertEqual(schedule["pairCount"], 128)
        self.assertEqual(schedule["gameCount"], 256)
        self.assertEqual(len(schedule["openings"]), 64)
        self.assertEqual(len(schedule["pairs"]), 128)
        self.assertEqual(
            [pair["openingId"] for pair in schedule["pairs"][:64]],
            [f"anhoku-v3-{index:03d}" for index in range(1, 65)],
        )
        self.assertEqual(
            [pair["openingId"] for pair in schedule["pairs"][64:]],
            [f"anhoku-v3-{index:03d}" for index in range(1, 65)],
        )
        self.assertEqual(schedule["pairs"][0]["games"][0]["aColor"], "black")
        self.assertEqual(schedule["pairs"][0]["games"][1]["aColor"], "white")
        self.assertTrue(schedule["scheduleSha256"].isalnum())
        self.assertTrue(all(len(pair["pairSha256"]) == 64 for pair in schedule["pairs"]))

    def test_artifact_status_distinguishes_missing_and_invalid(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = root / "model.nnue"
            payload.write_bytes(b"known bytes")
            binding = {
                "path": "model.nnue",
                "bytes": payload.stat().st_size,
                "sha256": hashlib.sha256(payload.read_bytes()).hexdigest(),
            }
            self.assertEqual(MODULE.artifact_status(root, binding, "fixture")["status"], "valid")
            missing = dict(binding, path="gone.nnue")
            self.assertEqual(MODULE.artifact_status(root, missing, "fixture")["status"], "missing")
            payload.write_bytes(b"changed bytes")
            self.assertEqual(MODULE.artifact_status(root, binding, "fixture")["status"], "invalid")

    def test_porcelain_v2_z_extracts_paths_without_status_metadata(self) -> None:
        tracked = "1 .M N... 100644 100644 100644 " + "0" * 40 + " " + "1" * 40 + " tracked name"
        untracked = "? untracked name"
        renamed = "2 R. N... 100644 100644 100644 " + "2" * 40 + " " + "3" * 40 + " R100 new name"
        status = "\0".join((tracked, untracked, renamed, "old name")) + "\0"
        self.assertEqual(
            MODULE._porcelain_v2_paths(status),
            ["tracked name", "untracked name", "new name", "old name"],
        )
        snapshot = MODULE.git_snapshot(MODULE.ROOT)
        self.assertTrue(all(" N... " not in path for path in snapshot["dirtyPaths"]))
        self.assertTrue(all(" 100644 " not in path for path in snapshot["dirtyPaths"]))

    def test_preflight_records_git_models_and_optional_jobs(self) -> None:
        report = MODULE.build_preflight(MODULE.ROOT)
        self.assertEqual(report["schema"], "haitaka-anhoku-strength-first-preflight-v1")
        self.assertTrue(report["readOnly"])
        self.assertEqual(report["contract"]["status"], "valid")
        self.assertEqual(report["developmentSchedule"]["status"], "valid")
        self.assertEqual(report["models"]["status"], "valid")
        self.assertEqual(report["jobInventory"]["status"], "not-supplied")
        self.assertIsInstance(report["git"]["commit"], str)
        self.assertIn("scripts/strength_first_preflight.py", report["git"]["dirtyPaths"])
        self.assertFalse(report["gpuRentalAuthorized"])
        self.assertFalse(report["trainingStarted"])

    def test_identical_engine_null_replay_raw_aggregation_and_report(self) -> None:
        games = []
        for pair_index in range(2):
            for game_index, color in ((pair_index * 2, "black"), (pair_index * 2 + 1, "white")):
                games.append(
                    {
                        "schema": "haitaka-self-play-game",
                        "schemaVersion": 3,
                        "gameIndex": game_index,
                        "pairIndex": pair_index,
                        "aColor": color,
                        "bColor": "white" if color == "black" else "black",
                        "startSfen": "fixture-start b - 1",
                        # Replayed identical-engine lines intentionally have
                        # the same moves and draw outcome in both colors.
                        "moves": ["7g7f", "3c3d"],
                        "plies": 2,
                        "result": "draw",
                        "winner": None,
                        "terminationReason": "repetition-draw",
                        "totalNodes": 100 + game_index,
                        "alphaBetaNodes": 100 + game_index,
                        "qnodes": 20,
                        "completedDepth": 2,
                        # Aggregate values must be ignored when detailed
                        # search records are present; otherwise they double
                        # count a browser/native hybrid row.
                        "fallbacks": 99,
                        "missingMoves": 99,
                        "searches": [
                            {"missingMove": False, "emergencyFallbackUsed": False},
                            {"missingMove": False, "emergencyFallbackUsed": False},
                        ],
                    }
                )
        expected_summary = {
            "games": 4,
            "aWins": 0,
            "bWins": 0,
            "draws": 4,
            "decidedGames": 0,
            "aScore": 2.0,
            "pairCount": 2,
            "pairScoreBins": [0, 0, 2, 0, 0],
            "fallbacks": 0,
            "missingMoves": 0,
            "terminationReasons": {"repetition-draw": 4},
            "totalNodes": 406,
            "alphaBetaNodes": 406,
            "qnodes": 80,
            "completedDepth": 2,
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            games_path = root / "games.jsonl"
            report_path = root / "report.json"
            games_path.write_text("".join(json.dumps(game) + "\n" for game in games), encoding="utf-8")
            report_path.write_text(json.dumps({"summary": expected_summary}) + "\n", encoding="utf-8")
            audit = MODULE.audit_raw_games(games_path, report_path)
        self.assertTrue(audit["passed"], audit["errors"])
        self.assertEqual(audit["recomputed"]["pairScoreBins"], [0, 0, 2, 0, 0])
        self.assertEqual(audit["recomputed"]["terminationReasons"], {"repetition-draw": 4})
        self.assertEqual(audit["recomputed"]["combinedNodes"], 486)
        self.assertEqual(audit["recomputed"]["completedDepth"], 2)

    def test_native_shaped_game_uses_explicit_aggregate_faults(self) -> None:
        games = []
        for game_index, color, result, winner, fallbacks, missing in (
            (1, "black", "a-win", "A", 7, 1),
            (2, "white", "b-win", "B", 11, 2),
        ):
            games.append(
                {
                    "schema": "haitaka-self-play-game",
                    "schemaVersion": 3,
                    "gameIndex": game_index,
                    "pairIndex": 0,
                    "aColor": color,
                    "bColor": "white" if color == "black" else "black",
                    "startSfen": "fixture-start b - 1",
                    "moves": ["7g7f"],
                    "plies": 1,
                    "result": result,
                    "winner": winner,
                    "terminationReason": "no-legal-move",
                    "totalNodes": 80,
                    "alphaBetaNodes": 80,
                    "qnodes": 20,
                    "completedDepth": 2,
                    "fallbacks": fallbacks,
                    "missingMoves": missing,
                    "failureState": None,
                }
            )
        expected_summary = {
            "games": 2,
            "aWins": 1,
            "bWins": 1,
            "draws": 0,
            "pairCount": 1,
            "pairScoreBins": [0, 0, 1, 0, 0],
            "fallbacks": 18,
            "missingMoves": 3,
            "totalNodes": 160,
            "alphaBetaNodes": 160,
            "qnodes": 40,
            "completedDepth": 2,
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            games_path = root / "native-games.jsonl"
            report_path = root / "native-report.json"
            games_path.write_text("".join(json.dumps(game) + "\n" for game in games), encoding="utf-8")
            report_path.write_text(json.dumps({"summary": expected_summary}) + "\n", encoding="utf-8")
            audit = MODULE.audit_raw_games(games_path, report_path)
        self.assertTrue(audit["passed"], audit["errors"])
        self.assertEqual(audit["recomputed"]["fallbacks"], 18)
        self.assertEqual(audit["recomputed"]["missingMoves"], 3)

    def test_raw_audit_rejects_stale_report(self) -> None:
        game = {
            "gameIndex": 0,
            "pairIndex": 0,
            "aColor": "black",
            "startSfen": "fixture-start b - 1",
            "moves": [],
            "plies": 0,
            "result": "draw",
            "winner": None,
            "terminationReason": "maximum-ply-capped",
            "totalNodes": 1,
            "qnodes": 2,
            "completedDepth": 0,
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            games_path = root / "games.jsonl"
            report_path = root / "report.json"
            games_path.write_text(json.dumps(game) + "\n", encoding="utf-8")
            report_path.write_text(json.dumps({"summary": {"games": 99}}) + "\n", encoding="utf-8")
            audit = MODULE.audit_raw_games(games_path, report_path)
        self.assertFalse(audit["passed"])
        self.assertEqual(audit["reportCheck"]["status"], "invalid")
        self.assertTrue(any("summary.games" in item for item in audit["reportCheck"]["mismatches"]))

    def test_native_capped_result_is_scored_as_draw(self) -> None:
        game = {
            "gameIndex": 0,
            "pairIndex": 0,
            "aColor": "black",
            "bColor": "white",
            "startSfen": "fixture-start b - 1",
            "moves": ["7g7f"],
            "plies": 1,
            "result": "capped",
            "winner": None,
            "terminationReason": "maximum-ply-capped",
            "totalNodes": 8,
            "alphaBetaNodes": 8,
            "qnodes": 2,
            "completedDepth": 1,
            "fallbacks": 0,
            "missingMoves": 0,
        }
        reverse = dict(game, gameIndex=1, aColor="white", bColor="black")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            games_path = root / "games.jsonl"
            games_path.write_text(json.dumps(game) + "\n" + json.dumps(reverse) + "\n", encoding="utf-8")
            audit = MODULE.audit_raw_games(games_path)
        self.assertTrue(audit["passed"], audit["errors"])
        self.assertEqual(audit["recomputed"]["draws"], 2)
        self.assertEqual(audit["recomputed"]["aScore"], 1.0)
        self.assertEqual(audit["recomputed"]["terminationReasons"], {"maximum-ply-capped": 2})


if __name__ == "__main__":
    unittest.main()
