"""Offline analysis must accept the same provisional-cap evidence as live play."""

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import strength_first_f1_analysis as analysis
import strength_first_f1_core as f1
import strength_first_f1_live as live


class CanonicalResultTests(unittest.TestCase):
    def test_live_and_offline_share_cap_and_final_ply_terminal_semantics(self):
        raw = {"rawEnd": "move-limit-reached", "rawWinnerColor": None, "plies": 200}
        for reason, winner, color, expected in (
            ("maximum-ply-capped", None, None, (0.5, True)),
            ("no-legal-move", "A", "black", (1, False)),
            ("king-captured", "B", "white", (0, False)),
            ("perpetual-check-loss", "B", "white", (0, False)),
            ("repetition-draw", None, None, (0.5, False)),
            ("jishogi-draw", None, None, (0.5, False)),
        ):
            with self.subTest(reason=reason):
                replay = {"terminationReason": reason, "winner": winner, "winnerColor": color}
                self.assertEqual(f1.canonical_replay_result(raw, replay), expected)
                self.assertEqual(live._canonical_replay_result(raw, replay), expected)

    def test_inconsistent_noncap_results_and_provisional_caps_still_fail(self):
        raw = {"rawEnd": "move-limit-reached", "rawWinnerColor": None, "plies": 200}
        replay = {"terminationReason": "maximum-ply-capped", "winner": None, "winnerColor": None}
        cases = [
            ({**raw, "plies": 199}, replay),
            ({**raw, "rawWinnerColor": "black"}, replay),
            (raw, {**replay, "winner": "A", "winnerColor": "black"}),
            (raw, {**replay, "terminationReason": "unknown"}),
            ({**raw, "rawEnd": "repetition-draw"}, replay),
            ({**raw, "rawEnd": "no-legal-move", "rawWinnerColor": "black"},
             {**replay, "terminationReason": "no-legal-move", "winnerColor": "white", "winner": "B"}),
        ]
        for game, result in cases:
            with self.subTest(game=game, result=result), self.assertRaises(f1.F1Error):
                f1.canonical_replay_result(game, result)


class PhaseAnalysisTests(unittest.TestCase):
    def fixture(self, root, phase, reason, winner):
        """Provide a complete frozen schedule and search-free replay receipts."""
        expected = 512 if phase == "development" else 1024
        block_size = 32 if phase == "development" else 16
        matchups = ("T-C", "T-I") if phase == "development" else ("T-I",)
        rows = []
        for matchup in matchups:
            for pair in range(128 if phase == "development" else 512):
                for leg in (0, 1):
                    rows.append({"gameId": f"{phase}-{matchup}-{pair}-{leg}", "leg": leg,
                        "matchup": matchup, "pairIndex": pair, "sfen": "fixture b - 1",
                        "treatmentColor": "black" if leg == 0 else "white"})
        self.assertEqual(len(rows), expected)
        binding = lambda name: {"path": str(root / name)}
        schedule = root / f"openings/{phase}-schedule.json"
        schedule.parent.mkdir(parents=True, exist_ok=True)
        schedule.write_text(json.dumps({"rows": rows}))
        docs = {schedule: {"rows": rows}, root / "runtime/freeze.json": {"package": binding("package"), "models": {}},
            root / "analysis/development.json": {"screenAdvances": True}}
        mapping = {"rows": []}
        replays = []
        games = []
        for index, row in enumerate(rows):
            game_id = row["gameId"]
            raw_path = root / f"raw/{game_id}.json"
            docs[raw_path] = {"gameId": game_id, "moves": ["7g7f"] * 200, "plies": 200,
                "rawEnd": "move-limit-reached", "rawWinnerColor": None}
            winner_color = None if winner is None else row["treatmentColor"] if winner == "A" else ("white" if row["treatmentColor"] == "black" else "black")
            replays.append({"globalGameIndex": index, "status": "passed", "searchDispatched": False,
                "modelLoaded": False, "browserLaunched": False, "checks": {"legal": True},
                "terminationReason": reason, "winner": winner, "winnerColor": winner_color, "replayedPlies": 200})
            mapping["rows"].append({"f1GameId": game_id, "replayGameIndex": index, "f1Leg": row["leg"], "matchup": row["matchup"]})
            games.append({"gameId": game_id, "schedule": row, "valid": True,
                "scoreTreatment": 0.5 if winner is None else 1 if winner == "A" else 0,
                "genuinePlyCap": reason == "maximum-ply-capped", "raw": {"path": str(raw_path)},
                "replay": {"searchFree": True, "legal": True, "mapping": binding("mapping.json"),
                    "receipt": binding("replay.jsonl"), "replayGameIndex": index}})
        docs[root / "mapping.json"] = mapping
        (root / "replay.jsonl").write_text("".join(json.dumps(row) + "\n" for row in replays))
        for block in range(expected // block_size):
            docs[root / f"games/{phase}/block-{block:02d}/accepted.json"] = {
                "schema": "haitaka-anhoku-strength-first-f1-accepted-block-v1", "phase": phase,
                "blockIndex": block, "scheduleSha256": f1.sha256(schedule),
                "runtimeFreeze": binding("runtime/freeze.json"), "parts": [], "wallSeconds": 1,
                "games": games[block * block_size:(block + 1) * block_size]}
        return docs, games

    def run_phase(self, root, phase, docs):
        # Artifact validation is independently covered; isolate replay acceptance here.
        with mock.patch.object(analysis, "_load", side_effect=lambda path: docs[path]), \
             mock.patch.object(f1, "verify_artifact", side_effect=lambda value: Path(value["path"])), \
             mock.patch.object(f1, "artifact", side_effect=lambda path: {"path": str(path)}):
            return analysis._phase(root, phase)

    def test_development_and_holdout_accept_caps_and_final_ply_terminals(self):
        for phase in ("development", "holdout"):
            for reason, winner in (("maximum-ply-capped", None), ("no-legal-move", "A"),
                                   ("king-captured", "B"), ("repetition-draw", None)):
                with self.subTest(phase=phase, reason=reason), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    docs, games = self.fixture(root, phase, reason, winner)
                    result = self.run_phase(root, phase, docs)
                    self.assertEqual(result["games"], len(games))
                    for stats in result["matchups"].values():
                        self.assertEqual(stats["caps"], stats["games"] if reason == "maximum-ply-capped" else 0)
                        expected = 0.5 if winner is None else 1 if winner == "A" else 0
                        self.assertEqual(stats["ordinaryScore"], expected)
                        self.assertEqual(stats["conservativeScore"], 0 if reason == "maximum-ply-capped" else expected)

    def test_phase_rejects_wrong_score_cap_flag_and_replayed_length(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            docs, games = self.fixture(root, "development", "no-legal-move", "A")
            for key, value in (("scoreTreatment", 0.5), ("genuinePlyCap", True)):
                with self.subTest(key=key):
                    original = games[0][key]
                    games[0][key] = value
                    with self.assertRaisesRegex(f1.F1Error, "game score differs"):
                        self.run_phase(root, "development", docs)
                    games[0][key] = original
            replay_path = root / "replay.jsonl"
            replays = [json.loads(line) for line in replay_path.read_text().splitlines()]
            replays[0]["replayedPlies"] = 199
            replay_path.write_text("".join(json.dumps(row) + "\n" for row in replays))
            with self.assertRaisesRegex(f1.F1Error, "ply count differs"):
                self.run_phase(root, "development", docs)


if __name__ == "__main__":
    unittest.main()
