#!/usr/bin/env python3
"""R2 deterministic CPU/CUDA debug trainer over the production C++ loader.

This is deliberately a compact learnability model, not a candidate Anhoku
network.  It tests the data/exposure/checkpoint contract without spending a GPU
hour or conflating the result with architecture strength.
"""

import argparse
import ctypes
import hashlib
import json
import math
import os
import pathlib
import random
import struct
import sys
from dataclasses import dataclass

import numpy as np
import torch


ENTRY_BYTES = 72
FEATURE_DIM = 256
FEATURE_SET = "HalfKAv2^+DonorSingleEff"
SCHEDULE = "finite-source-groups-sequential-v1"


def canonical_json(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def write_json(path, value):
    path.write_bytes(json.dumps(value, indent=2, sort_keys=True).encode() + b"\n")


def sha256_bytes(payload):
    return hashlib.sha256(payload).hexdigest()


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def artifact(path):
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256(path)}


class SparseBatch(ctypes.Structure):
    _fields_ = [
        ("num_inputs", ctypes.c_int),
        ("size", ctypes.c_int),
        ("is_white", ctypes.POINTER(ctypes.c_float)),
        ("outcome", ctypes.POINTER(ctypes.c_float)),
        ("score", ctypes.POINTER(ctypes.c_float)),
        ("num_active_white_features", ctypes.c_int),
        ("num_active_black_features", ctypes.c_int),
        ("max_active_features", ctypes.c_int),
        ("white", ctypes.POINTER(ctypes.c_int)),
        ("black", ctypes.POINTER(ctypes.c_int)),
        ("white_values", ctypes.POINTER(ctypes.c_float)),
        ("black_values", ctypes.POINTER(ctypes.c_float)),
        ("psqt_indices", ctypes.POINTER(ctypes.c_int)),
        ("layer_stack_indices", ctypes.POINTER(ctypes.c_int)),
    ]


class ProductionLoader:
    def __init__(self, library, filename, batch_size):
        self.dll = ctypes.cdll.LoadLibrary(str(library))
        self.dll.create_sparse_batch_stream.restype = ctypes.c_void_p
        self.dll.create_sparse_batch_stream.argtypes = [
            ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_int,
            ctypes.c_bool, ctypes.c_bool, ctypes.c_int,
        ]
        self.dll.fetch_next_sparse_batch.restype = ctypes.POINTER(SparseBatch)
        self.dll.fetch_next_sparse_batch.argtypes = [ctypes.c_void_p]
        self.dll.destroy_sparse_batch.argtypes = [ctypes.POINTER(SparseBatch)]
        self.dll.destroy_sparse_batch_stream.argtypes = [ctypes.c_void_p]
        self.stream = self.dll.create_sparse_batch_stream(
            FEATURE_SET.encode(), 1, str(filename).encode(), batch_size,
            False, False, 0,
        )
        if not self.stream:
            raise RuntimeError("production loader rejected the R2 feature family")

    def close(self):
        if self.stream:
            self.dll.destroy_sparse_batch_stream(self.stream)
            self.stream = None

    def __del__(self):
        self.close()

    def next(self):
        pointer = self.dll.fetch_next_sparse_batch(self.stream)
        if not pointer:
            return None
        batch = pointer.contents
        size = batch.size
        width = batch.max_active_features

        def copy(pointer_value, shape, dtype):
            return np.ctypeslib.as_array(pointer_value, shape=shape).astype(dtype, copy=True)

        result = {
            "us": copy(batch.is_white, (size, 1), np.float32),
            "outcome": copy(batch.outcome, (size, 1), np.float32),
            "score": copy(batch.score, (size, 1), np.float32),
            "white": copy(batch.white, (size, width), np.int64),
            "black": copy(batch.black, (size, width), np.int64),
            "white_values": copy(batch.white_values, (size, width), np.float32),
            "black_values": copy(batch.black_values, (size, width), np.float32),
            "psqt": copy(batch.psqt_indices, (size,), np.int64),
            "layer_stack": copy(batch.layer_stack_indices, (size,), np.int64),
        }
        self.dll.destroy_sparse_batch(pointer)
        return result


