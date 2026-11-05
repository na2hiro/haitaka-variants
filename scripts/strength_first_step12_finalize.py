#!/usr/bin/env python3
"""Finalize a completed Step 12 u2688 run after a retention-ledger defect.

This command is deliberately separate from the training backend.  It loads
the immutable CPU-mapped resume state, validates every completed boundary and
artifact hash, and publishes a derived nine-bundle inventory.  It never
initializes a model, creates an optimizer, imports CUDA, or performs an update.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
from typing import Any


EXPERIMENT_ID = "strength-first-step12-independent-replication-20260916-v1"
STATE_SCHEMA = "haitaka-anhoku-strength-first-step12-resume-state-v1"
RUN_SCHEMA = "haitaka-anhoku-strength-first-step12-training-run-v1"
SELECTION_INPUT_SCHEMA = "haitaka-anhoku-strength-first-step12-selection-input-v1"
REPAIR_SCHEMA = "haitaka-anhoku-strength-first-step12-retention-finalization-repair-v1"
CANDIDATE_UPDATES = [128, 256, 384, 512, 640, 768, 896, 1024]
CONTROL_UPDATE = 2688
SCHEDULE_HORIZON = 4096


class FinalizationError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise FinalizationError(message)


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def artifact(path: Path, root: Path) -> dict[str, Any]:
    if not path.is_file() or path.is_symlink():
        fail(f"missing or symlinked artifact: {path}")
    try:
        relative = path.resolve().relative_to(root.resolve())
    except ValueError:
        relative = path
    return {"path": str(relative), "bytes": path.stat().st_size, "sha256": sha256(path)}


def atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{__import__('os').getpid()}.tmp")
    temporary.write_bytes(json.dumps(value, indent=2, sort_keys=True).encode("utf-8") + b"\n")
    temporary.replace(path)


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        fail(f"cannot read JSON {path}: {exc}")
    if not isinstance(value, dict):
        fail(f"JSON object required: {path}")
    return value


def path_from_ref(ref: Any, root: Path, label: str) -> Path:
    if not isinstance(ref, dict) or not isinstance(ref.get("path"), str):
        fail(f"{label} is not an artifact reference")
    path = Path(ref["path"])
    if not path.is_absolute():
        path = root / path
    return path.resolve()


def verify_ref(ref: Any, root: Path, label: str) -> Path:
    path = path_from_ref(ref, root, label)
    if not path.is_file() or path.is_symlink():
        fail(f"{label} is missing or symlinked: {path}")
    if ref.get("bytes") != path.stat().st_size or ref.get("sha256") != sha256(path):
        fail(f"{label} hash or byte count differs: {path}")
    return path


def normalize_artifacts(value: Any, root: Path) -> Any:
    """Make state references portable while preserving non-artifact fields."""

    if isinstance(value, dict):
        if {"path", "bytes", "sha256"}.issubset(value):
            candidate = Path(value["path"])
            candidate = candidate if candidate.is_absolute() else root / candidate
            # Some producer metadata (for example the serializer name) has
            # the same three keys but intentionally names a context file that
            # is not rooted in this experiment.  Normalize only refs that
            # resolve to an actual bound file.
            if candidate.is_file():
                return artifact(candidate.resolve(), root)
        return {key: normalize_artifacts(item, root) for key, item in value.items()}
    if isinstance(value, list):
        return [normalize_artifacts(item, root) for item in value]
    return value


def verify_boundary_item(item: Any, root: Path, update: int) -> dict[str, Any]:
    if not isinstance(item, dict) or item.get("update") != update:
        fail(f"retained boundary {update} is malformed")
    for key in ("validation", "checkpoint", "export"):
        if key == "export":
            export = item.get("export")
            if not isinstance(export, dict):
                fail(f"retained[{update}].export is missing")
            verify_ref(export.get("sourceCheckpoint"), root, f"retained[{update}].export.sourceCheckpoint")
            verify_ref(export.get("integerNetwork"), root, f"retained[{update}].export.integerNetwork")
            if export["sourceCheckpoint"].get("sha256") != item["checkpoint"].get("sha256"):
                fail(f"retained[{update}].export.sourceCheckpoint differs from checkpoint")
        else:
            verify_ref(item.get(key), root, f"retained[{update}].{key}")
    selection = item.get("selection")
    if not isinstance(selection, dict):
        fail(f"retained[{update}].selection is missing")
    for key in ("checkpoint", "export"):
        verify_ref(selection.get(key), root, f"retained[{update}].selection.{key}")
    verify_ref(selection.get("validation", {}).get("receipt"), root, f"retained[{update}].selection.validation.receipt")
    normalized = normalize_artifacts(item, root)
    actual_order = normalized["selection"]["validation"].get("order")
    if actual_order != "fixed-source-order-once":
        fail(f"retained[{update}].selection.validation.order is unexpected: {actual_order}")
    normalized["selection"]["validation"]["order"] = "fixed-source-order-once-per-evaluation-v1"
    return normalized


def check_static(root: Path, source_run: Path, config_path: Path) -> dict[str, Any]:
    if not source_run.is_dir():
        fail(f"source run directory is missing: {source_run}")
    if (source_run / "run.json").exists():
        fail("source run already has a finalized run.json; use the idempotent path")
    state_path = source_run / "last-state.pt"
    resolved_config_path = source_run / "resolved-config.json"
    metrics_path = source_run / "metrics.jsonl"
    for path in (state_path, resolved_config_path, metrics_path):
        if not path.is_file() or path.is_symlink():
            fail(f"required original evidence is missing: {path}")
    config_hash = sha256(config_path)
    if sha256(resolved_config_path) != config_hash:
        fail("original resolved config differs from the authorized config")

    # Import torch only for CPU deserialization.  No model or optimizer is
    # constructed, and map_location prevents any CUDA initialization.
    try:
        import torch  # type: ignore

        state = torch.load(state_path, map_location="cpu", weights_only=False)
    except Exception as exc:
        fail(f"cannot load original CPU-mapped resume state: {exc}")
    if not isinstance(state, dict):
        fail("resume state is not a mapping")
    if state.get("schema") != STATE_SCHEMA or state.get("experimentId") != EXPERIMENT_ID:
        fail("resume state schema or experiment identity differs")
    if state.get("configSha256") != config_hash:
        fail("resume state is not bound to the authorized config")
    if state.get("optimizerUpdates") != CONTROL_UPDATE:
        fail("resume state did not reach u2688")
    schedule = state.get("schedule")
    if not isinstance(schedule, dict) or schedule.get("horizonUpdates") != SCHEDULE_HORIZON or schedule.get("lastAppliedUpdate") != CONTROL_UPDATE or schedule.get("nextOptimizerUpdate") != CONTROL_UPDATE + 1:
        fail("resume state schedule is incomplete or inconsistent")
    if not isinstance(state.get("examplesSeen"), int) or not isinstance(state.get("dataCursor"), dict):
        fail("resume state progress fields are malformed")

    full_updates = CANDIDATE_UPDATES + list(range(1152, CONTROL_UPDATE, 128)) + [CONTROL_UPDATE]
    if full_updates != list(range(128, CONTROL_UPDATE + 1, 128)):
        fail("internal boundary inventory is malformed")
    retained = state.get("retained")
    if not isinstance(retained, list) or [item.get("update") for item in retained if isinstance(item, dict)] != full_updates:
        fail("original retained inventory does not contain every completed validation boundary in order")
    retained_by_update: dict[int, dict[str, Any]] = {}
    for update, item in zip(full_updates, retained):
        if update in retained_by_update:
            fail(f"duplicate retained boundary: {update}")
        retained_by_update[update] = verify_boundary_item(item, root, update)
    validation_history = state.get("validationHistory")
    expected_history = [0] + full_updates
    if not isinstance(validation_history, list) or [item.get("update") for item in validation_history if isinstance(item, dict)] != expected_history:
        fail("validation history does not contain the initial evaluation and all 21 boundaries")
    metrics = []
    for line in metrics_path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            try:
                row = json.loads(line)
            except json.JSONDecodeError as exc:
                fail(f"metrics.jsonl contains invalid JSON: {exc}")
            if not isinstance(row, dict):
                fail("metrics.jsonl row is not an object")
            metrics.append(row)
    if [row.get("optimizerUpdates") for row in metrics] != full_updates:
        fail("metrics.jsonl does not contain all completed boundaries in order")

    for update in full_updates:
        if not (root / "checkpoints" / f"u{update:06d}.pt").is_file() or not (root / "exports" / f"u{update:06d}.nnue").is_file() or not (root / "training" / "validation" / f"u{update:06d}.json").is_file():
            fail(f"completed boundary files are incomplete at u{update}")
    required_updates = CANDIDATE_UPDATES + [CONTROL_UPDATE]
    repaired_retained = [retained_by_update[update] for update in required_updates]
    original_files = []
    for path in sorted(source_run.rglob("*")):
        if path.is_file() and not path.is_symlink():
            original_files.append(artifact(path, root))
    return {
        "state": state,
        "statePath": state_path,
        "resolvedConfigPath": resolved_config_path,
        "metricsPath": metrics_path,
        "fullUpdates": full_updates,
        "requiredUpdates": required_updates,
        "extraUpdates": [update for update in full_updates if update not in required_updates],
        "retained": repaired_retained,
        "originalFiles": original_files,
        "configHash": config_hash,
        "metrics": metrics,
    }


def idempotent(root: Path, repair_path: Path) -> dict[str, Any] | None:
    if not repair_path.is_file():
        return None
    repair = load_json(repair_path)
    if repair.get("schema") != REPAIR_SCHEMA or repair.get("status") != "passed":
        fail("existing retention finalization receipt is not a passed receipt")
    run_path = root / "training" / "run" / "run.json"
    selection_input = root / "training" / "selection-input.json"
    verify_ref(repair.get("publishedRun"), root, "repair.publishedRun")
    verify_ref(repair.get("publishedSelectionInput"), root, "repair.publishedSelectionInput")
    if not run_path.is_file() or not selection_input.is_file():
        fail("passed finalization receipt lacks its published outputs")
    return {"status": "passed-idempotent", "repair": artifact(repair_path, root), "run": artifact(run_path, root), "selectionInput": artifact(selection_input, root)}


def finalize(root: Path, config_path: Path) -> dict[str, Any]:
    repair_path = root / "preflight" / "retention-finalization-repair.json"
    existing = idempotent(root, repair_path)
    if existing is not None:
        print(json.dumps(existing, sort_keys=True))
        return existing
    source_run = root / "training" / "run"
    checked = check_static(root, source_run, config_path)
    state = checked["state"]
    package_path = root / "training" / "package.json"
    failure_path = root / "preflight" / "production-training-failure-maximumUpdates.json"
    if not failure_path.is_file():
        failure_path = root / "training" / "failure.json"
    if not failure_path.is_file():
        fail("the original retention failure receipt is missing")
    failure = load_json(failure_path)
    if failure.get("error") != "retention contains more than the eight candidates plus control" or failure.get("trainingOrGpuAttempted") is not True:
        fail("the preserved failure is not the expected post-u2688 retention failure")

    flags = {"confirmationAuthorized": False, "deploymentAuthorized": False, "outcomesOpened": False, "handcraftedRemainsProduction": True}
    repair = {
        "schema": REPAIR_SCHEMA,
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "passed",
        "kind": "finalization-only-compatibility-repair",
        "originalProducer": {
            "package": artifact(package_path, root),
            "config": artifact(config_path, root),
            "backendImplementation": package_path.read_text(encoding="utf-8"),
        },
        "failure": artifact(failure_path, root),
        "originalState": artifact(checked["statePath"], root),
        "originalResolvedConfig": artifact(checked["resolvedConfigPath"], root),
        "originalMetrics": artifact(checked["metricsPath"], root),
        "originalRunFiles": checked["originalFiles"],
        "originalStateFacts": {
            "optimizerUpdates": state["optimizerUpdates"],
            "schedule": state["schedule"],
            "examplesSeen": state["examplesSeen"],
            "dataCursor": state["dataCursor"],
            "validationUpdates": [item["update"] for item in state["validationHistory"]],
            "retainedUpdates": [item["update"] for item in state["retained"]],
        },
        "requiredUpdates": checked["requiredUpdates"],
        "extraCompletedUpdatesPreserved": checked["extraUpdates"],
        "derivedRetainedUpdates": checked["requiredUpdates"],
        "additionalOptimizerUpdates": 0,
        "modelStateChanged": False,
        "validationRecomputed": False,
        "exportsRegenerated": False,
        "trainingLaunchedByRepair": False,
        "flags": flags,
    }
    # Do not embed a self-hash.  The repair receipt binds the published
    # outputs after they are written, avoiding a circular artifact reference.
    atomic_json(repair_path, repair)

    selection_input = {
        "schema": SELECTION_INPUT_SCHEMA,
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "config": artifact(config_path, root),
        "candidates": [{"update": item["update"], **item["selection"]} for item in checked["retained"][:-1]],
        "control": {"update": checked["retained"][-1]["update"], **checked["retained"][-1]["selection"]},
        "matchEvidenceUsed": False,
        "finalizationRepair": {"path": str(repair_path.relative_to(root))},
        "flags": flags,
    }
    selection_input_path = root / "training" / "selection-input.json"
    atomic_json(selection_input_path, selection_input)

    training_auth = load_json(root / "authorization" / "training.json")
    gpu_auth = load_json(root / "authorization" / "gpu-rental.json")
    production_resource = load_json(root / "preflight" / "production-instance-20260919.json").get("instance", {}) if (root / "preflight" / "production-instance-20260919.json").is_file() else {}
    run = {
        "schema": RUN_SCHEMA,
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "configSha256": checked["configHash"],
        "authorization": {"training": training_auth, "gpuRental": gpu_auth},
        "optimizerUpdates": CONTROL_UPDATE,
        "scheduleHorizonUpdates": SCHEDULE_HORIZON,
        "executionStopUpdate": CONTROL_UPDATE,
        "validationEvaluations": len(state["validationHistory"]),
        "expectedValidationEvaluations": 22,
        "examplesSeen": state["examplesSeen"],
        "dataCursor": state["dataCursor"],
        "retained": checked["retained"],
        "complete": True,
        "stopReason": "u2688-control-reached",
        "selectionInput": artifact(selection_input_path, root),
        "resource": {**state.get("resource", {}), "providerResource": production_resource, "elapsedSeconds": failure.get("resource", {}).get("elapsedSeconds")},
        "timingSeconds": state.get("timingSeconds", {}),
        "attempts": [{"kind": "fresh", "status": "completed-before-finalization-repair", "additionalOptimizerUpdates": 0}],
        "deterministicCudaRuntime": {"path": "preflight/deterministic-cuda-v1/production-adapter-install.json"},
        "finalizationOnly": True,
        "retentionFinalizationRepair": {"path": str(repair_path.relative_to(root))},
        "trainingOrGpuAttempted": True,
        "outcomesOpened": False,
        "flags": flags,
    }
    run_path = source_run / "run.json"
    atomic_json(run_path, run)

    repair = load_json(repair_path)
    repair["publishedRun"] = artifact(run_path, root)
    repair["publishedSelectionInput"] = artifact(selection_input_path, root)
    repair["publishedAt"] = now()
    atomic_json(repair_path, repair)
    result = {"status": "passed", "optimizerUpdates": CONTROL_UPDATE, "additionalOptimizerUpdates": 0, "repair": artifact(repair_path, root), "run": artifact(run_path, root), "selectionInput": artifact(selection_input_path, root), "derivedRetainedUpdates": checked["requiredUpdates"], "preservedExtraUpdates": checked["extraUpdates"]}
    print(json.dumps(result, sort_keys=True))
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--config", type=Path)
    args = parser.parse_args()
    try:
        root = args.root.expanduser().resolve()
        config_path = (args.config.expanduser().resolve() if args.config else root / "training" / "config.json")
        if not config_path.is_file():
            fail(f"config is missing: {config_path}")
        finalize(root, config_path)
        return 0
    except (FinalizationError, OSError, UnicodeError, json.JSONDecodeError, KeyError, TypeError) as exc:
        print(f"Step 12 finalization blocked: {exc}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
