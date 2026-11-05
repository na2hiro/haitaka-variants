#!/usr/bin/env python3
"""F1 real CPU NNUE/loader/Ranger tiny-overfit and fresh-process resume fixture.

Run `uninterrupted`, `partial`, and `resume` in separate processes for each
arm, then run `verify`. CUDA visibility must be disabled. No paid compute or
scored games are dispatched.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import platform
import sys
import time
from datetime import datetime, timezone
from typing import Any

import strength_first_f1_core as f1
from strength_first_f1_loss import forward, per_record


PATCH_HASH = "f37477059c6833ec20c992a3de715272cdee0026d880837e05595da712c4da7a"
PREFIX_ROWS = 9
BATCH = 5
MAX_UPDATES = 12


def _recipe() -> dict[str, Any]:
    recipe = copy.deepcopy(json.loads(f1.RECIPE.read_text())["recipe"])
    recipe["seed"] = 2026092501
    recipe["width"] = {"l1": 16, "l2": 8, "l3": 8}
    recipe["loader"]["batchSize"] = BATCH
    recipe["evaluation"]["batchSize"] = BATCH
    recipe["schedule"] = {"kind": "linear-warmup-then-cosine-by-optimizer-update-v1", "baseLearningRate": .0015, "minimumLearningRate": .00005, "warmupUpdates": 2, "maximumUpdates": MAX_UPDATES, "firstUpdateLearningRate": .00075, "warmupEndpointLearningRate": .0015, "finalUpdateLearningRate": .00005}
    return recipe


def _prefix(source: Path, output: Path) -> Path:
    f1.require(source.is_file() and source.stat().st_size == f1.INPUTS["trainingBinary"][1] and f1.sha256(source) == f1.INPUTS["trainingBinary"][2], "fixture parent training binary differs")
    data = source.open("rb").read(PREFIX_ROWS * 72)
    f1.require(len(data) == PREFIX_ROWS * 72, "fixture prefix short")
    path = output / "fixture.bin"
    if path.exists():
        f1.require(path.read_bytes() == data, "fixture prefix changed")
    else:
        output.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    return path


def _evaluate(core: Any, torch: Any, model: Any, loader: Any, recipe: dict[str, Any], source: Path, tau: float, arm: str) -> dict[str, Any]:
    count, total_h, total_p = 0, 0.0, 0.0
    prediction = hashlib.sha256()
    model.eval()
    with torch.no_grad():
        for batch in core.finite_batches(loader, recipe["featureFamily"], source, PREFIX_ROWS, BATCH, torch.device("cpu"), recipe["loader"]):
            us, them, white, white_values, black, black_values, _outcome, score, psqt, stacks = batch
            raw = model(us, them, white, white_values, black, black_values, psqt, stacks)
            tensors = per_record(torch, raw, score, tau)
            if arm == "C":
                _mean, old_losses, old_predictions, old_targets = core.loss_tensors(torch, model, batch, recipe["objective"])
                f1.require(torch.equal(old_losses, tensors["huber"]) and torch.equal(old_predictions, tensors["prediction"]) and torch.equal(old_targets, tensors["target"]), "C Huber differs from production core")
            total_h += float(tensors["huber"].sum())
            total_p += float(tensors["probability"].sum())
            count += int(tensors["huber"].numel())
            prediction.update(tensors["prediction"].detach().cpu().numpy().astype("<f8").tobytes())
    f1.require(count == PREFIX_ROWS, "fixture validation record count differs")
    return {"records": count, "huber": total_h/count, "probability": total_p/count, "objective": (total_h if arm == "C" else total_p)/count, "predictionSha256": prediction.hexdigest()}


def _train(core: Any, torch: Any, model_module: Any, features: Any, loader: Any, recipe: dict[str, Any], source: Path, output: Path, tau: float, arm: str, mode: str, context: Path) -> dict[str, Any]:
    started_at = datetime.now(timezone.utc).isoformat()
    started_monotonic = time.monotonic()
    core.seed_everything(torch, 2026092501)
    feature_set = features.get_feature_set_from_name(recipe["featureFamily"])
    model = model_module.NNUE(feature_set=feature_set, lambda_=1.0, learning_rate=.0015).to(torch.device("cpu"))
    optimizer = core.configured_optimizer(model, recipe)
    model._clip_weights()
    initial_identity = {
        "model": hashlib.sha256(b"".join(parameter.detach().cpu().contiguous().numpy().tobytes() for parameter in model.parameters())).hexdigest(),
        "optimizer": hashlib.sha256(repr(optimizer.state_dict()).encode()).hexdigest(),
        "torchRng": hashlib.sha256(torch.random.get_rng_state().numpy().tobytes()).hexdigest(),
    }
    cursor = core.Cursor()
    update = 0
    seen = 0
    trace = []
    if mode == "resume":
        state = torch.load(output / "partial-state.pt", map_location="cpu", weights_only=False)
        f1.require(state["schema"] == "haitaka-anhoku-strength-first-f1-cpu-fixture-state-v1" and state["arm"] == arm and state["update"] == 1, "partial fixture state differs")
        model.load_state_dict(state["model"])
        optimizer.load_state_dict(state["optimizer"])
        core.restore_rng(torch, state["rng"])
        cursor = core.Cursor(**state["cursor"])
        update, seen, trace = state["update"], state["examplesSeen"], list(state["trace"])
    initial = _evaluate(core, torch, model, loader, recipe, source, tau, arm) if update == 0 else None
    target = 0 if mode == "initial" else 1 if mode == "partial" else MAX_UPDATES
    order_dir = output / f"orders-{mode}"
    order_dir.mkdir(parents=True, exist_ok=True)
    while update < target:
        order = core.epoch_permutation(PREFIX_ROWS, 2026092501, cursor.epoch)
        shuffled = order_dir / f"epoch-{cursor.epoch:04d}.bin"
        core.materialize_epoch(source, shuffled, PREFIX_ROWS, 72, order)
        for batch in core.finite_batches(loader, recipe["featureFamily"], shuffled, PREFIX_ROWS, BATCH, torch.device("cpu"), recipe["loader"], skip=cursor.record):
            if update >= target:
                break
            count = int(batch[0].shape[0])
            indices = order[cursor.record:cursor.record+count]
            next_update = update+1
            model.train(); model._clip_weights()
            lr = core.set_learning_rates(optimizer, recipe, next_update)
            optimizer.zero_grad(set_to_none=True)
            objective, _losses = forward(torch, model, batch, tau, arm)
            objective.backward()
            grad_l2, grad_max = core.gradient_magnitude(torch, model)
            optimizer.step(); model._clip_weights()
            core.assert_finite_training_state(torch, model, optimizer, f"F1 CPU fixture update {next_update}")
            trace.append({"update": next_update, "epoch": cursor.epoch, "record": cursor.record, "records": count, "indicesSha256": hashlib.sha256(indices.astype("<i8").tobytes()).hexdigest(), "learningRate": lr, "loss": float(objective.detach()), "gradL2": grad_l2, "gradMax": grad_max})
            cursor.advance(count, PREFIX_ROWS)
            update = next_update
            seen += count
    final = _evaluate(core, torch, model, loader, recipe, source, tau, arm)
    state = {"schema": "haitaka-anhoku-strength-first-f1-cpu-fixture-state-v1", "arm": arm, "update": update, "model": model.state_dict(), "optimizer": optimizer.state_dict(), "rng": core.rng_state(torch), "cursor": {"epoch": cursor.epoch, "record": cursor.record}, "examplesSeen": seen, "trace": trace}
    state_path = output / f"{mode}-state.pt"
    core.atomic_torch_save(torch, state, state_path)
    checkpoint_path = output / f"{mode}-model.pt"
    core.save_portable_model(torch, model, checkpoint_path)
    core.install_cpu_fixture_cupy_module(context)
    export = core.export_checkpoint(torch, checkpoint_path, output / f"{mode}.nnue", context, recipe, f1.EXPERIMENT_ID + f"-cpu-fixture-{arm}")
    report = {"schema": "haitaka-anhoku-strength-first-f1-cpu-fixture-stage-v2", "arm": arm, "mode": mode, "initialIdentity": initial_identity, "initial": initial, "final": final, "update": update, "examplesSeen": seen, "cursor": state["cursor"], "trace": trace, "state": f1.artifact(state_path), "checkpoint": f1.artifact(checkpoint_path), "export": export["integerNetwork"], "host": socket_name(), "processId": os.getpid(), "startedAtUtc": started_at, "endedAtUtc": datetime.now(timezone.utc).isoformat(), "durationSeconds": time.monotonic() - started_monotonic, "gpuOccurred": False}
    f1.immutable_json(output / f"{mode}.json", report)
    return report


def socket_name() -> str:
    import socket
    return socket.gethostname()


def run_stage(repo: Path, external: Path, patch: Path, source: Path, output: Path, arm: str, mode: str, tau: float, cmake: str) -> dict[str, Any]:
    f1.require(os.environ.get("CUDA_VISIBLE_DEVICES") in {"", "-1"}, "CPU fixture requires CUDA visibility disabled")
    f1.require(f1.sha256(patch) == PATCH_HASH, "GNU patch binding differs")
    f1.require(subprocess_revision(external) == "61666d9e3653e4df9881b14c23f8fdcc4bf7779b", "trainer revision differs")
    fixture_source = _prefix(source, output)
    import r5_trainer as r5
    import strength_first_trainer as core
    from strength_first_step12_context import prepare_context
    import torch
    f1.require(not torch.cuda.is_available(), "CPU fixture sees a CUDA device")
    recipe = _recipe()
    context_temp, context = prepare_context(repo_root=repo, root=output, external=external, patch_binding={"path": str(patch), "bytes": patch.stat().st_size, "sha256": PATCH_HASH}, width=recipe["width"], r5=r5, cmake=cmake, purpose=f"f1-cpu-{arm}-{mode}", experiment_id=f1.EXPERIMENT_ID, assembly_schema="haitaka-anhoku-strength-first-f1-context-assembly-v1")
    shims = []
    try:
        torch, features, model_module, loader = core.import_cpu_fixture_trainer(context)
        shims.append(core.install_cpu_feature_transformer(model_module, torch))
        shims.append(core.install_cpu_batch_tensors(loader, torch))
        return _train(core, torch, model_module, features, loader, recipe, fixture_source, output, tau, arm, mode, context)
    finally:
        if len(shims) == 2:
            shims[1][0].get_tensors = shims[1][1]
        if shims:
            shims[0][0].forward = shims[0][1]
        context_temp.cleanup()


def subprocess_revision(external: Path) -> str:
    import subprocess
    return subprocess.check_output(["git", "-C", str(external), "rev-parse", "HEAD"], text=True).strip()


def verify(output: Path, arm: str) -> dict[str, Any]:
    import torch
    import numpy as np
    def equal(left: Any, right: Any) -> bool:
        if torch.is_tensor(left) or torch.is_tensor(right):
            return bool(torch.is_tensor(left) and torch.is_tensor(right) and torch.equal(left.detach().cpu(), right.detach().cpu()))
        if isinstance(left, np.ndarray) or isinstance(right, np.ndarray):
            return bool(isinstance(left, np.ndarray) and isinstance(right, np.ndarray) and np.array_equal(left, right))
        if isinstance(left, dict) or isinstance(right, dict):
            return bool(isinstance(left, dict) and isinstance(right, dict) and set(left) == set(right) and all(equal(left[key], right[key]) for key in left))
        if isinstance(left, (list, tuple)) or isinstance(right, (list, tuple)):
            return bool(type(left) is type(right) and len(left) == len(right) and all(equal(a, b) for a, b in zip(left, right)))
        return bool(left == right)
    reports = {mode: json.loads((output / f"{mode}.json").read_text()) for mode in ("uninterrupted", "partial", "resume")}
    states = {mode: torch.load(output / f"{mode}-state.pt", map_location="cpu", weights_only=False) for mode in reports}
    a, b = states["uninterrupted"], states["resume"]
    process_ids = [reports[mode].get("processId") for mode in ("uninterrupted", "partial", "resume")]
    optimizer_steps = [entry.get("step") for entry in a["optimizer"]["state"].values() if isinstance(entry, dict) and "step" in entry]
    checks = {"modelExact": equal(a["model"], b["model"]), "optimizerExact": equal(a["optimizer"], b["optimizer"]), "rngExact": equal(a["rng"], b["rng"]), "cursorExact": a["cursor"] == b["cursor"], "examplesExact": a["examplesSeen"] == b["examplesSeen"], "traceExact": a["trace"] == b["trace"], "exportExact": reports["uninterrupted"]["export"]["sha256"] == reports["resume"]["export"]["sha256"], "predictionExact": reports["uninterrupted"]["final"]["predictionSha256"] == reports["resume"]["final"]["predictionSha256"], "partialBatchObserved": any(row["records"] == 4 for row in a["trace"]), "lossDecreased": reports["resume"]["final"]["objective"] < reports["uninterrupted"]["initial"]["objective"], "freshProcessResume": all(type(pid) is int and pid > 0 for pid in process_ids) and len(set(process_ids)) == 3, "lookaheadExercised": any(int(step) >= 6 for step in optimizer_steps) and bool(a["optimizer"]["state"])}
    passed = all(checks.values()) and a["update"] == b["update"] == MAX_UPDATES and states["partial"]["update"] == 1
    report = {"schema": "haitaka-anhoku-strength-first-f1-real-cpu-fixture-v2", "arm": arm, "passed": passed, "checks": checks, "stageProcessIds": process_ids, "optimizerStepMaximum": max(map(int, optimizer_steps), default=0), "stages": {mode: f1.artifact(output / f"{mode}.json") for mode in reports}, "device": "cpu", "gpuOccurred": False, "trainingOccurred": False, "gamesOccurred": False}
    f1.immutable_json(output / "report.json", report)
    f1.require(passed, f"F1 {arm} CPU fixture did not pass")
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("initial", "uninterrupted", "partial", "resume", "verify"))
    parser.add_argument("--arm", choices=("C", "T"), required=True)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--external", type=Path, required=True)
    parser.add_argument("--patch", type=Path, required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--tau", type=float, required=True)
    parser.add_argument("--cmake", default="cmake")
    args = parser.parse_args()
    started = time.monotonic()
    try:
        if args.mode == "verify":
            result = verify(args.output, args.arm)
        else:
            result = run_stage(args.repo, args.external, args.patch, args.source, args.output, args.arm, args.mode, args.tau, args.cmake)
    except Exception as exc:
        print(f"F1 fixture blocked: {exc}", file=sys.stderr)
        return 2
    print(json.dumps({"arm": args.arm, "mode": args.mode, "passed": result.get("passed", True), "elapsedSeconds": time.monotonic()-started}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