def slice_batch(batch, start):
    return {key: value[start:].copy() for key, value in batch.items()}


def batch_identity(batch):
    digest = hashlib.sha256()
    for key in sorted(batch):
        value = np.ascontiguousarray(batch[key])
        digest.update(key.encode() + b"\0")
        digest.update(str(value.dtype).encode() + b"\0")
        digest.update(struct.pack("<I", value.ndim))
        for extent in value.shape:
            digest.update(struct.pack("<Q", extent))
        digest.update(value.tobytes())
    return digest.hexdigest()


def hashed_features(batch):
    size = len(batch["score"])
    dense = np.zeros((size, FEATURE_DIM), dtype=np.float32)
    us = batch["us"][:, 0]
    for color in ("white", "black"):
        indices = batch[color]
        values = batch[color + "_values"]
        valid = indices >= 0
        rows, columns = np.nonzero(valid)
        source = indices[rows, columns]
        buckets = source % FEATURE_DIM
        hash_sign = np.where(((source // FEATURE_DIM) & 1) == 0, 1.0, -1.0)
        color_sign = np.where(
            (us[rows] > 0.5) == (color == "white"), 1.0, -1.0
        )
        np.add.at(dense, (rows, buckets), values[rows, columns] * hash_sign * color_sign)
    active = np.maximum(1.0, np.count_nonzero(dense, axis=1)).astype(np.float32)
    dense /= np.sqrt(active)[:, None]
    dense[:, 0] += np.where(us > 0.5, 0.25, -0.25)
    dense[:, 1] += batch["psqt"].astype(np.float32) / 8.0
    return dense


def synthetic_target(features):
    # A deliberately low-dimensional target that is exactly representable by
    # the debug model and exercised by every ordinary position. R2 asks whether
    # the production loader/optimizer path can learn, not whether sparse tail
    # rows generalize across held-out openings in 96 updates.
    weights = np.zeros(FEATURE_DIM, dtype=np.float64)
    weights[:8] = np.asarray([0.30, -0.20, 0.15, -0.12, 0.10, -0.08, 0.06, -0.04])
    return (features.astype(np.float64) @ weights + 0.075).astype(np.float32)[:, None]


def split_source(corpus_bin, corpus_jsonl, output_dir):
    payload = corpus_bin.read_bytes()
    if len(payload) == 0 or len(payload) % ENTRY_BYTES:
        raise RuntimeError("R2 source corpus is not a non-empty 72-byte record stream")
    rows = [json.loads(line) for line in corpus_jsonl.read_text().splitlines()]
    if len(rows) * ENTRY_BYTES != len(payload):
        raise RuntimeError("R2 JSONL and packed corpus lengths disagree")
    by_id = {row["id"]: row for row in rows}
    roots = {}
    for row in rows:
        current = row
        seen = set()
        while current.get("parentId") is not None:
            if current["id"] in seen:
                raise RuntimeError("cycle in R1-A trajectory ancestry")
            seen.add(current["id"])
            current = by_id[current["parentId"]]
        roots[row["id"]] = current["id"]
    root_ids = sorted(set(roots.values()))
    validation_roots = {
        root for root in root_ids
        if int(hashlib.sha256(("r2-val-v1:" + root).encode()).hexdigest()[:8], 16) % 5 == 0
    }
    if not validation_roots or validation_roots == set(root_ids):
        raise RuntimeError("R2 source-group split is degenerate")
    split_records = {"training-validation": [], "development-test": []}
    split_indices = {"training-validation": [], "development-test": []}
    for index, row in enumerate(rows):
        split = "development-test" if roots[row["id"]] in validation_roots else "training-validation"
        split_records[split].append(payload[index * ENTRY_BYTES:(index + 1) * ENTRY_BYTES])
        split_indices[split].append(index)
    paths = {}
    for name, records in split_records.items():
        path = output_dir / (name + ".bin")
        path.write_bytes(b"".join(records))
        paths[name] = path
    manifest = {
        "schema": "haitaka-r2-source-group-split-v1",
        "assignment": "sha256('r2-val-v1:' + rootId) modulo 5; zero is development-test",
        "sourceGroups": len(root_ids),
        "trainingValidationGroups": len(root_ids) - len(validation_roots),
        "developmentTestGroups": len(validation_roots),
        "canonicalAndColorSwapPolicy": "ancestry root is indivisible; derived copies must inherit rootId",
        "splits": {
            name: {
                "records": len(indices),
                "sourceIndexSha256": sha256_bytes(b"".join(struct.pack("<Q", i) for i in indices)),
                "artifact": artifact(paths[name]),
            }
            for name, indices in split_indices.items()
        },
        "overlapGroups": 0,
    }
    manifest_path = output_dir / "source-group-split.json"
    write_json(manifest_path, manifest)
    return paths, manifest_path


@dataclass
class Cursor:
    epoch: int = 0
    record: int = 0


class EpochStream:
    def __init__(self, library, filename, batch_size, cursor):
        self.library = library
        self.filename = filename
        self.batch_size = batch_size
        self.physical_records = filename.stat().st_size // ENTRY_BYTES
        self.cursor = cursor
        self.loader = None
        self.pending = None
        self._open_at_cursor()

    def _open_at_cursor(self):
        self.loader = ProductionLoader(self.library, self.filename, self.batch_size)
        remaining = self.cursor.record
        while remaining:
            batch = self.loader.next()
            if batch is None:
                raise RuntimeError("resume cursor exceeds physical epoch")
            count = len(batch["score"])
            if remaining >= count:
                remaining -= count
            else:
                self.pending = slice_batch(batch, remaining)
                remaining = 0

    def next(self):
        if self.pending is not None:
            batch, self.pending = self.pending, None
        else:
            batch = self.loader.next()
        if batch is None:
            if self.cursor.record != self.physical_records:
                raise RuntimeError("finite epoch ended at the wrong physical record count")
            self.loader.close()
            self.cursor.epoch += 1
            self.cursor.record = 0
            self._open_at_cursor()
            batch = self.loader.next()
            if batch is None:
                raise RuntimeError("empty training epoch")
        self.cursor.record += len(batch["score"])
        return batch


class DebugModel(torch.nn.Module):
    def __init__(self):
        super().__init__()
        self.linear = torch.nn.Linear(FEATURE_DIM, 1)
        torch.nn.init.zeros_(self.linear.weight)
        torch.nn.init.zeros_(self.linear.bias)

    def forward(self, value):
        return self.linear(value)


def rng_state():
    return {
        "python": random.getstate(),
        "numpy": np.random.get_state(),
        "torch": torch.random.get_rng_state(),
        "torch_cuda": torch.cuda.get_rng_state_all() if torch.cuda.is_available() else None,
    }


def restore_rng(state):
    random.setstate(state["python"])
    np.random.set_state(state["numpy"])
    torch.random.set_rng_state(state["torch"])
    if state.get("torch_cuda") is not None:
        torch.cuda.set_rng_state_all(state["torch_cuda"])


def learning_rate(step, maximum_steps):
    warmup = 8
    maximum = 0.045
    minimum = 0.0045
    if step < warmup:
        return maximum * (step + 1) / warmup
    progress = (step - warmup) / max(1, maximum_steps - warmup - 1)
    return minimum + (maximum - minimum) * 0.5 * (1.0 + math.cos(math.pi * progress))


def canonical_model(path, model):
    header = canonical_json({
        "schema": "haitaka-r2-debug-reference-v1",
        "dtype": "float32-le",
        "featureDimension": FEATURE_DIM,
        "keys": sorted(model.state_dict()),
    })
    payload = bytearray(header)
    for key in sorted(model.state_dict()):
        value = model.state_dict()[key].detach().cpu().contiguous().numpy().astype("<f4", copy=False)
        payload.extend(struct.pack("<I", len(key)))
        payload.extend(key.encode())
        payload.extend(struct.pack("<I", value.ndim))
        payload.extend(b"".join(struct.pack("<Q", extent) for extent in value.shape))
        payload.extend(value.tobytes())
    path.write_bytes(payload)


def save_checkpoint(path, model, optimizer, step, cursor, batch_hashes, metrics):
    torch.save({
        "schema": "haitaka-r2-debug-checkpoint-v1",
        "model": model.state_dict(),
        "optimizer": optimizer.state_dict(),
        "global_step": step,
        "data_cursor": {"epoch": cursor.epoch, "record": cursor.record},
        "rng": rng_state(),
        # This debug lane deliberately uses float32 on CPU and CUDA. Keeping
        # the field explicit prevents a later mixed-precision lane from
        # silently omitting GradScaler state from the checkpoint contract.
        "scaler": None,
        "batch_hashes": batch_hashes,
        "metrics": metrics,
    }, path)


def load_checkpoint(path, model, optimizer):
    value = torch.load(path, map_location="cpu", weights_only=False)
    if value.get("schema") != "haitaka-r2-debug-checkpoint-v1":
        raise RuntimeError("wrong R2 checkpoint schema")
    model.load_state_dict(value["model"])
    optimizer.load_state_dict(value["optimizer"])
    restore_rng(value["rng"])
    cursor = Cursor(value["data_cursor"]["epoch"], value["data_cursor"]["record"])
    return value["global_step"], cursor, value["batch_hashes"], value["metrics"]


def finite_validation(library, filename, batch_size, model, device):
    loader = ProductionLoader(library, filename, batch_size)
    predictions, targets, raw_scores, active_rows = [], [], [], set()
    batch_hashes = []
    while True:
        batch = loader.next()
        if batch is None:
            break
        batch_hashes.append(batch_identity(batch))
        dense = hashed_features(batch)
        target = synthetic_target(dense)
        with torch.no_grad():
            prediction = model(torch.from_numpy(dense).to(device)).cpu().numpy()
        predictions.append(prediction)
        targets.append(target)
        raw_scores.append(batch["score"])
        for key in ("white", "black"):
            active_rows.update(int(x) for x in batch[key][batch[key] >= 0])
    loader.close()
    prediction = np.concatenate(predictions)[:, 0].astype(np.float64)
    target = np.concatenate(targets)[:, 0].astype(np.float64)
    score = np.concatenate(raw_scores)[:, 0].astype(np.float64)
    error = prediction - target
    teacher_probability = 1.0 / (1.0 + np.exp(-score / 410.0))
    predicted_probability = 1.0 / (1.0 + np.exp(-prediction))
    rank_prediction = np.argsort(np.argsort(prediction))
    rank_target = np.argsort(np.argsort(target))
    band = 0.02
    outside = np.abs(target) > band
    return {
        "records": int(len(target)),
        "batches": len(batch_hashes),
        "batchHashes": batch_hashes,
        "teacherOnlyLoss": float(np.mean(error ** 2)),
        "resultLoss": None,
        "rawRobustScoreError": float(np.mean(np.where(np.abs(error) < 1.0, 0.5 * error ** 2, np.abs(error) - 0.5))),
        "calibratedProbabilityLoss": float(np.mean((predicted_probability - (1.0 / (1.0 + np.exp(-target)))) ** 2)),
        "teacherProbabilityDiagnostic": float(np.mean((predicted_probability - teacher_probability) ** 2)),
        "pearson": float(np.corrcoef(prediction, target)[0, 1]),
        "spearman": float(np.corrcoef(rank_prediction, rank_target)[0, 1]),
        "signAccuracyOutsideNearZeroBand": float(np.mean(np.sign(prediction[outside]) == np.sign(target[outside]))),
        "nearZeroBand": band,
        "predictionActivationRate": float(np.mean(np.abs(prediction) > 1.0e-8)),
        "predictionClippingRate": float(np.mean(np.abs(prediction) >= 4.0)),
        "activeRows": len(active_rows),
        "predictionSha256": sha256_bytes(prediction.astype("<f8").tobytes()),
        "predictionValues": prediction.tolist(),
    }


def run_training(name, root, library, train_file, validation_file, batch_size, maximum_steps, seed, device, stop_step=None, resume=False):
    root.mkdir(parents=True, exist_ok=True)
    checkpoint = root / "checkpoint.pt"
    canonical = root / "model.r2fp"
    report_path = root / "run.json"
    if resume:
        if not checkpoint.is_file():
            raise RuntimeError(f"explicit resume for {name} has no checkpoint")
    elif checkpoint.exists() or canonical.exists():
        raise RuntimeError(f"fresh run {name} refuses an output directory containing training state")
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)
    torch.use_deterministic_algorithms(True)
    torch.set_num_threads(1)
    model = DebugModel().to(device)
    optimizer = torch.optim.AdamW(model.parameters(), lr=0.01, weight_decay=0.0)
    step, cursor, batch_hashes, step_metrics = 0, Cursor(), [], []
    if resume:
        step, cursor, batch_hashes, step_metrics = load_checkpoint(checkpoint, model, optimizer)
        for state in optimizer.state.values():
            for key, value in state.items():
                if torch.is_tensor(value):
                    state[key] = value.to(device)
    starting_step = step
    target_step = min(maximum_steps, stop_step if stop_step is not None else maximum_steps)
    stream = EpochStream(library, train_file, batch_size, cursor)
    while step < target_step:
        batch = stream.next()
        batch_hashes.append(batch_identity(batch))
        dense = torch.from_numpy(hashed_features(batch)).to(device)
        target = torch.from_numpy(synthetic_target(dense.cpu().numpy())).to(device)
        lr = learning_rate(step, maximum_steps)
        for group in optimizer.param_groups:
            group["lr"] = lr
        before = torch.cat([value.detach().flatten() for value in model.parameters()]).clone()
        optimizer.zero_grad(set_to_none=True)
        prediction = model(dense)
        loss = torch.mean(torch.square(prediction - target))
        loss.backward()
        gradient_norm = float(torch.linalg.vector_norm(torch.cat([value.grad.detach().flatten() for value in model.parameters()])))
        optimizer.step()
        after = torch.cat([value.detach().flatten() for value in model.parameters()])
        step_metrics.append({
            "step": step,
            "records": len(batch["score"]),
            "loss": float(loss.detach().cpu()),
            "learningRate": lr,
            "gradientNorm": gradient_norm,
            "updateNorm": float(torch.linalg.vector_norm(after - before)),
            "epoch": cursor.epoch,
            "recordCursor": cursor.record,
        })
        step += 1
    save_checkpoint(checkpoint, model, optimizer, step, cursor, batch_hashes, step_metrics)
    complete = step == maximum_steps
    validation = None
    quantization = None
    if complete:
        validation = finite_validation(library, validation_file, batch_size, model, device)
        repeated = finite_validation(library, validation_file, batch_size, model, device)
        if canonical_json(validation) != canonical_json(repeated):
            raise RuntimeError("finite validation is not repeatable")
        with torch.no_grad():
            original = model.linear.weight.detach().clone()
            model.linear.weight.copy_(torch.round(original * 256.0) / 256.0)
            quantized_validation = finite_validation(library, validation_file, batch_size, model, device)
            model.linear.weight.copy_(original)
        quantization = {
            "scale": 256,
            "fullPrecisionLoss": validation["teacherOnlyLoss"],
            "quantizedLoss": quantized_validation["teacherOnlyLoss"],
            "lossRatio": quantized_validation["teacherOnlyLoss"] / max(validation["teacherOnlyLoss"], 1.0e-30),
            "survived": quantized_validation["teacherOnlyLoss"] <= max(1.0e-5, validation["teacherOnlyLoss"] * 4.0),
        }
        canonical_model(canonical, model)
    report = {
        "schema": "haitaka-r2-debug-training-run-v1",
        "name": name,
        "device": str(device),
        "seed": seed,
        "fresh": not resume,
        "startingGlobalStep": starting_step,
        "endingGlobalStep": step,
        "maximumSteps": maximum_steps,
        "complete": complete,
        "schedule": SCHEDULE,
        "randomFenSkipping": 0,
        "loaderConcurrency": 1,
        "physicalTrainRecords": train_file.stat().st_size // ENTRY_BYTES,
        "recordsTrained": int(sum(item["records"] for item in step_metrics)),
        "equivalentPasses": sum(item["records"] for item in step_metrics) / (train_file.stat().st_size // ENTRY_BYTES),
        "optimizerUpdates": step,
        "dataCursor": {"epoch": cursor.epoch, "record": cursor.record},
        "batchHashes": batch_hashes,
        "stepMetrics": step_metrics,
        "validation": validation,
        "quantization": quantization,
        "artifacts": {
            "checkpoint": artifact(checkpoint),
            **({"canonicalModel": artifact(canonical)} if complete else {}),
        },
    }
    write_json(report_path, report)
    return report, report_path


def cpu_gate(args):
    output = args.output_dir
    if output.exists() and any(output.iterdir()):
        raise RuntimeError("R2 CPU gate refuses a non-empty output directory")
    output.mkdir(parents=True, exist_ok=True)
    split_paths, split_manifest = split_source(args.corpus_bin, args.corpus_jsonl, output)
    train_file = split_paths["training-validation"]
    validation_file = split_paths["development-test"]
    common = (args.loader, train_file, validation_file, args.batch_size, args.maximum_steps, args.seed, torch.device("cpu"))
    same_a, same_a_path = run_training("same-seed-a", output / "same-seed-a", *common)
    same_b, same_b_path = run_training("same-seed-b", output / "same-seed-b", *common)
    split_step = args.interrupt_step
    interrupted, interrupted_path = run_training(
        "resume", output / "resume", *common, stop_step=split_step
    )
    interrupted_path = output / "resume" / "interrupted-run.json"
    write_json(interrupted_path, interrupted)
    resumed, resumed_path = run_training(
        "resume", output / "resume", *common, resume=True
    )
    stale_refused = False
    try:
        run_training("resume", output / "resume", *common)
    except RuntimeError as error:
        stale_refused = "refuses" in str(error)
    same_model = same_a["artifacts"]["canonicalModel"]["sha256"] == same_b["artifacts"]["canonicalModel"]["sha256"]
    resumed_model = same_a["artifacts"]["canonicalModel"]["sha256"] == resumed["artifacts"]["canonicalModel"]["sha256"]
    same_batches = same_a["batchHashes"] == same_b["batchHashes"]
    resumed_batches = same_a["batchHashes"] == resumed["batchHashes"]
    same_metrics = canonical_json(same_a["stepMetrics"]) == canonical_json(same_b["stepMetrics"])
    resumed_metrics = canonical_json(same_a["stepMetrics"]) == canonical_json(resumed["stepMetrics"])
    physical_validation = validation_file.stat().st_size // ENTRY_BYTES
    finite = same_a["validation"]["records"] == physical_validation
    learns = (
        same_a["stepMetrics"][-1]["loss"]
        <= same_a["stepMetrics"][0]["loss"] * args.max_final_to_initial_loss_ratio
        and same_a["validation"]["teacherOnlyLoss"] <= args.max_validation_teacher_loss
    )
    gates = {
        "immutablePinnedBuildContext": True,
        "randomSkippingDisabled": True,
        "sourceGroupSplitDisjoint": True,
        "sameSeedBatchHashes": same_batches,
        "sameSeedMetrics": same_metrics,
        "sameSeedSerializedOutput": same_model,
        "explicitResumeStartingStep": resumed["startingGlobalStep"] == split_step,
        "resumeBatchHashes": resumed_batches,
        "resumeMetrics": resumed_metrics,
        "resumeCpuReferenceOutput": resumed_model,
        "freshRunRefusesExistingCheckpoint": stale_refused,
        "finiteValidationPhysicalCount": finite,
        "finiteValidationRepeatable": True,
        "productionLoaderKnownTargetLearns": learns,
        "quantizationSurvivalReported": bool(same_a["quantization"]["survived"]),
        "gpuGateNotRun": True,
    }
    report = {
        "schema": "haitaka-anhoku-r2-cpu-gate-v1",
        "device": "cpu",
        "gpuUsed": False,
        "featureFamily": FEATURE_SET,
        "schedule": SCHEDULE,
        "batchSize": args.batch_size,
        "maximumSteps": args.maximum_steps,
        "interruptStep": split_step,
        "seed": args.seed,
        "numericToleranceForGpu": args.gpu_tolerance,
        "learnabilityThresholds": {
            "maximumFinalToInitialBatchLossRatio": args.max_final_to_initial_loss_ratio,
            "maximumValidationTeacherLoss": args.max_validation_teacher_loss,
        },
        "validationLayers": {
            "trainingValidation": "visible source-group split used in this gate",
            "developmentTest": "visible source-group split used repeatedly in this gate",
            "versionedStageBlocks": "assignment registry frozen; labels remain unopened until their declared stage",
            "finalSealedStaticSearch": "reserved; unavailable to trainer and hard-mining commands",
            "sealedPromotionOpenings": "reserved; unavailable to recipe selection",
        },
        "runs": {
            "sameSeedA": artifact(same_a_path),
            "sameSeedB": artifact(same_b_path),
            "interrupted": artifact(interrupted_path),
            "resumed": artifact(resumed_path),
        },
        "cpuReference": {
            "batchHashes": same_a["batchHashes"],
            "validation": same_a["validation"],
            "canonicalModelSha256": same_a["artifacts"]["canonicalModel"]["sha256"],
        },
        "artifacts": {
            "splitManifest": artifact(split_manifest),
            "loader": artifact(args.loader),
            "corpusBin": artifact(args.corpus_bin),
            "corpusJsonl": artifact(args.corpus_jsonl),
            "canonicalCpuReference": same_a["artifacts"]["canonicalModel"],
        },
        "gates": gates,
        "passedCpu": all(value for name, value in gates.items() if name != "gpuGateNotRun"),
        "passed": False,
        "nextRequiredGate": "CUDA run must reproduce batch hashes and stay within numericToleranceForGpu; byte-identical GPU checkpoints are not required",
    }
    report_path = output / "r2-cpu-gate-report.json"
    write_json(report_path, report)
    if not report["passedCpu"]:
        raise RuntimeError("R2 CPU gate failed")
    return report_path


def gpu_gate(args):
    if not torch.cuda.is_available():
        raise RuntimeError("CUDA gate requested but torch.cuda.is_available() is false")
    reference = json.loads(args.cpu_reference.read_text())
    if reference.get("schema") != "haitaka-anhoku-r2-cpu-gate-v1" or not reference.get("passedCpu"):
        raise RuntimeError("GPU gate requires a passing R2 CPU reference")
    args.output_dir.mkdir(parents=True, exist_ok=False)
    split_paths, _ = split_source(args.corpus_bin, args.corpus_jsonl, args.output_dir)
    run, run_path = run_training(
        "gpu", args.output_dir / "gpu", args.loader,
        split_paths["training-validation"], split_paths["development-test"],
        args.batch_size, args.maximum_steps, args.seed, torch.device("cuda")
    )
    cpu_run = reference["cpuReference"]
    batch_match = run["batchHashes"] == cpu_run["batchHashes"]
    validation_loss_delta = abs(
        run["validation"]["teacherOnlyLoss"]
        - cpu_run["validation"]["teacherOnlyLoss"]
    )
    cpu_predictions = np.asarray(cpu_run["validation"]["predictionValues"], dtype=np.float64)
    gpu_predictions = np.asarray(run["validation"]["predictionValues"], dtype=np.float64)
    if cpu_predictions.shape != gpu_predictions.shape:
        raise RuntimeError("CPU/GPU validation prediction counts differ")
    maximum_prediction_delta = float(np.max(np.abs(cpu_predictions - gpu_predictions)))
    metric_names = [
        "teacherOnlyLoss", "rawRobustScoreError", "calibratedProbabilityLoss",
        "teacherProbabilityDiagnostic", "pearson", "spearman",
        "signAccuracyOutsideNearZeroBand", "predictionActivationRate",
        "predictionClippingRate",
    ]
    metric_deltas = {
        name: abs(run["validation"][name] - cpu_run["validation"][name])
        for name in metric_names
    }
    passed = (
        batch_match
        and maximum_prediction_delta <= args.gpu_tolerance
        and max(metric_deltas.values()) <= args.gpu_tolerance
    )
    report = {
        "schema": "haitaka-anhoku-r2-gpu-gate-v1",
        "device": str(torch.cuda.get_device_name(0)),
        "cuda": torch.version.cuda,
        "torch": torch.__version__,
        "batchHashesMatchCpu": batch_match,
        "maximumPredictionAbsoluteDelta": maximum_prediction_delta,
        "validationLossAbsoluteDelta": validation_loss_delta,
        "metricAbsoluteDeltas": metric_deltas,
        "tolerance": args.gpu_tolerance,
        "byteIdenticalCheckpointRequired": False,
        "run": artifact(run_path),
        "passed": passed,
    }
    report_path = args.output_dir / "r2-gpu-gate-report.json"
    write_json(report_path, report)
    if not passed:
        raise RuntimeError("R2 GPU gate failed")
    return report_path


def parse_args():
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", choices=("cpu", "gpu"), required=True)
    parser.add_argument("--loader", type=pathlib.Path, required=True)
    parser.add_argument("--corpus-bin", type=pathlib.Path, required=True)
    parser.add_argument("--corpus-jsonl", type=pathlib.Path, required=True)
    parser.add_argument("--output-dir", type=pathlib.Path, required=True)
    parser.add_argument("--cpu-reference", type=pathlib.Path)
    parser.add_argument("--batch-size", type=int, default=257)
    parser.add_argument("--maximum-steps", type=int, default=96)
    parser.add_argument("--interrupt-step", type=int, default=37)
    parser.add_argument("--seed", type=int, default=20260902)
    parser.add_argument("--gpu-tolerance", type=float, default=2.0e-4)
    parser.add_argument("--max-final-to-initial-loss-ratio", type=float, default=0.3)
    parser.add_argument("--max-validation-teacher-loss", type=float, default=0.01)
    args = parser.parse_args()
    if args.batch_size <= 0 or args.maximum_steps < 16:
        parser.error("invalid R2 exposure budget")
    if not 0 < args.interrupt_step < args.maximum_steps:
        parser.error("interrupt step must be inside the run")
    if args.mode == "gpu" and args.cpu_reference is None:
        parser.error("--cpu-reference is required in GPU mode")
    return args


def main():
    args = parse_args()
    for path in (args.loader, args.corpus_bin, args.corpus_jsonl):
        if not path.is_file():
            raise RuntimeError(f"missing R2 input: {path}")
    report = cpu_gate(args) if args.mode == "cpu" else gpu_gate(args)
    print(report)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"R2 trainer error: {error}", file=sys.stderr)
        raise
