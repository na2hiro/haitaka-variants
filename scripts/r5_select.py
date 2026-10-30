#!/usr/bin/env python3
"""Validate and rank completed R5 recipe runs under the frozen rule.

This is deliberately an offline report builder.  It does not open the R5
stage block, compare against C/16, or claim an R5 gate pass; those require
additional sealed evidence after the recipe-and-width choice.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
from typing import Any


class SelectionError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise SelectionError(message)


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot read JSON {path}: {exc}")
    if not isinstance(value, dict):
        fail(f"JSON root is not an object: {path}")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
    except OSError as exc:
        fail(f"cannot hash {path}: {exc}")
    return digest.hexdigest()


def finite(value: Any, label: str) -> float:
    try:
        number = float(value)
    except (TypeError, ValueError) as exc:
        fail(f"{label} is not numeric: {value!r}")
    if not math.isfinite(number):
        fail(f"{label} is not finite: {number!r}")
    return number


def artifact(workspace: Path, record: dict[str, Any], label: str) -> dict[str, Any]:
    path = workspace / record["path"]
    if not path.is_file():
        fail(f"{label} is missing: {path}")
    actual_bytes = path.stat().st_size
    actual_hash = sha256(path)
    if actual_bytes != record["bytes"] or actual_hash != record["sha256"]:
        fail(
            f"{label} identity mismatch: expected {record['bytes']} bytes/{record['sha256']}, "
            f"got {actual_bytes} bytes/{actual_hash}"
        )
    return {"path": record["path"], "bytes": actual_bytes, "sha256": actual_hash}


def metric(report: dict[str, Any], split: str, name: str) -> float:
    return finite(report[split][name], f"{report['lane']} {split}.{name}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--width", choices=("current", "compact"), default="current")
    parser.add_argument("--lane", default=None, help="restrict validation to one declared lane")
    parser.add_argument("--learning-rate", type=float, default=None, help="restrict validation to one declared LR")
    parser.add_argument("--seed", type=int, default=None, help="restrict validation to one declared seed")
    parser.add_argument("--output", type=Path, default=None)
    args = parser.parse_args()

    contract = load_json(args.contract)
    if contract.get("schema") != "haitaka-anhoku-r5-fixed-data-exposure-contract-v1":
        fail("wrong R5 contract schema")
    output_dir = args.output_dir.resolve()
    workspace = output_dir.parents[1]
    recipe_root = output_dir / ("recipes" if args.width == "current" else "compact-recipes")
    width = contract["architecture"]["currentWidth" if args.width == "current" else "compactWidth"]
    expected_seed = contract["initialization"]["selectionSeed"] if args.seed is None else args.seed
    rates = contract["optimization"]["initialLearningRateCandidates"] if args.learning_rate is None else [args.learning_rate]
    expected = []
    for lane in contract["initialization"]["lanes"]:
        if args.lane is not None and lane["id"] != args.lane:
            continue
        for rate in rates:
            expected.append((lane, rate, expected_seed))

    runs: list[dict[str, Any]] = []
    for lane, rate, seed in expected:
        recipe_id = f"{lane['id']}-lr-{rate:g}-seed-{seed}"
        recipe_dir = recipe_root / recipe_id
        run_path = recipe_dir / "run.json"
        if not run_path.is_file():
            fail(f"expected recipe is incomplete: {run_path}")
        report = load_json(run_path)
        if report.get("schema") != "haitaka-anhoku-r5-recipe-run-v1":
            fail(f"wrong recipe schema: {run_path}")
        if report.get("lane") != lane["id"] or report.get("seed") != seed:
            fail(f"recipe identity mismatch: {run_path}")
        if float(report.get("learningRateCandidate")) != float(rate):
            fail(f"recipe learning-rate mismatch: {run_path}")
        if report.get("width") != width:
            fail(f"recipe width mismatch: {run_path}")

        exposure = report.get("finiteExposure", {})
        expected_exposure = {
            "trainRecordsPerPass": contract["optimization"]["trainRecordsPerPass"],
            "validationRecordsPerEvaluation": contract["optimization"]["validationRecords"],
            "developmentRecordsPerEvaluation": contract["optimization"]["developmentRecords"],
            "batchSize": contract["optimization"]["batchSize"],
            "randomFenSkipping": 0,
            "filtered": False,
            "cyclic": False,
            "loaderWorkers": contract["optimization"]["numWorkers"],
        }
        if any(exposure.get(key) != value for key, value in expected_exposure.items()):
            fail(f"finite exposure mismatch: {run_path}")
        updates = int(report["optimizerUpdates"])
        if updates < contract["optimization"]["minimumUpdates"]:
            fail(f"recipe stopped below minimum updates: {run_path}")
        if updates > contract["optimization"]["maximumUpdates"]:
            fail(f"recipe exceeded maximum updates: {run_path}")
        if int(report["checkpointCount"]) <= 0:
            fail(f"recipe has no checkpoints: {run_path}")

        final_checkpoint = artifact(workspace, report["finalCheckpoint"], "final checkpoint")
        exported_integer = artifact(workspace, report["exportedInteger"], "exported integer network")
        development = report["finalDevelopment"]
        validation = report["finalValidation"]
        primary = metric({"lane": recipe_id, "development": development}, "development", "teacher_only_loss")
        val_loss = metric({"lane": recipe_id, "validation": validation}, "validation", "teacher_only_loss")
        dev_mae = metric({"lane": recipe_id, "development": development}, "development", "teacher_only_mae")
        val_mae = metric({"lane": recipe_id, "validation": validation}, "validation", "teacher_only_mae")

        metrics_path = recipe_dir / "checkpoint-metrics.jsonl"
        if not metrics_path.is_file():
            fail(f"checkpoint metrics are missing: {metrics_path}")
        rows = [json.loads(line) for line in metrics_path.read_text(encoding="utf-8").splitlines() if line.strip()]
        if len(rows) != int(report["checkpointCount"]):
            fail(f"checkpoint count mismatch: {metrics_path}")
        previous_updates = 0
        missing_train_mae = False
        for index, row in enumerate(rows):
            row_updates = int(row["optimizerUpdates"])
            if row_updates <= previous_updates or row_updates % contract["optimization"]["checkpointEveryUpdates"] != 0:
                fail(f"non-monotonic checkpoint updates at row {index}: {metrics_path}")
            previous_updates = row_updates
            for split in ("train", "validation", "development"):
                finite(row[split]["teacher_only_loss"], f"{metrics_path}:{split}.teacher_only_loss")
                if "teacher_only_mae" not in row[split]:
                    if split != "train":
                        fail(f"missing held-out MAE at row {index}: {metrics_path}:{split}")
                    missing_train_mae = True
                else:
                    finite(row[split]["teacher_only_mae"], f"{metrics_path}:{split}.teacher_only_mae")
        first_dev = finite(rows[0]["development"]["teacher_only_loss"], f"{metrics_path}:first development loss")
        last_dev = finite(rows[-1]["development"]["teacher_only_loss"], f"{metrics_path}:last development loss")
        converged = report.get("stopReason") == "development-plateau"
        if rows[-1]["optimizerUpdates"] != updates:
            fail(f"final update mismatch: {metrics_path}")

        runs.append({
            "recipeId": recipe_id,
            "lane": report["lane"],
            "laneKind": report["laneKind"],
            "learningRateCandidate": rate,
            "seed": seed,
            "width": width,
            "path": str(recipe_dir.relative_to(workspace)),
            "runSha256": sha256(run_path),
            "optimizerUpdates": updates,
            "examplesSeen": int(report["examplesSeen"]),
            "stopReason": report.get("stopReason"),
            "convergedByPlateauRule": converged,
            "curve": {
                "firstDevelopmentLoss": first_dev,
                "finalDevelopmentLoss": last_dev,
                "absoluteDevelopmentImprovement": first_dev - last_dev,
                "reachedMaximumUpdates": updates == contract["optimization"]["maximumUpdates"],
            },
            "finalDevelopment": development,
            "finalValidation": validation,
            "finalCheckpoint": final_checkpoint,
            "exportedInteger": exported_integer,
            "staticPretraining": report.get("staticPretraining"),
            "reportCompleteness": {
                "trainTeacherOnlyMaeInCheckpointRows": not missing_train_mae,
                "note": "completed before runner schema update" if missing_train_mae else None,
            },
        })

    def sort_key(item: dict[str, Any]) -> tuple[float, float, float, int, str]:
        return (
            item["finalDevelopment"]["teacher_only_loss"],
            item["finalValidation"]["teacher_only_loss"],
            item["finalDevelopment"]["teacher_only_mae"],
            item["optimizerUpdates"],
            item["recipeId"],
        )

    ranked = sorted(runs, key=sort_key)
    selected = ranked[0]
    all_converged = all(item["convergedByPlateauRule"] for item in runs)
    report = {
        "schema": "haitaka-anhoku-r5-selection-report-v1",
        "contract": {
            "path": str(args.contract),
            "sha256": sha256(args.contract),
        },
        "width": width,
        "recipeRoot": str(recipe_root.relative_to(workspace)),
        "selectionRule": contract["offlineSelection"],
        "expectedRecipeCount": len(expected),
        "completedRecipeCount": len(ranked),
        "rankedRecipes": ranked,
        "selected": selected,
        "convergence": {
            "allRecipesReachedPlateauRule": all_converged,
            "status": "converged" if all_converged else "maximum-updates-before-plateau",
        },
        "c16Comparison": {
            "required": True,
            "available": False,
            "status": "pending-runtime-and-full-precision-baseline-evaluation",
        },
        "groupedUncertainty": {
            "requiredGroupingUnit": contract["offlineSelection"]["groupingUnit"],
            "available": False,
            "status": "pending-disjoint-stage-block",
        },
        "stageBlock": {
            "status": contract["offlineSelection"]["oneShotStageBlock"]["status"],
            "opened": False,
        },
        "r5DecisionMade": False,
        "passed": False,
        "nextAction": "run compact diagnostic under selected recipe, then replicate selected recipe-and-width with seeds 29 and 41",
    }
    output = args.output or (output_dir / ("r5-selection-report.json" if args.width == "current" else "compact-selection-report.json"))
    output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"R5 selection report written to {output}")
    print(f"selected={selected['recipeId']} development_loss={selected['finalDevelopment']['teacher_only_loss']:.12g}")
    print(f"convergence={report['convergence']['status']}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except SelectionError as exc:
        print(f"R5 selection FAILED: {exc}")
        raise SystemExit(1)
