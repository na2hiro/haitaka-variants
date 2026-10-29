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

    def test_matching_checkpoint_exports_its_own_final_metrics(self):
        def export(*args):
            (self.output / "final.nnue").write_bytes(b"serialized model")
        with mock.patch.object(runner, "run", side_effect=export):
            result = self.recover()
        self.torch.load.assert_called_once_with(self.checkpoint, map_location="cpu", weights_only=False)
        self.assertEqual(result["finalValidation"], self.row["validation"])
        self.assertEqual(result["finalDevelopment"], self.row["development"])
        self.assertEqual(result["examplesSeen"], self.row["examplesSeen"])
        self.assertEqual(result["optimizerUpdates"], self.row["optimizerUpdates"])
        self.assertTrue((self.output / "run.json").is_file())


if __name__ == "__main__":
    unittest.main()
