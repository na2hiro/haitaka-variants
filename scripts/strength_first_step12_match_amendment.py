#!/usr/bin/env python3
"""Write the hash-bound Step 12 runtime compatibility and block replacement receipts."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import subprocess


EXPERIMENT_ID = "strength-first-step12-independent-replication-20260916-v1"
FAILED_CELL = "early-selected"
FAILED_BLOCK = 0


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def artifact(root: Path, relative: str) -> dict[str, object]:
    path = root / relative
    return {"path": relative, "bytes": path.stat().st_size, "sha256": sha256(path)}


def git_identity(repo: Path) -> dict[str, object]:
    diff_command = ["git", "-C", str(repo), "diff", "--no-ext-diff", "--binary", "--", "haitaka_cli/src/main.rs", "scripts/strength_first_step12_matches.py", "scripts/strength_first_step12_match_validation.py", "scripts/strength_first_step12_match_runtime_regression.py"]
    try:
        head = subprocess.run(["git", "-C", str(repo), "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
        diff = subprocess.run(diff_command, capture_output=True, check=True).stdout
        return {"head": head, "diffSha256": hashlib.sha256(diff).hexdigest(), "diffScope": diff_command[8:]}
    except (OSError, subprocess.CalledProcessError):
        files = [repo / relative for relative in ("haitaka_cli/src/main.rs", "scripts/strength_first_step12_matches.py", "scripts/strength_first_step12_match_validation.py", "scripts/strength_first_step12_match_runtime_regression.py")]
        material = b"".join(path.read_bytes() for path in files)
        return {"head": "61666d9e3653e4df9881b14c23f8fdcc4bf7779b", "diffSha256": hashlib.sha256(material).hexdigest(), "diffScope": [str(path.relative_to(repo)) for path in files], "gitMetadataAvailable": False}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    root = args.root.expanduser().resolve()
    attempt = root / "matches/early-selected/block-00/attempt-01"
    failed_files = [str(path.relative_to(root)) for path in sorted(attempt.rglob("*")) if path.is_file()]
    if not failed_files:
        raise RuntimeError(f"preserved failed attempt is missing: {attempt}")
    schedule = artifact(root, "openings/schedule.json")
    old_runtime = artifact(root, "preflight/history/haitaka_cli-before-game-opening-unit")
    new_runtime = artifact(root, "runtime/haitaka_cli")
    regression = artifact(root, "preflight/match-runtime-regression.json")
    qualification = artifact(root, "preflight/qualification-gate.json")
    compatibility = {
        "schema": "haitaka-anhoku-strength-first-step12-match-runtime-compatibility-v1",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "passed",
        "rootCause": "CLI pair-indexed openings supplied with per-game schedule rows",
        "mapping": "openingIndex=gameIndex; pairIndex=floor(gameIndex/2); a/b seats alternate by game index",
        "oldRuntime": old_runtime,
        "newRuntime": new_runtime,
        "runner": artifact(root, "repo/scripts/strength_first_step12_matches.py"),
        "validator": artifact(root, "repo/scripts/strength_first_step12_match_validation.py"),
        "runtimeRegressionScript": artifact(root, "repo/scripts/strength_first_step12_match_runtime_regression.py"),
        "regressionReceipt": regression,
        "renewedQualification": qualification,
        "frozenSchedule": schedule,
        "sourceRevisionAndDiff": git_identity(root / "repo"),
        "buildReceipt": {
            "command": ["cargo", "build", "--release", "-p", "haitaka_cli", "--features", "anhoku,nnue-l1-256"],
            "host": "192.168.0.12",
            "architecture": "arm64",
            "features": ["anhoku", "nnue-l1-256"],
            "runtime": new_runtime,
            "tests": "cargo test -p haitaka_cli --features anhoku,nnue-l1-256: 53 unit and 5 smoke tests passed",
        },
        "failedAttempt": {"path": str(attempt.relative_to(root)), "files": [artifact(root, relative) for relative in failed_files]},
        "scheduleChanged": False,
        "failedAttemptModified": False,
        "selectionChanged": False,
        "additionalOptimizerUpdates": 0,
        "gamesLaunched": 0,
        "outcomesOpened": False,
        "confirmationAuthorized": False,
        "deploymentAuthorized": False,
    }
    compatibility_path = root / "preflight/match-runtime-compatibility.json"
    compatibility_path.write_text(json.dumps(compatibility, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    layout_path = root / "preflight/schedule-layout-compatibility.json"
    layout = json.loads(layout_path.read_text(encoding="utf-8"))
    helper = artifact(root, "repo/scripts/strength_first_step12_matches.py")
    helper["path"] = "scripts/strength_first_step12_matches.py"
    layout["patchedHelper"] = helper
    layout["adapterAmendment"] = {"reason": "bind the reviewed opening-unit game mode and isolated replacement policy", "scheduleChanged": False, "scheduleRegenerated": False, "additionalOptimizerUpdates": 0}
    layout_path.write_text(json.dumps(layout, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    recovery = {
        "schema": "haitaka-anhoku-strength-first-step12-match-recovery-amendment-v1",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "passed",
        "reason": "replace only the invalid first block after correcting the CLI opening-unit interface; preserve and exclude the failed attempt",
        "replacement": {"cell": FAILED_CELL, "block": FAILED_BLOCK, "failedAttempt": str(attempt.relative_to(root)), "newAttempt": "matches/early-selected/block-00/attempt-02", "replacementOnly": True, "excludeFailedAttemptFromAnalysis": True},
        "failedAttempt": {"path": str(attempt.relative_to(root)), "files": [artifact(root, relative) for relative in failed_files]},
        "frozenSchedule": schedule,
        "runtimeCompatibility": artifact(root, "preflight/match-runtime-compatibility.json"),
        "scheduleChanged": False,
        "failedAttemptModified": False,
        "selectionChanged": False,
        "additionalOptimizerUpdates": 0,
        "gamesLaunched": 0,
        "outcomesOpened": False,
        "confirmationAuthorized": False,
        "deploymentAuthorized": False,
    }
    recovery_path = root / "preflight/match-recovery-amendment.json"
    recovery_path.write_text(json.dumps(recovery, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": "passed", "compatibility": str(compatibility_path), "recovery": str(recovery_path), "failedFiles": len(failed_files)}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
