#!/usr/bin/env python3
"""CPU-only F1 loss ablation contract, frozen-data audit, and score primitives.

This module has no device imports and dispatches no searches or jobs.  It is
deliberately independent of the historical Step 10/12 command entry points.
"""

from __future__ import annotations

from collections import Counter
from datetime import datetime, timezone
from fractions import Fraction
import hashlib
import json
import math
import mmap
import os
from pathlib import Path
import statistics
import struct
from typing import Any, Iterable, Mapping, Sequence


ROOT = Path(__file__).resolve().parents[1]
F1_ROOT = ROOT / "out/anhoku-strength-first/f1-loss-ablation-20260925-v1"
EXPERIMENT_ID = "strength-first-f1-loss-ablation-20260925-v1"
RECIPE = ROOT / "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v1/training/resolved-config.json"
S = 12000.0
OUTPUT_SCALE = 600.0
UPDATES = tuple(range(128, 1025, 128))
INPUTS = {
    "incumbent": ("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/inputs/model/step10-u768.nnue", 83048703, "ddf42d5ca2e7a2a380bb13b8c4de8b3b7ee6807766d614cdb22757eead7f142b"),
    "wasm": ("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/package/web-pkg/haitaka_wasm_bg.wasm", 18279053, "8677f7c83f48186b8132a534185044402d25cc9f3d071363912506ca0203dfc0"),
    "glue": ("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/package/web-pkg/haitaka_wasm.js", 36924, "d8eaf8ad3bcf842588f4fc2d3a83d6cb7f2d29637a5097396db3b0877036a724"),
    "trainingBinary": ("out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/training.bin", 63360000, "51ecff36f4aa69a98d52e1cac78a47a0e62b15c834d9f33902557c138055b121"),
    "validationBinary": ("out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/validation.bin", 4320000, "c128c7b58ab2b782badb4d504b6520a3dde8050820feb567250b127de6d96fee"),
    "trainingSidecar": ("out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/training.jsonl", 149594430, "f213ae17df30478d12f9fcacdde057e85161cf40fd2a046cdb173cb1185a5fa0"),
    "validationSidecar": ("out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/validation.jsonl", 10123281, "4e576f0d4e0a0e80509f6ab695a71d1fe10754a231b76b7d20f30491050a4fb3"),
}


class F1Error(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise F1Error(message)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def artifact(path: Path) -> dict[str, Any]:
    require(path.is_file() and not path.is_symlink(), f"missing or symlinked artifact: {path}")
    try:
        name = str(path.relative_to(ROOT))
    except ValueError:
        name = str(path)
    return {"path": name, "bytes": path.stat().st_size, "sha256": sha256(path)}


def verify_artifact(binding: Mapping[str, Any]) -> Path:
    path = ROOT / str(binding["path"])
    actual = artifact(path)
    require(actual == dict(binding), f"artifact bytes/hash changed: {path}")
    return path


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def canonical_bytes(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False) + "\n").encode()


