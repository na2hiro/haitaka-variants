#!/usr/bin/env python3
"""Verify and publish the single F1 outcome without launching work.

Run only when the F1 campaign has reached its authorized stopping point.
The command is idempotent and reads all strength evidence offline.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys
from typing import Any

import strength_first_f1_core as f1
from strength_first_f1_package import verify as verify_package


def _load(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text())
    f1.require(isinstance(value, dict), f"JSON object required: {path}")
    return value


def close(root: Path) -> dict[str, Any]:
    package = verify_package(root)["package"]
    development = root / "analysis/development.json"
    holdout = root / "analysis/holdout.json"
    training = {arm: root / f"training/arms/{arm}/run.json" for arm in ("C", "T")}
    for phase, path in (("development", development), ("holdout", holdout)):
        if path.is_file():
            result = _load(path)
            f1.require(result.get("phase") == phase and result.get("searchesDispatchedByAnalysis") == 0 and result.get("jobsDispatchedByAnalysis") == 0, f"{phase} analysis invalid")
            for block in result["acceptedBlocks"]:
                f1.verify_artifact(block)
    if holdout.is_file():
        f1.require(development.is_file() and _load(development).get("screenAdvances") is True, "holdout exists without advanced development")
        category = "holdout-superiority-supported" if _load(holdout).get("superioritySupported") is True else "holdout-superiority-unconfirmed"
    elif development.is_file():
        category = "preparation-qualification-or-execution-incomplete" if _load(development).get("screenAdvances") is True else "screen-did-not-advance"
    else:
        category = "preparation-qualification-or-execution-incomplete"
    evidence = {
        "package": package,
        "cpuQualification": f1.artifact(root / "preflight/cpu-qualification-v2.json"),
        "arms": {arm: f1.artifact(path) if path.is_file() else None for arm, path in training.items()},
        "development": f1.artifact(development) if development.is_file() else None,
        "holdout": f1.artifact(holdout) if holdout.is_file() else None,
    }
    for arm, path in training.items():
        if path.is_file():
            f1.require(_load(path).get("complete") is True and _load(path).get("arm") == arm, f"{arm} run incomplete")
    active_games = list((root / "games").glob("*/block-*/attempt-*/*/launch.json"))
    outcome = {
        "schema": "haitaka-anhoku-strength-first-f1-closeout-v1",
        "experimentId": f1.EXPERIMENT_ID,
        "category": category,
        "evidence": evidence,
        "gpuOccurred": any((root / "preflight/cuda-qualification").glob("attempt-*/*/uninterrupted.json")) or any((root / f"preflight/cuda-qualification/{arm}/report.json").is_file() for arm in ("C", "T")),
        "productionTrainingOccurred": any((root / "training/arms").glob("*/attempts/*/launch.json")),
        "scoredGamesOccurred": bool(active_games),
        "deploymentAuthorized": False,
        "searchesDispatchedByCloseout": 0,
        "jobsDispatchedByCloseout": 0,
        "instanceCreatedByCloseout": False,
    }
    output = root / "closeout.json"
    if output.exists():
        prior = _load(output)
        f1.require(prior == outcome, "idempotent closeout differs from published evidence")
    else:
        f1.immutable_json(output, outcome)
    return outcome


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=f1.F1_ROOT)
    args = parser.parse_args()
    try:
        outcome = close(args.root.resolve())
    except (f1.F1Error, OSError, ValueError, KeyError, TypeError) as exc:
        print(f"F1 closeout blocked: {exc}", file=sys.stderr)
        return 2
    print(json.dumps({"category": outcome["category"], "closeout": f1.artifact(args.root.resolve() / "closeout.json")}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
