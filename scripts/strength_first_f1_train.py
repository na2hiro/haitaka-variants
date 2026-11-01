#!/usr/bin/env python3
"""F1-owned sequential two-arm trainer and validation-only checkpoint producer.

The paid path fails closed before importing torch. CPU fixture use is separate.
Historical trainer entry points are never invoked.
"""

from __future__ import annotations

import argparse
import contextlib
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import socket
import sys
import time
from typing import Any, Mapping

import strength_first_f1_core as f1
from strength_first_f1_loss import forward


STATE_SCHEMA = "haitaka-anhoku-strength-first-f1-resume-state-v1"


def _load(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text())
    f1.require(isinstance(value, dict), f"JSON object required: {path}")
    return value


def _utc(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    f1.require(parsed.tzinfo is not None, "grant timestamp must be timezone-aware")
    return parsed.astimezone(timezone.utc)


def verify_paid_gates(root: Path, *, arm: str, resume: bool) -> dict[str, Any]:
    """Verify both independent user grants before device import or directory creation."""
    f1.require(arm in {"C", "T"}, "invalid arm")
    config_path = root / "training/config.json"
    package_path = root / "package.json"
    contract_path = root / "contract.json"
    calibration_path = root / "calibration.json"
    for path in (config_path, package_path, contract_path, calibration_path):
        f1.require(path.is_file(), f"missing F1 gate artifact: {path}")
    config = _load(config_path)
    f1.require(config.get("experimentId") == f1.EXPERIMENT_ID, "config experiment differs")
    f1.require(config.get("recipe", {}).get("schedule", {}).get("executionStopUpdate") == 1024, "F1 stop differs")
    f1.require(config["recipe"]["schedule"]["horizonUpdates"] == 4096, "F1 horizon differs")
    package = _load(package_path)
    f1.require(package.get("experimentId") == f1.EXPERIMENT_ID and package.get("status") == "frozen", "F1 package is not frozen")
    for item in package.get("files", {}).values():
        f1.verify_artifact(item)
    for name in ("config", "contract", "calibration"):
        actual = f1.artifact(root / ("training/config.json" if name == "config" else f"{name}.json"))
        f1.require(package.get(name) == actual, f"package {name} binding differs")
    cpu_path = root / "preflight/cpu-qualification-v2.json"
    cpu = _load(cpu_path)
    f1.require(cpu.get("passed") is True and package.get("cpuQualification") == f1.artifact(cpu_path), "CPU qualification absent or package binding differs")
    f1.frozen_inputs()
    grants = {}
    for role in ("execution", "rental"):
        path = root / "authorization" / f"{role}.json"
        grant = _load(path)
        f1.require(grant.get("schema") == f"haitaka-anhoku-strength-first-f1-{role}-grant-v1", f"{role} grant schema differs")
        f1.require(grant.get("experimentId") == f1.EXPERIMENT_ID, f"{role} experiment differs")
        f1.require(grant.get("authorized") is True and grant.get("userReference"), f"{role} grant absent")
        f1.require(grant.get("packageSha256") == f1.sha256(package_path), f"{role} package binding differs")
        f1.require(grant.get("configSha256") == f1.sha256(config_path), f"{role} config binding differs")
        f1.require(grant.get("inputHashes") == {name: value[2] for name, value in f1.INPUTS.items()}, f"{role} input hashes differ")
        f1.require(_utc(grant["validUntilUtc"]) > datetime.now(timezone.utc), f"{role} grant expired")
        f1.require(grant.get("oneInstance") is True and grant.get("maximumBilledHours", 99) <= 4 and grant.get("maximumAllInUsd", 99) <= 5, f"{role} cap differs")
        f1.require(os.environ.get("F1_VAST_INSTANCE_ID") and str(grant.get("instanceId")) == os.environ["F1_VAST_INSTANCE_ID"], f"{role} instance binding differs")
        if role == "execution":
            f1.require(grant.get("cudaFixturesAuthorized") is True and grant.get("productionTrainingAuthorized") is True, "production execution scope absent")
            f1.require(grant.get("arms") == ["C", "T"] and grant.get("updatesPerArm") == 1024, "training scope differs")
        else:
            f1.require(grant.get("terminationAuthorized") is True, "owned instance shutdown authority absent")
        grants[role] = f1.artifact(path)
    f1.require(grants["execution"]["sha256"] != grants["rental"]["sha256"], "execution and rental grants must be separate")
    cuda_path = root / f"preflight/cuda-qualification/{arm}/report.json"
    cuda = _load(cuda_path)
    f1.require(cuda.get("passed") is True and cuda.get("arm") == arm and cuda.get("device") == "cuda" and cuda.get("packageSha256") == f1.sha256(package_path), "F1 loss-specific CUDA qualification absent")
    for key in ("modelExact", "optimizerExact", "rngExact", "nextDrawExact", "cursorExact", "traceExact", "exportExact", "freshProcessResume", "partialBatchObserved", "lookaheadExercised"):
        f1.require(cuda.get("checks", {}).get(key) is True, f"CUDA qualification {key} absent")
    _monitor_gate(root)
    if arm == "T":
        control = root / "training/arms/C/run.json"
        f1.require(control.is_file() and _load(control).get("complete") is True, "T requires completed frozen C arm")
    if resume:
        f1.require((root / "training/arms" / arm / "last-state.pt").is_file(), "resume state missing")
    else:
        f1.require(not (root / "training/arms" / arm).exists(), "fresh arm refuses existing output")
    return {"config": config, "configPath": config_path, "package": f1.artifact(package_path), "grants": grants}


def _monitor_gate(root: Path) -> dict[str, Any]:
    """Require a current external billing/heartbeat sample before each update."""
    value = _load(root / "authorization/monitor.json")
    observed = _utc(value["observedAtUtc"])
    age = (datetime.now(timezone.utc) - observed).total_seconds()
    f1.require(0 <= age <= 30, "paid instance heartbeat older than 30 seconds")
    f1.require(os.environ.get("F1_VAST_INSTANCE_ID") and str(value.get("instanceId")) == os.environ["F1_VAST_INSTANCE_ID"], "monitor instance differs")
    f1.require(value.get("status") == "running" and value.get("sshReachable") is True, "paid instance monitor is unhealthy")
    f1.require(value.get("schema") == "haitaka-anhoku-strength-first-f1-paid-monitor-v1" and value.get("experimentId") == f1.EXPERIMENT_ID and value.get("packageSha256") == f1.sha256(root / "package.json"), "paid monitor package binding differs")
    f1.require(value.get("executionGrantSha256") == f1.sha256(root / "authorization/execution.json") and value.get("rentalGrantSha256") == f1.sha256(root / "authorization/rental.json"), "paid monitor grant binding differs")
    f1.require(value.get("gpuQueryStatus") == 0 and isinstance(value.get("gpu"), str) and value["gpu"], "paid GPU telemetry absent")
    f1.require(type(value.get("gpuMemoryUsedMiB")) is int and type(value.get("gpuMemoryTotalMiB")) is int and 0 <= value["gpuMemoryUsedMiB"] <= value["gpuMemoryTotalMiB"] and value["gpuMemoryTotalMiB"] >= 22 * 1024, "paid GPU memory telemetry absent")
    f1.require(type(value.get("gpuUtilizationPercent")) is int and 0 <= value["gpuUtilizationPercent"] <= 100, "paid GPU utilization telemetry absent")
    f1.require(type(value.get("ramAvailableBytes")) is int and value["ramAvailableBytes"] >= 1024**3 and type(value.get("ramTotalBytes")) is int and value["ramTotalBytes"] >= value["ramAvailableBytes"], "paid RAM reserve or telemetry absent")
    f1.require(value.get("ramSource") in ("cgroup-v2", "cgroup-v1", "host-meminfo"), "paid RAM source telemetry absent")
    if value["ramSource"] != "host-meminfo":
        f1.require(type(value.get("ramLimitBytes")) is int and type(value.get("ramUsedBytes")) is int and 0 <= value["ramUsedBytes"] <= value["ramLimitBytes"] and value["ramAvailableBytes"] <= value["ramLimitBytes"] - value["ramUsedBytes"], "paid cgroup RAM headroom differs")
    progress = value.get("lastUpdate")
    if progress is not None:
        f1.require(isinstance(progress, dict) and progress.get("arm") in ("C", "T") and type(progress.get("update")) is int and 1 <= progress["update"] <= 1024, "paid update telemetry differs")
        f1.require(_utc(progress["completedAtUtc"]) <= observed, "paid update timestamp follows monitor sample")
    f1.require(type(value.get("billedHours")) in (int, float) and 0 <= value["billedHours"] < 4, "paid billed-hour cap reached")
    f1.require(type(value.get("allInUsd")) in (int, float) and 0 <= value["allInUsd"] < 5, "paid dollar cap reached")
    f1.require(value.get("freeDiskBytes", 0) >= 16 * 1024**3, "paid instance disk reserve exhausted")
    return value


@contextlib.contextmanager
def _exclusive_arm(root: Path, arm: str, resume: bool):
    # This lock also excludes a second arm on the same paid instance.
    lock_path = root / ".paid-active.lock"
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with lock_path.open("a+b") as stream:
        try:
            fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise f1.F1Error("another F1 paid training process is active") from exc
        try:
            gate = verify_paid_gates(root, arm=arm, resume=resume)
            attempt_index = 1 if resume else 0
            attempt_path = root / f"training/arms/{arm}/attempts/{attempt_index:02d}/launch.json"
            f1.require(not attempt_path.exists(), "F1 arm recovery limit or duplicate dispatch reached")
            f1.immutable_json(attempt_path, {"schema": "haitaka-anhoku-strength-first-f1-arm-attempt-v1", "arm": arm, "attempt": attempt_index, "resume": resume, "startedAtUtc": f1.utc_now(), "processId": os.getpid(), "instanceId": os.environ.get("F1_VAST_INSTANCE_ID")})
            yield attempt_index, gate
        finally:
            fcntl.flock(stream.fileno(), fcntl.LOCK_UN)


def schedule_view(recipe: dict[str, Any]) -> dict[str, Any]:
    schedule = recipe["schedule"]
    f1.require(schedule["horizonUpdates"] == 4096 and schedule["executionStopUpdate"] == 1024, "schedule contract differs")
    return {**recipe, "schedule": {**schedule, "maximumUpdates": 4096, "firstUpdateLearningRate": schedule["baseLearningRate"] / 64, "warmupEndpointLearningRate": schedule["baseLearningRate"], "finalUpdateLearningRate": schedule["minimumLearningRate"]}}


def tensor_digest(torch: Any, value: Any) -> str:
    """Stable full-precision identity independent of torch.save archive metadata."""
    digest = hashlib.sha256()
    def visit(item: Any) -> None:
        if torch.is_tensor(item):
            cpu = item.detach().cpu().contiguous()
            digest.update(b"T")
            digest.update(str(cpu.dtype).encode())
            digest.update(repr(tuple(cpu.shape)).encode())
            digest.update(cpu.numpy().tobytes())
        elif isinstance(item, dict):
            digest.update(b"D")
            for key in sorted(item, key=repr):
                visit(key)
                visit(item[key])
        elif isinstance(item, (tuple, list)):
            digest.update(b"Q" if isinstance(item, tuple) else b"L")
            for child in item:
                visit(child)
        else:
            digest.update(b"S")
            digest.update(repr(item).encode())
    visit(value)
    return digest.hexdigest()


def state_equal(torch: Any, left: Any, right: Any) -> bool:
    import numpy as np
    if torch.is_tensor(left) or torch.is_tensor(right):
        return bool(torch.is_tensor(left) and torch.is_tensor(right) and torch.equal(left.detach().cpu(), right.detach().cpu()))
    if isinstance(left, np.ndarray) or isinstance(right, np.ndarray):
        return bool(isinstance(left, np.ndarray) and isinstance(right, np.ndarray) and np.array_equal(left, right))
    if isinstance(left, dict) or isinstance(right, dict):
        return bool(isinstance(left, dict) and isinstance(right, dict) and set(left) == set(right) and all(state_equal(torch, left[key], right[key]) for key in left))
    if isinstance(left, (tuple, list)) or isinstance(right, (tuple, list)):
        return bool(type(left) is type(right) and len(left) == len(right) and all(state_equal(torch, a, b) for a, b in zip(left, right)))
    return bool(left == right)


def checkpoint_reference(reference: Mapping[str, Any]) -> dict[str, Any]:
    """Persist artifact identity without transient checkpoint-recovery metadata."""
    return {key: reference[key] for key in ("path", "bytes", "sha256")}


def snapshot_identity(state: dict[str, Any]) -> dict[str, Any]:
    """Ignore checkpoint recovery bookkeeping, preserving all training state."""
    return {
        **state,
        "candidates": [
            {
                **candidate,
                "checkpoint": {key: value for key, value in candidate["checkpoint"].items() if key != "recoveredOrphan"},
            }
            for candidate in state["candidates"]
        ],
    }


def evaluate_both(torch: Any, model: Any, loader: Any, recipe: dict[str, Any], tau: float, device: Any, path: Path, records: int) -> dict[str, Any]:
    import strength_first_trainer as core
    total_h = 0.0
    total_p = 0.0
    count = 0
    prediction_at_clip = 0
    sigmoid_endpoint = 0
    sigmoid_tail = 0
    target_clipped = 0
    model.eval()
    with torch.no_grad():
        for batch in core.finite_batches(loader, recipe["featureFamily"], path, records, recipe["evaluation"]["batchSize"], device, recipe["loader"]):
            us, them, white, white_values, black, black_values, _outcome, score, psqt, stacks = batch
            raw = model(us, them, white, white_values, black, black_values, psqt, stacks)
            from strength_first_f1_loss import per_record
            metrics = per_record(torch, raw, score, tau)
            batch_count = int(metrics["huber"].numel())
            total_h += float(metrics["huber"].sum().detach().cpu())
            total_p += float(metrics["probability"].sum().detach().cpu())
            prediction_at_clip += int((metrics["prediction"].abs() >= 12000).sum().detach().cpu())
            sigmoid_endpoint += int(((metrics["predictionSigmoid"] == 0) | (metrics["predictionSigmoid"] == 1)).sum().detach().cpu())
            sigmoid_tail += int(((metrics["predictionSigmoid"] < .01) | (metrics["predictionSigmoid"] > .99)).sum().detach().cpu())
            target_clipped += int((score.abs() >= 12000).sum().detach().cpu())
            count += batch_count
    f1.require(count == records, "validation did not consume all records in source order")
    return {"records": count, "order": "fixed-source-order-once-per-evaluation-v1", "validationHuber": total_h / count, "validationProbability": total_p / count, "predictionAtClipCount": prediction_at_clip, "predictionSigmoidEndpointCount": sigmoid_endpoint, "predictionSigmoidTailCount": sigmoid_tail, "targetAtClipCount": target_clipped, "finite": True}


def _initial_identical(root: Path, arm: str, value: dict[str, Any]) -> None:
    path = root / "training/arms" / arm / "initialization.json"
    f1.immutable_json(path, value)
    if arm == "T":
        control = _load(root / "training/arms/C/initialization.json")
        for key in ("modelSha256", "optimizerSha256", "rngSha256"):
            f1.require(value[key] == control[key], f"C/T initial {key} differs")


def _step_intent(output: Path, attempt: int, update: int, monitor: dict[str, Any]) -> Path:
    path = output / f"attempts/{attempt:02d}/updates/u{update:06d}-intent.json"
    f1.immutable_json(path, {"schema": "haitaka-anhoku-strength-first-f1-physical-update-intent-v1", "attempt": attempt, "update": update, "startedAtUtc": f1.utc_now(), "instanceId": monitor["instanceId"], "monitor": monitor})
    return path


def _step_complete(output: Path, attempt: int, update: int, intent: Path, elapsed: float, row: dict[str, Any]) -> None:
    f1.immutable_json(output / f"attempts/{attempt:02d}/updates/u{update:06d}-complete.json", {"schema": "haitaka-anhoku-strength-first-f1-physical-update-complete-v1", "attempt": attempt, "update": update, "completedAtUtc": f1.utc_now(), "durationSeconds": elapsed, "intent": f1.artifact(intent), "trace": row})


def _run_arm_locked(root: Path, arm: str, resume: bool, attempt: int, gate: dict[str, Any]) -> dict[str, Any]:
    config = gate["config"]
    recipe = config["recipe"]
    calibration = _load(root / "calibration.json")
    tau = float(calibration["tau"])
    f1.require(math.isfinite(tau) and tau > 0 and calibration["data"]["sha256"] == f1.INPUTS["trainingBinary"][2], "temperature binding differs")
    # Device import begins only after both grants and all immutable inputs pass.
    import torch
    import r5_trainer as r5
    import strength_first_trainer as core
    import strength_first_step12_cuda_runtime as cuda_runtime
    from strength_first_step12_context import prepare_context
    f1.require(torch.cuda.is_available(), "approved CUDA device unavailable")
    device = torch.device("cuda")
    output = root / "training/arms" / arm
    output.mkdir(parents=True, exist_ok=True)
    state_path = output / "last-state.pt"
    core.seed_everything(torch, 2026092501)
    external = Path(config["trainer"]["pinnedCheckout"])
    context_temp, context = prepare_context(repo_root=f1.ROOT, root=root, external=external, patch_binding=config["trainer"]["patchExecutable"], width=recipe["width"], r5=r5, cmake=config["trainer"]["cmake"], purpose=f"f1-{arm}-{'resume' if resume else 'fresh'}", experiment_id=f1.EXPERIMENT_ID, assembly_schema="haitaka-anhoku-strength-first-f1-context-assembly-v1")
    try:
        torch, features, model_module, nnue_dataset = r5.import_trainer(context)
        torch.use_deterministic_algorithms(True)
        torch.backends.cuda.matmul.allow_tf32 = False
        torch.backends.cudnn.allow_tf32 = False
        torch.backends.cudnn.benchmark = False
        adapter = cuda_runtime.install(model_module, features, torch, recipe["featureFamily"], binding={"adapter": config["trainer"]["cudaAdapter"], "kernel": config["trainer"]["cudaKernel"], "profile": config["trainer"]["cudaProfile"]})
        f1.immutable_json(output / f"attempts/{attempt:02d}/cuda-adapter.json", adapter)
        core.seed_everything(torch, 2026092501)
        feature_set = features.get_feature_set_from_name(recipe["featureFamily"])
        model = model_module.NNUE(feature_set=feature_set, lambda_=1.0, learning_rate=recipe["schedule"]["baseLearningRate"]).to(device)
        optimizer = core.configured_optimizer(model, recipe)
        model._clip_weights()
        cursor = core.Cursor()
        update = 0
        seen = 0
        trace: list[dict[str, Any]] = []
        validations: list[dict[str, Any]] = []
        candidates: list[dict[str, Any]] = []
        initialized = {"schema": "haitaka-anhoku-strength-first-f1-initialization-v1", "experimentId": f1.EXPERIMENT_ID, "arm": arm, "seed": 2026092501, "modelSha256": tensor_digest(torch, model.state_dict()), "optimizerSha256": tensor_digest(torch, optimizer.state_dict()), "rngSha256": tensor_digest(torch, core.rng_state(torch)), "freshScratch": True}
        if resume:
            state = torch.load(state_path, map_location=device, weights_only=False)
            f1.require(state.get("schema") == STATE_SCHEMA and state.get("arm") == arm and state.get("configSha256") == f1.sha256(gate["configPath"]), "resume state identity differs")
            f1.require(state.get("grants") == gate["grants"], "resume grant identity differs")
            model.load_state_dict(state["model"])
            optimizer.load_state_dict(state["optimizer"])
            core.restore_rng(torch, state["rng"])
            cursor = core.Cursor(**state["cursor"])
            update = state["update"]
            seen = state["examplesSeen"]
            trace = list(state["trace"])
            validations = list(state["validations"])
            candidates = [
                {**candidate, "checkpoint": checkpoint_reference(candidate["checkpoint"])}
                for candidate in state["candidates"]
            ]
            core.assert_finite_training_state(torch, model, optimizer, "F1 resume")
        else:
            _initial_identical(root, arm, initialized)
        training = recipe["data"]["training"]
        validation = recipe["data"]["validation"]
        if update == 0 and not validations:
            initial = evaluate_both(torch, model, nnue_dataset, recipe, tau, device, f1.ROOT / validation["binary"]["path"], 60000)
            validations.append({"update": 0, **initial})
            f1.immutable_json(output / "validation/u000000.json", validations[-1])
            initial_state = {"schema": STATE_SCHEMA, "arm": arm, "configSha256": f1.sha256(gate["configPath"]), "grants": gate["grants"], "model": model.state_dict(), "optimizer": optimizer.state_dict(), "rng": core.rng_state(torch), "update": 0, "examplesSeen": 0, "cursor": {"epoch": 0, "record": 0}, "trace": [], "validations": validations, "candidates": [], "scheduleHorizon": 4096}
            core.atomic_torch_save(torch, initial_state, output / "states/u000000.pt")
            core.atomic_torch_save(torch, initial_state, state_path)
        private_recipe = schedule_view(recipe)
        cache = output / "order-cache"
        cache.mkdir(exist_ok=True)
        started = time.monotonic()
        while update < 1024:
            _monitor_gate(root)
            order = core.epoch_permutation(880000, 2026092501, cursor.epoch)
            shuffled = cache / f"epoch-{cursor.epoch:04d}.bin"
            core.materialize_epoch(f1.ROOT / training["binary"]["path"], shuffled, 880000, 72, order)
            for batch in core.finite_batches(nnue_dataset, recipe["featureFamily"], shuffled, 880000, 16384, device, recipe["loader"], skip=cursor.record):
                monitor = _monitor_gate(root)
                if update == 1024:
                    break
                count = int(batch[0].shape[0])
                indices = order[cursor.record:cursor.record+count]
                next_update = update + 1
                intent = _step_intent(output, attempt, next_update, monitor)
                step_started = time.monotonic()
                model.train()
                model._clip_weights()
                core.assert_finite_nested(torch, model.state_dict(), f"F1 pre-update {next_update}")
                lr = core.set_learning_rates(optimizer, private_recipe, next_update)
                optimizer.zero_grad(set_to_none=True)
                objective, losses = forward(torch, model, batch, tau, arm)
                objective.backward()
                grad_l2, grad_max = core.gradient_magnitude(torch, model)
                optimizer.step()
                model._clip_weights()
                torch.cuda.synchronize(device)
                core.assert_finite_training_state(torch, model, optimizer, f"F1 post-update {next_update}")
                row = {"update": next_update, "epoch": cursor.epoch, "record": cursor.record, "batchRecords": count, "indicesSha256": hashlib.sha256(indices.astype("<i8").tobytes()).hexdigest(), "learningRate": lr, "objective": float(objective.detach().cpu()), "huber": float(losses["huber"].mean().detach().cpu()), "probability": float(losses["probability"].mean().detach().cpu()), "gradientL2": grad_l2, "gradientMaximum": grad_max}
                _step_complete(output, attempt, next_update, intent, time.monotonic() - step_started, row)
                trace.append(row)
                cursor.advance(count, 880000)
                seen += count
                update = next_update
                if update % 128:
                    continue
                metrics = evaluate_both(torch, model, nnue_dataset, recipe, tau, device, f1.ROOT / validation["binary"]["path"], 60000)
                validation_row = {"update": update, **metrics}
                validations.append(validation_row)
                f1.immutable_json(output / f"validation/u{update:06d}.json", validation_row)
                checkpoint_path = output / f"checkpoints/u{update:06d}.pt"
                export_path = output / f"exports/u{update:06d}.nnue"
                checkpoint = checkpoint_reference(core.save_portable_model(torch, model, checkpoint_path))
                export = core.export_checkpoint(torch, checkpoint_path, export_path, context, recipe, f1.EXPERIMENT_ID + f"-{arm}")["integerNetwork"]
                candidate = {"update": update, "validationHuber": metrics["validationHuber"], "validationProbability": metrics["validationProbability"], "checkpoint": checkpoint, "export": export}
                candidates.append(candidate)
                state = {"schema": STATE_SCHEMA, "arm": arm, "configSha256": f1.sha256(gate["configPath"]), "grants": gate["grants"], "model": model.state_dict(), "optimizer": optimizer.state_dict(), "rng": core.rng_state(torch), "update": update, "examplesSeen": seen, "cursor": {"epoch": cursor.epoch, "record": cursor.record}, "trace": trace, "validations": validations, "candidates": candidates, "scheduleHorizon": 4096}
                core.assert_finite_nested(torch, state, f"F1 committed state {update}")
                snapshot = output / f"states/u{update:06d}.pt"
                if snapshot.exists():
                    previous = torch.load(snapshot, map_location=device, weights_only=False)
                    f1.require(state_equal(torch, snapshot_identity(previous), snapshot_identity(state)), f"immutable state snapshot conflicts at {update}")
                else:
                    core.atomic_torch_save(torch, state, snapshot)
                core.atomic_torch_save(torch, state, state_path)
                f1.immutable_json(output / f"commits/u{update:06d}.json", {"update": update, "state": f1.artifact(snapshot), "checkpoint": checkpoint, "export": export, "validation": f1.artifact(output / f"validation/u{update:06d}.json")})
        f1.require([item["update"] for item in candidates] == list(f1.UPDATES), "missing eligible checkpoint")
        f1.require(len(validations) == 9 and seen > 0 and update == 1024, "F1 exposure incomplete")
        run = {"schema": "haitaka-anhoku-strength-first-f1-arm-run-v1", "experimentId": f1.EXPERIMENT_ID, "arm": arm, "complete": True, "updates": update, "examplesSeen": seen, "cursor": {"epoch": cursor.epoch, "record": cursor.record}, "scheduleHorizon": 4096, "validations": validations, "candidates": candidates, "traceSha256": hashlib.sha256(f1.canonical_bytes([{key: row[key] for key in ("update", "epoch", "record", "batchRecords", "indicesSha256", "learningRate")} for row in trace])).hexdigest(), "durationSeconds": time.monotonic()-started, "grants": gate["grants"]}
        f1.immutable_json(output / "run.json", run)
        if arm == "T":
            control = _load(root / "training/arms/C/run.json")
            f1.require(run["traceSha256"] == control["traceSha256"], "C/T loader and LR traces differ")
        return run
    finally:
        context_temp.cleanup()


def run_arm(root: Path, arm: str, resume: bool) -> dict[str, Any]:
    with _exclusive_arm(root, arm, resume) as (attempt, gate):
        return _run_arm_locked(root, arm, resume, attempt, gate)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=f1.F1_ROOT)
    parser.add_argument("--arm", choices=("C", "T"), required=True)
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args(argv)
    try:
        result = run_arm(args.root.resolve(), args.arm, args.resume)
    except (f1.F1Error, OSError, KeyError, ValueError) as exc:
        print(f"F1 training blocked: {exc}", file=sys.stderr)
        return 2
    print(json.dumps({"arm": args.arm, "complete": result["complete"], "updates": result["updates"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