def immutable_json(path: Path, value: Any) -> None:
    data = json.dumps(value, indent=2, sort_keys=True, allow_nan=False).encode() + b"\n"
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        require(path.read_bytes() == data, f"immutable artifact conflicts: {path}")
        return
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    try:
        with temporary.open("xb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def frozen_inputs() -> dict[str, dict[str, Any]]:
    bound = {}
    for role, (name, size, digest) in INPUTS.items():
        value = {"path": name, "bytes": size, "sha256": digest}
        verify_artifact(value)
        bound[role] = value
    recipe_binding = artifact(RECIPE)
    config = json.loads(RECIPE.read_text())
    require(config["recipe"]["data"]["training"]["records"] == 880000, "training record count changed")
    require(config["recipe"]["data"]["validation"]["records"] == 60000, "validation record count changed")
    require(config["recipe"]["loader"]["batchSize"] == 16384, "loader batch size changed")
    require(config["recipe"]["schedule"]["horizonUpdates"] == 4096, "schedule horizon changed")
    require(config["recipe"]["objective"]["modelOutputToRuntimeScore"] == OUTPUT_SCALE, "runtime score scale changed")
    for split in ("training", "validation"):
        data = config["recipe"]["data"][split]
        require(data["entryBytes"] == 72, f"{split} entry size changed")
        for key in ("binary", "sidecar", "manifest"):
            verify_artifact(data[key])
            bound[f"{split}{key.capitalize()}"] = dict(data[key])
        manifest = json.loads((ROOT / data["manifest"]["path"]).read_text())
        require(manifest["records"] == data["records"] and manifest["entryBytes"] == 72, f"{split} manifest differs")
        for key in ("binary", "sidecar", "source"):
            verify_artifact(manifest[key])
        bound[f"{split}Source"] = dict(manifest["source"])
    bound["incumbentRecipe"] = recipe_binding
    return bound


def _audit_split(split: str, config: Mapping[str, Any], *, collect_labels: bool) -> tuple[dict[str, Any], list[int]]:
    """Check each frozen row against its sourceIndex and original source bytes.

    The pinned C++ loader copies the signed little-endian i16 at offset 64
    directly to float score (nnue_training_data_formats.h and overlay loader).
    Source-index byte equality ties the sidecar to the exact frozen record.
    """
    data = config["recipe"]["data"][split]
    manifest = json.loads((ROOT / data["manifest"]["path"]).read_text())
    labels: list[int] = []
    score_counts: Counter[str] = Counter()
    records = int(data["records"])
    source_records = int(manifest["source"]["bytes"]) // 72
    require(manifest["source"]["bytes"] % 72 == 0, f"{split} source is not 72-byte aligned")
    with (ROOT / data["binary"]["path"]).open("rb") as frozen_file, (ROOT / manifest["source"]["path"]).open("rb") as source_file, (ROOT / data["sidecar"]["path"]).open("rb") as sidecar_file:
        frozen = mmap.mmap(frozen_file.fileno(), 0, access=mmap.ACCESS_READ)
        source = mmap.mmap(source_file.fileno(), 0, access=mmap.ACCESS_READ)
        try:
            require(len(frozen) == records * 72, f"{split} binary size differs")
            seen = 0
            for index, line in enumerate(sidecar_file):
                require(index < records, f"{split} sidecar has extra rows")
                row = json.loads(line)
                require(row.get("schema") == "haitaka-strength-first-frozen-training-metadata-v1", f"{split} sidecar schema differs at {index}")
                source_index = row.get("sourceIndex")
                require(type(source_index) is int and 0 <= source_index < source_records, f"{split} source index invalid at {index}")
                lo = index * 72
                source_lo = source_index * 72
                require(frozen[lo:lo+72] == source[source_lo:source_lo+72], f"{split} binary/sidecar/source mismatch at {index}")
                score = struct.unpack_from("<h", frozen, lo + 64)[0]
                y = max(-12000, min(12000, score))
                score_counts["positive" if y > 0 else "negative" if y < 0 else "zero"] += 1
                if y != score:
                    score_counts["clipped"] += 1
                if collect_labels:
                    labels.append(score)
                seen += 1
            require(seen == records, f"{split} sidecar row count differs")
        finally:
            frozen.close()
            source.close()
    return {"records": records, "sourceRecords": source_records, "binarySidecarSourceAgreement": True, "scoreCounts": dict(score_counts), "orientation": "signed-i16-le-at-offset-64-copied-directly-to-float-by-production-loader"}, labels


def _quantile(sorted_values: Sequence[int], numerator: int, denominator: int) -> float:
    require(sorted_values, "empty values")
    position = (len(sorted_values) - 1) * numerator / denominator
    lower = math.floor(position)
    upper = math.ceil(position)
    return float(sorted_values[lower] + (sorted_values[upper] - sorted_values[lower]) * (position - lower))


def calibrate(labels: Iterable[float]) -> dict[str, Any]:
    values: list[float] = []
    total = 0
    zeros = 0
    clipped = 0
    for value in labels:
        require(isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value), "nonfinite or nonnumeric training label")
        total += 1
        bounded = max(-S, min(S, float(value)))
        clipped += bounded != value
        if bounded == 0:
            zeros += 1
        else:
            values.append(abs(bounded))
    require(values, "no nonzero training labels")
    values.sort()
    median = statistics.median(values)
    require(math.isfinite(median) and median > 0, "nonfinite or zero median")
    tau = median / math.log(3.0)
    require(math.isfinite(tau) and tau > 0, "nonfinite or zero temperature")
    return {"algorithm": "median-nonzero-abs-clipped-training-label-divided-by-ln3-float64-v1", "totalCount": total, "zeroCount": zeros, "nonzeroCount": len(values), "clippedCount": clipped, "quantiles": {"minimum": values[0], "p25": _quantile(values, 1, 4), "median": median, "p75": _quantile(values, 3, 4), "maximum": values[-1]}, "medianNonzeroAbsoluteScore": median, "tau": tau, "tauFloat64Hex": tau.hex()}


def sigmoid(x: float) -> float:
    require(math.isfinite(x), "nonfinite sigmoid input")
    if x >= 0:
        z = math.exp(-x)
        return 1.0 / (1.0 + z)
    z = math.exp(x)
    return z / (1.0 + z)


