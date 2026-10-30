"""F1 snapshot recovery identity without importing torch or starting training."""

import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import strength_first_f1_train as trainer


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


if __name__ == "__main__":
    unittest.main()
