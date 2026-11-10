#!/usr/bin/env python3
"""CPU-only F1 failure-mode tests; no search, training or device import."""

from __future__ import annotations

import math
from pathlib import Path
import unittest
from unittest.mock import patch

import strength_first_f1_core as f1
import strength_first_f1_openings as openings
import strength_first_f1_replay as replay
import strength_first_f1_train as train
import strength_first_f1_live as live
import strength_first_f1_cuda_fixture as cuda_fixture
import strength_first_f1_monitor as monitor


class F1OfflineTests(unittest.TestCase):
    def test_runtime_fixture_uses_excluded_source_families(self) -> None:
        fixture = live.fixture_schedule(f1.F1_ROOT)
        self.assertEqual(len(fixture["rows"]), 8)
        self.assertEqual(fixture["rows"][0]["gameId"], "fixture-0000-T-C-0")
        self.assertEqual(fixture["rows"][-1]["gameId"], "fixture-0001-T-C-1")
        self.assertTrue(all(row["maxPlies"] == 2 for row in fixture["rows"]))

    def test_runtime_dispatch_preserves_all_frozen_game_indices(self) -> None:
        import json
        for phase, blocks in (("development", 16), ("holdout", 64)):
            schedule = json.loads((f1.F1_ROOT / f"openings/{phase}-schedule.json").read_text())
            for block in range(blocks):
                rows = [row for row in schedule["rows"] if row["blockIndex"] == block]
                parts = live._ordered_parts(rows)
                self.assertEqual([row for _, part in parts for row in part], rows)
                self.assertTrue(all({row["matchup"] for row in part} == {f"T-{opponent}"} for opponent, part in parts))

    def test_runtime_ply_200_terminal_precedes_provisional_cap(self) -> None:
        game = {"rawEnd": "move-limit-reached", "rawWinnerColor": None, "plies": 200}
        replay = {"terminationReason": "no-legal-move", "winnerColor": "black", "winner": "A"}
        self.assertEqual(live._canonical_replay_result(game, replay), (1, False))
        self.assertEqual(live._canonical_replay_result(game, {"terminationReason": "maximum-ply-capped", "winnerColor": None, "winner": None}), (0.5, True))
        with self.assertRaises(f1.F1Error):
            live._canonical_replay_result({**game, "plies": 199}, replay)

    def test_runtime_host_timeout_keeps_replay_reserve(self) -> None:
        from unittest.mock import patch
        with patch.object(live.time, "monotonic", return_value=100):
            self.assertEqual(live._budget_timeout(1100, 2000, 900), 100)
            with self.assertRaises(f1.F1Error):
                live._budget_timeout(1001, 2000, 900)

    def test_runtime_active_ledger_excludes_idle_gap_and_charges_whole_attempts(self) -> None:
        from unittest.mock import patch
        from datetime import datetime, timezone
        root = Path("/virtual-f1")
        start = "2026-09-27T00:00:00+00:00"
        end = "2026-09-27T12:00:00+00:00"
        setup = {"schema": "haitaka-anhoku-strength-first-f1-mbp-setup-ledger-v1", "host": "ITSO-WX6303", "accountingMethod": "retained-command-intervals-plus-monotonic-freeze-attempts", "includesIdleTime": False, "preexistingStagingUnmetered": True, "setupStart": {"path": "runtime/setup-start.json"}, "runtimeFreeze": {"path": "runtime/freeze.json"}, "package": {"path": "package.json"}, "activities": [{"path": "runtime/setup-activity/segment-00.json"}], "freezeAttempts": [{"launch": {"path": "runtime/freeze-attempts/attempt-00/launch.json"}, "duration": {"path": "runtime/freeze-attempts/attempt-00/duration.json"}}], "startedAtUtc": start, "endedAtUtc": end, "wallIntervalSeconds": 12 * 3600, "wallIntervalRole": "upper-bound-includes-idle-and-unmetered-prior-staging", "elapsedSeconds": 20}
        qualification = root / "runtime/qualification/attempt-00/launch.json"
        block = root / "games/development/block-00/attempt-00/launch.json"
        def glob(path: Path, pattern: str):
            return [qualification] if "qualification" in str(path) else [block]
        def load(path: Path):
            if path.name == "setup-ledger.json":
                return setup
            if path.name == "setup-start.json":
                return {"startedAtUtc": start}
            return {"elapsedSeconds": 30 if "qualification" in str(path) else 70}
        def verify(binding):
            return root / binding["path"]
        with patch.object(Path, "glob", autospec=True, side_effect=glob), patch.object(Path, "is_file", autospec=True, return_value=True), patch.object(live, "_load", side_effect=load), patch.object(live, "_setup_components", return_value=(setup["activities"], setup["freezeAttempts"], 20, datetime.fromisoformat(start))), patch.object(f1, "verify_artifact", side_effect=verify):
            self.assertEqual(live._spent_seconds(root), 120)
            setup["elapsedSeconds"] = 19
            with self.assertRaises(f1.F1Error):
                live._spent_seconds(root)

    def test_setup_command_receipt_charges_active_intervals_only(self) -> None:
        from datetime import datetime, timezone
        observed = datetime(2026, 9, 28, 1, tzinfo=timezone.utc)
        receipt = {"schema": "haitaka-anhoku-strength-first-f1-mbp-command-receipt-v1", "host": "ITSO-WX6303", "commands": [
            {"commandId": "stage", "command": "rsync inputs", "source": "retained-tool-call", "startedAtUtc": "2026-09-27T13:00:00+00:00", "endedAtUtc": "2026-09-27T13:00:30+00:00"},
            {"commandId": "verify", "command": "package verify", "source": "retained-tool-call", "startedAtUtc": "2026-09-28T00:00:00+00:00", "endedAtUtc": "2026-09-28T00:00:10+00:00"}]}
        seconds, started, ended, intervals = live._setup_activity(receipt, observed=observed)
        self.assertEqual(seconds, 40)
        self.assertEqual((ended - started).total_seconds(), 11 * 3600 + 10)
        self.assertEqual(len(intervals), 2)
        receipt["commands"][1]["command"] = "freeze-runtime"
        with self.assertRaises(f1.F1Error):
            live._setup_activity(receipt, observed=observed)

    def test_paid_hours_use_owned_start_and_duration_not_host_uptime(self) -> None:
        with patch.object(monitor.time, "time", return_value=1_000_000.0):
            row = {"start_date": 1_000_000.0 - 691, "duration": 690, "uptime_mins": 128.7}
            self.assertAlmostEqual(monitor._hours(row), 691 / 3600)
            self.assertAlmostEqual(monitor._hours({**row, "duration": 700}), 700 / 3600)
            with self.assertRaises(f1.F1Error):
                monitor._hours({"uptime_mins": 128.7})

    def row(self, color: str, leg: int) -> dict:
        return {"gameId": f"development-0003-T-C-{leg}", "gameIndex": 12 + leg, "phase": "development", "matchup": "T-C", "pairIndex": 3, "leg": leg,
                "sfen": "9/9/9/9/9/9/9/9/9 b - 1", "initialSideToMove": "black", "blackEngine": "T" if color == "black" else "C", "whiteEngine": "C" if color == "black" else "T", "treatmentColor": color, "openingId": "development-0003"}

    def test_replay_identity_maps_color_independent_of_chronological_leg(self) -> None:
        row = self.row("white", 0)
        raw = {**row, "moves": [], "plies": 0, "rawEnd": "repetition-draw", "rawWinnerColor": None, "freshInstances": True, "perInstanceNetworks": True}
        adapted, mapping = replay.adapt(row, raw)
        self.assertEqual((adapted["globalGameIndex"], adapted["leg"], mapping["f1Leg"]), (7, 1, 0))
        self.assertEqual((adapted["blackEngine"], adapted["whiteEngine"], adapted["rawResult"]), ("B", "A", "draw"))
        raw["moves"] = ["7g7f"]
        raw["plies"] = 1
        raw["rawWinnerColor"] = "black"
        adapted, _ = replay.adapt(row, raw)
        self.assertEqual(adapted["rawResult"], "b-win")

    def test_replay_fails_on_mutated_identity(self) -> None:
        row = self.row("black", 1)
        raw = {**row, "moves": [], "plies": 0, "rawEnd": "move-limit-reached", "rawWinnerColor": None, "freshInstances": True, "perInstanceNetworks": True}
        for change in ({"sfen": "changed"}, {"freshInstances": False}, {"plies": 1}, {"blackEngine": "C"}):
            with self.subTest(change=change), self.assertRaises(f1.F1Error):
                replay.adapt(row, {**raw, **change})

    def test_fixed_proposal_seed_and_conditional_replacement_schedule(self) -> None:
        self.assertEqual(openings.seed("development", 0), openings.seed("development", 0))
        self.assertNotEqual(openings.seed("development", 0), openings.seed("holdout", 0))
        rows = [{"openingId": f"development-{i:04d}", "sfen": "9/9/9/9/9/9/9/9/9 b - 1", "canonicalFamily": "same"} for i in range(128)]
        scheduled = openings._schedule("development", rows)
        self.assertEqual(len(scheduled), 512)
        self.assertEqual([item["matchup"] for item in scheduled[:4]], ["T-C", "T-C", "T-I", "T-I"])
        self.assertEqual([item["matchup"] for item in scheduled[4:8]], ["T-I", "T-I", "T-C", "T-C"])
        self.assertEqual({item["canonicalFamily"] for item in scheduled}, {"same"})

    def test_paid_gate_blocks_without_package_or_grants(self) -> None:
        with self.assertRaises(f1.F1Error):
            train.verify_paid_gates(Path("/does-not-exist/f1-gate-fixture"), arm="C", resume=False)
        with self.assertRaises(f1.F1Error):
            cuda_fixture._grant(Path("/does-not-exist/f1-gate-fixture"), "C", 0, "partial")

    def test_live_trace_requires_full_history_and_actual_timer(self) -> None:
        row = self.row("black", 0)
        row["sfen"] = "9/9/9/9/9/9/9/9/9 b - 1"
        trace = {"gameId": row["gameId"], "moveIndex": 0, "ply": 1, "color": "black", "engine": "T", "positionCommand": f"position sfen {row['sfen']}", "command": "go movetime 1000", "requestedMs": 1000, "elapsedMs": 1000.0, "reportedTimeMs": 1000, "bestMove": "7g7f", "terminalReason": None, "rootResultSchema": "haitaka-search-root-result-v1", "playMoveWasSearched": True, "missingMove": False, "emergencyFallbackUsed": False, "unsearchedMove": False}
        raw = {"moves": ["7g7f"], "plies": 1, "searches": [trace], "rawEnd": "move-limit-reached", "freshInstances": True, "perInstanceNetworks": True, "modelDescriptions": {"T": "model", "opponent": "model"}, "handshake": {"T": {"hash": [], "ready": ["readyok"]}, "opponent": {"hash": [], "ready": ["readyok"]}}}
        live._strict_searches(row, raw)
        for change in ({"positionCommand": "position startpos"}, {"reportedTimeMs": None}, {"requestedMs": 999}):
            with self.subTest(change=change), self.assertRaises(f1.F1Error):
                live._strict_searches(row, {**raw, "searches": [{**trace, **change}]})

    def test_temperature_and_huber_selector_boundaries(self) -> None:
        self.assertEqual(f1.calibrate([0, -10, 20])["medianNonzeroAbsoluteScore"], 15)
        for bad in ([], [0], [math.inf], [math.nan]):
            with self.subTest(bad=bad), self.assertRaises(f1.F1Error):
                f1.calibrate(bad)
        dummy = {"path": "dummy", "bytes": 0, "sha256": "0" * 64}
        candidates = [{"update": update, "validationHuber": 1.0, "checkpoint": dummy, "export": dummy} for update in f1.UPDATES]
        candidates[3]["validationHuber"] = 0.2 + 0.5e-12
        candidates[4]["validationHuber"] = 0.2
        self.assertEqual(f1.select_by_common_huber(candidates, verify_files=False)["selectedUpdate"], 512)


if __name__ == "__main__":
    unittest.main()
