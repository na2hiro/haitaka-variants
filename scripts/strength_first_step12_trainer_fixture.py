#!/usr/bin/env python3
"""Step 12 owned real CPU fixture worker.

The parent adapter starts this file in a fresh subprocess and gives it a
unique, non-existent output directory.  This worker creates that directory,
performs the nine-record fixture against the pinned NNUE/Ranger/loader stack,
and writes either a passed report or preserved failure evidence.  It never
calls a historical orchestration entry point.
"""

from __future__ import annotations

import argparse
import copy
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import sys
import time
from typing import Any

from strength_first_step12_context import ContextAssemblyError, artifact as context_artifact, context_receipt_path, prepare_context
from strength_first_step12_trainer import validate_cuda_grant_document


REPO_ROOT = Path(__file__).resolve().parents[1]
EXPERIMENT_ID = "strength-first-step12-independent-replication-20260916-v1"
REPORT_SCHEMA = "haitaka-anhoku-strength-first-step12-real-cpu-fixture-v1"
FAILURE_SCHEMA = "haitaka-anhoku-strength-first-step12-real-cpu-fixture-failure-v1"
CUDA_REPORT_SCHEMA = "haitaka-anhoku-strength-first-step12-real-cuda-fixture-v1"
CUDA_FAILURE_SCHEMA = "haitaka-anhoku-strength-first-step12-real-cuda-fixture-failure-v1"
CUDA_SUMMARY_SCHEMA = "haitaka-anhoku-strength-first-step12-training-fixtures-cuda-v1"
CUDA_WIDTH = {"l1": 256, "l2": 16, "l3": 32}


class FixtureError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise FixtureError(message)


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        fail(f"cannot load JSON {path}: {exc}")
    if not isinstance(value, dict):
        fail(f"JSON root is not an object: {path}")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def atomic_json(path: Path, value: Any) -> None:
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_bytes(json.dumps(value, indent=2, sort_keys=True).encode("utf-8") + b"\n")
    os.replace(temporary, path)


def artifact(path: Path) -> dict[str, Any]:
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256(path)}


