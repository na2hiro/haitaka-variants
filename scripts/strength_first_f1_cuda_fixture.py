#!/usr/bin/env python3
"""F1 production-width CUDA loss/resume fixture for C and T.

Each stage is a separate process. A fresh, bound fixture/rental grant is
required before importing torch or creating a CUDA context. No scored games.
"""

from __future__ import annotations

import argparse
import copy
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
from typing import Any

import strength_first_f1_core as f1
from strength_first_f1_loss import forward, per_record
from strength_first_f1_train import state_equal, tensor_digest


def _load(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text())
    f1.require(isinstance(value, dict), f"object required: {path}")
    return value


def _grant(root: Path, arm: str, attempt: int, mode: str) -> dict[str, Any]:
    f1.require(arm in ("C", "T") and type(attempt) is int and attempt in (0, 1), "invalid CUDA fixture identity")
    f1.require(os.environ.get("CUBLAS_WORKSPACE_CONFIG") == ":4096:8", "deterministic CUDA workspace setting absent")
    package_path = root / "package.json"
    package = _load(package_path)
    f1.require(package.get("status") == "frozen" and package.get("experimentId") == f1.EXPERIMENT_ID, "F1 package not frozen")
    f1.require(package.get("cpuQualification") == f1.artifact(root / "preflight/cpu-qualification-v2.json"), "CPU qualification package binding differs")
    for item in package.get("files", {}).values():
        f1.verify_artifact(item)
    config_path = root / "training/config.json"
    config = _load(config_path)
    f1.require(package.get("config") == f1.artifact(config_path) and package.get("contract") == f1.artifact(root / "contract.json") and package.get("calibration") == f1.artifact(root / "calibration.json"), "F1 package scientific binding differs")
    f1.frozen_inputs()
    for role in ("execution", "rental"):
        grant = _load(root / f"authorization/{role}.json")
        f1.require(grant.get("schema") == f"haitaka-anhoku-strength-first-f1-{role}-grant-v1" and grant.get("authorized") is True and grant.get("userReference"), f"{role} grant absent")
        f1.require(grant.get("packageSha256") == f1.sha256(package_path) and grant.get("configSha256") == f1.sha256(config_path), f"{role} grant package/config differs")
        f1.require(grant.get("inputHashes") == {name: value[2] for name, value in f1.INPUTS.items()}, f"{role} grant input binding differs")
        f1.require(datetime.fromisoformat(grant["validUntilUtc"].replace("Z", "+00:00")).astimezone(timezone.utc) > datetime.now(timezone.utc), f"{role} grant expired")
        f1.require(os.environ.get("F1_VAST_INSTANCE_ID") and str(grant.get("instanceId")) == os.environ["F1_VAST_INSTANCE_ID"] and grant.get("oneInstance") is True and grant.get("maximumBilledHours", 99) <= 4 and grant.get("maximumAllInUsd", 99) <= 5, f"{role} instance/budget differs")
        if role == "execution":
            f1.require(grant.get("cudaFixturesAuthorized") is True and arm in grant.get("arms", []), "CUDA fixture execution scope absent")
        else:
            f1.require(grant.get("terminationAuthorized") is True, "owned rental shutdown authority absent")
    from strength_first_f1_train import _monitor_gate
    _monitor_gate(root)
    output = root / f"preflight/cuda-qualification/attempt-{attempt:02d}/{arm}"
    f1.require(not (output / f"{mode}.json").exists(), "CUDA fixture stage already exists")
    if mode == "resume":
        f1.require((output / "partial-state.pt").is_file(), "CUDA fixture partial state missing")
    if mode == "verify":
        f1.require(all((output / f"{name}.json").is_file() for name in ("uninterrupted", "partial", "resume")), "CUDA fixture stages incomplete")
    return {"config": config, "output": output, "package": f1.artifact(package_path)}


def _recipe(config: dict[str, Any]) -> dict[str, Any]:
    recipe = copy.deepcopy(config["recipe"])
    f1.require(recipe["width"] == {"l1": 256, "l2": 16, "l3": 32} and recipe["schedule"]["executionStopUpdate"] == 1024 and recipe["schedule"]["horizonUpdates"] == 4096, "production recipe differs")
    recipe["loader"]["batchSize"] = 5
    recipe["evaluation"]["batchSize"] = 5
    return recipe


