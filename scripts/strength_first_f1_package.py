#!/usr/bin/env python3
"""Freeze and verify the F1 implementation, recipe, and CPU preparation package.

This command performs no GPU, rental, training, or game dispatch.
"""

from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path
import sys
from typing import Any

import strength_first_f1_core as f1
import strength_first_f1_live as live_repair


IMPLEMENTATION = (
    "scripts/strength_first_f1.py",
    "scripts/strength_first_f1_core.py",
    "scripts/strength_first_f1_loss.py",
    "scripts/strength_first_f1_fixture.py",
    "scripts/strength_first_f1_cuda_fixture.py",
    "scripts/strength_first_f1_train.py",
    "scripts/strength_first_f1_openings.py",
    "scripts/strength_first_f1_mbp_generate.py",
    "scripts/strength_first_f1_browser_worker.js",
    "scripts/strength_first_f1_replay.py",
    "scripts/strength_first_f1_live.py",
    "scripts/strength_first_f1_analysis.py",
    "scripts/strength_first_f1_monitor.py",
    "scripts/strength_first_f1_closeout.py",
    "scripts/strength_first_f1_test.py",
    "scripts/strength_first_f1_package.py",
    "scripts/strength_first_trainer.py",
    "scripts/r5_trainer.py",
    "scripts/strength_first_step12_context.py",
    "scripts/strength_first_step12_trainer.py",
    "scripts/strength_first_step12_trainer_backend.py",
    "scripts/strength_first_step12_cuda_runtime.py",
    "scripts/strength_first_step12_cuda_backward.py",
    "scripts/strength_first_step12_cuda_backward.cu",
    "scripts/strength_first_step12_cuda_backward_reference.py",
    "scripts/strength_first_step12_cuda_backward_tests.py",
    "scripts/strength_first_step18_sources.py",
    "scripts/strength_first_confirmation_core.py",
    "scripts/strength_first_step14_browser.py",
    "scripts/strength_first_step14_contract.py",
    "scripts/strength_first_step14_browser_page.html",
    "scripts/strength_first_step11_browser_page.html",
    "scripts/strength_first_step11_browser_parity_worker.js",
    "scripts/strength_first_step11_browser_smoke_worker.js",
    "scripts/strength_first_step14_terminal_worker.js",
    "haitaka_learn/src/bin/strength_first_step14_replay.rs",
    "haitaka_learn/src/step11_opening_gate.rs",
    "haitaka_learn/src/step12_openings.rs",
    "trainer-patches/variant-nnue-pytorch-phase7.1.patch",
    "trainer-patches/variant-nnue-pytorch-r5.patch",
    "trainer-patches/evaluate.py",
    "haitaka_learn/trainer_overlay/feature_set.py",
    "haitaka_learn/trainer_overlay/features.py",
    "haitaka_learn/trainer_overlay/donor_features.py",
    "haitaka_learn/trainer_overlay/training_data_loader.cpp",
)


def config(root: Path) -> dict[str, Any]:
    source = f1.RECIPE
    original = json.loads(source.read_text())
    f1.require(f1.artifact(source) == f1.frozen_inputs()["incumbentRecipe"], "Step 10 recipe changed")
    recipe = copy.deepcopy(original["recipe"])
    recipe["seed"] = 2026092501
    recipe["schedule"]["executionStopUpdate"] = 1024
    recipe["checkpoints"]["controlUpdate"] = None
    recipe["selection"]["metric"] = "finite-validation-clipped-huber-for-both-arms"
    recipe["selection"]["globalMinimumTolerance"] = 1e-12
    f1.require(recipe["checkpoints"]["candidateUpdates"] == list(f1.UPDATES), "candidate set changed")
    f1.require(recipe["schedule"]["horizonUpdates"] == 4096 and recipe["loader"]["batchSize"] == 16384 and recipe["loader"]["workers"] == 1, "shared recipe changed")
    adapter = f1.artifact(f1.ROOT / "scripts/strength_first_step12_cuda_backward.py")
    kernel = f1.artifact(f1.ROOT / "scripts/strength_first_step12_cuda_backward.cu")
    return {
        "schema": "haitaka-anhoku-strength-first-f1-training-config-v1",
        "experimentId": f1.EXPERIMENT_ID,
        "status": "frozen-scientific-recipe-pending-user-grants",
        "sourceRecipe": f1.artifact(source),
        "recipe": recipe,
        "arms": ["C", "T"],
        "trainingObjectives": {"C": "clipped-huber-H", "T": "temperature-scaled-sigmoid-P"},
        "commonCheckpointMetric": "clipped-huber-H",
        "calibration": f1.artifact(root / "calibration.json"),
        "trainer": {
            "pinnedRevision": "61666d9e3653e4df9881b14c23f8fdcc4bf7779b",
            "pinnedBundle": f1.artifact(root / "package/pinned-trainer-61666d9e.bundle"),
            "pinnedCheckout": "/home/na2hiro/proj/engine/variant-nnue-pytorch",
            "patchExecutable": {"path": "/opt/conda/bin/patch", "bytes": 218016, "sha256": "c2ea223616e9144b937d8445cedad95da16aeac0d1fd7828cc0007e7ef1ff270"},
            "cmake": "/opt/conda/bin/cmake",
            "cudaAdapter": adapter,
            "cudaKernel": kernel,
            "cudaProfile": "ordered-segment-v1",
            "cudaWorkspace": ":4096:8",
            "image": "pytorch/pytorch:2.8.0-cuda12.8-cudnn9-devel",
        },
        "authorization": {"gpuFixturesAuthorized": False, "productionTrainingAuthorized": False, "rentalAuthorized": False, "gamesAuthorized": False},
    }