def main_cpu_worker(config_path: Path, output: Path, device: str) -> dict[str, Any]:
    if device != "cpu":
        fail("the Step 12 worker currently supports only the permission-free CPU fixture")
    if os.environ.get("CUDA_VISIBLE_DEVICES") not in {"", "-1"}:
        fail("CPU fixture requires CUDA visibility to be disabled")
    config = load_json(config_path)
    if config.get("experimentId") != EXPERIMENT_ID:
        fail("fixture experiment identity differs")
    root = config_path.resolve().parent.parent
    recipe = config.get("recipe", {})
    fixture = recipe.get("realCpuFixture", {})
    if fixture.get("dataOrigin") != "frozen-training-prefix":
        fail("fixture data origin is not the frozen training prefix")
    if fixture.get("records") != 9 or fixture.get("entryBytes") != 72 or fixture.get("batchSize") != 5:
        fail("fixture shape differs from the bound nine-record 5/4 fixture")
    if fixture.get("width") != {"l1": 16, "l2": 8, "l3": 8} or fixture.get("maximumUpdates") != 12 or fixture.get("interruptAfterUpdates") != 1:
        fail("fixture network or update budget differs from the bound recipe")
    source_ref = recipe.get("data", {}).get("training", {}).get("binary", {})
    source_path = Path(source_ref.get("path", ""))
    if not source_path.is_absolute():
        source_path = (root / source_path).resolve()
    if not source_path.is_file() or source_path.is_symlink():
        fail(f"fixture source is missing or symlinked: {source_path}")
    if source_path.stat().st_size != source_ref.get("bytes") or sha256(source_path) != source_ref.get("sha256"):
        fail("fixture source does not match the hash-bound training binary")
    if output.exists():
        fail(f"fixture output directory already exists: {output}")
    output.mkdir(parents=True)
    started = time.monotonic()
    payload_length = fixture["records"] * fixture["entryBytes"]
    with source_path.open("rb") as stream:
        payload = stream.read(payload_length)
    if len(payload) != payload_length:
        fail("fixture source is shorter than the required prefix")
    fixture_source = output / "fixture.bin"
    fixture_source.write_bytes(payload)

    # These imports intentionally occur only after the Step 12 contract and
    # source hash checks above.  The parent process also sets CUDA visibility
    # off, so the fixture cannot accidentally turn into a device run.
    try:
        import strength_first_trainer as core  # type: ignore
        import r5_trainer as r5  # type: ignore
    except (ImportError, OSError) as exc:
        fail(f"pinned CPU fixture dependencies are unavailable: {exc}")

    context_temp = None
    patched_transformer: tuple[Any, Any] | None = None
    patched_batch: tuple[Any, Any] | None = None
    try:
        external = Path(config["trainer"]["pinnedCheckout"]["path"])
        context_temp, context = prepare_context(
            repo_root=REPO_ROOT,
            root=root,
            external=external,
            patch_binding=config["trainer"]["patchExecutable"],
            width=fixture["width"],
            r5=r5,
            cmake=config.get("environment", {}).get("cmakeExecutable", "cmake"),
            purpose="cpu-fixture",
        )
        torch, features, model_module, nnue_dataset = core.import_cpu_fixture_trainer(context)
        if torch.cuda.is_available():
            fail("CPU fixture imported a CUDA-visible torch environment")
        patched_transformer = core.install_cpu_feature_transformer(model_module, torch)
        patched_batch = core.install_cpu_batch_tensors(nnue_dataset, torch)
        fixture_recipe = core.fixture_recipe(config)
        device_value = torch.device("cpu")
        core.seed_everything(torch, fixture["seed"])
        feature_set = features.get_feature_set_from_name(fixture_recipe["featureFamily"])
        initial_model = model_module.NNUE(feature_set=feature_set, lambda_=fixture_recipe["objective"]["lambda"], learning_rate=fixture_recipe["schedule"]["baseLearningRate"]).to(device_value)
        initial_model._clip_weights()
        initial_loss, initial_predictions = core.fixture_evaluate(torch, initial_model, nnue_dataset, fixture_recipe, fixture_source, fixture["records"], device_value)
        del initial_model

        uninterrupted = core.fixture_train(torch, model_module, features, nnue_dataset, fixture_recipe, fixture_source, output / "orders-uninterrupted", fixture["maximumUpdates"])
        partial = core.fixture_train(torch, model_module, features, nnue_dataset, fixture_recipe, fixture_source, output / "orders-resume", fixture["interruptAfterUpdates"])
        partial_model, partial_optimizer, partial_cursor, partial_examples, partial_trace = partial
        resume_state = {
            # Keep the state shape bound to the historical mathematical
            # helper's documented loader contract while the orchestration
            # and receipt remain Step 12-owned.
            "schema": "haitaka-anhoku-strength-first-real-fixture-state-v1",
            "model": partial_model.state_dict(),
            "optimizer": partial_optimizer.state_dict(),
            "schedule": {"kind": fixture["schedule"]["kind"], "nextOptimizerUpdate": fixture["interruptAfterUpdates"] + 1},
            "rng": core.rng_state(torch),
            "optimizerUpdates": fixture["interruptAfterUpdates"],
            "examplesSeen": partial_examples,
            "dataCursor": {"epoch": partial_cursor.epoch, "record": partial_cursor.record},
            "trace": partial_trace,
        }
        resume_path = output / "actual-resume-state.pt"
        core.atomic_torch_save(torch, resume_state, resume_path)
        del partial_model, partial_optimizer, resume_state
        loaded = torch.load(resume_path, map_location="cpu", weights_only=False)
        resumed = core.fixture_train(torch, model_module, features, nnue_dataset, fixture_recipe, fixture_source, output / "orders-resume", fixture["maximumUpdates"], loaded)

        uninterrupted_model, uninterrupted_optimizer, uninterrupted_cursor, uninterrupted_examples, uninterrupted_trace = uninterrupted
        resumed_model, resumed_optimizer, resumed_cursor, resumed_examples, resumed_trace = resumed
        uninterrupted_loss, uninterrupted_predictions = core.fixture_evaluate(torch, uninterrupted_model, nnue_dataset, fixture_recipe, fixture_source, fixture["records"], device_value)
        resumed_loss, resumed_predictions = core.fixture_evaluate(torch, resumed_model, nnue_dataset, fixture_recipe, fixture_source, fixture["records"], device_value)
        weights_equal = core.nested_equal(torch, uninterrupted_model.state_dict(), resumed_model.state_dict())
        optimizers_equal = core.nested_equal(torch, uninterrupted_optimizer.state_dict(), resumed_optimizer.state_dict())
        predictions_equal = bool(__import__("numpy").array_equal(uninterrupted_predictions, resumed_predictions))
        traces_equal = uninterrupted_trace == resumed_trace
        cursors_equal = uninterrupted_cursor == resumed_cursor and uninterrupted_examples == resumed_examples
        partial_observed = fixture["finiteBatchSizesPerEpoch"][-1] in [item["records"] for item in uninterrupted_trace]

        uninterrupted_checkpoint = output / "uninterrupted-final.pt"
        resumed_checkpoint = output / "resumed-final.pt"
        core.save_portable_model(torch, uninterrupted_model, uninterrupted_checkpoint)
        core.save_portable_model(torch, resumed_model, resumed_checkpoint)
        core.install_cpu_fixture_cupy_module(context)
        uninterrupted_export = core.export_checkpoint(torch, uninterrupted_checkpoint, output / "uninterrupted-final.nnue", context, fixture_recipe, EXPERIMENT_ID + "-real-cpu-fixture")
        resumed_export = core.export_checkpoint(torch, resumed_checkpoint, output / "resumed-final.nnue", context, fixture_recipe, EXPERIMENT_ID + "-real-cpu-fixture")
        exports_equal = uninterrupted_export["integerNetwork"]["sha256"] == resumed_export["integerNetwork"]["sha256"]
        loss_decreased = resumed_loss < initial_loss
        passed = all((weights_equal, optimizers_equal, predictions_equal, traces_equal, cursors_equal, partial_observed, exports_equal, loss_decreased, resumed_loss == uninterrupted_loss))
        report = {
            "schema": REPORT_SCHEMA,
            "createdAt": now(),
            "experimentId": EXPERIMENT_ID,
            "status": "passed" if passed else "failed",
            "device": "cpu",
            "host": {"hostname": __import__("socket").gethostname(), "architecture": __import__("platform").machine()},
            "dataOrigin": "frozen-training-prefix",
            "source": {"parentBinary": artifact(source_path), "offset": 0, "length": payload_length, "prefixSha256": hashlib.sha256(payload).hexdigest(), "fixtureBinary": artifact(fixture_source)},
            "actualComponents": {"pinnedNnue": True, "optimizer": type(resumed_optimizer).__name__, "productionLoader": True, "serializer": True, "contextAssembly": context_artifact(root / "preflight" / "training-context-assembly.json"), "device": "cpu", "fixtureOnlyCuPyImportStub": True, "fixtureOnlyCpuFeatureTransformer": True, "fixtureOnlyCpuTensorTransfer": True},
            "fixture": {"records": fixture["records"], "entryBytes": fixture["entryBytes"], "batchSize": fixture["batchSize"], "observedBatchSizes": [item["records"] for item in resumed_trace], "width": fixture["width"], "seed": fixture["seed"], "maximumUpdates": fixture["maximumUpdates"], "interruptAfterUpdates": fixture["interruptAfterUpdates"], "schedule": fixture["schedule"]},
            "exposure": {"initialRecords": fixture["records"], "uninterruptedExamples": uninterrupted_examples, "resumedExamples": resumed_examples, "finalCursor": {"epoch": resumed_cursor.epoch, "record": resumed_cursor.record}, "trace": uninterrupted_trace},
            "loss": {"initial": initial_loss, "uninterruptedFinal": uninterrupted_loss, "resumedFinal": resumed_loss, "decreased": loss_decreased},
            "resumeEquivalence": {"weightsExact": weights_equal, "optimizerExact": optimizers_equal, "predictionsExact": predictions_equal, "traceExact": traces_equal, "cursorExact": cursors_equal},
            "partialBatchObserved": partial_observed,
            "exportsExact": exports_equal,
            "uninterruptedExport": uninterrupted_export,
            "resumedExport": resumed_export,
            "resumeState": artifact(resume_path),
            "qualificationLimits": {"productionTrainingLoop": "not-exercised", "cudaRngAndOptimizer": "not-exercised", "productionCommitOrdering": "not-exercised", "nineCheckpointRetention": "not-exercised", "runtimeParity": "not-exercised", "matches": "not-exercised"},
            "resource": {"elapsedSeconds": time.monotonic() - started, "productionTrainingAttempted": False, "gpuAttempted": False},
            "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "outcomesOpened": False, "handcraftedRemainsProduction": True},
        }
        atomic_json(output / "report.json", report)
        if not passed:
            fail("real CPU fixture equivalence or tiny-fit requirement failed")
        return report
    except Exception as exc:
        failure = {
            "schema": FAILURE_SCHEMA,
            "createdAt": now(),
            "experimentId": EXPERIMENT_ID,
            "status": "failed",
            "device": device,
            "error": str(exc),
            "source": {"parentBinary": artifact(source_path), "offset": 0, "length": payload_length, "prefixSha256": hashlib.sha256(payload).hexdigest(), "fixtureBinary": artifact(fixture_source)},
            "resource": {"elapsedSeconds": time.monotonic() - started, "productionTrainingAttempted": False, "gpuAttempted": False},
            "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "outcomesOpened": False, "handcraftedRemainsProduction": True},
        }
        atomic_json(output / "failure.json", failure)
        raise
    finally:
        if patched_batch is not None:
            batch_type, original = patched_batch
            batch_type.get_tensors = original
        if patched_transformer is not None:
            transformer, original = patched_transformer
            transformer.forward = original
        if context_temp is not None:
            context_temp.cleanup()


