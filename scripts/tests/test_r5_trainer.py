"""CPU-only regression tests for R5 metrics and export provenance."""

import hashlib
import json
import math
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import r5_trainer as runner
import r5_eval_models as model_evaluation


class MetricTests(unittest.TestCase):
    def test_spearman_uses_average_ranks_for_ties_in_both_arrays(self):
        tied = np.array([1.0, 1.0, 3.0])
        ordered = np.array([1.0, 2.0, 3.0])
        expected = math.sqrt(3) / 2
        for predictions, targets in [(tied, ordered), (ordered, tied)]:
            result = runner.metrics_from_predictions(predictions, targets)
            self.assertAlmostEqual(result["spearman"], expected)
            permutation = [1, 0, 2]
            permuted = runner.metrics_from_predictions(predictions[permutation], targets[permutation])
            self.assertAlmostEqual(permuted["spearman"], expected)

    def test_constant_values_have_no_rank_correlation(self):
        result = runner.metrics_from_predictions(np.ones(3), np.arange(3.0))
        self.assertIsNone(result["spearman"])


class InputIdentityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        patch = mock.patch.object(runner, "ROOT", self.root)
        patch.start()
        self.addCleanup(patch.stop)
        self.payload = b"a" * runner.ENTRY_BYTES
        self.splits = {}
        for name in ("training", "validation", "development", "static"):
            path = self.root / f"{name}.bin"
            path.write_bytes(self.payload)
            self.splits[name] = {"records": 1, "binary": {
                "path": path.name, "bytes": len(self.payload), "sha256": runner.sha256(path),
            }}
        self.contract = {"optimization": {}, "pilot": {"splits": self.splits}}
        self.lane = {"id": "static", "kind": "static-clone-full-precision",
                     "staticClone": self.splits["static"]}
        self.contract["initialization"] = {"lanes": [{"kind": "scratch"}, self.lane]}

    def test_same_sized_replacements_fail_before_any_evaluation_or_update(self):
        for name, split in self.splits.items():
            with self.subTest(split=name):
                path = self.root / split["binary"]["path"]
                path.write_bytes(b"b" * len(self.payload))
                torch = mock.Mock()
                features = mock.Mock()
                output = self.root / "recipe"
                with mock.patch.object(runner, "evaluate") as evaluate:
                    with self.assertRaisesRegex(runner.R5Error, "binary hash"):
                        runner.run_lane(torch, mock.Mock(), features, mock.Mock(),
                            self.contract, self.lane, 0.001, 7, {}, "cpu", output)
                self.assertEqual(torch.mock_calls, [])
                features.get_feature_set_from_name.assert_not_called()
                evaluate.assert_not_called()
                self.assertFalse(output.exists())
                path.write_bytes(self.payload)

    def test_model_evaluation_reverifies_every_consumed_binary(self):
        deep = self.splits["training"]
        for name, split in self.splits.items():
            with self.subTest(split=name):
                path = self.root / split["binary"]["path"]
                path.write_bytes(b"b" * len(self.payload))
                with self.assertRaisesRegex(runner.R5Error, "binary hash"):
                    model_evaluation.verified_evaluation_splits(self.contract, deep)
                path.write_bytes(self.payload)

    def test_static_clone_direct_entry_checks_the_consumed_binary(self):
        static = self.splits["static"]
        (self.root / static["binary"]["path"]).write_bytes(b"b" * len(self.payload))
        model = mock.Mock()
        with self.assertRaisesRegex(runner.R5Error, "binary hash"):
            runner.train_static_clone(mock.Mock(), mock.Mock(), mock.Mock(), model,
                mock.Mock(), static, "cpu", self.root / "static-output", 0.001)
        model.train.assert_not_called()

    def test_matching_binary_identity_is_accepted(self):
        split = self.splits["training"]
        path = runner.verified_binary(split["binary"], split["records"], "training")
        self.assertEqual(path.read_bytes(), self.payload)


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.output = self.root / "recipe"
        (self.output / "checkpoints").mkdir(parents=True)
        self.checkpoint = self.output / "checkpoints" / "last.pt"
        self.checkpoint.write_bytes(b"final evaluated model")
        self.width = {"l1": 256, "l2": 16, "l3": 32}
        self.row = {
            "lane": "scratch", "width": self.width,
            "learningRateCandidate": 0.001, "seed": 7,
            "optimizerUpdates": 10, "examplesSeen": 163840,
            "equivalentPasses": 0.5,
            "validation": {"teacher_only_mae": 0.1},
            "development": {"teacher_only_mae": 0.2},
            "checkpoint": {"sha256": hashlib.sha256(self.checkpoint.read_bytes()).hexdigest()},
        }
        runner.record_training_completion(self.output, self.checkpoint, "development-plateau",
                                          self.row["optimizerUpdates"], self.row["examplesSeen"])
        self.contract = {
            "initialization": {"lanes": [{"id": "scratch", "kind": "scratch"}]},
            "optimization": {"trainRecordsPerPass": 327680,
                             "validationRecords": 4, "developmentRecords": 4},
        }
        self.torch = mock.Mock()
        self.torch.save.side_effect = lambda model, path: Path(path).write_bytes(b"CPU model")
        self.root_patch = mock.patch.object(runner, "ROOT", self.root)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)

    def recover(self, path=None):
        (self.output / "checkpoint-metrics.jsonl").write_text(json.dumps(self.row) + "\n")
        return runner.recover_existing_export(
            self.torch, self.contract, path or self.checkpoint,
            self.output, self.root, self.width,
        )

    def test_different_checkpoint_is_rejected_before_loading_or_exporting(self):
        older = self.output / "checkpoints" / "older.pt"
        older.write_bytes(b"older model")
        with mock.patch.object(runner, "run") as export:
            with self.assertRaisesRegex(runner.R5Error, "checkpoint hash"):
                self.recover(older)
        self.torch.load.assert_not_called()
        export.assert_not_called()
        self.assertFalse((self.output / "run.json").exists())

    def test_missing_checkpoint_identity_is_rejected(self):
        self.row.pop("checkpoint")
        with self.assertRaisesRegex(runner.R5Error, "checkpoint hash"):
            self.recover()
        self.torch.load.assert_not_called()

    def test_width_mismatch_is_rejected_before_loading(self):
        self.row["width"] = {"l1": 512, "l2": 16, "l3": 32}
        with self.assertRaisesRegex(runner.R5Error, "width"):
            self.recover()
        self.torch.load.assert_not_called()

    def test_missing_training_completion_fails_before_loading(self):
        (self.output / "training-complete.json").unlink()
        with self.assertRaisesRegex(runner.R5Error, "training-complete.json"):
            self.recover()
        self.torch.load.assert_not_called()

    def test_completion_must_bind_the_same_checkpoint_and_counters(self):
        path = self.output / "training-complete.json"
        original = json.loads(path.read_text())
        for key, value in [("checkpointSha256", "0" * 64), ("optimizerUpdates", 99),
                           ("examplesSeen", 99)]:
            with self.subTest(field=key):
                changed = {**original, key: value}
                path.write_text(json.dumps(changed))
                with self.assertRaisesRegex(runner.R5Error, "completion record"):
                    self.recover()
                self.torch.load.assert_not_called()

    def test_recovery_retains_maximum_updates_reason(self):
        runner.record_training_completion(self.output, self.checkpoint, "maximum-updates",
                                          self.row["optimizerUpdates"], self.row["examplesSeen"])
        with mock.patch.object(runner, "run", side_effect=lambda *args:
                               (self.output / "final.nnue").write_bytes(b"serialized model")):
            result = self.recover()
        self.assertEqual(result["stopReason"], "maximum-updates")
        self.assertEqual(result["recovery"], "export-only-from-completed-checkpoint")

    def test_matching_checkpoint_exports_its_own_final_metrics(self):
        def export(*args):
            (self.output / "final.nnue").write_bytes(b"serialized model")
        with mock.patch.object(runner, "run", side_effect=export):
            result = self.recover()
        self.torch.load.assert_called_once_with(self.checkpoint, map_location="cpu", weights_only=False)
        self.assertEqual(result["stopReason"], "development-plateau")
        self.assertEqual(result["recovery"], "export-only-from-completed-checkpoint")
        self.assertEqual(result["finalValidation"], self.row["validation"])
        self.assertEqual(result["finalDevelopment"], self.row["development"])
        self.assertEqual(result["examplesSeen"], self.row["examplesSeen"])
        self.assertEqual(result["optimizerUpdates"], self.row["optimizerUpdates"])
        self.assertTrue((self.output / "run.json").is_file())


if __name__ == "__main__":
    unittest.main()