def _prefix(root: Path, output: Path) -> Path:
    source = f1.ROOT / f1.INPUTS["trainingBinary"][0]
    f1.require(f1.sha256(source) == f1.INPUTS["trainingBinary"][2], "frozen training corpus differs")
    data = source.open("rb").read(9 * 72)
    path = output / "fixture.bin"
    output.mkdir(parents=True, exist_ok=True)
    if path.exists():
        f1.require(path.read_bytes() == data, "CUDA fixture prefix differs")
    else:
        path.write_bytes(data)
    return path


def _evaluate(torch: Any, core: Any, model: Any, loader: Any, recipe: dict[str, Any], source: Path, tau: float, arm: str, device: Any) -> dict[str, Any]:
    model.eval()
    count, huber, probability = 0, 0.0, 0.0
    predictions = hashlib.sha256()
    with torch.no_grad():
        for batch in core.finite_batches(loader, recipe["featureFamily"], source, 9, 5, device, recipe["loader"]):
            us, them, white, white_values, black, black_values, _outcome, score, psqt, stacks = batch
            raw = model(us, them, white, white_values, black, black_values, psqt, stacks)
            losses = per_record(torch, raw, score, tau)
            if arm == "C":
                _mean, old_loss, old_prediction, old_target = core.loss_tensors(torch, model, batch, recipe["objective"])
                f1.require(torch.equal(old_loss, losses["huber"]) and torch.equal(old_prediction, losses["prediction"]) and torch.equal(old_target, losses["target"]), "CUDA C Huber differs from retained core")
            huber += float(losses["huber"].sum().detach().cpu())
            probability += float(losses["probability"].sum().detach().cpu())
            count += int(losses["huber"].numel())
            predictions.update(losses["prediction"].detach().cpu().numpy().astype("<f8").tobytes())
    f1.require(count == 9, "CUDA fixture validation record count differs")
    return {"records": count, "huber": huber / count, "probability": probability / count, "objective": (huber if arm == "C" else probability) / count, "predictionSha256": predictions.hexdigest()}


def _next_draw(torch: Any, core: Any, device: Any) -> str:
    state = core.rng_state(torch)
    try:
        values = torch.rand(8, device=device).detach().cpu().numpy().astype("<f4").tobytes()
        return hashlib.sha256(values).hexdigest()
    finally:
        core.restore_rng(torch, state)


