"""CPU-only crash-recovery tests; no torch, trainer checkout or GPU required."""

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import strength_first_step12_trainer_backend as backend


def encode(row):
    return (json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n").encode()


class MetricsRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / "metrics.jsonl"
        self.first = {"optimizerUpdates": 128, "loss": 0.3}
        self.last = {"optimizerUpdates": 256, "loss": 0.2}
        self.state = {"optimizerUpdates": 256, "lastMetricsRow": self.last}

    def recover(self, original):
        if original is not None:
            self.path.write_bytes(original)
        backend.reconcile_metrics_journal(self.path, self.state)
        recovered = self.path.read_bytes()
        backend.reconcile_metrics_journal(self.path, self.state)
        self.assertEqual(self.path.read_bytes(), recovered, "recovery must be idempotent")
        # The next normal append must remain a separate, parseable JSON row.
        next_row = {"optimizerUpdates": 384, "loss": 0.1}
        with self.path.open("ab") as stream:
            stream.write(encode(next_row))
        rows = [json.loads(line) for line in self.path.read_bytes().splitlines()]
        self.assertEqual(rows[-2:], [self.last, next_row])
        return recovered

    def test_checkpoint_saved_before_append_restores_missing_row(self):
        recovered = self.recover(encode(self.first))
        self.assertEqual(recovered, encode(self.first) + encode(self.last))

    def test_missing_or_empty_journal_restores_committed_row(self):
        for original in (None, b""):
            with self.subTest(original=original):
                self.path.unlink(missing_ok=True)
                self.assertEqual(self.recover(original), encode(self.last))

    def test_every_interrupted_append_prefix_is_recoverable(self):
        # Cover invalid JSON, complete JSON without its newline, and complete append.
        payload = encode(self.last)
        for length in range(len(payload) + 1):
            with self.subTest(length=length):
                recovered = self.recover(encode(self.first) + payload[:length])
                self.assertEqual(recovered, encode(self.first) + payload)

    def test_complete_committed_journal_is_not_replaced(self):
        original = encode(self.first) + encode(self.last)
        self.path.write_bytes(original)
        with mock.patch.object(backend.os, "replace") as replace:
            backend.reconcile_metrics_journal(self.path, self.state)
        replace.assert_not_called()
        self.assertEqual(self.path.read_bytes(), original)

    def test_conflicts_ahead_rows_duplicates_and_interior_corruption_fail_closed(self):
        bad_journals = [
            encode(self.first) + encode({**self.last, "loss": 9}),
            encode(self.last) + encode({"optimizerUpdates": 384}),
            encode(self.first) + encode(self.first),
            encode(self.first) + b'{"optimizerUpdates":\n' + encode(self.last),
            encode(self.first) + b'{"optimizerUpdates":\n',
            encode(self.first) + b'{}\n',
        ]
        for original in bad_journals:
            with self.subTest(original=original):
                self.path.write_bytes(original)
                with self.assertRaises(backend.Step12BackendError):
                    backend.reconcile_metrics_journal(self.path, self.state)
                self.assertEqual(self.path.read_bytes(), original)

    def test_state_and_last_row_update_must_match(self):
        with self.assertRaisesRegex(backend.Step12BackendError, "resume update"):
            backend.reconcile_metrics_journal(self.path, {**self.state, "optimizerUpdates": 384})
        self.assertFalse(self.path.exists())

    def test_interrupted_replacement_preserves_old_journal_for_next_resume(self):
        original = encode(self.first) + encode(self.last)[:10]
        self.path.write_bytes(original)
        with mock.patch.object(backend.os, "replace", side_effect=OSError("simulated interruption")):
            with self.assertRaises(OSError):
                backend.reconcile_metrics_journal(self.path, self.state)
        self.assertEqual(self.path.read_bytes(), original)
        self.assertEqual(list(self.path.parent.glob("*.tmp")), [])
        self.assertEqual(self.recover(original), encode(self.first) + encode(self.last))


if __name__ == "__main__":
    unittest.main()