def _fixture_source(config: dict[str, Any], config_path: Path, output: Path) -> tuple[Path, Path, int]:
    """Copy and verify the bounded frozen prefix before device imports."""
    root = config_path.resolve().parent.parent
    recipe = config.get("recipe", {})
    fixture = recipe.get("realCpuFixture", {})
    if fixture.get("dataOrigin") != "frozen-training-prefix":
        fail("CUDA fixture data origin is not the frozen training prefix")
    if fixture.get("records") != 9 or fixture.get("entryBytes") != 72 or fixture.get("batchSize") != 5:
        fail("CUDA fixture shape differs from the bound nine-record 5/4 fixture")
    source_ref = recipe.get("data", {}).get("training", {}).get("binary", {})
    source_path = Path(source_ref.get("path", ""))
    if not source_path.is_absolute():
        source_path = (root / source_path).resolve()
    if not source_path.is_file() or source_path.is_symlink():
        fail(f"CUDA fixture source is missing or symlinked: {source_path}")
    if source_path.stat().st_size != source_ref.get("bytes") or sha256(source_path) != source_ref.get("sha256"):
        fail("CUDA fixture source does not match the hash-bound training binary")
    output.mkdir(parents=True, exist_ok=True)
    payload_length = fixture["records"] * fixture["entryBytes"]
    fixture_source = output / "fixture.bin"
    if fixture_source.exists():
        if fixture_source.is_symlink() or fixture_source.stat().st_size != payload_length:
            fail("existing CUDA fixture prefix is invalid")
        with fixture_source.open("rb") as stream:
            existing_prefix = stream.read()
        with source_path.open("rb") as stream:
            expected_prefix = stream.read(payload_length)
        if hashlib.sha256(existing_prefix).hexdigest() != hashlib.sha256(expected_prefix).hexdigest():
            fail("existing CUDA fixture prefix differs from the frozen source")
    else:
        with source_path.open("rb") as stream:
            payload = stream.read(payload_length)
        if len(payload) != payload_length:
            fail("CUDA fixture source is shorter than the required prefix")
        fixture_source.write_bytes(payload)
    return source_path, fixture_source, payload_length