def losses_and_raw_gradients(raw: float, teacher: float, tau: float) -> dict[str, float]:
    require(all(math.isfinite(x) for x in (raw, teacher, tau)) and tau > 0, "nonfinite loss input or invalid tau")
    p = OUTPUT_SCALE * raw
    y = max(-S, min(S, teacher))
    e = (p - y) / S
    a = abs(e)
    h = 0.5 * e * e if a <= 0.1 else 0.1 * (a - 0.05)
    dh = (e if a <= 0.1 else math.copysign(0.1, e)) * OUTPUT_SCALE / S
    sp = sigmoid(p / tau)
    sy = sigmoid(y / tau)
    d = sp - sy
    prefactor = 8.0 * (tau / S) ** 2
    prob = prefactor * d * d
    dp = (16.0 * tau / (S * S)) * d * sp * (1.0 - sp) * OUTPUT_SCALE
    require(all(math.isfinite(x) for x in (h, dh, prob, dp)), "nonfinite loss or gradient")
    return {"huber": h, "huberRawGradient": dh, "probability": prob, "probabilityRawGradient": dp, "prediction": p, "target": y, "predictionSigmoid": sp, "targetSigmoid": sy}


def select_by_common_huber(candidates: Sequence[Mapping[str, Any]], *, verify_files: bool = True) -> dict[str, Any]:
    require(len(candidates) == len(UPDATES), "all eight eligible checkpoints are required")
    by_update = {}
    for candidate in candidates:
        update = candidate.get("update")
        require(type(update) is int and update in UPDATES and update not in by_update, "checkpoint update missing, repeated or ineligible")
        loss = candidate.get("validationHuber")
        require(type(loss) in (int, float) and math.isfinite(loss) and loss >= 0, "validation Huber is invalid")
        require(candidate.get("checkpoint") and candidate.get("export"), "checkpoint or export missing")
        if verify_files:
            for key in ("checkpoint", "export"):
                verify_artifact(candidate[key])
        by_update[update] = candidate
    minimum = min(float(row["validationHuber"]) for row in by_update.values())
    selected = next(update for update in UPDATES if by_update[update]["validationHuber"] <= minimum + 1e-12)
    return {"selectedUpdate": selected, "globalMinimumHuber": minimum, "selectedHuber": by_update[selected]["validationHuber"], "tieTolerance": 1e-12, "criterion": "earliest-eligible-Huber-within-global-minimum-plus-tolerance", "matchEvidenceUsed": False, "selected": dict(by_update[selected])}


def pair_statistics(pair_legs: Sequence[Sequence[Mapping[str, Any]]]) -> dict[str, Any]:
    from strength_first_confirmation_core import conservative_lower_bound, rejects_half
    ordinary: list[Fraction] = []
    conservative: list[Fraction] = []
    wdl = Counter()
    capped = 0
    for legs in pair_legs:
        require(len(legs) == 2, "each pair needs two color-swapped legs")
        scores = []
        conservative_scores = []
        for leg in legs:
            require(leg.get("valid") is True, "technical-invalid game cannot be scored")
            result = leg.get("scoreTreatment")
            require(result in (0, 0.5, 1), "game score must be win/draw/loss")
            is_cap = leg.get("genuinePlyCap") is True
            if is_cap:
                require(result == 0.5, "a genuine ply cap has ordinary draw score")
                capped += 1
            scores.append(Fraction(result))
            conservative_scores.append(Fraction(0) if is_cap else Fraction(result))
            wdl[{1: "wins", Fraction(1, 2): "draws", 0: "losses"}[Fraction(result)]] += 1
        ordinary.append(sum(scores) / 2)
        conservative.append(sum(conservative_scores) / 2)
    require(ordinary, "empty pair set")
    ordinary_score = sum(ordinary) / len(ordinary)
    conservative_score = sum(conservative) / len(conservative)
    hist = Counter(str(value) for value in conservative)
    conservative_float = [float(value) for value in conservative]
    return {"pairs": len(ordinary), "games": len(ordinary) * 2, "wdl": dict(wdl), "caps": capped, "ordinaryScore": float(ordinary_score), "conservativeScore": float(conservative_score), "ordinaryAtLeast052": ordinary_score >= Fraction(13, 25), "conservativeAtLeast052": conservative_score >= Fraction(13, 25), "conservativePairHistogram": {str(key): hist.get(str(Fraction(key)), 0) for key in (0, 0.25, 0.5, 0.75, 1)}, "holdoutRejectsHalf": rejects_half(conservative_float) if len(conservative) == 512 else None, "holdoutLowerBound": conservative_lower_bound(conservative_float) if len(conservative) == 512 else None}
