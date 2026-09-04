#!/usr/bin/env python3
"""Dedicated R5 trainer for the immutable replacement-R4 pilot.

The legacy ``haitaka_learn train`` command intentionally is not used here: it
accepts the older game-count manifest and its trainer objective is the retired
sigmoid loss.  This runner builds a disposable pinned trainer context, uses the
finite C++ stream directly, applies the frozen robust Huber objective, and
records exact physical exposure at every checkpoint.

The command requires ``--device cuda``.  It is therefore safe to invoke during
CPU preparation: it fails before a training batch when CUDA is unavailable.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import random
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path
from typing import Any, Iterable


# Must be set before torch is imported.  The R2 cross-hardware gate found that
# cuBLAS deterministic mode otherwise depends on process startup order.
os.environ.setdefault("CUBLAS_WORKSPACE_CONFIG", ":4096:8")

import numpy as np


ROOT = Path(__file__).resolve().parents[1]
PINNED_TRAINER = "61666d9e3653e4df9881b14c23f8fdcc4bf7779b"
FEATURE_SET = "HalfKAv2^+DonorSingleEff"
BATCH_SIZE = 16384
ENTRY_BYTES = 72
SCORE_CLIP = 12000.0
HUBER_DELTA = 0.10
NNUE_SCORE_SCALE = 600.0


class R5Error(RuntimeError):
    pass


def fail(message: str) -> None:
    raise R5Error(message)


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


def run(command: list[str], cwd: Path, label: str) -> None:
    try:
        subprocess.run(command, cwd=cwd, check=True)
    except (OSError, subprocess.CalledProcessError) as exc:
        fail(f"{label} failed: {exc}")


def workspace_path(value: str) -> Path:
    path = Path(value)
    if path.is_absolute():
        return path
    return ROOT / path


def verify_preflight(report_path: Path, contract_path: Path) -> dict[str, Any]:
    report = load_json(report_path)
    contract = load_json(contract_path)
    if report.get("schema") != "haitaka-anhoku-r5-preflight-report-v1":
        fail("wrong R5 preflight report schema")
    if report.get("readyForTraining") is not True or report.get("trainingStarted") is not False:
        fail("R5 preflight is not a fresh ready-for-training handoff")
    if report.get("r5Authorized") is not False or report.get("passed") is not False:
        fail("preflight report incorrectly claims an R5 decision")
    expected_contract_hash = sha256(contract_path)
    if report.get("contract", {}).get("sha256") != expected_contract_hash:
        fail("preflight report does not bind the supplied R5 contract")
    if contract.get("trainer", {}).get("pinnedRevision") != PINNED_TRAINER:
        fail("R5 contract trainer revision is not the pinned revision")
    if contract.get("optimization", {}).get("batchSize") != BATCH_SIZE:
        fail("R5 trainer only supports the frozen batch size")
    if contract.get("optimization", {}).get("randomFenSkipping") != 0:
        fail("R5 trainer refuses random FEN skipping")
    if contract.get("optimization", {}).get("filtered") is not False:
        fail("R5 trainer refuses filtered input")
    return contract


def write_variant(context: Path) -> None:
    (context / "variant.py").write_text(
        '''RANKS = 9
FILES = 9
SQUARES = RANKS * FILES
KING_SQUARES = RANKS * FILES
PIECE_TYPES = 10
PIECES = 2 * PIECE_TYPES
USE_POCKETS = True
POCKETS = 2 * FILES if USE_POCKETS else 0
RULESET = "anhoku"
DONOR_MODE = "single-front"

PIECE_VALUES = {
    1: 700,
    2: 800,
    3: 400,
    4: 1000,
    5: 100,
    6: 300,
    7: 300,
    8: 500,
    9: 900,
}
''',
        encoding="utf-8",
    )
    (context / "variant.h").write_text(
        """#define FILES 9
