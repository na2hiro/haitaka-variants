#!/usr/bin/env python3
"""Run the preregistered independent-data Model B training lane."""

from __future__ import annotations

import copy
import hashlib
from pathlib import Path
import sys

import strength_first_trainer as base


EXPERIMENT_ID = "strength-first-step9-model-b-training-20260912-v1"
TRAINING_SEED = 2026091108
OUTPUT_DIRECTORY = "out/anhoku-strength-first/step9-model-b-training-20260912-v1"
SCHEDULE_ID = "anhoku-strength-first-model-b-development-gates-v1"
PRIMARY_FIXED_TIME_MS = 1000
FIXED_CHECKPOINT_UPDATE = 2688
BASE_RUNNER_SHA256 = "c75e7691f1b105afdf0bd1f7d6b9f7eab42c715de3e1fbc29779fad109a4d44d"
CORPUS_DIRECTORY = "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1"

_base_validate_config = base.validate_config


def validate_model_b_config(config: dict) -> None:
    """Validate Model B bindings, then reuse the qualified shared validator."""

    base.exact(config["experimentId"], EXPERIMENT_ID, "config.experimentId")
    base.exact(config["recipe"]["seed"], TRAINING_SEED, "recipe.seed")
    base.exact(config["output"]["directory"], OUTPUT_DIRECTORY, "output.directory")
    base.exact(
        config["strengthProtocol"]["scheduleId"],
        SCHEDULE_ID,
        "strengthProtocol.scheduleId",
    )
    base.exact(
        config["strengthProtocol"]["fixedTimeMillisecondsPerMove"],
        PRIMARY_FIXED_TIME_MS,
        "strengthProtocol.fixedTimeMillisecondsPerMove",
    )
    for split in ("training", "validation"):
        expected_prefix = f"{CORPUS_DIRECTORY}/{split}"
        data = config["recipe"]["data"][split]
        for role, suffix in (("binary", ".bin"), ("manifest", ".json"), ("sidecar", ".jsonl")):
            base.exact(data[role]["path"], expected_prefix + suffix, f"recipe.data.{split}.{role}.path")
    expected_updates = [256, 1024, 2048, FIXED_CHECKPOINT_UPDATE, 4096]
    base.exact(
        config["recipe"]["checkpoints"]["scheduledUpdates"],
        expected_updates,
        "recipe.checkpoints.scheduledUpdates",
    )
    base.exact(
        config["recipe"]["export"]["automaticScheduledUpdates"],
        expected_updates,
        "recipe.export.automaticScheduledUpdates",
    )

    normalized = copy.deepcopy(config)
    normalized["experimentId"] = "strength-first-step3-compact-scratch-seed-20260906-v1"
    normalized["recipe"]["seed"] = 20260906
    normalized["recipe"]["checkpoints"]["scheduledUpdates"] = [256, 1024, 2048, 4096]
    normalized["recipe"]["export"]["automaticScheduledUpdates"] = [256, 1024, 2048, 4096]
    normalized["output"]["directory"] = (
        "out/anhoku-strength-first/step3-compact-scratch-seed-20260906-v1"
    )
    normalized["strengthProtocol"]["fixedTimeMillisecondsPerMove"] = 100
    normalized["strengthProtocol"]["finalSelection"]["postSelectionScreen"] = (
        "selected-checkpoint-only-128-pairs-at-100-ms-per-move"
    )
    _base_validate_config(normalized)


base.validate_config = validate_model_b_config


if __name__ == "__main__":
    try:
        base_path = Path(base.__file__).resolve()
        if hashlib.sha256(base_path.read_bytes()).hexdigest() != BASE_RUNNER_SHA256:
            raise base.StrengthFirstError(f"base trainer identity differs: {base_path}")
        raise SystemExit(base.main())
    except base.StrengthFirstError as exc:
        print(f"strength-first Model B training FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
