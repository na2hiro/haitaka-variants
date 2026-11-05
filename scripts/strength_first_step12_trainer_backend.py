#!/usr/bin/env python3
"""Authorized Step 12 production backend.

This file is imported only after ``strength_first_step12_trainer.py`` has
validated the two separate execution ledgers.  It owns the Step 12 loop and
uses the historical modules only as hash-bound mathematical/loader helpers;
their historical entry points and configuration validators are never called.
"""

from __future__ import annotations

from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import time
from typing import Any

from strength_first_step12_context import artifact as context_artifact, context_receipt_path, prepare_context


EXPERIMENT_ID = "strength-first-step12-independent-replication-20260916-v1"
RUN_SCHEMA = "haitaka-anhoku-strength-first-step12-training-run-v1"
STATE_SCHEMA = "haitaka-anhoku-strength-first-step12-resume-state-v1"
VALIDATION_SCHEMA = "haitaka-anhoku-strength-first-step12-validation-receipt-v1"
INITIALIZATION_SCHEMA = "haitaka-anhoku-strength-first-step12-initialization-v1"
CANDIDATE_UPDATES = [128, 256, 384, 512, 640, 768, 896, 1024]
CONTROL_UPDATE = 2688
SCHEDULE_HORIZON = 4096
TRAINER_REVISION = "61666d9e3653e4df9881b14c23f8fdcc4bf7779b"
REPO_ROOT = Path(__file__).resolve().parents[1]


class Step12BackendError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise Step12BackendError(message)


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def artifact(path: Path) -> dict[str, Any]:
    if not path.is_file() or path.is_symlink():
        fail(f"missing or symlinked backend artifact: {path}")
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256(path)}


def artifact_ref(value: dict[str, Any]) -> dict[str, Any]:
    """Project a rich helper receipt to the exact selection artifact shape."""
    return {key: value[key] for key in ("path", "bytes", "sha256")}


def core_schedule_view(recipe: dict[str, Any]) -> dict[str, Any]:
    """Adapt the Step 12 schedule to the hash-bound historical helper."""

    schedule = recipe["schedule"]
    if schedule["horizonUpdates"] != SCHEDULE_HORIZON or schedule["executionStopUpdate"] != CONTROL_UPDATE:
        fail("Step12 schedule horizon/stop mismatch")
    return {
        **recipe,
        "schedule": {
            "kind": schedule["kind"],
            "baseLearningRate": schedule["baseLearningRate"],
            "minimumLearningRate": schedule["minimumLearningRate"],
            "warmupUpdates": schedule["warmupUpdates"],
            "maximumUpdates": schedule["horizonUpdates"],
            "firstUpdateLearningRate": schedule["baseLearningRate"] / schedule["warmupUpdates"],
            "warmupEndpointLearningRate": schedule["baseLearningRate"],
            "finalUpdateLearningRate": schedule["minimumLearningRate"],
        },
    }


def atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_bytes(json.dumps(value, indent=2, sort_keys=True).encode("utf-8") + b"\n")
    os.replace(temporary, path)


