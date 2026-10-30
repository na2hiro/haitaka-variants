#!/usr/bin/env python3
"""Run the third, independent-seed width-256 strength-first training lane.

This wrapper reuses the already qualified Step 3 trainer implementation while
keeping the new experiment identity, seed, fixed-time policy, output directory,
and rental envelope closed-world validated.  Historical Step 3 contracts keep
their original runner hash and semantics.
"""

from __future__ import annotations

import copy
import hashlib
from pathlib import Path
import sys

import strength_first_trainer as base


EXPERIMENT_ID = "strength-first-step8-compact-seed-replication-20260910-v1"
TRAINING_SEED = 2026091002
OUTPUT_DIRECTORY = "out/anhoku-strength-first/step8-compact-seed-replication-20260910-v1"
SCHEDULE_ID = "anhoku-strength-first-development-seed-replication-128-pairs-v1"
PRIMARY_FIXED_TIME_MS = 1000
POST_SELECTION_SCREEN = "selected-checkpoint-only-128-pairs-at-1000-ms-per-move"
BASE_RUNNER_SHA256 = "c75e7691f1b105afdf0bd1f7d6b9f7eab42c715de3e1fbc29779fad109a4d44d"

_base_validate_config = base.validate_config


def validate_replication_config(config: dict) -> None:
    """Validate the new lane, then delegate every shared field to Step 3."""

    base.exact(config["experimentId"], EXPERIMENT_ID, "config.experimentId")
    base.exact(config["recipe"]["seed"], TRAINING_SEED, "recipe.seed")
    base.exact(config["output"]["directory"], OUTPUT_DIRECTORY, "output.directory")
    base.exact(config["strengthProtocol"]["scheduleId"], SCHEDULE_ID, "strengthProtocol.scheduleId")
    base.exact(
        config["strengthProtocol"]["fixedTimeMillisecondsPerMove"],
        PRIMARY_FIXED_TIME_MS,
        "strengthProtocol.fixedTimeMillisecondsPerMove",
    )
    base.exact(
        config["strengthProtocol"]["finalSelection"]["postSelectionScreen"],
        POST_SELECTION_SCREEN,
        "strengthProtocol.finalSelection.postSelectionScreen",
    )
    base.exact(config["budget"]["firstRentalEnvelopeGpuHours"], 4.0, "budget.firstRentalEnvelopeGpuHours")
    base.exact(config["budget"]["firstRentalEnvelopeUsd"], 5.0, "budget.firstRentalEnvelopeUsd")

    # The qualified implementation intentionally hard-coded the Step 3 lane.
    # Normalize only the independently declared lane fields for its exhaustive
    # shared-schema validator; all execution continues with the original values.
    normalized = copy.deepcopy(config)
    normalized["experimentId"] = "strength-first-step3-compact-scratch-seed-20260906-v1"
    normalized["recipe"]["seed"] = 20260906
    normalized["output"]["directory"] = "out/anhoku-strength-first/step3-compact-scratch-seed-20260906-v1"
    normalized["strengthProtocol"]["fixedTimeMillisecondsPerMove"] = 100
    normalized["strengthProtocol"]["finalSelection"]["postSelectionScreen"] = (
        "selected-checkpoint-only-128-pairs-at-100-ms-per-move"
    )
    _base_validate_config(normalized)


base.validate_config = validate_replication_config


if __name__ == "__main__":
    try:
        base_path = Path(base.__file__).resolve()
        if hashlib.sha256(base_path.read_bytes()).hexdigest() != BASE_RUNNER_SHA256:
            raise base.StrengthFirstError(f"base trainer identity differs: {base_path}")
        raise SystemExit(base.main())
    except base.StrengthFirstError as exc:
        print(f"strength-first seed replication FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