def _verify_cuda_grant(root: Path, config_path: Path) -> dict[str, Any]:
    auth_path = root / "authorization" / "gpu-rental.json"
    auth = load_json(auth_path)
    validate_cuda_grant_document(auth, observed_instance=os.environ.get("STEP12_VAST_INSTANCE_ID"))
    binding = auth.get("fixtureBinding")
    if not isinstance(binding, dict):
        fail("CUDA fixture grant has no host-specific package binding")
    for role, relative in (("config", "training/config.json"), ("inputs", "training/inputs.json"), ("package", "training/package.json")):
        expected = binding.get(role, {})
        target = root / relative
        if not isinstance(expected, dict) or expected.get("sha256") != sha256(target) or expected.get("bytes") != target.stat().st_size:
            fail(f"CUDA fixture grant is not bound to the current {relative}")
    return auth


def _cpu_state(torch: Any, value: Any) -> Any:
    if torch.is_tensor(value):
        return value.detach().cpu().clone()
    if isinstance(value, dict):
        return {key: _cpu_state(torch, item) for key, item in value.items()}
    if isinstance(value, list):
        return [_cpu_state(torch, item) for item in value]
    if isinstance(value, tuple):
        return tuple(_cpu_state(torch, item) for item in value)
    return value


def _state_digest(torch: Any, value: Any) -> str:
    digest = hashlib.sha256()

    def visit(item: Any) -> None:
        if torch.is_tensor(item):
            tensor = item.detach().cpu().contiguous()
            digest.update(b"tensor:")
            digest.update(str(tensor.dtype).encode("ascii"))
            digest.update(repr(tuple(tensor.shape)).encode("ascii"))
            digest.update(tensor.numpy().tobytes())
            return
        if isinstance(item, dict):
            digest.update(b"dict:")
            for key in sorted(item, key=lambda candidate: repr(candidate)):
                visit(key)
                visit(item[key])
            return
        if isinstance(item, (list, tuple)):
            digest.update(b"tuple:" if isinstance(item, tuple) else b"list:")
            for child in item:
                visit(child)
            return
        digest.update(b"scalar:")
        digest.update(json.dumps(item, sort_keys=True, default=repr).encode("utf-8"))

    visit(value)
    return digest.hexdigest()


def _state_inventory(torch: Any, value: Any) -> dict[str, Any]:
    entries: list[dict[str, Any]] = []

    def visit(item: Any, path: str) -> None:
        if torch.is_tensor(item):
            tensor = item.detach().cpu().contiguous()
            entries.append({"path": path, "kind": "tensor", "dtype": str(tensor.dtype), "shape": list(tensor.shape), "sha256": hashlib.sha256(tensor.numpy().tobytes()).hexdigest()})
            return
        if isinstance(item, dict):
            for key in sorted(item, key=lambda candidate: repr(candidate)):
                visit(key, f"{path}.<key>")
                visit(item[key], f"{path}.{key}")
            return
        if isinstance(item, (list, tuple)):
            for index, child in enumerate(item):
                visit(child, f"{path}[{index}]")
            return
        entries.append({"path": path, "kind": "scalar", "value": json.dumps(item, sort_keys=True, default=repr)})

    visit(value, "root")
    return {"entries": entries}


def _cuda_state_fingerprints(torch: Any) -> list[str]:
    return [hashlib.sha256(item.detach().cpu().numpy().tobytes()).hexdigest() for item in torch.cuda.get_rng_state_all()]


def _cuda_rng_probe(torch: Any, device: Any, seed: int) -> dict[str, Any]:
    profile = "seeded-cuda-save-restore-v2"
    torch.cuda.synchronize(device)
    before = [item.cpu().clone() for item in torch.cuda.get_rng_state_all()]
    try:
        first = torch.rand(32, device=device)
        torch.cuda.synchronize(device)
        torch.cuda.set_rng_state_all(before)
        restored = [item.cpu().clone() for item in torch.cuda.get_rng_state_all()]
        second = torch.rand(32, device=device)
        torch.cuda.synchronize(device)
        exact = bool(torch.equal(first.detach().cpu(), second.detach().cpu()))
        state_before = [hashlib.sha256(item.numpy().tobytes()).hexdigest() for item in before]
        state_after_restore = [hashlib.sha256(item.numpy().tobytes()).hexdigest() for item in restored]
        state_restored_exact = bool(len(restored) == len(before) and restored and all(torch.equal(observed, expected) for observed, expected in zip(restored, before)))
        draw_sha256 = hashlib.sha256(first.detach().cpu().numpy().tobytes()).hexdigest()
        if not exact:
            fail("CUDA RNG save/draw/restore probe was not exact")
        if not state_restored_exact:
            fail("CUDA RNG restore probe did not restore the saved state exactly")
        if not all(len(item) == 64 for item in state_before + state_after_restore) or len(draw_sha256) != 64:
            fail("CUDA RNG probe produced an invalid digest")
        return {"profile": profile, "seed": int(seed), "exact": exact, "stateBefore": state_before, "stateAfterRestore": state_after_restore, "stateRestoredExact": state_restored_exact, "drawSha256": draw_sha256}
    finally:
        torch.cuda.set_rng_state_all(before)


