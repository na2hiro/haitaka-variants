"""Fixed F1 dispatch budgets require a matching campaign grant."""

from pathlib import Path
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import strength_first_f1_core as f1
import strength_first_f1_live as live


class CampaignGrantTests(unittest.TestCase):
    def check_grant(self, hours, games, *, qualification):
        root = Path("/fixture/f1")
        runtime = {"host": {"name": "fixture-host"}}
        grant = {"schema": "haitaka-anhoku-strength-first-f1-match-grant-v1",
                 "experimentId": f1.EXPERIMENT_ID, "authorized": True,
                 "qualificationAuthorized": True, "scoredGamesAuthorized": True,
                 "userReference": "fixture-user-grant", "runtimeSha256": "fixture-hash",
                 "packageSha256": "fixture-hash", "setupLedgerSha256": "fixture-hash",
                 "validUntilUtc": "2099-01-01T00:00:00Z", "host": "fixture-host",
                 "oneHost": True, "maximumHostHours": hours, "maximumScoredGames": games}
        documents = {root / "authorization/match.json": grant,
                     root / "runtime/qualification.json": {"passed": True, "runtimeSha256": "fixture-hash"}}
        with mock.patch.object(live, "_runtime", return_value=runtime), \
             mock.patch.object(live, "_load", side_effect=lambda path: documents[path]), \
             mock.patch.object(live, "_spent_seconds", return_value=0), \
             mock.patch.object(f1, "sha256", return_value="fixture-hash"), \
             mock.patch.object(f1, "artifact", return_value={"path": "fixture"}):
            if qualification:
                return live._qualification_grant(root)
            return live._grant(root, "development")

    def test_full_campaign_grant_allows_qualification_and_scored_admission(self):
        for qualification in (False, True):
            with self.subTest(qualification=qualification):
                self.check_grant(32, 1536, qualification=qualification)

    def test_smaller_grants_cannot_authorize_fixed_budget_dispatch(self):
        for qualification in (False, True):
            for hours, games in ((16, 1536), (32, 512), (1, 1), (0, 0), (-1, -1)):
                with self.subTest(qualification=qualification, hours=hours, games=games):
                    with self.assertRaises(f1.F1Error):
                        self.check_grant(hours, games, qualification=qualification)

    def test_larger_or_malformed_limits_are_rejected(self):
        for qualification in (False, True):
            for hours, games in ((33, 1536), (32, 1537), (None, 1536),
                                 (32, None), ("32", 1536), (32, 1536.0), (True, True)):
                with self.subTest(qualification=qualification, hours=hours, games=games):
                    with self.assertRaises(f1.F1Error):
                        self.check_grant(hours, games, qualification=qualification)


if __name__ == "__main__":
    unittest.main()