def _stage(root: Path, arm: str, attempt: int, mode: str, gate: dict[str, Any]) -> dict[str, Any]:
    from strength_first_f1_train import _monitor_gate, schedule_view
    _monitor_gate(root)
    import torch
    import r5_trainer as r5
    import strength_first_trainer as core
    import strength_first_step12_cuda_runtime as cuda_runtime
    from strength_first_step12_context import prepare_context
    f1.require(torch.cuda.is_available(), "CUDA fixture device unavailable")
    device = torch.device("cuda:0")
    output = gate["output"]
    source = _prefix(root, output)
    recipe = _recipe(gate["config"])
    tau = float(_load(root / "calibration.json")["tau"])
    f1.require(math.isfinite(tau) and tau > 0, "invalid frozen temperature")
    started = time.monotonic()
    external = Path(gate["config"]["trainer"]["pinnedCheckout"])
    context_temp, context = prepare_context(repo_root=f1.ROOT, root=root, external=external, patch_binding=gate["config"]["trainer"]["patchExecutable"], width=recipe["width"], r5=r5, cmake=gate["config"]["trainer"]["cmake"], purpose=f"f1-cuda-{arm}-{attempt}-{mode}-{os.getpid()}", experiment_id=f1.EXPERIMENT_ID, assembly_schema="haitaka-anhoku-strength-first-f1-context-assembly-v1")
    try:
        torch, features, model_module, loader = r5.import_trainer(context)
        torch.use_deterministic_algorithms(True)
        torch.backends.cuda.matmul.allow_tf32 = False
        torch.backends.cudnn.allow_tf32 = False
        torch.backends.cudnn.benchmark = False
        adapter = cuda_runtime.install(model_module, features, torch, recipe["featureFamily"], binding={"adapter": gate["config"]["trainer"]["cudaAdapter"], "kernel": gate["config"]["trainer"]["cudaKernel"], "profile": gate["config"]["trainer"]["cudaProfile"]})
        f1.immutable_json(output / f"{mode}-cuda-adapter.json", adapter)
        core.seed_everything(torch, 2026092501)
        feature_set = features.get_feature_set_from_name(recipe["featureFamily"])
        model = model_module.NNUE(feature_set=feature_set, lambda_=1.0, learning_rate=.0015).to(device)
        optimizer = core.configured_optimizer(model, recipe)
        model._clip_weights()
        initial_identity = {"model": tensor_digest(torch, model.state_dict()), "optimizer": tensor_digest(torch, optimizer.state_dict()), "rng": tensor_digest(torch, core.rng_state(torch))}
        cursor = core.Cursor()
        update = 0
        seen = 0
        trace: list[dict[str, Any]] = []
        if mode == "resume":
            state = torch.load(output / "partial-state.pt", map_location=device, weights_only=False)
            f1.require(state.get("schema") == "haitaka-anhoku-strength-first-f1-cuda-fixture-state-v1" and state.get("arm") == arm and state.get("update") == 1, "CUDA partial state differs")
            model.load_state_dict(state["model"])
            optimizer.load_state_dict(state["optimizer"])
            core.restore_rng(torch, state["rng"])
            cursor = core.Cursor(**state["cursor"])
            update, seen, trace = state["update"], state["examplesSeen"], list(state["trace"])
        initial = _evaluate(torch, core, model, loader, recipe, source, tau, arm, device) if update == 0 else None
        private_recipe = schedule_view(recipe)
        target = 1 if mode == "partial" else 12
        order_dir = output / f"orders-{mode}"
        order_dir.mkdir(parents=True, exist_ok=True)
        while update < target:
            _monitor_gate(root)
            order = core.epoch_permutation(9, 2026092501, cursor.epoch)
            shuffled = order_dir / f"epoch-{cursor.epoch:04d}.bin"
            core.materialize_epoch(source, shuffled, 9, 72, order)
            for batch in core.finite_batches(loader, recipe["featureFamily"], shuffled, 9, 5, device, recipe["loader"], skip=cursor.record):
                if update >= target:
                    break
                _monitor_gate(root)
                count = int(batch[0].shape[0])
                indices = order[cursor.record:cursor.record+count]
                next_update = update + 1
                model.train(); model._clip_weights()
                lr = core.set_learning_rates(optimizer, private_recipe, next_update)
                optimizer.zero_grad(set_to_none=True)
                loss, tensors = forward(torch, model, batch, tau, arm)
                loss.backward()
                grad_l2, grad_max = core.gradient_magnitude(torch, model)
                optimizer.step(); model._clip_weights(); torch.cuda.synchronize(device)
                core.assert_finite_training_state(torch, model, optimizer, f"F1 CUDA fixture {arm} update {next_update}")
                trace.append({"update": next_update, "epoch": cursor.epoch, "record": cursor.record, "records": count, "indicesSha256": hashlib.sha256(indices.astype("<i8").tobytes()).hexdigest(), "learningRate": lr, "loss": float(loss.detach().cpu()), "gradientL2": grad_l2, "gradientMaximum": grad_max})
                cursor.advance(count, 9)
                update = next_update
                seen += count
        final = _evaluate(torch, core, model, loader, recipe, source, tau, arm, device)
        next_draw = _next_draw(torch, core, device)
        state = {"schema": "haitaka-anhoku-strength-first-f1-cuda-fixture-state-v1", "arm": arm, "update": update, "model": model.state_dict(), "optimizer": optimizer.state_dict(), "rng": core.rng_state(torch), "cursor": {"epoch": cursor.epoch, "record": cursor.record}, "examplesSeen": seen, "trace": trace, "nextDraw": next_draw}
        state_path = output / f"{mode}-state.pt"
        core.atomic_torch_save(torch, state, state_path)
        checkpoint_path = output / f"{mode}-model.pt"
        core.save_portable_model(torch, model, checkpoint_path)
        export = core.export_checkpoint(torch, checkpoint_path, output / f"{mode}.nnue", context, recipe, f1.EXPERIMENT_ID + f"-cuda-fixture-{arm}")
        report = {"schema": "haitaka-anhoku-strength-first-f1-cuda-fixture-stage-v1", "arm": arm, "mode": mode, "attempt": attempt, "packageSha256": gate["package"]["sha256"], "processId": os.getpid(), "host": socket.gethostname(), "device": torch.cuda.get_device_name(device), "torchVersion": torch.__version__, "cudaVersion": torch.version.cuda, "initialIdentity": initial_identity, "initial": initial, "final": final, "update": update, "examplesSeen": seen, "cursor": state["cursor"], "trace": trace, "nextDraw": next_draw, "state": f1.artifact(state_path), "checkpoint": f1.artifact(checkpoint_path), "export": export["integerNetwork"], "adapter": f1.artifact(output / f"{mode}-cuda-adapter.json"), "durationSeconds": time.monotonic()-started, "gpuOccurred": True, "productionTrainingOccurred": False, "gamesOccurred": False}
        f1.immutable_json(output / f"{mode}.json", report)
        return report
    finally:
        context_temp.cleanup()