def _cuda_next_draw(torch: Any, device: Any) -> dict[str, Any]:
    torch.cuda.synchronize(device)
    state = [item.cpu().clone() for item in torch.cuda.get_rng_state_all()]
    draw = torch.rand(32, device=device)
    torch.cuda.synchronize(device)
    digest = hashlib.sha256(draw.detach().cpu().numpy().tobytes()).hexdigest()
    torch.cuda.set_rng_state_all(state)
    return {"state": [hashlib.sha256(item.numpy().tobytes()).hexdigest() for item in state], "drawSha256": digest}


def _cuda_optimizer_on_device(torch: Any, optimizer: Any, device: Any) -> bool:
    for value in optimizer.state.values():
        for item in value.values():
            if torch.is_tensor(item) and item.device != device:
                return False
    return True


def _cuda_train(
    core: Any,
    torch: Any,
    model_module: Any,
    features: Any,
    nnue_dataset: Any,
    recipe: dict[str, Any],
    source: Path,
    order_dir: Path,
    device: Any,
    target_update: int,
    resume_state: dict[str, Any] | None = None,
) -> tuple[Any, Any, Any, int, list[dict[str, Any]]]:
    fixture = recipe["realCpuFixture"]
    core.seed_everything(torch, fixture["seed"])
    feature_set = features.get_feature_set_from_name(recipe["featureFamily"])
    model = model_module.NNUE(feature_set=feature_set, lambda_=recipe["objective"]["lambda"], learning_rate=recipe["schedule"]["baseLearningRate"]).to(device)
    optimizer = core.configured_optimizer(model, recipe)
    cursor = core.Cursor()
    update = 0
    examples_seen = 0
    trace: list[dict[str, Any]] = []
    if resume_state is not None:
        model.load_state_dict(resume_state["model"])
        optimizer.load_state_dict(resume_state["optimizer"])
        cursor = core.Cursor(**resume_state["dataCursor"])
        update = int(resume_state["optimizerUpdates"])
        examples_seen = int(resume_state["examplesSeen"])
        trace = list(resume_state["trace"])
        core.restore_rng(torch, resume_state["rng"])
        if not _cuda_optimizer_on_device(torch, optimizer, device):
            fail("CUDA resume loaded optimizer state on the wrong device")
    records = fixture["records"]
    while update < target_update:
        order = core.epoch_permutation(records, fixture["seed"], cursor.epoch)
        shuffled = order_dir / f"epoch-{cursor.epoch:04d}.bin"
        core.materialize_epoch(source, shuffled, records, fixture["entryBytes"], order)
        start_record = cursor.record
        for batch in core.finite_batches(nnue_dataset, recipe["featureFamily"], shuffled, records, fixture["batchSize"], device, recipe["loader"], skip=start_record):
            if update >= target_update:
                break
            count = int(batch[0].shape[0])
            indices = order[cursor.record:cursor.record + count]
            next_update = update + 1
            model.train()
            model._clip_weights()
            base_lr = core.set_learning_rates(optimizer, recipe, next_update)
            optimizer.zero_grad(set_to_none=True)
            loss, _losses, _predictions, _targets = core.loss_tensors(torch, model, batch, recipe["objective"])
            if not bool(torch.isfinite(loss)):
                fail(f"CUDA fixture produced non-finite loss at update {next_update}")
            loss.backward()
            core.gradient_magnitude(torch, model)
            optimizer.step()
            model._clip_weights()
            torch.cuda.synchronize(device)
            core.assert_finite_training_state(torch, model, optimizer, f"CUDA fixture post-update {next_update}")
            trace.append({"update": next_update, "epoch": cursor.epoch, "record": cursor.record, "records": count, "indicesSha256": hashlib.sha256(indices.astype("<i8").tobytes()).hexdigest(), "learningRate": base_lr, "loss": float(loss.detach().cpu())})
            cursor.advance(count, records)
            update = next_update
            examples_seen += count
            if update >= target_update:
                break
    torch.cuda.synchronize(device)
    return model, optimizer, cursor, examples_seen, trace


def _cuda_recipe(config: dict[str, Any]) -> dict[str, Any]:
    base = copy.deepcopy(config)
    base["recipe"]["realCpuFixture"]["width"] = CUDA_WIDTH
    base["recipe"]["realCpuFixture"]["cudaProfile"] = "production-width-cuda-fixture-v1"
    recipe = __import__("strength_first_trainer").fixture_recipe(base)
    recipe["width"] = CUDA_WIDTH
    return recipe


def _cuda_setup(config_path: Path, output: Path, purpose: str) -> tuple[dict[str, Any], Path, Path, int, Any, Any, Any, Any, Any, Any, Any, Any, dict[str, Any]]:
    config = load_json(config_path)
    if config.get("experimentId") != EXPERIMENT_ID:
        fail("CUDA fixture experiment identity differs")
    root = config_path.resolve().parent.parent
    _verify_cuda_grant(root, config_path)
    source_path, fixture_source, payload_length = _fixture_source(config, config_path, output)
    try:
        import strength_first_trainer as core  # type: ignore
        import r5_trainer as r5  # type: ignore
        import strength_first_step12_cuda_runtime as cuda_runtime  # type: ignore
    except (ImportError, OSError) as exc:
        fail(f"pinned CUDA fixture dependencies are unavailable: {exc}")
    external = Path(config["trainer"]["pinnedCheckout"]["path"])
    assembled_purpose = f"{purpose}-{output.name}-{os.getpid()}"
    context_temp, context = prepare_context(
        repo_root=REPO_ROOT,
        root=root,
        external=external,
        patch_binding=config["trainer"]["patchExecutable"],
        width=CUDA_WIDTH,
        r5=r5,
        cmake=config.get("environment", {}).get("cmakeExecutable", "cmake"),
        purpose=assembled_purpose,
    )
    torch, features, model_module, nnue_dataset = r5.import_trainer(context)
    if not torch.cuda.is_available() or torch.cuda.device_count() < 1:
        context_temp.cleanup()
        fail("CUDA fixture requires an available CUDA device")
    device = torch.device("cuda:0")
    if tuple(torch.cuda.get_device_capability(device)) != (8, 6):
        context_temp.cleanup()
        fail("CUDA fixture is not running on the bound RTX 3090 compute capability")
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
        config["recipe"]["featureFamily"],
        binding={"adapter": config.get("trainer", {}).get("cudaAdapter"), "kernel": config.get("trainer", {}).get("cudaKernel"), "profile": config.get("recipe", {}).get("cudaDeterminism", {}).get("profile")},
    )
    runtime_receipt["contextAssembly"] = context_artifact(context_receipt_path(root, assembled_purpose))
    atomic_json(output / "cuda-runtime.json", runtime_receipt)
    return config, source_path, fixture_source, payload_length, context_temp, context, torch, features, model_module, nnue_dataset, core, device, runtime_receipt


