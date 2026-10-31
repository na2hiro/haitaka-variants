"""Statistical boundary cases and tamper detection for the strength audit."""
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('strength', ROOT / 'scripts/tuyouraou11-strength-summary.py')
strength = importlib.util.module_from_spec(spec)
spec.loader.exec_module(strength)


class StrengthAuditTests(unittest.TestCase):
    def test_all_wins_and_draws(self):
        self.assertEqual(strength.interval([[1] * 50]), [1, 1])
        self.assertEqual(strength.interval([[.5] * 50]), [.5, .5])

    def test_opposite_time_strata_stay_balanced(self):
        self.assertEqual(strength.interval([[1] * 50, [0] * 50]), [.5, .5])

    def test_shared_opening_blocks_preserve_correlation(self):
        first = [1] * 25 + [0] * 25
        second = [1 - x for x in first]
        self.assertEqual(strength.interval([first, second], shared=True), [.5, .5])
        independent = strength.interval([first, second])
        self.assertLess(independent[0], .5)
        self.assertGreater(independent[1], .5)

    def test_changed_game_move_is_rejected_against_browser_transcript(self):
        protocol = json.loads((strength.META / 'p6-strength-protocol.json').read_text())
        openings = (strength.META / 'p4-openings.sfen').read_text().splitlines()
        source = ROOT / 'plans/tuyouraou11/p6-strength-results/continuation/chromium'
        with tempfile.TemporaryDirectory() as temp:
            target = Path(temp)
            shutil.copytree(source, target / 'chromium')
            strength.audit_browser(target, 'chromium', protocol, openings[16:], False, 'continuation')
            path = target / 'chromium/match-1000/self-play-games.jsonl'
            rows = [json.loads(line) for line in path.read_text().splitlines()]
            rows[0]['moves'][0] = '1a1a'
            path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
            with self.assertRaises(AssertionError):
                strength.audit_browser(target, 'chromium', protocol, openings[16:], False, 'continuation')


if __name__ == '__main__':
    unittest.main()