def file_inventory(root: Path) -> dict[str, dict[str, Any]]:
    paths: list[Path] = [f1.ROOT / value for value in IMPLEMENTATION]
    paths += [root / "package/pinned-trainer-61666d9e.bundle", root / "package/quote.json", root / "package/forecast.json", root / "commands.md", root / "openings/source-exclusion.json", root / "runtime/fixture-schedule.json"]
    paths += [root / "openings" / f"{phase}{suffix}.json" for phase in ("development", "holdout") for suffix in ("", "-schedule")]
    paths += sorted((root / "openings/source-suites").glob("*"))
    paths += [f1.ROOT / "target/step18-preparation-helper/release/haitaka_learn"]
    paths += [f1.ROOT / "out/anhoku-strength-first/step18-model-strength-closeout-20260920-v1/preflight/current-families/union.jsonl"]
    paths += [f1.ROOT / relative for relative, _, _ in f1.INPUTS.values()]
    paths += sorted((root / "openings/proposals").glob("*/*/*"))
    inventory: dict[str, dict[str, Any]] = {}
    for path in paths:
        binding = f1.artifact(path)
        inventory[binding["path"]] = binding
    f1.require(len(inventory) == len(paths), "package inventory has duplicate dependencies")
    return dict(sorted(inventory.items()))


def freeze(root: Path) -> dict[str, Any]:
    f1.require(not (root / "package.json").exists(), "package already frozen; use verify")
    f1.require(json.loads((root / "preflight/cpu-qualification-v2.json").read_text()).get("passed") is True, "CPU qualification not passed")
    for phase, count in (("development", 128), ("holdout", 512)):
        openings = json.loads((root / f"openings/{phase}.json").read_text())
        schedule = json.loads((root / f"openings/{phase}-schedule.json").read_text())
        f1.require(len(openings["rows"]) == count and len(schedule["rows"]) == (count * (4 if phase == "development" else 2)), "opening/schedule count differs")
    fixture_schedule = live_repair.fixture_schedule(root)
    f1.immutable_json(root / "runtime/fixture-schedule.json", fixture_schedule)
    fixture_rows = fixture_schedule["rows"]
    f1.require(len(fixture_rows) == 8 and {row["matchup"] for row in fixture_rows} == {"T-C", "T-I"} and {row["treatmentColor"] for row in fixture_rows} == {"black", "white"}, "runtime fixture case coverage differs")
    config_path = root / "training/config.json"
    f1.immutable_json(config_path, config(root))
    package = {
        "schema": "haitaka-anhoku-strength-first-f1-execution-package-v1",
        "experimentId": f1.EXPERIMENT_ID,
        "status": "frozen",
        "createdAtUtc": f1.utc_now(),
        "config": f1.artifact(config_path),
        "contract": f1.artifact(root / "contract.json"),
        "calibration": f1.artifact(root / "calibration.json"),
        "cpuQualification": f1.artifact(root / "preflight/cpu-qualification-v2.json"),
        "quote": f1.artifact(root / "package/quote.json"),
        "forecast": f1.artifact(root / "package/forecast.json"),
        "files": file_inventory(root),
        "runtimeFixtureGameIds": [row["gameId"] for row in fixture_rows],
        "runtimeFixtureSchedule": f1.artifact(root / "runtime/fixture-schedule.json"),
        "gpuOccurred": False,
        "productionTrainingOccurred": False,
        "gamesOccurred": False,
        "authorization": {"execution": False, "rental": False, "match": False},
    }
    f1.immutable_json(root / "package.json", package)
    return package


def verify(root: Path) -> dict[str, Any]:
    package = json.loads((root / "package.json").read_text())
    f1.require(package.get("experimentId") == f1.EXPERIMENT_ID and package.get("status") == "frozen", "package identity differs")
    for name in ("config", "contract", "calibration", "cpuQualification", "quote", "forecast"):
        f1.verify_artifact(package[name])
    for item in package["files"].values():
        f1.verify_artifact(item)
    fixture_path = root / "runtime/fixture-schedule.json"
    expected_fixture = live_repair.fixture_schedule(root)
    f1.require(package.get("runtimeFixtureSchedule") == f1.artifact(fixture_path) and package.get("runtimeFixtureGameIds") == [row["gameId"] for row in expected_fixture["rows"]] and json.loads(fixture_path.read_text()) == expected_fixture, "F1 runtime fixture package binding differs")
    f1.require(json.loads((root / "training/config.json").read_text()) == config(root), "F1 config no longer matches recipe")
    return {"package": f1.artifact(root / "package.json"), "files": len(package["files"]), "verified": True}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("freeze", "verify"))
    parser.add_argument("--root", type=Path, default=f1.F1_ROOT)
    args = parser.parse_args()
    try:
        result = freeze(args.root.resolve()) if args.command == "freeze" else verify(args.root.resolve())
    except (f1.F1Error, OSError, KeyError, ValueError) as exc:
        print(f"F1 package blocked: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(result if args.command == "verify" else {"package": f1.artifact(args.root.resolve() / "package.json"), "files": len(result["files"])}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