def _cuda_resume_segment(config_path: Path, output: Path, resume_path: Path, source_path: Path) -> dict[str, Any]:
    config, _parent_source, fixture_source, payload_length, context_temp, context, torch, features, model_module, nnue_dataset, core, device, runtime_receipt = _cuda_setup(config_path, output, "cuda-fixture-resume")
    try:
        recipe = _cuda_recipe(config)
        state = torch.load(resume_path, map_location="cpu", weights_only=False)
        model, optimizer, cursor, examples, trace = _cuda_train(core, torch, model_module, features, nnue_dataset, recipe, fixture_source, output / "orders-resume", device, 12, state)
        torch.cuda.synchronize(device)
        checkpoint = output / "resumed-final.pt"
        core.save_portable_model(torch, model, checkpoint)
        export = core.export_checkpoint(torch, checkpoint, output / "resumed-final.nnue", context, recipe, EXPERIMENT_ID + "-real-cuda-fixture")
        resumed_loss, resumed_predictions = core.fixture_evaluate(torch, model, nnue_dataset, recipe, fixture_source, recipe["realCpuFixture"]["records"], device)
        _cpu_result = {"model": torch.load(checkpoint, map_location="cpu", weights_only=False).state_dict(), "optimizer": _cpu_state(torch, optimizer.state_dict()), "rng": core.rng_state(torch), "cursor": {"epoch": cursor.epoch, "record": cursor.record}, "examplesSeen": examples, "trace": trace, "loss": resumed_loss, "predictions": resumed_predictions.tolist(), "nextRandomDraw": _cuda_next_draw(torch, device), "export": export}
        result_path = output / "resumed-result.pt"
        core.atomic_torch_save(torch, _cpu_result, result_path)
        report = {"schema": CUDA_REPORT_SCHEMA + "-resume-segment", "status": "passed", "device": "cuda", "resumeState": str(resume_path), "result": artifact(result_path), "checkpoint": artifact(checkpoint), "export": export, "payloadLength": payload_length, "cudaRuntime": runtime_receipt}
        atomic_json(output / "segment.json", report)
        return report
    finally:
        context_temp.cleanup()