def _verify(root: Path, arm: str, attempt: int, gate: dict[str, Any]) -> dict[str, Any]:
    import torch
    output = gate["output"]
    stages = {mode: _load(output / f"{mode}.json") for mode in ("uninterrupted", "partial", "resume")}
    states = {mode: torch.load(output / f"{mode}-state.pt", map_location="cpu", weights_only=False) for mode in stages}
    for stage in stages.values():
        for key in ("state", "checkpoint", "export", "adapter"):
            f1.verify_artifact(stage[key])
    a, b = states["uninterrupted"], states["resume"]
    steps = [item.get("step") for item in a["optimizer"]["state"].values() if isinstance(item, dict) and "step" in item]
    checks = {"modelExact": state_equal(torch, a["model"], b["model"]), "optimizerExact": state_equal(torch, a["optimizer"], b["optimizer"]), "rngExact": state_equal(torch, a["rng"], b["rng"]), "nextDrawExact": a["nextDraw"] == b["nextDraw"], "cursorExact": a["cursor"] == b["cursor"] and a["examplesSeen"] == b["examplesSeen"], "traceExact": a["trace"] == b["trace"], "exportExact": stages["uninterrupted"]["export"]["sha256"] == stages["resume"]["export"]["sha256"], "predictionExact": stages["uninterrupted"]["final"]["predictionSha256"] == stages["resume"]["final"]["predictionSha256"], "freshProcessResume": len({stages[mode]["processId"] for mode in stages}) == 3, "partialBatchObserved": any(item["records"] == 4 for item in a["trace"]), "lookaheadExercised": any(int(step) >= 6 for step in steps), "lossDecreased": stages["resume"]["final"]["objective"] < stages["uninterrupted"]["initial"]["objective"]}
    f1.require(a["update"] == b["update"] == 12 and states["partial"]["update"] == 1 and all(checks.values()), f"{arm} CUDA fixture equivalence failed")
    receipt = {"schema": "haitaka-anhoku-strength-first-f1-cuda-qualification-v1", "experimentId": f1.EXPERIMENT_ID, "arm": arm, "attempt": attempt, "device": "cuda", "passed": True, "packageSha256": gate["package"]["sha256"], "checks": checks, "stages": {mode: f1.artifact(output / f"{mode}.json") for mode in stages}, "productionTrainingOccurred": False, "scoredGamesOccurred": False}
    final = root / f"preflight/cuda-qualification/{arm}/report.json"
    f1.immutable_json(final, receipt)
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("uninterrupted", "partial", "resume", "verify"))
    parser.add_argument("--arm", choices=("C", "T"), required=True)
    parser.add_argument("--attempt", type=int, choices=(0, 1), default=0)
    parser.add_argument("--root", type=Path, default=f1.F1_ROOT)
    args = parser.parse_args()
    try:
        gate = _grant(args.root.resolve(), args.arm, args.attempt, args.mode)
        if args.mode == "verify":
            result = _verify(args.root.resolve(), args.arm, args.attempt, gate)
        else:
            lock_path = args.root.resolve() / ".paid-active.lock"
            with lock_path.open("a+b") as lock:
                try:
                    fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError as exc:
                    raise f1.F1Error("another F1 paid CUDA process is active") from exc
                gate = _grant(args.root.resolve(), args.arm, args.attempt, args.mode)
                result = _stage(args.root.resolve(), args.arm, args.attempt, args.mode, gate)
    except (f1.F1Error, OSError, KeyError, TypeError, ValueError) as exc:
        print(f"F1 CUDA fixture blocked: {exc}", file=sys.stderr)
        return 2
    print(json.dumps({"arm": args.arm, "mode": args.mode, "passed": result.get("passed", True)}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
