"""F1 snapshot recovery identity without importing torch or starting training."""

import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import strength_first_f1_train as trainer
import strength_first_f1_core as f1


class SnapshotIdentityTests(unittest.TestCase):
    def setUp(self):
        self.checkpoint = {"path": "checkpoints/u000128.pt", "bytes": 123, "sha256": "a" * 64}
        self.state = {
            "model": {"weight": [1, 2]}, "optimizer": {"momentum": [3, 4]},
            "rng": {"state": [5, 6]}, "cursor": {"epoch": 0, "record": 128},
            "update": 128, "examplesSeen": 128, "configSha256": "b" * 64,
            "grants": {"execution": "grant"}, "trace": [{"learningRate": 0.1}],
            "validations": [{"validationHuber": 0.2}],
            "candidates": [{"update": 128, "validationHuber": 0.2,
                "checkpoint": self.checkpoint,
                "export": {"path": "exports/u000128.nnue", "bytes": 456, "sha256": "c" * 64}}],
        }

    def test_replay_after_snapshot_before_last_state_matches_fresh_identity(self):
        replayed = copy.deepcopy(self.state)
        replayed["candidates"][0]["checkpoint"]["recoveredOrphan"] = True
        self.assertNotEqual(self.state, replayed)
        self.assertEqual(trainer.snapshot_identity(self.state), trainer.snapshot_identity(replayed))

    def test_preexisting_recovery_flags_on_either_snapshot_are_ignored(self):
        left = copy.deepcopy(self.state)
        right = copy.deepcopy(self.state)
        left["candidates"][0]["checkpoint"]["recoveredOrphan"] = True
        right["candidates"][0]["checkpoint"]["recoveredOrphan"] = False
        self.assertEqual(trainer.snapshot_identity(left), trainer.snapshot_identity(right))

    def test_every_retained_candidate_is_normalized_without_mutating_snapshots(self):
        replayed = copy.deepcopy(self.state)
        replayed["candidates"].append(copy.deepcopy(replayed["candidates"][0]))
        expected = copy.deepcopy(replayed)
        for candidate in replayed["candidates"]:
            candidate["checkpoint"]["recoveredOrphan"] = True
        original = copy.deepcopy(replayed)
        normalized = trainer.snapshot_identity(replayed)
        self.assertEqual(normalized, expected)
        self.assertEqual(replayed, original)
        self.assertIs(normalized["model"], replayed["model"])
        self.assertIs(normalized["optimizer"], replayed["optimizer"])
        self.assertIs(normalized["rng"], replayed["rng"])

    def test_training_state_differences_still_conflict(self):
        for key in ("model", "optimizer", "rng", "cursor", "update", "examplesSeen",
                    "configSha256", "grants", "trace", "validations"):
            with self.subTest(key=key):
                changed = copy.deepcopy(self.state)
                changed[key] = "different"
                self.assertNotEqual(trainer.snapshot_identity(self.state), trainer.snapshot_identity(changed))

    def test_artifact_and_validation_differences_still_conflict(self):
        for kind in ("checkpoint", "export"):
            for key, value in (("path", "other"), ("bytes", 999), ("sha256", "d" * 64)):
                with self.subTest(kind=kind, key=key):
                    changed = copy.deepcopy(self.state)
                    changed["candidates"][0][kind][key] = value
                    self.assertNotEqual(trainer.snapshot_identity(self.state), trainer.snapshot_identity(changed))
        changed = copy.deepcopy(self.state)
        changed["candidates"][0]["validationHuber"] = 0.9
        self.assertNotEqual(trainer.snapshot_identity(self.state), trainer.snapshot_identity(changed))

    def test_recovery_named_fields_elsewhere_are_not_ignored(self):
        for key in ("model", "optimizer", "rng"):
            with self.subTest(key=key):
                changed = copy.deepcopy(self.state)
                changed[key]["recoveredOrphan"] = True
                self.assertNotEqual(trainer.snapshot_identity(self.state), trainer.snapshot_identity(changed))
        changed = copy.deepcopy(self.state)
        changed["candidates"][0]["checkpoint"]["otherMetadata"] = True
        self.assertNotEqual(trainer.snapshot_identity(self.state), trainer.snapshot_identity(changed))


class CandidateArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        patch = mock.patch.object(f1, "ROOT", self.root)
        patch.start()
        self.addCleanup(patch.stop)
        self.candidates = []
        for update in f1.UPDATES:
            checkpoint = self.root / f"u{update:06d}.pt"
            network = self.root / f"u{update:06d}.nnue"
            checkpoint.write_bytes(f"checkpoint-{update}".encode())
            network.write_bytes(f"network-{update}".encode())
            self.candidates.append({"update": update, "validationHuber": 0.1 if update == 512 else 1.0,
                "checkpoint": {**f1.artifact(checkpoint), "recoveredOrphan": True},
                "export": f1.artifact(network)})

    def normalized(self):
        return [{**candidate, "checkpoint": trainer.checkpoint_reference(candidate["checkpoint"])}
                for candidate in self.candidates]

    def test_recovered_candidates_round_trip_through_run_json_and_real_selection(self):
        with self.assertRaisesRegex(f1.F1Error, "artifact bytes/hash changed"):
            f1.select_by_common_huber(self.candidates)
        run_path = self.root / "run.json"
        f1.immutable_json(run_path, {"candidates": self.normalized()})
        candidates = json.loads(run_path.read_text())["candidates"]
        for candidate in candidates:
            self.assertEqual(set(candidate["checkpoint"]), {"path", "bytes", "sha256"})
        self.assertEqual(f1.select_by_common_huber(candidates)["selectedUpdate"], 512)
        self.assertTrue(all(candidate["checkpoint"]["recoveredOrphan"] for candidate in self.candidates))

    def test_checkpoint_identity_is_stable_for_fresh_and_recovered_references(self):
        expected = f1.artifact(self.root / "u000128.pt")
        for metadata in ({}, {"recoveredOrphan": True}, {"recoveredOrphan": False}):
            self.assertEqual(trainer.checkpoint_reference({**expected, **metadata}), expected)

    def test_normalization_does_not_hide_artifact_identity_conflicts(self):
        for field, value in (("path", "missing.pt"), ("bytes", 999), ("sha256", "0" * 64)):
            with self.subTest(field=field):
                candidates = self.normalized()
                candidates[0]["checkpoint"][field] = value
                with self.assertRaises(f1.F1Error):
                    f1.select_by_common_huber(candidates)
        candidates = self.normalized()
        path = self.root / candidates[0]["checkpoint"]["path"]
        original = path.read_bytes()
        path.write_bytes(b"x" * len(original))
        with self.assertRaisesRegex(f1.F1Error, "artifact bytes/hash changed"):
            f1.select_by_common_huber(candidates)


if __name__ == "__main__":
    unittest.main()