def main_cuda_worker(config_path: Path, output: Path, resume_path: Path | None = None, source_path: Path | None = None) -> dict[str, Any]:
    if os.environ.get("CUDA_VISIBLE_DEVICES") in {"", "-1"}:
        fail("CUDA fixture requires CUDA visibility")
    if os.environ.get("CUBLAS_WORKSPACE_CONFIG") != ":4096:8":
        fail("CUDA fixture requires CUBLAS_WORKSPACE_CONFIG=:4096:8")
    if resume_path is not None:
        if source_path is None:
            fail("CUDA resume segment requires its frozen fixture prefix")
        return _cuda_resume_segment(config_path, output, resume_path, source_path)
    config, source, fixture_source, payload_length, context_temp, context, torch, features, model_module, nnue_dataset, core, device, runtime_receipt = _cuda_setup(config_path, output, "cuda-fixture")
    started = time.monotonic()
    try:
        recipe = _cuda_recipe(config)
        fixture_seed = int(recipe["realCpuFixture"]["seed"])
        core.seed_everything(torch, fixture_seed)
        rng_probe = _cuda_rng_probe(torch, device, fixture_seed)
        feature_set = features.get_feature_set_from_name(recipe["featureFamily"])
        core.seed_everything(torch, fixture_seed)
        initial_model = model_module.NNUE(feature_set=feature_set, lambda_=recipe["objective"]["lambda"], learning_rate=recipe["schedule"]["baseLearningRate"]).to(device)
        initial_model._clip_weights()
        initial_state_inventory = {"model": _state_inventory(torch, initial_model.state_dict())}
        initial_loss, _initial_predictions = core.fixture_evaluate(torch, initial_model, nnue_dataset, recipe, fixture_source, recipe["realCpuFixture"]["records"], device)
        del initial_model
        uninterrupted = _cuda_train(core, torch, model_module, features, nnue_dataset, recipe, fixture_source, output / "orders-uninterrupted", device, 12)
        uninterrupted_model, uninterrupted_optimizer, uninterrupted_cursor, uninterrupted_examples, uninterrupted_trace = uninterrupted
        uninterrupted_loss, uninterrupted_predictions = core.fixture_evaluate(torch, uninterrupted_model, nnue_dataset, recipe, fixture_source, recipe["realCpuFixture"]["records"], device)
        uninterrupted_checkpoint = output / "uninterrupted-final.pt"
        core.save_portable_model(torch, uninterrupted_model, uninterrupted_checkpoint)
        uninterrupted_export = core.export_checkpoint(torch, uninterrupted_checkpoint, output / "uninterrupted-final.nnue", context, recipe, EXPERIMENT_ID + "-real-cuda-fixture")
        uninterrupted_rng = core.rng_state(torch)
        uninterrupted_next_draw = _cuda_next_draw(torch, device)

        partial_model, partial_optimizer, partial_cursor, partial_examples, partial_trace = _cuda_train(core, torch, model_module, features, nnue_dataset, recipe, fixture_source, output / "orders-resume", device, 1)
        resume_state = {"schema": "haitaka-anhoku-strength-first-real-cuda-fixture-state-v1", "model": _cpu_state(torch, partial_model.state_dict()), "optimizer": _cpu_state(torch, partial_optimizer.state_dict()), "schedule": {"kind": recipe["schedule"]["kind"], "nextOptimizerUpdate": 2}, "rng": core.rng_state(torch), "optimizerUpdates": 1, "examplesSeen": partial_examples, "dataCursor": {"epoch": partial_cursor.epoch, "record": partial_cursor.record}, "trace": partial_trace}
        resume_path = output / "actual-resume-state.pt"
        core.atomic_torch_save(torch, resume_state, resume_path)
        child_output = output / "resume-child"
        child_command = [sys.executable, str(Path(__file__).resolve()), "--config", str(config_path), "--output-dir", str(child_output), "--device", "cuda", "--worker", "--resume-state", str(resume_path), "--source", str(fixture_source)]
        child_env = os.environ.copy()
        child = __import__("subprocess").run(child_command, cwd=REPO_ROOT, env=child_env, capture_output=True, text=True, check=False)
        (child_output / "wrapper.stdout").parent.mkdir(parents=True, exist_ok=True)
        (child_output / "wrapper.stdout").write_text(child.stdout, encoding="utf-8")
        (child_output / "wrapper.stderr").write_text(child.stderr, encoding="utf-8")
        if child.returncode != 0 or not (child_output / "segment.json").is_file():
            fail(f"CUDA resume child failed: {child.stderr[-2000:]}")
        child_report = load_json(child_output / "segment.json")
        result = torch.load(child_output / "resumed-result.pt", map_location="cpu", weights_only=False)
        resumed_model_state = result["model"]
        resumed_optimizer_state = result["optimizer"]
        weights_equal = core.nested_equal(torch, uninterrupted_model.state_dict(), resumed_model_state)
        optimizers_equal = core.nested_equal(torch, uninterrupted_optimizer.state_dict(), resumed_optimizer_state)
        predictions_equal = bool(__import__("numpy").array_equal(uninterrupted_predictions, __import__("numpy").array(result["predictions"])))
        traces_equal = uninterrupted_trace == result["trace"]
        cursors_equal = {"epoch": uninterrupted_cursor.epoch, "record": uninterrupted_cursor.record} == result["cursor"] and uninterrupted_examples == result["examplesSeen"]
        def rng_equal(left: Any, right: Any) -> bool:
            import numpy as np
            import torch as rng_torch
            if rng_torch.is_tensor(left) or rng_torch.is_tensor(right):
                return bool(rng_torch.is_tensor(left) and rng_torch.is_tensor(right) and rng_torch.equal(left.detach().cpu(), right.detach().cpu()))
            if isinstance(left, np.ndarray) or isinstance(right, np.ndarray):
                return bool(isinstance(left, np.ndarray) and isinstance(right, np.ndarray) and np.array_equal(left, right))
            if isinstance(left, dict) or isinstance(right, dict):
                return bool(isinstance(left, dict) and isinstance(right, dict) and set(left) == set(right) and all(rng_equal(left[key], right[key]) for key in left))
            if isinstance(left, (list, tuple)) or isinstance(right, (list, tuple)):
                return bool(type(left) is type(right) and len(left) == len(right) and all(rng_equal(a, b) for a, b in zip(left, right)))
            return left == right
        rng_equal = rng_equal(uninterrupted_rng, result["rng"])
        next_draw_equal = uninterrupted_next_draw == result["nextRandomDraw"]
        resumed_export = result["export"]
        exports_equal = uninterrupted_export["integerNetwork"]["sha256"] == resumed_export["integerNetwork"]["sha256"]
        partial_observed = 4 in [item["records"] for item in uninterrupted_trace]
        loss_decreased = uninterrupted_loss < initial_loss
        resumed_loss = float(result["loss"])
        passed = all((weights_equal, optimizers_equal, predictions_equal, traces_equal, cursors_equal, rng_equal, next_draw_equal, partial_observed, exports_equal, loss_decreased, resumed_loss == uninterrupted_loss))
        state_inventory_path = output / "state-inventory.json"
        atomic_json(state_inventory_path, {"schema": "haitaka-anhoku-strength-first-step12-cuda-state-inventory-v1", "initial": initial_state_inventory, "uninterrupted": {"model": _state_inventory(torch, uninterrupted_model.state_dict()), "optimizer": _state_inventory(torch, uninterrupted_optimizer.state_dict())}, "resumed": {"model": _state_inventory(torch, resumed_model_state), "optimizer": _state_inventory(torch, resumed_optimizer_state)}, "uninterruptedRngDigest": _state_digest(torch, uninterrupted_rng), "resumedRngDigest": _state_digest(torch, result["rng"]), "uninterruptedTrace": uninterrupted_trace, "resumedTrace": result["trace"], "uninterruptedExportSha256": uninterrupted_export["integerNetwork"]["sha256"], "resumedExportSha256": resumed_export["integerNetwork"]["sha256"]})
        report = {"schema": CUDA_REPORT_SCHEMA, "createdAt": now(), "experimentId": EXPERIMENT_ID, "status": "passed" if passed else "failed", "device": "cuda", "host": {"hostname": __import__("socket").gethostname(), "architecture": __import__("platform").machine()}, "cuda": {"device": torch.cuda.get_device_name(device), "capability": list(torch.cuda.get_device_capability(device)), "torch": torch.__version__, "cudaRuntime": torch.version.cuda}, "dataOrigin": "frozen-training-prefix", "source": {"parentBinary": artifact(source), "offset": 0, "length": payload_length, "prefixSha256": hashlib.sha256(fixture_source.read_bytes()).hexdigest(), "fixtureBinary": artifact(fixture_source)}, "actualComponents": {"pinnedNnue": True, "optimizer": type(uninterrupted_optimizer).__name__, "productionLoader": True, "serializer": True, "realCuPy": True, "device": "cuda", "cpuFixtureShims": False, "deterministicCudaRuntime": runtime_receipt, "contextAssembly": runtime_receipt["contextAssembly"]}, "fixture": {"records": 9, "entryBytes": 72, "batchSize": 5, "observedBatchSizes": [item["records"] for item in uninterrupted_trace], "width": CUDA_WIDTH, "seed": recipe["realCpuFixture"]["seed"], "maximumUpdates": 12, "interruptAfterUpdates": 1, "optimizerStepsAcrossBranches": 24, "schedule": recipe["realCpuFixture"]["schedule"], "profile": "production-width-cuda-fixture-v1"}, "loss": {"initial": initial_loss, "uninterruptedFinal": uninterrupted_loss, "resumedFinal": resumed_loss, "decreased": loss_decreased}, "resumeEquivalence": {"weightsExact": weights_equal, "optimizerExact": optimizers_equal, "predictionsExact": predictions_equal, "traceExact": traces_equal, "cursorExact": cursors_equal, "cudaRngExact": rng_equal, "nextRandomDrawExact": next_draw_equal, "exportsExact": exports_equal}, "cudaRngProbe": rng_probe, "nextRandomDraw": uninterrupted_next_draw, "partialBatchObserved": partial_observed, "uninterruptedExport": uninterrupted_export, "resumedExport": resumed_export, "resumeState": artifact(resume_path), "resumeChild": {"command": child_command, "report": child_report, "result": artifact(child_output / "resumed-result.pt")}, "stateInventory": artifact(state_inventory_path), "crossAttemptStateDigest": {"uninterruptedModelOptimizer": _state_digest(torch, {"model": uninterrupted_model.state_dict(), "optimizer": uninterrupted_optimizer.state_dict()}), "resumedModelOptimizer": _state_digest(torch, {"model": resumed_model_state, "optimizer": resumed_optimizer_state})}, "qualificationLimits": {"productionTrainingLoop": "not-exercised", "productionCommitOrdering": "not-exercised", "nineCheckpointRetention": "not-exercised", "runtimeParity": "not-exercised", "matches": "not-exercised"}, "resource": {"elapsedSeconds": time.monotonic() - started, "productionTrainingAttempted": False, "gpuAttempted": True}, "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "outcomesOpened": False, "handcraftedRemainsProduction": True}}
        atomic_json(output / "report.json", report)
        if not passed:
            fail("CUDA fixture equivalence or bounded-fit requirement failed")
        return report
    except Exception as exc:
        failure = {"schema": CUDA_FAILURE_SCHEMA, "createdAt": now(), "experimentId": EXPERIMENT_ID, "status": "failed", "device": "cuda", "error": str(exc), "resource": {"elapsedSeconds": time.monotonic() - started, "productionTrainingAttempted": False, "gpuAttempted": True}, "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "outcomesOpened": False, "handcraftedRemainsProduction": True}}
        atomic_json(output / "failure.json", failure)
        raise
    finally:
        context_temp.cleanup()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--device", choices=("cpu", "cuda"), required=True)
    parser.add_argument("--worker", action="store_true")
    parser.add_argument("--resume-state", type=Path)
    parser.add_argument("--source", type=Path)
    args = parser.parse_args()
    if not args.worker:
        print("fixture worker requires --worker", file=sys.stderr)
        return 2
    try:
        if args.device == "cuda":
            report = main_cuda_worker(args.config.expanduser().resolve(), args.output_dir.expanduser().resolve(), args.resume_state.expanduser().resolve() if args.resume_state else None, args.source.expanduser().resolve() if args.source else None)
        else:
            report = main_cpu_worker(args.config.expanduser().resolve(), args.output_dir.expanduser().resolve(), args.device)
        print(json.dumps({"status": report["status"], "report": str(args.output_dir / "report.json")}, sort_keys=True))
        return 0
    except (FixtureError, OSError, ImportError, RuntimeError, ValueError, TypeError) as exc:
        print(f"Step 12 {args.device} fixture failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