def reconcile_metrics_journal(path: Path, state: dict[str, Any]) -> None:
    """Recover the last committed row and discard only an incomplete tail."""
    committed_update = state["optimizerUpdates"]
    committed_row = state["lastMetricsRow"]
    if committed_row.get("optimizerUpdates") != committed_update:
        fail("committed metrics row does not match the resume update")
    original = path.read_bytes() if path.exists() else b""
    lines = original.splitlines(keepends=True)
    rows: list[dict[str, Any]] = []
    previous_update = -1
    for index, line in enumerate(lines):
        try:
            row = json.loads(line)
        except (ValueError, UnicodeError):
            if index == len(lines) - 1 and not line.endswith(b"\n"):
                break  # An interrupted append is recoverable from lastMetricsRow.
            fail("metrics journal contains an invalid complete row")
        if not isinstance(row, dict) or type(row.get("optimizerUpdates")) is not int:
            fail("metrics journal row is missing an integer optimizer update")
        update = row["optimizerUpdates"]
        if update <= previous_update:
            fail("metrics journal updates are not strictly increasing")
        if update > committed_update:
            fail("metrics journal is ahead of committed last state")
        if update == committed_update and row != committed_row:
            fail("metrics journal conflicts with committed last state")
        rows.append(row)
        previous_update = update
    if previous_update != committed_update:
        rows.append(committed_row)
    payload = b"".join(
        (json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
        for row in rows
    )
    if payload == original:
        return
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    try:
        with temporary.open("wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def digest_torch_value(torch: Any, value: Any) -> str:
    temporary = Path(os.path.join(os.getcwd(), f".step12-tensor-digest-{os.getpid()}.tmp"))
    try:
        torch.save(value, temporary)
        return sha256(temporary)
    finally:
        temporary.unlink(missing_ok=True)


def state_digest(torch: Any, model: Any, optimizer: Any, rng: dict[str, Any]) -> dict[str, str]:
    return {
        "model": digest_torch_value(torch, model.state_dict()),
        "optimizer": digest_torch_value(torch, optimizer.state_dict()),
        "rng": digest_torch_value(torch, rng),
    }


def finite_state(core: Any, torch: Any, model: Any, optimizer: Any, where: str) -> None:
    core.assert_finite_training_state(torch, model, optimizer, where)


def checkpoint_record(core: Any, torch: Any, model: Any, path: Path, update: int) -> dict[str, Any]:
    saved = core.save_portable_model(torch, model, path)
    return {"update": update, **saved}


def write_validation(root: Path, update: int, validation: dict[str, Any], config_sha256: str) -> dict[str, Any]:
    path = root / "training" / "validation" / f"u{update:06d}.json"
    receipt = {
        "schema": VALIDATION_SCHEMA,
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "configSha256": config_sha256,
        "update": update,
        "records": validation["records"],
        "order": validation["order"],
        "teacherOnlyLoss": validation["teacherOnlyLoss"],
        "finite": True,
        "diagnostics": validation,
        "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "outcomesOpened": False, "handcraftedRemainsProduction": True},
    }
    atomic_json(path, receipt)
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256(path), "receipt": receipt}


def build_resume_state(core: Any, torch: Any, model: Any, optimizer: Any, config_sha256: str, update: int, examples_seen: int, cursor: Any, validation_history: list[dict[str, Any]], retained: list[dict[str, Any]], timing: dict[str, float], resource: dict[str, Any], last_row: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema": STATE_SCHEMA,
        "experimentId": EXPERIMENT_ID,
        "configSha256": config_sha256,
        "model": model.state_dict(),
        "optimizer": optimizer.state_dict(),
        "schedule": {"kind": "linear-warmup-then-cosine-by-optimizer-update-v1", "horizonUpdates": 4096, "lastAppliedUpdate": update, "nextOptimizerUpdate": update + 1},
        "rng": core.rng_state(torch),
        "optimizerUpdates": update,
        "examplesSeen": examples_seen,
        "dataCursor": {"epoch": cursor.epoch, "record": cursor.record},
        "validationHistory": validation_history,
        "retained": retained,
        "timingSeconds": timing,
        "resource": resource,
        "lastMetricsRow": last_row,
    }


def run_training(*, config_path: Path, root: Path, training_authorization: dict[str, Any], gpu_authorization: dict[str, Any], resume: bool) -> dict[str, Any]:
    """Run the exact one-run contract after authorization has already passed."""

    config = json.loads(config_path.read_text(encoding="utf-8"))
    recipe = config["recipe"]
    output = root / "training" / "run"
    state_path = output / "last-state.pt"
    config_hash = sha256(config_path)
    if resume:
        if not state_path.is_file():
            fail("--resume requires training/run/last-state.pt")
        if not (output / "resolved-config.json").is_file() or sha256(output / "resolved-config.json") != config_hash:
            fail("resume resolved config differs from the authorized config")
    elif output.exists():
        fail("fresh production training refuses an existing output directory")

    context_temp = None
    try:
        import torch  # type: ignore
        import strength_first_trainer as core  # type: ignore
        import r5_trainer as r5  # type: ignore
        import strength_first_step12_cuda_runtime as cuda_runtime  # type: ignore
    except (ImportError, OSError) as exc:
        fail(f"authorized pinned trainer dependencies are unavailable: {exc}")
    if not torch.cuda.is_available():
        fail("the authorized production backend requires CUDA; no CUDA device is available")
    device = torch.device("cuda")

    started = time.monotonic()
    output.mkdir(parents=True, exist_ok=True)
    (output / "checkpoints").mkdir(exist_ok=True)
    (output / "exports").mkdir(exist_ok=True)
    (output / "validation").mkdir(exist_ok=True)
    if not resume:
        shutil.copy2(config_path, output / "resolved-config.json")

    core.seed_everything(torch, recipe["seed"])
    external = Path(config["trainer"]["pinnedCheckout"]["path"])
    assembled_purpose = f"production-{'resume' if resume else 'fresh'}-{os.getpid()}"
    context_temp, context = prepare_context(
        repo_root=REPO_ROOT,
        root=root,
        external=external,
        patch_binding=config["trainer"]["patchExecutable"],
        width=recipe["width"],
        r5=r5,
        cmake=config.get("environment", {}).get("cmakeExecutable", "cmake"),
        purpose=assembled_purpose,
    )
    try:
        torch, features, model_module, nnue_dataset = r5.import_trainer(context)
        torch.use_deterministic_algorithms(True)
        if hasattr(torch.backends, "cuda") and hasattr(torch.backends.cuda, "matmul"):
            torch.backends.cuda.matmul.allow_tf32 = False
        if hasattr(torch.backends, "cudnn"):
            torch.backends.cudnn.allow_tf32 = False
            torch.backends.cudnn.benchmark = False
        runtime_receipt = cuda_runtime.install(
            model_module,
            features,
            torch,
            recipe["featureFamily"],
            binding={"adapter": config.get("trainer", {}).get("cudaAdapter"), "kernel": config.get("trainer", {}).get("cudaKernel"), "profile": config.get("recipe", {}).get("cudaDeterminism", {}).get("profile")},
        )
        runtime_receipt["contextAssembly"] = context_artifact(context_receipt_path(root, assembled_purpose))
        atomic_json(root / "preflight" / "deterministic-cuda-v1" / "production-adapter-install.json", runtime_receipt)
        feature_set = features.get_feature_set_from_name(recipe["featureFamily"])
        model = model_module.NNUE(feature_set=feature_set, lambda_=recipe["objective"]["lambda"], learning_rate=recipe["schedule"]["baseLearningRate"]).to(device)
        optimizer = core.configured_optimizer(model, recipe)
        cursor = core.Cursor()
        update = 0
        examples_seen = 0
        validation_history: list[dict[str, Any]] = []
        retained: list[dict[str, Any]] = []
        timing = {"shuffleSeconds": 0.0, "trainingSeconds": 0.0, "validationSeconds": 0.0, "checkpointSeconds": 0.0, "exportSeconds": 0.0}
        resource = {"host": config.get("environment", {}).get("host", "192.168.0.12"), "device": "cuda", "billedHours": 0.0, "costUsd": 0.0, "recoveryCount": 0}
        # The historical helper uses ``maximumUpdates`` for the same fixed
        # 4096-update schedule that Step 12 calls ``horizonUpdates``.  Keep
        # the authorized Step 12 recipe immutable and pass only this private
        # compatibility view to the helper.
        core_recipe = core_schedule_view(recipe)

        if resume:
            state = torch.load(state_path, map_location=device, weights_only=False)
            if state.get("schema") != STATE_SCHEMA or state.get("experimentId") != EXPERIMENT_ID or state.get("configSha256") != config_hash:
                fail("resume state is not bound to the exact Step 12 run")
            model.load_state_dict(state["model"])
            optimizer.load_state_dict(state["optimizer"])
            cursor = core.Cursor(**state["dataCursor"])
            update = state["optimizerUpdates"]
            examples_seen = state["examplesSeen"]
            validation_history = list(state["validationHistory"])
            retained = list(state["retained"])
            timing = dict(state["timingSeconds"])
            resource = dict(state["resource"])
            core.restore_rng(torch, state["rng"])
            finite_state(core, torch, model, optimizer, "resumed Step 12 state")
            reconcile_metrics_journal(output / "metrics.jsonl", state)

        initialization_path = root / "training" / "initialization.json"
        if not resume:
            model._clip_weights()
            finite_state(core, torch, model, optimizer, "initial Step 12 state")
            initial_rng = core.rng_state(torch)
            initialization = {
                "schema": INITIALIZATION_SCHEMA,
                "createdAt": now(),
                "experimentId": EXPERIMENT_ID,
                "configSha256": config_hash,
                "seed": recipe["seed"],
                "freshScratch": True,
                "modelTensorSha256": digest_torch_value(torch, model.state_dict()),
                "optimizerStateSha256": digest_torch_value(torch, optimizer.state_dict()),
                "rngStateSha256": digest_torch_value(torch, initial_rng),
                "inheritedState": None,
                "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "outcomesOpened": False, "handcraftedRemainsProduction": True},
            }
            atomic_json(initialization_path, initialization)

        train_data = recipe["data"]["training"]
        validation_data = recipe["data"]["validation"]
        train_metadata = core.Metadata(Path(train_data["sidecar"]["path"]), train_data["records"])
        validation_metadata = core.Metadata(Path(validation_data["sidecar"]["path"]), validation_data["records"])
        if update == 0 and not validation_history:
            validation_started = time.monotonic()
            initial_validation = core.evaluate(torch, model, nnue_dataset, recipe, validation_metadata, device)
            timing["validationSeconds"] += time.monotonic() - validation_started
            validation_history.append({"update": 0, "validation": initial_validation})
            atomic_json(root / "training" / "initial-validation.json", initial_validation)

        window = core.Window(recipe["objective"])
        cache = output / "order-cache"
        cache.mkdir(exist_ok=True)
        while update < CONTROL_UPDATE:
            order = core.epoch_permutation(train_data["records"], recipe["seed"], cursor.epoch)
            shuffled = cache / f"epoch-{cursor.epoch:04d}.bin"
            shuffle_started = time.monotonic()
            core.materialize_epoch(Path(train_data["binary"]["path"]), shuffled, train_data["records"], train_data["entryBytes"], order)
            timing["shuffleSeconds"] += time.monotonic() - shuffle_started
            for batch in core.finite_batches(nnue_dataset, recipe["featureFamily"], shuffled, train_data["records"], recipe["loader"]["batchSize"], device, recipe["loader"], skip=cursor.record):
                if update >= CONTROL_UPDATE:
                    break
                count = int(batch[0].shape[0])
                indices = order[cursor.record:cursor.record + count]
                next_update = update + 1
                before = [parameter.detach().float().clone() for parameter in model.parameters()]
                model.train()
                model._clip_weights()
                core.assert_finite_nested(torch, model.state_dict(), f"pre-update {next_update}")
                base_lr = core.set_learning_rates(optimizer, core_recipe, next_update)
                optimizer.zero_grad(set_to_none=True)
                train_started = time.monotonic()
                mean_loss, losses, predictions, targets = core.loss_tensors(torch, model, batch, recipe["objective"])
                if not bool(torch.isfinite(mean_loss)):
                    fail(f"non-finite loss at update {next_update}")
                mean_loss.backward()
                grad_norm, grad_max = core.gradient_magnitude(torch, model)
                optimizer.step()
                model._clip_weights()
                finite_state(core, torch, model, optimizer, f"post-update {next_update}")
                timing["trainingSeconds"] += time.monotonic() - train_started
                window.add(losses.detach().cpu().numpy(), predictions.detach().cpu().numpy(), targets.detach().cpu().numpy(), (predictions / recipe["objective"]["modelOutputToRuntimeScore"]).detach().cpu().numpy(), train_metadata, indices, grad_norm, grad_max, core.update_magnitude(torch, before, model))
                window.add_coverage(torch, batch)
                cursor.advance(count, train_data["records"])
                update = next_update
                examples_seen += count
                if update % recipe["evaluation"]["everyUpdates"] != 0:
                    continue

                validation_started = time.monotonic()
                validation = core.evaluate(torch, model, nnue_dataset, recipe, validation_metadata, device)
                timing["validationSeconds"] += time.monotonic() - validation_started
                validation_receipt = write_validation(root, update, validation, config_hash)
                validation_history.append({"update": update, "validation": validation, "receipt": validation_receipt})
                checkpoint_started = time.monotonic()
                checkpoint_path = root / "checkpoints" / f"u{update:06d}.pt"
                export_path = root / "exports" / f"u{update:06d}.nnue"
                checkpoint = checkpoint_record(core, torch, model, checkpoint_path, update)
                timing["checkpointSeconds"] += time.monotonic() - checkpoint_started
                export_started = time.monotonic()
                exported = core.export_checkpoint(torch, checkpoint_path, export_path, context, recipe, EXPERIMENT_ID)
                timing["exportSeconds"] += time.monotonic() - export_started
                selection_validation = {
                    "records": validation["records"],
                    "order": validation["order"],
                    "teacherOnlyLoss": validation["teacherOnlyLoss"],
                    "finite": True,
                    "receipt": artifact_ref(validation_receipt),
                }
                selection_checkpoint = artifact_ref(checkpoint)
                selection_export = artifact_ref(exported["integerNetwork"])
                retained_item = {
                    "update": update,
                    "validation": validation_receipt,
                    "checkpoint": checkpoint,
                    "export": exported,
                    "selection": {"validation": selection_validation, "checkpoint": selection_checkpoint, "export": selection_export},
                }
                if update in CANDIDATE_UPDATES or update == CONTROL_UPDATE:
                    retained.append(retained_item)
                row = {"schema": "haitaka-anhoku-strength-first-step12-metrics-v1", "optimizerUpdates": update, "learningRate": base_lr, "groupLearningRates": [group["lr"] for group in optimizer.param_groups], "examplesSeen": examples_seen, "dataCursor": {"epoch": cursor.epoch, "record": cursor.record}, "validation": validation, "finiteChecks": True, "window": window.report(), "resource": resource, "timingSeconds": dict(timing)}
                state = build_resume_state(core, torch, model, optimizer, config_hash, update, examples_seen, cursor, validation_history, retained, timing, resource, row)
                core.assert_finite_nested(torch, state, f"committed Step 12 state at update {update}")
                core.atomic_torch_save(torch, state, state_path)
                with (output / "metrics.jsonl").open("ab") as stream:
                    stream.write((json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n").encode())
                    stream.flush()
                    os.fsync(stream.fileno())
                window = core.Window(recipe["objective"])
                if update in CANDIDATE_UPDATES or update == CONTROL_UPDATE:
                    if len(retained) > len(CANDIDATE_UPDATES) + 1:
                        fail("retention contains more than the eight candidates plus control")
                if update >= CONTROL_UPDATE:
                    break
            for old in cache.glob("epoch-*.bin"):
                if old != shuffled:
                    old.unlink(missing_ok=True)

        expected = CANDIDATE_UPDATES + [CONTROL_UPDATE]
        if update != CONTROL_UPDATE or [item["update"] for item in retained] != expected:
            fail("production run did not retain exactly the eight candidates and u2688 control")
        run = {
            "schema": RUN_SCHEMA,
            "createdAt": now(),
            "experimentId": EXPERIMENT_ID,
            "configSha256": config_hash,
            "authorization": {"training": training_authorization, "gpuRental": gpu_authorization},
            "optimizerUpdates": update,
            "scheduleHorizonUpdates": SCHEDULE_HORIZON,
            "executionStopUpdate": CONTROL_UPDATE,
            "validationEvaluations": len(validation_history),
            "expectedValidationEvaluations": 22,
            "examplesSeen": examples_seen,
            "dataCursor": {"epoch": cursor.epoch, "record": cursor.record},
            "retained": retained,
            "complete": True,
            "stopReason": "u2688-control-reached",
            "selectionInput": None,
            "resource": {**resource, "elapsedSeconds": time.monotonic() - started},
            "timingSeconds": timing,
            "attempts": [{"kind": "fresh" if not resume else "resume", "status": "completed"}],
            "deterministicCudaRuntime": runtime_receipt,
            "trainingOrGpuAttempted": True,
            "outcomesOpened": False,
            "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "outcomesOpened": False, "handcraftedRemainsProduction": True},
        }
        selection_items = [
            {"update": item["update"], **item["selection"]}
            for item in retained[:-1]
        ]
        control_selection = {"update": retained[-1]["update"], **retained[-1]["selection"]}
        selection_input = {"schema": "haitaka-anhoku-strength-first-step12-selection-input-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "config": artifact(config_path), "candidates": selection_items, "control": control_selection, "matchEvidenceUsed": False, "flags": run["flags"]}
        selection_path = root / "training" / "selection-input.json"
        atomic_json(selection_path, selection_input)
        run["selectionInput"] = artifact(selection_path)
        atomic_json(output / "run.json", run)
        return run
    except Exception as exc:
        failure = {"schema": "haitaka-anhoku-strength-first-step12-training-failure-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "configSha256": config_hash, "status": "failed", "error": str(exc), "trainingOrGpuAttempted": True, "outcomesOpened": False, "resource": {"elapsedSeconds": time.monotonic() - started}, "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "outcomesOpened": False, "handcraftedRemainsProduction": True}}
        atomic_json(root / "training" / "failure.json", failure)
        raise
    finally:
        if context_temp is not None:
            context_temp.cleanup()