#define RANKS 9
#define PIECE_TYPES 10
#define PIECE_COUNT 40
#define POCKETS true
#define KING_SQUARES FILES * RANKS
#define DATA_SIZE 512
#define HAITAKA_DONOR_MODE 2
""",
        encoding="utf-8",
    )


def prepare_context(width: dict[str, int], cmake: str) -> tuple[tempfile.TemporaryDirectory[str], Path]:
    external = (ROOT / "../engine/variant-nnue-pytorch").resolve()
    if not external.is_dir():
        fail(f"pinned trainer checkout is missing: {external}")
    try:
        revision = subprocess.check_output(
            ["git", "-C", str(external), "rev-parse", "HEAD"], text=True
        ).strip()
    except (OSError, subprocess.CalledProcessError) as exc:
        fail(f"cannot inspect trainer checkout: {exc}")
    if revision != PINNED_TRAINER:
        fail(f"trainer checkout is {revision}, expected pinned {PINNED_TRAINER}")

    temp = tempfile.TemporaryDirectory(prefix="haitaka-r5-trainer-")
    root = Path(temp.name)
    context = root / "context"
    context.mkdir()
    archive = root / "trainer.tar"
    try:
        with archive.open("wb") as stream:
            subprocess.run(
                ["git", "-C", str(external), "archive", "--format=tar", PINNED_TRAINER],
                stdout=stream,
                check=True,
            )
    except (OSError, subprocess.CalledProcessError) as exc:
        temp.cleanup()
        fail(f"cannot archive pinned trainer: {exc}")
    run(["tar", "-xf", str(archive), "-C", str(context)], root, "extract pinned trainer")

    for patch_name, label in (
        ("variant-nnue-pytorch-phase7.1.patch", "apply Phase 7.1 trainer patch"),
        ("variant-nnue-pytorch-r5.patch", "apply R5 trainer patch"),
    ):
        run(
            ["patch", "--batch", "--fuzz=3", "-p1", "-i", str(ROOT / "trainer-patches" / patch_name)],
            context,
            label,
        )
    trainer_overlay = ROOT / "haitaka_learn" / "trainer_overlay"
    for name in ("feature_set.py", "features.py", "donor_features.py", "training_data_loader.cpp"):
        shutil.copy2(trainer_overlay / name, context / name)
    shutil.copy2(ROOT / "trainer-patches" / "evaluate.py", context / "evaluate.py")
    write_variant(context)
    os.environ.update({
        "HAITAKA_NNUE_L1": str(width["l1"]),
        "HAITAKA_NNUE_L2": str(width["l2"]),
        "HAITAKA_NNUE_L3": str(width["l3"]),
    })
    run([cmake, "-S", ".", "-B", "build"], context, "configure R5 data loader")
    run([cmake, "--build", "build"], context, "build R5 data loader")
    run([cmake, "--install", "build", "--prefix", "."], context, "install R5 data loader")
    return temp, context


def import_trainer(context: Path):
    os.chdir(context)
    sys.path.insert(0, str(context))
    import torch  # type: ignore
    import features  # type: ignore
    import model as model_module  # type: ignore
    import nnue_dataset  # type: ignore

    return torch, features, model_module, nnue_dataset


def finite_batches(nnue_dataset, filename: Path, records: int, device) -> Iterable[tuple[Any, ...]]:
    dataset = nnue_dataset.SparseBatchDataset(
        FEATURE_SET,
        str(filename),
        BATCH_SIZE,
        cyclic=False,
        num_workers=1,
        filtered=False,
        random_fen_skipping=0,
        device=device,
    )
    seen = 0
    for batch in dataset:
        size = int(batch[0].shape[0])
        if size <= 0 or seen + size > records:
            fail(f"finite loader overran {filename}: {seen}+{size}>{records}")
        seen += size
        yield batch
    if seen != records:
        fail(f"finite loader consumed {seen} records from {filename}, expected {records}")


def robust_loss(torch, model, batch):
    us, them, white, white_values, black, black_values, _outcome, score, psqt, stacks = batch
    raw = model(us, them, white, white_values, black, black_values, psqt, stacks)
    prediction = raw * NNUE_SCORE_SCALE
    target = score.clamp(-SCORE_CLIP, SCORE_CLIP)
    error = (prediction - target) / SCORE_CLIP
    absolute = error.abs()
    loss = torch.where(
        absolute <= HUBER_DELTA,
        0.5 * error.square(),
        HUBER_DELTA * (absolute - 0.5 * HUBER_DELTA),
    )
    return loss.mean(), prediction, target


def optimizer_for(torch, model, learning_rate: float):
    configured = model.configure_optimizers()
    optimizer = configured[0][0] if isinstance(configured[0], list) else configured[0]
    if not hasattr(optimizer, "param_groups"):
        fail("pinned trainer returned an unexpected optimizer")
    set_learning_rate(optimizer, learning_rate, 0)
    return optimizer


def set_learning_rate(optimizer, base: float, update: int) -> float:
    warmup = 16
    step_updates = 128
    gamma = 0.5
    minimum = 0.00005
    if update < warmup:
        value = base * (update + 1) / warmup
    else:
        value = max(minimum, base * gamma ** ((update - warmup) // step_updates))
    for index, group in enumerate(optimizer.param_groups):
        group["lr"] = value / 10.0 if index >= len(optimizer.param_groups) - 2 else value
    return value


def metrics_from_predictions(np_predictions: np.ndarray, np_targets: np.ndarray) -> dict[str, Any]:
    error = (np_predictions - np_targets) / SCORE_CLIP
    absolute = np.abs(error)
    huber = np.where(
        absolute <= HUBER_DELTA,
        0.5 * error * error,
        HUBER_DELTA * (absolute - 0.5 * HUBER_DELTA),
    )
    if len(np_predictions) > 1 and np.std(np_predictions) > 0 and np.std(np_targets) > 0:
        pearson = float(np.corrcoef(np_predictions, np_targets)[0, 1])
        prediction_rank = np.argsort(np.argsort(np_predictions))
        target_rank = np.argsort(np.argsort(np_targets))
        spearman = float(np.corrcoef(prediction_rank, target_rank)[0, 1])
    else:
        pearson = None
        spearman = None
    outside = np.abs(np_targets) > 100.0
    return {
        "records": int(len(np_targets)),
        "teacher_only_loss": float(np.mean(huber)),
        "teacher_only_mae": float(np.mean(absolute)),
        "raw_score_mae": float(np.mean(np.abs(np_predictions - np_targets))),
        "pearson": pearson,
        "spearman": spearman,
        "sign_accuracy_outside_near_zero": float(
            np.mean(np.sign(np_predictions[outside]) == np.sign(np_targets[outside]))
        ) if np.any(outside) else None,
        "near_zero_band": 100.0,
        "prediction_activation_rate": float(np.mean(np.abs(np_predictions) > 1.0)),
        "prediction_clipping_rate": float(np.mean(np.abs(np_predictions) >= SCORE_CLIP)),
    }


def evaluate(torch, model, nnue_dataset, filename: Path, records: int, device) -> dict[str, Any]:
    predictions: list[np.ndarray] = []
    targets: list[np.ndarray] = []
    active_rows: set[int] = set()
    model.eval()
    with torch.no_grad():
        for batch in finite_batches(nnue_dataset, filename, records, device):
            _loss, prediction, target = robust_loss(torch, model, batch)
            predictions.append(prediction.detach().cpu().numpy()[:, 0])
            targets.append(target.detach().cpu().numpy()[:, 0])
            for indices in (batch[2], batch[4]):
                active_rows.update(int(value) for value in indices.detach().cpu().numpy().ravel() if value >= 0)
    prediction = np.concatenate(predictions).astype(np.float64, copy=False)
    target = np.concatenate(targets).astype(np.float64, copy=False)
    result = metrics_from_predictions(prediction, target)
    result["active_rows"] = len(active_rows)
    result["prediction_sha256"] = hashlib.sha256(prediction.astype("<f8").tobytes()).hexdigest()
    return result


def seed_everything(torch, seed: int) -> None:
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)
    if torch.cuda.is_available():
        torch.cuda.manual_seed_all(seed)
    torch.use_deterministic_algorithms(True)
    torch.backends.cudnn.benchmark = False
    torch.backends.cudnn.deterministic = True
    torch.set_num_threads(1)


def checkpoint_model(torch, model, path: Path) -> str:
    torch.save(model, path)
    return sha256(path)


def tensor_devices(torch, root: Any) -> set[str]:
    """Return every torch device reachable from a trusted checkpoint object."""
    devices: set[str] = set()
    pending = [root]
    visited: set[int] = set()
    while pending:
        value = pending.pop()
        identity = id(value)
        if identity in visited:
            continue
        visited.add(identity)
        if torch.is_tensor(value):
            devices.add(str(value.device))
        elif isinstance(value, dict):
            pending.extend(value.keys())
            pending.extend(value.values())
        elif isinstance(value, (list, tuple, set, frozenset)):
            pending.extend(value)
        elif hasattr(value, "__dict__"):
            pending.extend(vars(value).values())
    return devices


def materialize_cpu_checkpoint(torch, source: Path, target: Path) -> str:
    """Atomically write and verify a portable CPU copy for serialize.py."""
    temporary = target.with_name(f".{target.name}.{os.getpid()}.tmp")
    try:
        model = torch.load(source, map_location="cpu", weights_only=False)
        model.cpu()
        devices = tensor_devices(torch, model)
        if devices != {"cpu"}:
            fail(f"CPU checkpoint conversion retained unexpected tensor devices: {sorted(devices)}")
        torch.save(model, temporary)

        # Reload without map_location to validate the device tags persisted in
        # the artifact that will be handed to the unchanged pinned serializer.
        persisted = torch.load(temporary, weights_only=False)
        persisted_devices = tensor_devices(torch, persisted)
        if persisted_devices != {"cpu"}:
            fail(
                "persisted CPU checkpoint has unexpected tensor devices: "
                f"{sorted(persisted_devices)}"
            )
        os.replace(temporary, target)
    except (OSError, RuntimeError, TypeError, ValueError) as exc:
        fail(f"cannot materialize CPU checkpoint {target}: {exc}")
    finally:
        temporary.unlink(missing_ok=True)
    return sha256(target)


def train_static_clone(torch, model_module, nnue_dataset, model, optimizer, static: dict[str, Any], device, output: Path, learning_rate: float) -> dict[str, Any]:
    binary = workspace_path(static["binary"]["path"])
    records = static["records"]
    steps = static["pretrainingUpdates"]
    model.train()
    step = 0
    seen = 0
    losses: list[float] = []
    while step < steps:
        for batch in finite_batches(nnue_dataset, binary, records, device):
            if step >= steps:
                break
            model._clip_weights()
            set_learning_rate(optimizer, learning_rate, step)
            optimizer.zero_grad(set_to_none=True)
            loss, _prediction, _target = robust_loss(torch, model, batch)
            loss.backward()
            optimizer.step()
            step += 1
            seen += int(batch[0].shape[0])
            losses.append(float(loss.detach().cpu()))
    model.eval()
    output.mkdir(parents=True, exist_ok=True)
    report = {
        "schema": "haitaka-anhoku-r5-static-clone-pretraining-v1",
        "recordsPerPass": records,
        "optimizerUpdates": step,
        "examplesSeen": seen,
        "learningRate": learning_rate,
        "teacherOnlyLoss": float(np.mean(losses)),
        "source": static["binary"],
    }
    (output / "static-pretraining.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report


def run_lane(
    torch,
    model_module,
    features,
    nnue_dataset,
    contract: dict[str, Any],
    lane: dict[str, Any],
    learning_rate: float,
    seed: int,
    width: dict[str, int],
    device,
    output: Path,
) -> dict[str, Any]:
    if output.exists():
        fail(f"fresh R5 recipe refuses existing output directory: {output}")
    output.mkdir(parents=True)
    optimization = contract["optimization"]
    pilot = contract["pilot"]["splits"]
    train_file = workspace_path(pilot["training"]["binary"]["path"])
    validation_file = workspace_path(pilot["validation"]["binary"]["path"])
    development_file = workspace_path(pilot["development"]["binary"]["path"])
    train_records = pilot["training"]["records"]
    validation_records = pilot["validation"]["records"]
    development_records = pilot["development"]["records"]

    seed_everything(torch, seed)
    feature_set = features.get_feature_set_from_name(FEATURE_SET)
    model = model_module.NNUE(feature_set=feature_set, lambda_=1.0).to(device)
    optimizer = optimizer_for(torch, model, learning_rate)

    initial_validation = evaluate(torch, model, nnue_dataset, validation_file, validation_records, device)
    initial_development = evaluate(torch, model, nnue_dataset, development_file, development_records, device)
    (output / "untrained-initialization.json").write_text(
        json.dumps(
            {
                "schema": "haitaka-anhoku-r5-untrained-initialization-v1",
                "validation": initial_validation,
                "development": initial_development,
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )

    static_report = None
    if lane["kind"] == "static-clone-full-precision":
        static_report = train_static_clone(
            torch,
            model_module,
            nnue_dataset,
            model,
            optimizer,
            lane["staticClone"],
            device,
            output,
            learning_rate,
        )
        optimizer = optimizer_for(torch, model, learning_rate)

    checkpoints = output / "checkpoints"
    checkpoints.mkdir()
    checkpoint_path = checkpoints / "last.pt"
    checkpoint_state_path = checkpoints / "last-state.pt"
    checkpoint_rows = output / "checkpoint-metrics.jsonl"
    step = 0
    examples_seen = 0
    best_development = math.inf
    stale_validations = 0
    last_reason = "maximum-updates"
    window_loss = 0.0
    window_mae = 0.0
    window_raw_mae = 0.0
    window_records = 0
    window_active_rows: set[int] = set()
    max_updates = optimization["maximumUpdates"]
    min_updates = optimization["minimumUpdates"]
    validation_every = optimization["validateEveryUpdates"]
    checkpoint_every = optimization["checkpointEveryUpdates"]
    records_per_pass = train_records
    checkpoint_count = 0
    plateau_stop = False
    parameter_count = sum(parameter.numel() for parameter in model.parameters())

    with checkpoint_rows.open("w", encoding="utf-8") as rows:
        while step < max_updates:
            for batch in finite_batches(nnue_dataset, train_file, records_per_pass, device):
                if step >= max_updates:
                    break
                model.train()
                model._clip_weights()
                current_lr = set_learning_rate(optimizer, learning_rate, step)
                optimizer.zero_grad(set_to_none=True)
                loss, prediction, target = robust_loss(torch, model, batch)
                loss.backward()
                gradient_norm = float(torch.nn.utils.clip_grad_norm_(model.parameters(), math.inf).detach().cpu())
                optimizer.step()
                # Project before evaluation and checkpointing. Previously the
                # next iteration did this, leaving checkpoint metrics one
                # update ahead of the effective weights exported by the
                # serializer's clipping path.
                model._clip_weights()
                step += 1
                count = int(batch[0].shape[0])
                examples_seen += count
                window_records += count
                window_loss += float(loss.detach().cpu()) * count
                batch_raw_mae = torch.abs(prediction - target).mean()
                window_raw_mae += float(batch_raw_mae.detach().cpu()) * count
                window_mae += float((batch_raw_mae / SCORE_CLIP).detach().cpu()) * count
                window_active_rows.update(
                    int(value)
                    for indices in (batch[2], batch[4])
                    for value in indices.detach().cpu().numpy().ravel()
                    if value >= 0
                )

                if step % validation_every != 0 and step < max_updates:
                    continue
                if step % checkpoint_every != 0 and step < max_updates:
                    continue
                validation = evaluate(torch, model, nnue_dataset, validation_file, validation_records, device)
                development = evaluate(torch, model, nnue_dataset, development_file, development_records, device)
                model_path_hash = checkpoint_model(torch, model, checkpoint_path)
                state = {
                    "schema": "haitaka-anhoku-r5-checkpoint-state-v1",
                    "lane": lane["id"],
                    "learningRateCandidate": learning_rate,
                    "seed": seed,
                    "width": width,
                    "optimizerUpdates": step,
                    "examplesSeen": examples_seen,
                    "optimizer": optimizer.state_dict(),
                    "model": model.state_dict(),
                    "dataCursor": {
                        "trainRecordsSeen": examples_seen,
                        "equivalentPasses": examples_seen / records_per_pass,
                    },
                    "rng": {
                        "python": repr(random.getstate()),
                        "numpy": repr(np.random.get_state()),
                        "torch": torch.random.get_rng_state().cpu().tolist(),
                        "torchCuda": [state.cpu().tolist() for state in torch.cuda.get_rng_state_all()] if torch.cuda.is_available() else None,
                    },
                }
                torch.save(state, checkpoint_state_path)
                row = {
                    "schema": "haitaka-anhoku-r5-checkpoint-metrics-v1",
                    "lane": lane["id"],
                    "learningRateCandidate": learning_rate,
                    "seed": seed,
                    "width": width,
                    "optimizerUpdates": step,
                    "examplesSeen": examples_seen,
                    "equivalentPasses": examples_seen / records_per_pass,
                    "learningRate": current_lr,
                    "train": {
                        "records": window_records,
                        "teacher_only_loss": window_loss / max(1, window_records),
                        "teacher_only_mae": window_mae / max(1, window_records),
                        "raw_score_mae": window_raw_mae / max(1, window_records),
                        "active_rows": len(window_active_rows),
                        "gradient_norm": gradient_norm,
                    },
                    "validation": validation,
                    "development": development,
                    "checkpoint": {
                        "path": str(checkpoint_path.relative_to(ROOT)),
                        "sha256": model_path_hash,
                        "bytes": checkpoint_path.stat().st_size,
                    },
                    "parameterCount": parameter_count,
                    "serializedBytes": None,
                    "fullRefreshLatency": None,
                    "incrementalLatency": None,
                }
                rows.write(json.dumps(row, sort_keys=True) + "\n")
                rows.flush()
                checkpoint_count += 1
                window_loss = 0.0
                window_mae = 0.0
                window_raw_mae = 0.0
                window_records = 0
                window_active_rows = set()
                development_loss = development["teacher_only_loss"]
                if step >= min_updates:
                    if development_loss < best_development - optimization["plateau"]["minimumDelta"]:
                        best_development = development_loss
                        stale_validations = 0
                    else:
                        stale_validations += 1
                    if stale_validations >= optimization["plateau"]["patienceValidations"]:
                        last_reason = "development-plateau"
                        plateau_stop = True
                        break
            if plateau_stop:
                break
            else:
                continue

    if step < min_updates:
        fail(f"{lane['id']} stopped at {step} updates below minimum {min_updates}")
    final_model = checkpoints / "final.pt"
    # Preserve the CUDA-resident resumable checkpoint and give the unchanged
    # pinned serializer a separately verified CPU artifact. Loading with
    # map_location also converts tensor attributes outside model.state_dict().
    final_model_hash = materialize_cpu_checkpoint(torch, checkpoint_path, final_model)
    export = output / "final.nnue"
    run(
        [
            sys.executable,
            "serialize.py",
            str(final_model),
            str(export),
            "--features",
            FEATURE_SET,
            "--description",
            f"Haitaka Anhoku R5 {lane['id']} width {width['l1']}x{width['l2']}x{width['l3']}",
        ],
        Path.cwd(),
        "serialize R5 final network",
    )
    final_validation = evaluate(torch, model, nnue_dataset, validation_file, validation_records, device)
    final_development = evaluate(torch, model, nnue_dataset, development_file, development_records, device)
    report = {
        "schema": "haitaka-anhoku-r5-recipe-run-v1",
        "lane": lane["id"],
        "laneKind": lane["kind"],
        "learningRateCandidate": learning_rate,
        "seed": seed,
        "width": width,
        "optimizerUpdates": step,
        "examplesSeen": examples_seen,
        "equivalentPasses": examples_seen / records_per_pass,
        "stopReason": last_reason,
        "checkpointCount": checkpoint_count,
        "staticPretraining": static_report,
        "finalValidation": final_validation,
        "finalDevelopment": final_development,
        "finalCheckpoint": {
            "path": str(final_model.relative_to(ROOT)),
            "bytes": final_model.stat().st_size,
            "sha256": final_model_hash,
        },
        "exportedInteger": {
            "path": str(export.relative_to(ROOT)),
            "bytes": export.stat().st_size,
            "sha256": sha256(export),
        },
        "finiteExposure": {
            "trainRecordsPerPass": records_per_pass,
            "validationRecordsPerEvaluation": validation_records,
            "developmentRecordsPerEvaluation": development_records,
            "batchSize": BATCH_SIZE,
            "randomFenSkipping": 0,
            "filtered": False,
            "cyclic": False,
            "loaderWorkers": 1,
        },
        "parameterCount": parameter_count,
        "runtimeMetrics": {
            "serializedBytes": export.stat().st_size,
            "fullRefreshLatency": None,
            "incrementalLatency": None,
        },
    }
    (output / "run.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report


def recover_existing_export(
    torch,
    contract: dict[str, Any],
    model_path: Path,
    output: Path,
    context: Path,
    width: dict[str, int],
) -> dict[str, Any]:
    """Finish a lane whose training/checkpoints completed but export failed.

    This recovery path is deliberately export-only.  It consumes the last
    checkpoint and the already-written final checkpoint metrics; it never
    replays training or changes the frozen data exposure.
    """
    metrics_path = output / "checkpoint-metrics.jsonl"
    if not model_path.is_file() or not metrics_path.is_file():
        fail(f"cannot recover export without checkpoint and metrics: {output}")
    rows = [json.loads(line) for line in metrics_path.read_text(encoding="utf-8").splitlines() if line.strip()]
    if not rows:
        fail(f"cannot recover export from an empty metrics file: {metrics_path}")
    last = rows[-1]
    lane_id = last.get("lane")
    lanes = {lane["id"]: lane for lane in contract["initialization"]["lanes"]}
    lane = lanes.get(lane_id)
    if lane is None:
        fail(f"checkpoint metrics contain an unknown lane: {lane_id!r}")

    final_model = output / "checkpoints" / "final.pt"
    try:
        model = torch.load(model_path, map_location="cpu", weights_only=False)
        model.cpu()
        model.eval()
        torch.save(model, final_model)
    except (OSError, RuntimeError, TypeError) as exc:
        fail(f"cannot materialize CPU export checkpoint: {exc}")
    export = output / "final.nnue"
    run(
        [
            sys.executable,
            "serialize.py",
            str(final_model),
            str(export),
            "--features",
            FEATURE_SET,
            "--description",
            f"Haitaka Anhoku R5 {lane_id} width {width['l1']}x{width['l2']}x{width['l3']}",
        ],
        context,
        "recover R5 final network export",
    )
    static_report_path = output / "static-pretraining.json"
    static_report = load_json(static_report_path) if static_report_path.exists() else None
    report = {
        "schema": "haitaka-anhoku-r5-recipe-run-v1",
        "lane": lane_id,
        "laneKind": lane["kind"],
        "learningRateCandidate": last["learningRateCandidate"],
        "seed": last["seed"],
        "width": width,
        "optimizerUpdates": last["optimizerUpdates"],
        "examplesSeen": last["examplesSeen"],
        "equivalentPasses": last["equivalentPasses"],
        "stopReason": "recovered-after-export-failure",
        "checkpointCount": len(rows),
        "staticPretraining": static_report,
        "finalValidation": last["validation"],
        "finalDevelopment": last["development"],
        "finalCheckpoint": {
            "path": str(final_model.relative_to(ROOT)),
            "bytes": final_model.stat().st_size,
            "sha256": sha256(final_model),
        },
        "exportedInteger": {
            "path": str(export.relative_to(ROOT)),
            "bytes": export.stat().st_size,
            "sha256": sha256(export),
        },
        "finiteExposure": {
            "trainRecordsPerPass": contract["optimization"]["trainRecordsPerPass"],
            "validationRecordsPerEvaluation": contract["optimization"]["validationRecords"],
            "developmentRecordsPerEvaluation": contract["optimization"]["developmentRecords"],
            "batchSize": BATCH_SIZE,
            "randomFenSkipping": 0,
            "filtered": False,
            "cyclic": False,
            "loaderWorkers": 1,
        },
        "recovery": "export-only-from-completed-checkpoint",
    }
    (output / "run.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preflight-report", type=Path, required=True)
    parser.add_argument("--contract", type=Path, default=ROOT / "r0/anhoku-reboot/r5-fixed-data-exposure-contract.json")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "out/anhoku-reboot-r5")
    parser.add_argument("--device", choices=("cuda",), required=True)
    parser.add_argument("--width", choices=("current", "compact"), default="current")
    parser.add_argument("--lane", choices=("scratch", "static-clone-search-finetune", "all"), default="all")
    parser.add_argument("--seed", type=int, default=None)
    parser.add_argument("--learning-rate", type=float, default=None)
    parser.add_argument(
        "--skip-existing",
        action="store_true",
        help="reuse recipe directories that already contain a completed run.json",
    )
    parser.add_argument(
        "--export-existing",
        type=Path,
        default=None,
        help="recover only the export/report from an existing last.pt checkpoint",
    )
    parser.add_argument("--cmake", default="cmake")
    args = parser.parse_args()

    contract_path = args.contract if args.contract.is_absolute() else ROOT / args.contract
    preflight_path = args.preflight_report if args.preflight_report.is_absolute() else ROOT / args.preflight_report
    output_dir = args.output_dir if args.output_dir.is_absolute() else ROOT / args.output_dir
    contract = verify_preflight(preflight_path, contract_path)

    # Importing torch is intentionally delayed until after contract binding and
    # CUDA-only validation has been requested.
    try:
        import torch  # type: ignore
    except ImportError as exc:
        print(f"R5 trainer unavailable: {exc}", file=sys.stderr)
        return 2
    if not torch.cuda.is_available():
        print("R5 trainer requires an approved CUDA device; no GPU is available", file=sys.stderr)
        return 2
    device = torch.device("cuda")

    width = contract["architecture"]["currentWidth" if args.width == "current" else "compactWidth"]
    temp, context = prepare_context(width, args.cmake)
    try:
        torch, features, model_module, nnue_dataset = import_trainer(context)
        if args.export_existing is not None:
            checkpoint = args.export_existing if args.export_existing.is_absolute() else ROOT / args.export_existing
            recipe_output = checkpoint.parent.parent
            report = recover_existing_export(torch, contract, checkpoint, recipe_output, context, width)
            print(f"R5 export recovery written to {recipe_output / 'run.json'}")
            return 0
        initialization = contract["initialization"]
        lanes = initialization["lanes"]
        if args.lane != "all":
            lanes = [lane for lane in lanes if lane["id"] == args.lane]
        rates = contract["optimization"]["initialLearningRateCandidates"]
        if args.learning_rate is not None:
            rates = [args.learning_rate]
        seeds = [args.seed if args.seed is not None else initialization["selectionSeed"]]
        recipe_root = output_dir / ("recipes" if args.width == "current" else "compact-recipes")
        reports = []
        for lane in lanes:
            for rate in rates:
                for seed in seeds:
                    name = f"{lane['id']}-lr-{rate:g}-seed-{seed}"
                    recipe_output = recipe_root / name
                    if recipe_output.exists():
                        if not args.skip_existing or not (recipe_output / "run.json").is_file():
                            fail(f"fresh R5 recipe refuses existing or incomplete output directory: {recipe_output}")
                        reports.append(load_json(recipe_output / "run.json"))
                        continue
                    reports.append(run_lane(
                        torch,
                        model_module,
                        features,
                        nnue_dataset,
                        contract,
                        lane,
                        rate,
                        seed,
                        width,
                        device,
                        recipe_output,
                    ))
        summary = {
            "schema": "haitaka-anhoku-r5-training-summary-v1",
            "width": width,
            "recipeRuns": [
                f"{report['lane']}-lr-{report['learningRateCandidate']:g}-seed-{report['seed']}"
                for report in reports
            ],
            "reports": [
                {
                    "lane": report["lane"],
                    "learningRateCandidate": report["learningRateCandidate"],
                    "seed": report["seed"],
                    "path": str((recipe_root / f"{report['lane']}-lr-{report['learningRateCandidate']:g}-seed-{report['seed']}").relative_to(ROOT)),
                    "runSha256": sha256(recipe_root / f"{report['lane']}-lr-{report['learningRateCandidate']:g}-seed-{report['seed']}" / "run.json"),
                }
                for report in reports
            ],
            "trainingStarted": True,
            "r5DecisionMade": False,
        }
        is_selection_matrix = (
            args.width == "current"
            and args.seed in (None, initialization["selectionSeed"])
            and args.lane == "all"
            and args.learning_rate is None
        )
        if is_selection_matrix:
            summary_name = "training-summary.json"
        elif args.width == "compact" and args.seed in (None, initialization["selectionSeed"]):
            summary_name = "compact-training-summary.json"
        else:
            lane_name = args.lane
            rate_name = "all" if args.learning_rate is None else f"{args.learning_rate:g}"
            seed_name = args.seed if args.seed is not None else initialization["selectionSeed"]
            summary_name = (
                f"replication-{args.width}-{lane_name}-lr-{rate_name}-seed-{seed_name}"
                "-training-summary.json"
            )
        summary_path = output_dir / summary_name
        summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(f"R5 training summary written to {summary_path}")
    finally:
        temp.cleanup()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except R5Error as exc:
        print(f"R5 trainer FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
