#!/usr/bin/env python3
"""CPU-only post-confirmation diagnosis for the bounded Step 9 runbook.

This file is deliberately separate from the frozen Step 9 confirmation
analyzer.  It owns the deterministic opening-subset construction, the
diagnostic corpus audit, strict per-block validation, and the final paired
analysis.  Match outcomes are written to sealed validation artifacts and are
not printed by the runner until every fixed block has completed.
"""

from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import time
from typing import Any, Iterable


ROOT = Path(__file__).resolve().parents[1]
DIAGNOSIS_SCHEMA = "haitaka-anhoku-strength-first-step9-diagnosis-v1"
BLOCK_SCHEMA = "haitaka-anhoku-strength-first-step9-diagnosis-block-validation-v1"
AUDIT_SCHEMA = "haitaka-anhoku-strength-first-step9-diagnosis-audit-v1"
FIXTURE_SCHEMA = "haitaka-anhoku-strength-first-step9-diagnosis-fixtures-v1"
REPORT_SCHEMA = "haitaka-anhoku-strength-first-step9-diagnosis-report-v1"
GAME_SCHEMA = "haitaka-self-play-game"
GAME_SCHEMA_VERSION = 3
TERMINATION_SCHEMA = "haitaka-self-play-termination-v1"
ALLOWED_TERMINATIONS = {"no-legal-move", "repetition-draw", "maximum-plies"}
BLOCK_PAIRS = 128
BLOCK_GAMES = 256
SELECTED_PAIRS = 512
FULL_PAIRS = 2048
AUDIT_SAMPLE_CAP = 10_000
BOOTSTRAP_REPLICATES = 10_000
BOOTSTRAP_SEED = 2026091401

MODEL_A_NETWORK = "out/anhoku-strength-first/step8-compact-seed-replication-20260910-v1/exports/selected-u00002688.nnue"
MODEL_B2688_NETWORK = "out/anhoku-strength-first/step9-model-b-training-20260912-v1/exports/candidates/candidate-u00002688.nnue"
MODEL_B768_NETWORK = "out/anhoku-strength-first/step9-model-b-training-20260912-v1/exports/selected-u00000768.nnue"
SUITE_A = "haitaka_learn/openings/anhoku-strength-confirmation-100ms-a-v1.tsv"
SUITE_B = "haitaka_learn/openings/anhoku-strength-confirmation-100ms-b-v1.tsv"
RUNTIME = "target/release/haitaka_cli"
PRESERVED_RUNTIME = "out/anhoku-strength-first/step9-model-b-training-20260912-v1/runtime/haitaka_cli-u00000256"
FROZEN_CONTRACT = "r0/anhoku-strength-first/step9-confirmation-100ms-contract-v1.json"
FROZEN_EXECUTION = "r0/anhoku-strength-first/step9-confirmation-100ms-execution-v1.json"
STEP9_RESULT = "r0/anhoku-strength-first/step9-result-report.json"
STEP9_RESULT_MD = "r0/anhoku-strength-first/step9-result-report.md"
HISTORICAL_ROOT = "out/anhoku-strength-first/step9-independent-confirmation-100ms-20260913-v1"
STEP8_ROOT = "out/anhoku-strength-first/step8-compact-seed-replication-20260910-v1"
STEP9_ROOT = "out/anhoku-strength-first/step9-model-b-training-20260912-v1"
STEP8_CONFIG = f"{STEP8_ROOT}/resolved-config.json"
STEP9_CONFIG = f"{STEP9_ROOT}/resolved-config.json"
STEP8_RUN = f"{STEP8_ROOT}/run.json"
STEP9_RUN = f"{STEP9_ROOT}/run.json"
STEP8_METRICS = f"{STEP8_ROOT}/metrics.jsonl"
STEP9_METRICS = f"{STEP9_ROOT}/metrics.jsonl"
STEP8_SELECTION = f"{STEP8_ROOT}/selection-history-v2.json"
STEP9_SELECTION = f"{STEP9_ROOT}/selection-history-v2.json"

EXPECTED_HASHES = {
    "modelA": "6c4f31ad5441aabe5b0e3feeefa411cc7b5bcee194ff896b2662f26c5069a8fa",
    "modelB2688": "0ecde2640520e5b2566527c4098decbb1050f729d8999188435c6d8b258fe221",
    "modelB768": "ba4cff98b0a94607ba92367888606d37fc5621a41c3b861558c013eb7117e6ef",
    "suiteA": "66fd0e8c365d85f965d02e214738a48bed23d7905f7bb3e2b45dfc3821c10e61",
    "suiteB": "53f1cd689da3615317144ef958ba731986191fbb6847e6b3e6df3f79231377cf",
    "runtime": "ad2e83f3921209c4006e3e23bdd105d951cd1919580560449cdff8fab2423009",
}


class DiagnosisError(RuntimeError):
    """An artifact, protocol, or analysis validation failure."""


def fail(message: str) -> None:
    raise DiagnosisError(message)


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        fail(f"cannot load JSON {path}: {exc}")
    if not isinstance(value, dict):
        fail(f"JSON root must be an object: {path}")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError as exc:
        fail(f"cannot hash {path}: {exc}")
    return digest.hexdigest()


def display(path: Path) -> str:
    resolved = path.resolve()
    try:
        return str(resolved.relative_to(ROOT.resolve()))
    except ValueError:
        return str(resolved)


def artifact(path: Path, expected: str | None = None) -> dict[str, Any]:
    if not path.is_file():
        fail(f"missing artifact: {path}")
    digest = sha256(path)
    if expected is not None and digest != expected:
        fail(f"hash mismatch for {path}: expected {expected}, got {digest}")
    return {"path": display(path), "bytes": path.stat().st_size, "sha256": digest}


def relative_artifact(path: Path, base: Path, expected: str | None = None) -> dict[str, Any]:
    value = artifact(path, expected)
    value["path"] = os.path.relpath(path.resolve(), base.resolve())
    return value


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def path_from(root: Path, value: str) -> Path:
    path = Path(value)
    return path if path.is_absolute() else root / path


def exact(actual: Any, expected: Any, where: str) -> None:
    if actual != expected:
        fail(f"{where}: expected {expected!r}, got {actual!r}")


def close(actual: Any, expected: float, where: str) -> None:
    if isinstance(actual, bool) or not isinstance(actual, (int, float)) or not math.isfinite(float(actual)):
        fail(f"{where} is not finite")
    if not math.isclose(float(actual), expected, rel_tol=1e-12, abs_tol=1e-12):
        fail(f"{where}: expected {expected!r}, got {actual!r}")


def finite(value: Any, where: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(float(value)):
        fail(f"{where} is not finite")
    return float(value)


def load_suite(path: Path, expected_hash: str) -> list[tuple[str, str]]:
    exact(sha256(path), expected_hash, f"suite hash {path}")
    try:
        raw = path.read_bytes()
    except OSError as exc:
        fail(f"cannot read suite {path}: {exc}")
    rows: list[tuple[str, str]] = []
    for line_number, line in enumerate(raw.split(b"\n"), 1):
        if line == b"":
            continue
        if line.startswith(b"#"):
            continue
        if b"\r" in line or line.count(b"\t") != 1:
            fail(f"invalid suite row {path}:{line_number}")
        opening_id_bytes, sfen_bytes = line.split(b"\t")
        if not opening_id_bytes or not sfen_bytes:
            fail(f"empty suite field {path}:{line_number}")
        try:
            opening_id = opening_id_bytes.decode("utf-8")
            sfen = sfen_bytes.decode("utf-8")
        except UnicodeError as exc:
            fail(f"suite is not UTF-8 {path}:{line_number}: {exc}")
        rows.append((opening_id, sfen))
    exact(len(rows), FULL_PAIRS, f"suite row count {path}")
    if len({row[0] for row in rows}) != FULL_PAIRS:
        fail(f"suite contains duplicate IDs: {path}")
    return rows


def selection_digest(suite_hash: str, opening_id: str) -> str:
    payload = f"step9-diagnosis-v1\n{suite_hash}\n{opening_id}".encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def make_selection(path: Path, suite_name: str, suite_hash: str) -> dict[str, Any]:
    rows = load_suite(path, suite_hash)
    ranked = sorted(
        ((selection_digest(suite_hash, opening_id), opening_id, index, sfen) for index, (opening_id, sfen) in enumerate(rows)),
        key=lambda item: (item[0], item[1]),
    )
    selected_indexes = {item[2] for item in ranked[:SELECTED_PAIRS]}
    selected: list[dict[str, Any]] = []
    for selected_index, (original_index, (opening_id, sfen)) in enumerate(
        (item for item in enumerate(rows) if item[0] in selected_indexes)
    ):
        selected.append(
            {
                "originalSuiteIndex": original_index,
                "openingId": opening_id,
                "sfen": sfen,
                "rankDigest": selection_digest(suite_hash, opening_id),
                "selectedIndex": selected_index,
                "blockIndex": selected_index // BLOCK_PAIRS,
            }
        )
    exact(len(selected), SELECTED_PAIRS, f"selected {suite_name} rows")
    rank_order = {item[2]: ordinal for ordinal, item in enumerate(ranked)}
    for item in selected:
        item["rankOrdinal"] = rank_order[item["originalSuiteIndex"]]
    return {
        "schema": f"{DIAGNOSIS_SCHEMA}-opening-selection-v1",
        "suite": suite_name,
        "selectionAlgorithm": "sha256-rank-v1",
        "rankInput": "step9-diagnosis-v1\\n<FULL_SUITE_SHA256>\\n<OPENING_ID>",
        "fullSuite": artifact(path, suite_hash),
        "fullPairCount": FULL_PAIRS,
        "selectedPairCount": SELECTED_PAIRS,
        "blockPairCount": BLOCK_PAIRS,
        "restoredOriginalSuiteOrder": True,
        "rows": selected,
    }


def write_selection_files(output_root: Path, suite_name: str, selection: dict[str, Any]) -> dict[str, Any]:
    prefix = "a" if suite_name == "A" else "b"
    schedule_dir = output_root / "schedules"
    rows = selection["rows"]
    tsv = "".join(f"{row['openingId']}\t{row['sfen']}\n" for row in rows)
    sfens = "".join(f"{row['sfen']}\n" for row in rows)
    selected_tsv = schedule_dir / f"{prefix}-selected.tsv"
    selected_sfen = schedule_dir / f"{prefix}-selected.sfen"
    selected_json = schedule_dir / f"{prefix}-selection.json"
    selected_tsv.parent.mkdir(parents=True, exist_ok=True)
    selected_tsv.write_text(tsv, encoding="utf-8")
    selected_sfen.write_text(sfens, encoding="utf-8")
    write_json(selected_json, selection)
    blocks: list[dict[str, Any]] = []
    block_dir = schedule_dir / "blocks"
    for block in range(4):
        block_rows = rows[block * BLOCK_PAIRS : (block + 1) * BLOCK_PAIRS]
        block_path = block_dir / f"{prefix}-selected-block-{block}.sfen"
        block_path.parent.mkdir(parents=True, exist_ok=True)
        block_path.write_text("".join(f"{row['sfen']}\n" for row in block_rows), encoding="utf-8")
        blocks.append({"block": block, "pairs": BLOCK_PAIRS, "rows": block_rows, "sfen": relative_artifact(block_path, output_root)})
    selection["artifacts"] = {
        "selectedTsv": relative_artifact(selected_tsv, output_root),
        "selectedSfen": relative_artifact(selected_sfen, output_root),
        "selectionJson": relative_artifact(selected_json, output_root),
        "blocks": blocks,
    }
    write_json(selected_json, selection)
    selection["artifacts"]["selectionJson"] = relative_artifact(selected_json, output_root)
    return selection


def default_paths(root: Path) -> dict[str, Path]:
    return {
        "modelA": root / MODEL_A_NETWORK,
        "modelB2688": root / MODEL_B2688_NETWORK,
        "modelB768": root / MODEL_B768_NETWORK,
        "suiteA": root / SUITE_A,
        "suiteB": root / SUITE_B,
        "runtime": root / RUNTIME,
        "preservedRuntime": root / PRESERVED_RUNTIME,
        "contract": root / FROZEN_CONTRACT,
        "execution": root / FROZEN_EXECUTION,
        "step9Result": root / STEP9_RESULT,
        "step9ResultMd": root / STEP9_RESULT_MD,
        "step8Config": root / STEP8_CONFIG,
        "step9Config": root / STEP9_CONFIG,
        "step8Run": root / STEP8_RUN,
        "step9Run": root / STEP9_RUN,
        "step8Metrics": root / STEP8_METRICS,
        "step9Metrics": root / STEP9_METRICS,
        "step8Selection": root / STEP8_SELECTION,
        "step9Selection": root / STEP9_SELECTION,
        "step8Root": root / STEP8_ROOT,
        "step9Root": root / STEP9_ROOT,
        "historicalRoot": root / HISTORICAL_ROOT,
    }


def selected_export_summary(path: Path) -> dict[str, Any]:
    value = load_json(path)
    exact(value.get("status", "passed"), "passed", f"export status {path}")
    return {
        "schema": value.get("schema"),
        "experimentId": value.get("experimentId"),
        "configSha256": value.get("configSha256"),
        "checkpoint": value.get("sourceCheckpoint") or value.get("checkpoint"),
        "integerNetwork": value.get("integerNetwork"),
        "serializer": value.get("serializer"),
        "selectionWasExplicit": value.get("selectionWasExplicit"),
        "recoveredOrphan": value.get("recoveredOrphan"),
    }


def parity_summary(path: Path, expected_network: str, expected_runtime: str, require_exact_runtime: bool = True) -> dict[str, Any]:
    value = load_json(path)
    exact(value.get("status"), "passed", f"parity status {path}")
    exact(value.get("integerNetwork", {}).get("sha256"), expected_network, f"parity network {path}")
    recorded_runtime = value.get("runtimeBuild", {}).get("sha256")
    if require_exact_runtime:
        exact(recorded_runtime, expected_runtime, f"parity runtime {path}")
    checks = value.get("checks", {})
    for key in ("fullPrecisionToIntegerMeasured", "integerReferenceMatchesNative", "integerReferenceMatchesWasm", "nativeMatchesWasm", "quantizationErrorReportedSeparately"):
        exact(checks.get(key), True, f"parity {path} checks.{key}")
    return {
        "path": display(path),
        "sha256": sha256(path),
        "status": value["status"],
        "networkSha256": value["integerNetwork"]["sha256"],
        "runtimeSha256": recorded_runtime,
        "runtimeMatchesDiagnosisRuntime": recorded_runtime == expected_runtime,
        "runtimeBinding": "exact-diagnosis-runtime" if recorded_runtime == expected_runtime else "historical-prior-runtime",
        "checkpoint": value.get("checkpoint"),
        "checks": checks,
    }


def metric_at(path: Path, updates: Iterable[int]) -> dict[int, dict[str, Any]]:
    wanted = set(updates)
    result: dict[int, dict[str, Any]] = {}
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        fail(f"cannot read metrics {path}: {exc}")
    for line in lines:
        value = json.loads(line)
        update = value.get("optimizerUpdates")
        if isinstance(update, int) and update in wanted:
            result[update] = value
    missing = sorted(wanted - set(result))
    if missing:
        fail(f"metrics {path} missing updates {missing}")
    return result


def compact_metric(value: dict[str, Any]) -> dict[str, Any]:
    validation = value.get("validation", {})
    train = value.get("trainWindow", {})
    return {
        "optimizerUpdates": value.get("optimizerUpdates"),
        "learningRate": value.get("learningRate"),
        "validation": {
            "teacherOnlyLoss": validation.get("teacherOnlyLoss"),
            "rawScoreMae": validation.get("rawScoreMae"),
            "outputScale": validation.get("outputScale"),
            "saturation": validation.get("saturation"),
            "scoreRange": validation.get("scoreRange"),
            "byStatus": validation.get("byStatus"),
        },
        "trainWindow": {
            "teacherOnlyLoss": train.get("teacherOnlyLoss"),
            "rawScoreMae": train.get("rawScoreMae"),
            "outputScale": train.get("outputScale"),
            "saturation": train.get("saturation"),
            "scoreRange": train.get("scoreRange"),
            "byStatus": train.get("byStatus"),
            "updateMagnitudeSamples": train.get("updateMagnitudeSamples"),
        },
        "bestValidation": value.get("bestValidation"),
        "exposure": value.get("exposure"),
        "timingSeconds": value.get("timingSeconds"),
    }


def recursive_differences(left: Any, right: Any, prefix: str = "") -> list[str]:
    if isinstance(left, dict) and isinstance(right, dict):
        differences: list[str] = []
        for key in sorted(set(left) | set(right)):
            child = f"{prefix}.{key}" if prefix else key
            if key not in left or key not in right:
                differences.append(child)
            else:
                differences.extend(recursive_differences(left[key], right[key], child))
        return differences
    if isinstance(left, list) and isinstance(right, list):
        if left == right:
            return []
        return [prefix]
    return [] if left == right else [prefix]


def audit_sample_indices(records: int, cap: int = AUDIT_SAMPLE_CAP) -> list[int]:
    count = min(records, cap)
    if count <= 0:
        fail("cannot sample an empty split")
    if count == 1:
        return [0]
    return sorted({round(index * (records - 1) / (count - 1)) for index in range(count)})


def quantile(values: list[int], fraction: float) -> float:
    if not values:
        fail("cannot compute a quantile over an empty sample")
    ordered = sorted(values)
    position = (len(ordered) - 1) * fraction
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return float(ordered[lower])
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def audit_split(name: str, binary: Path, sidecar: Path, records: int) -> dict[str, Any]:
    exact(binary.stat().st_size, records * 72, f"{name} binary size")
    indices = audit_sample_indices(records)
    wanted = set(indices)
    sidecar_rows: dict[int, dict[str, Any]] = {}
    try:
        with sidecar.open("r", encoding="utf-8") as stream:
            for index, line in enumerate(stream):
                if index in wanted:
                    try:
                        row = json.loads(line)
                    except json.JSONDecodeError as exc:
                        fail(f"malformed selected sidecar row {name}:{index + 1}: {exc}")
                    if not isinstance(row, dict) or not isinstance(row.get("position"), dict):
                        fail(f"invalid selected sidecar row {name}:{index + 1}")
                    sidecar_rows[index] = row
            actual_rows = index + 1 if "index" in locals() else 0
    except OSError as exc:
        fail(f"cannot read sidecar {sidecar}: {exc}")
    exact(actual_rows, records, f"{name} sidecar rows")
    exact(len(sidecar_rows), len(indices), f"{name} selected sidecar rows")

    scores: list[int] = []
    results: list[int] = []
    game_plies: list[int] = []
    categories: dict[str, Counter[str]] = {
        "phase": Counter(),
        "materialBucket": Counter(),
        "sideToMove": Counter(),
        "tactical": Counter(),
        "inCheck": Counter(),
        "donorState": Counter(),
        "source": Counter(),
        "openingGroup": Counter(),
    }
    unavailable: set[str] = set()
    source_indexes: list[int] = []
    reserved_nonzero = 0
    invalid_result = 0
    invalid_teacher_move = 0
    with binary.open("rb") as stream:
        for index in range(records):
            record = stream.read(72)
            if len(record) != 72:
                fail(f"short {name} binary record at index {index}")
            if index not in wanted:
                continue
            row = sidecar_rows[index]
            position = row["position"]
            score = struct.unpack_from("<h", record, 64)[0]
            teacher_move = struct.unpack_from("<H", record, 66)[0]
            game_ply = struct.unpack_from("<H", record, 68)[0]
            game_result = struct.unpack_from("<b", record, 70)[0]
            scores.append(score)
            results.append(game_result)
            game_plies.append(game_ply)
            reserved_nonzero += int(record[71] != 0)
            invalid_result += int(game_result not in (-1, 0, 1))
            invalid_teacher_move += int(teacher_move != 0)
            categories["phase"][str(position.get("phase"))] += 1
            categories["tactical"][str(position.get("tactical"))] += 1
            categories["inCheck"][str(position.get("inCheck"))] += 1
            categories["donorState"][str(position.get("donorState"))] += 1
            for field in ("materialBucket", "source", "openingGroup"):
                if field in position:
                    categories[field][str(position[field])] += 1
                else:
                    unavailable.add(f"position.{field}")
            if "sideToMove" in position:
                categories["sideToMove"][str(position["sideToMove"])] += 1
            else:
                unavailable.add("position.sideToMove (derived from packed ABI)")
                trainer_side_is_black = bool(record[0] & 1)
                categories["sideToMove"]["white" if trainer_side_is_black else "black"] += 1
            if "sourceIndex" in row:
                source_indexes.append(int(row["sourceIndex"]))
            else:
                unavailable.add("sourceIndex")
    if reserved_nonzero or invalid_result:
        fail(f"{name} sampled binary ABI has invalid reserved/result fields")
    if len(scores) != len(indices):
        fail(f"{name} binary sample count mismatch")
    duplicate_source_records = len(source_indexes) - len(set(source_indexes)) if source_indexes else None
    return {
        "split": name,
        "records": records,
        "sampleRule": "evenly-spaced-record-index-v1",
        "sampleCap": AUDIT_SAMPLE_CAP,
        "sampleRecords": len(indices),
        "sampleFirstIndex": indices[0],
        "sampleLastIndex": indices[-1],
        "binary": artifact(binary),
        "sidecar": artifact(sidecar),
        "categories": {key: dict(sorted(value.items())) for key, value in categories.items()},
        "labels": {
            "quantiles": {str(fraction): quantile(scores, fraction) for fraction in (0.0, 0.01, 0.1, 0.25, 0.5, 0.75, 0.9, 0.99, 1.0)},
            "mean": sum(scores) / len(scores),
            "meanAbsolute": sum(abs(value) for value in scores) / len(scores),
            "negative": sum(value < 0 for value in scores),
            "zero": sum(value == 0 for value in scores),
            "positive": sum(value > 0 for value in scores),
            "atOrAboveTargetClip": sum(abs(value) >= 12000 for value in scores),
            "atOrAboveMateThreshold": sum(abs(value) >= 29000 for value in scores),
            "gameResult": dict(sorted(Counter(results).items())),
            "gamePlyQuantiles": {str(fraction): quantile(game_plies, fraction) for fraction in (0.0, 0.5, 1.0)},
            "teacherMoveNonzero": invalid_teacher_move,
            "binaryLayout": "repository write_training_entry: packed[0:64], score i16le[64:66], teacher_move u16le[66:68], game_ply u16le[68:70], game_result i8[70], reserved[71]",
        },
        "sourceGroups": {
            "sourceIndexAvailable": bool(source_indexes),
            "uniqueSampledSourceIndexes": len(set(source_indexes)) if source_indexes else None,
            "duplicateSampledSourceIndexRecords": duplicate_source_records,
        },
        "unavailableFields": sorted(unavailable),
    }


def audit(root: Path, output_dir: Path, model_a_data_root: Path) -> dict[str, Any]:
    paths = default_paths(root)
    output_dir.parent.mkdir(parents=True, exist_ok=True)
    output_dir.mkdir(parents=True, exist_ok=False)
    for key in ("modelA", "modelB2688", "modelB768", "suiteA", "suiteB", "runtime", "preservedRuntime", "contract", "execution", "step9Result", "step9ResultMd", "step8Config", "step9Config", "step8Run", "step9Run", "step8Metrics", "step9Metrics", "step8Selection", "step9Selection"):
        if not paths[key].is_file():
            fail(f"audit required artifact missing ({key}): {paths[key]}")
    identity = {
        "modelA": artifact(paths["modelA"], EXPECTED_HASHES["modelA"]),
        "modelB2688": artifact(paths["modelB2688"], EXPECTED_HASHES["modelB2688"]),
        "modelB768": artifact(paths["modelB768"], EXPECTED_HASHES["modelB768"]),
        "suiteA": artifact(paths["suiteA"], EXPECTED_HASHES["suiteA"]),
        "suiteB": artifact(paths["suiteB"], EXPECTED_HASHES["suiteB"]),
        "runtime": artifact(paths["runtime"], EXPECTED_HASHES["runtime"]),
        "preservedRuntime": artifact(paths["preservedRuntime"], EXPECTED_HASHES["runtime"]),
    }
    exact(identity["runtime"]["sha256"], identity["preservedRuntime"]["sha256"], "runtime copy identity")
    contract = load_json(paths["contract"])
    execution = load_json(paths["execution"])
    result_report = load_json(paths["step9Result"])
    exact(result_report.get("confirmationPassed"), False, "historical confirmation decision")
    exact(result_report.get("deploymentAuthorized"), False, "historical deployment authorization")
    exact(result_report.get("models", {}).get("A", {}).get("networkSha256"), EXPECTED_HASHES["modelA"], "historical A network")
    exact(result_report.get("models", {}).get("B", {}).get("networkSha256"), EXPECTED_HASHES["modelB2688"], "historical B network")
    for model_key in ("A", "B"):
        exact(result_report["models"][model_key].get("completeSchedule"), True, f"historical {model_key} schedule")
        exact(result_report["models"][model_key].get("zeroFallbacks"), True, f"historical {model_key} fallbacks")
    historical_artifacts = {
        "resultReport": artifact(paths["step9Result"]),
        "resultReportMarkdown": artifact(paths["step9ResultMd"]),
        "validatedResult": artifact(paths["historicalRoot"] / "confirmation-result-v1.json", result_report["validatedResult"]["sha256"]),
        "aReport": artifact(paths["historicalRoot"] / "model-a-suite-a/native-raw/self-play-report.json", result_report["models"]["A"]["nativeReportSha256"]),
        "aGames": artifact(paths["historicalRoot"] / "model-a-suite-a/native-raw/self-play-games.jsonl", result_report["models"]["A"]["nativeGamesSha256"]),
        "bReport": artifact(paths["historicalRoot"] / "model-b-suite-b/native-raw/self-play-report.json", result_report["models"]["B"]["nativeReportSha256"]),
        "bGames": artifact(paths["historicalRoot"] / "model-b-suite-b/native-raw/self-play-games.jsonl", result_report["models"]["B"]["nativeGamesSha256"]),
    }
    # The abandoned 1,000 ms directory is recorded only as an exclusion.  No
    # existence check or file read is performed here.
    exclusions = {
        "abandoned1000msPartial": {
            "path": contract.get("abandoned1000msCampaign", {}).get("path"),
            "inspected": False,
            "eligibleForAnalysis": False,
            "reuseAllowed": False,
        },
        "production": "handcrafted evaluator remains unchanged",
        "newConfirmation": "no 1,000 ms or deployment qualification is inferred",
    }

    a_config = load_json(paths["step8Config"])
    b_config = load_json(paths["step9Config"])
    a_recipe = a_config["recipe"]
    b_recipe = b_config["recipe"]
    common_fields = ["featureFamily", "width", "loader", "objective", "optimizer", "schedule", "diagnostics", "export"]
    common_matches = {field: a_recipe.get(field) == b_recipe.get(field) for field in common_fields}
    exact_differences = {
        "trainingSeed": {"A": a_recipe["seed"], "B": b_recipe["seed"]},
        "initialization": {"A": a_recipe["initialization"], "B": b_recipe["initialization"]},
        "trainingCorpus": {"A": a_recipe["data"]["training"], "B": b_recipe["data"]["training"]},
        "validationCorpus": {"A": a_recipe["data"]["validation"], "B": b_recipe["data"]["validation"]},
        "trainerRunner": {"A": a_config["trainer"]["runner"], "B": b_config["trainer"]["runner"]},
        "checkpointSchedule": {"A": a_recipe["checkpoints"], "B": b_recipe["checkpoints"]},
        "experimentAndOutput": {"A": {"experimentId": a_config["experimentId"], "output": a_config["output"]}, "B": {"experimentId": b_config["experimentId"], "output": b_config["output"]}},
        "strengthScheduleId": {"A": a_config["strengthProtocol"]["scheduleId"], "B": b_config["strengthProtocol"]["scheduleId"]},
    }
    config_diffs = recursive_differences(a_recipe, b_recipe, "recipe")

    a_run = load_json(paths["step8Run"])
    b_run = load_json(paths["step9Run"])
    for label, run, expected_id in (("A", a_run, a_config["experimentId"]), ("B", b_run, b_config["experimentId"])):
        exact(run.get("schema"), "haitaka-anhoku-strength-first-training-run-v3", f"{label} run schema")
        exact(run.get("experimentId"), expected_id, f"{label} run experiment")
        exact(run.get("complete"), True, f"{label} run complete")
        exact(run.get("optimizerUpdates"), 4096, f"{label} run updates")
        exact(run.get("stopReason"), "maximum-updates", f"{label} run stop")
    run_summary = {
        "A": {key: a_run.get(key) for key in ("complete", "optimizerUpdates", "examplesSeen", "equivalentPasses", "stopReason", "bestValidation", "dataCursor", "timingSeconds")},
        "B": {key: b_run.get(key) for key in ("complete", "optimizerUpdates", "examplesSeen", "equivalentPasses", "stopReason", "bestValidation", "dataCursor", "timingSeconds")},
    }
    a_selection = load_json(paths["step8Selection"])
    b_selection = load_json(paths["step9Selection"])
    exact(a_selection.get("decision"), "selected-by-frozen-ranking", "A selection decision")
    exact(b_selection.get("decision"), "selected-by-frozen-ranking", "B selection decision")
    selection_summary = {
        "A": {"history": artifact(paths["step8Selection"]), "candidates": a_selection.get("candidates"), "selectedExport": selected_export_summary(paths["step8Root"] / "exports/selected-u00002688.nnue.json")},
        "B": {"history": artifact(paths["step9Selection"]), "candidates": b_selection.get("candidates"), "selectedExport": selected_export_summary(paths["step9Root"] / "exports/selected-u00000768.nnue.json"), "u2688Export": selected_export_summary(paths["step9Root"] / "exports/candidates/candidate-u00002688.nnue.json")},
    }
    parity = {
        "A_u2688": parity_summary(paths["step8Root"] / "comparison-u00002688/parity-receipt-u00002688.json", EXPECTED_HASHES["modelA"], EXPECTED_HASHES["runtime"], require_exact_runtime=False),
        "B_u768": parity_summary(paths["step9Root"] / "final-u00000768/parity-receipt-u00000768.json", EXPECTED_HASHES["modelB768"], EXPECTED_HASHES["runtime"]),
    }
    a_metrics = metric_at(paths["step8Metrics"], (2688, 4096))
    b_metrics = metric_at(paths["step9Metrics"], (768, 2688, 4096))
    diagnostics = {
        "A": {str(update): compact_metric(a_metrics[update]) for update in sorted(a_metrics)},
        "B": {str(update): compact_metric(b_metrics[update]) for update in sorted(b_metrics)},
    }

    a_data_root = model_a_data_root
    b_data_root = paths["step9Root"].parent / "step9-model-b-corpus-20260911-v1"
    corpus_manifests = {
        "A": {
            "training": load_json(a_data_root / "training.json"),
            "validation": load_json(a_data_root / "validation.json"),
        },
        "B": {
            "training": load_json(b_data_root / "training.json"),
            "validation": load_json(b_data_root / "validation.json"),
            "corpus": load_json(b_data_root / "model-b-corpus.json"),
        },
    }
    sample_a = {
        "training": audit_split("A training", a_data_root / "training.bin", a_data_root / "training.jsonl", 880000),
        "validation": audit_split("A validation", a_data_root / "validation.bin", a_data_root / "validation.jsonl", 60000),
    }
    sample_b = {
        "training": audit_split("B training", b_data_root / "training.bin", b_data_root / "training.jsonl", 880000),
        "validation": audit_split("B validation", b_data_root / "validation.bin", b_data_root / "validation.jsonl", 60000),
    }
    audit_value = {
        "schema": AUDIT_SCHEMA,
        "createdAt": now(),
        "status": "passed",
        "defects": [],
        "scope": "bounded CPU-only diagnosis; no model training, GPU rental, deployment, or new confirmation claim",
        "identity": identity,
        "historicalEvidence": historical_artifacts,
        "exclusions": exclusions,
        "configurationComparison": {
            "commonFields": common_matches,
            "allCommonFieldsMatch": all(common_matches.values()),
            "recipeDifferences": exact_differences,
            "recursiveRecipeDifferencePaths": config_diffs,
            "actualFeatureFamily": {"A": a_recipe["featureFamily"], "B": b_recipe["featureFamily"]},
            "actualWidth": {"A": a_recipe["width"], "B": b_recipe["width"]},
            "actualObjective": {"A": a_recipe["objective"], "B": b_recipe["objective"]},
            "actualOutputScale": {"A": a_recipe["objective"]["modelOutputToRuntimeScore"], "B": b_recipe["objective"]["modelOutputToRuntimeScore"]},
            "actualOptimizer": {"A": a_recipe["optimizer"], "B": b_recipe["optimizer"]},
            "actualSchedule": {"A": a_recipe["schedule"], "B": b_recipe["schedule"]},
            "actualUpdateCount": {"A": a_run["optimizerUpdates"], "B": b_run["optimizerUpdates"]},
            "dataOrderingAndExposure": {"A": {"loader": a_recipe["loader"], "run": run_summary["A"]}, "B": {"loader": b_recipe["loader"], "run": run_summary["B"]}},
            "exporters": {"A": a_config["trainer"]["runner"], "B": b_config["trainer"]["runner"]},
            "runtimes": {"frozenRuntime": identity["runtime"], "preservedRuntime": identity["preservedRuntime"]},
            "seeds": {"A": a_recipe["seed"], "B": b_recipe["seed"]},
        },
        "runHistory": run_summary,
        "selectionHistory": selection_summary,
        "parity": parity,
        "diagnostics": diagnostics,
        "corpus": {
            "manifests": corpus_manifests,
            "sampleRule": "evenly-spaced-record-index-v1; at most 10,000 records per split; binary fields decoded using the repository's 72-byte training ABI",
            "A": sample_a,
            "B": sample_b,
            "crossCorpusStaticValidation": {"run": False, "reason": "no existing exact-objective, exact-orientation tooling was used"},
        },
        "historicalConfirmationDecisionPreserved": True,
        "deploymentAuthorized": False,
    }
    write_json(output_dir / "audit.json", audit_value)
    write_json(output_dir / "corpus-sample-audit.json", {"schema": f"{AUDIT_SCHEMA}-corpus-sample-v1", "createdAt": audit_value["createdAt"], "A": sample_a, "B": sample_b})
    return audit_value


def prepare(root: Path, output_root: Path, audit_dir: Path, host_name: str, checkout: str, git_head: str, architecture: str, runner: Path, analyzer: Path) -> dict[str, Any]:
    if output_root.exists() and any(output_root.iterdir()):
        fail(f"diagnosis output root is nonempty; refusing overwrite: {output_root}")
    output_root.mkdir(parents=True, exist_ok=True)
    def bound(path: Path, expected: str | None = None) -> dict[str, Any]:
        return relative_artifact(path, output_root, expected)

    paths = default_paths(root)
    for key, expected in (("modelA", EXPECTED_HASHES["modelA"]), ("modelB2688", EXPECTED_HASHES["modelB2688"]), ("modelB768", EXPECTED_HASHES["modelB768"]), ("suiteA", EXPECTED_HASHES["suiteA"]), ("suiteB", EXPECTED_HASHES["suiteB"]), ("runtime", EXPECTED_HASHES["runtime"])):
        artifact(paths[key], expected)
    audit_json_source = audit_dir / "audit.json"
    sample_json_source = audit_dir / "corpus-sample-audit.json"
    audit_value = load_json(audit_json_source)
    exact(audit_value.get("status"), "passed", "audit status")
    exact(audit_value.get("deploymentAuthorized"), False, "audit deployment authorization")
    (output_root / "audit").mkdir(parents=True, exist_ok=True)
    shutil.copy2(audit_json_source, output_root / "audit/audit.json")
    shutil.copy2(sample_json_source, output_root / "audit/corpus-sample-audit.json")
    selection_a = make_selection(paths["suiteA"], "A", EXPECTED_HASHES["suiteA"])
    selection_b = make_selection(paths["suiteB"], "B", EXPECTED_HASHES["suiteB"])
    selection_a = write_selection_files(output_root, "A", selection_a)
    selection_b = write_selection_files(output_root, "B", selection_b)
    for directory in ("matches", "analysis"):
        (output_root / directory).mkdir(parents=True, exist_ok=True)
    audit_artifact = bound(output_root / "audit/audit.json")
    sample_artifact = bound(output_root / "audit/corpus-sample-audit.json")
    contract = load_json(paths["contract"])
    historical = load_json(paths["step9Result"])
    historical_cells = {
        "a2688-a": {
            "model": "A u2,688", "suite": "A", "source": "historical-selected-subset", "network": bound(paths["modelA"], EXPECTED_HASHES["modelA"]),
            "report": bound(paths["historicalRoot"] / "model-a-suite-a/native-raw/self-play-report.json", historical["models"]["A"]["nativeReportSha256"]),
            "games": bound(paths["historicalRoot"] / "model-a-suite-a/native-raw/self-play-games.jsonl", historical["models"]["A"]["nativeGamesSha256"]),
        },
        "b2688-b": {
            "model": "B u2,688", "suite": "B", "source": "historical-selected-subset", "network": bound(paths["modelB2688"], EXPECTED_HASHES["modelB2688"]),
            "report": bound(paths["historicalRoot"] / "model-b-suite-b/native-raw/self-play-report.json", historical["models"]["B"]["nativeReportSha256"]),
            "games": bound(paths["historicalRoot"] / "model-b-suite-b/native-raw/self-play-games.jsonl", historical["models"]["B"]["nativeGamesSha256"]),
        },
    }
    cells = {
        "a2688-b": {"model": "A u2,688", "suite": "B", "network": bound(paths["modelA"], EXPECTED_HASHES["modelA"]), "suiteSeed": 2026091302, "selection": "schedules/b-selection.json"},
        "b2688-a": {"model": "B u2,688", "suite": "A", "network": bound(paths["modelB2688"], EXPECTED_HASHES["modelB2688"]), "suiteSeed": 2026091301, "selection": "schedules/a-selection.json"},
        "b768-a": {"model": "B u768", "suite": "A", "network": bound(paths["modelB768"], EXPECTED_HASHES["modelB768"]), "suiteSeed": 2026091301, "selection": "schedules/a-selection.json"},
        "b768-b": {"model": "B u768", "suite": "B", "network": bound(paths["modelB768"], EXPECTED_HASHES["modelB768"]), "suiteSeed": 2026091302, "selection": "schedules/b-selection.json"},
    }
    execution_order = [
        ["a2688-b", 0], ["b2688-a", 0], ["b768-b", 0], ["b768-a", 0],
        ["b2688-a", 1], ["b768-b", 1], ["b768-a", 1], ["a2688-b", 1],
        ["b768-b", 2], ["b768-a", 2], ["a2688-b", 2], ["b2688-a", 2],
        ["b768-a", 3], ["a2688-b", 3], ["b2688-a", 3], ["b768-b", 3],
    ]
    manifest = {
        "schema": DIAGNOSIS_SCHEMA,
        "createdAt": now(),
        "scope": "bounded exploratory post-confirmation diagnosis; CPU-only; native MBP 100 ms fixed schedule",
        "deploymentAuthorized": False,
        "confirmationDecision": {"unchanged": True, "confirmationPassed": False, "production": "handcrafted", "candidateLaneClosed": True},
        "host": {"role": "mbp", "endpoint": "192.168.0.12", "hostname": host_name, "architecture": architecture, "checkout": checkout, "gitHead": git_head},
        "runtime": {"executed": bound(paths["runtime"], EXPECTED_HASHES["runtime"]), "preservedCopy": bound(paths["preservedRuntime"], EXPECTED_HASHES["runtime"])},
        "inputs": {
            "modelA_u2688": bound(paths["modelA"], EXPECTED_HASHES["modelA"]),
            "modelB_u2688": bound(paths["modelB2688"], EXPECTED_HASHES["modelB2688"]),
            "modelB_u768": bound(paths["modelB768"], EXPECTED_HASHES["modelB768"]),
            "suiteA": bound(paths["suiteA"], EXPECTED_HASHES["suiteA"]),
            "suiteB": bound(paths["suiteB"], EXPECTED_HASHES["suiteB"]),
            "frozenContract": bound(paths["contract"]),
            "frozenExecution": bound(paths["execution"]),
            "historicalResultReport": bound(paths["step9Result"]),
        },
        "originalEvidence": {"step9Result": bound(paths["step9Result"]), "historicalCells": historical_cells, "abandoned1000msPartialInspected": False},
        "audit": {"artifactAudit": audit_artifact, "corpusSampleAudit": sample_artifact, "crossCorpusStaticValidation": False},
        "selection": {
            "algorithm": "sha256('step9-diagnosis-v1\\n' || FULL_SUITE_SHA256 || '\\n' || OPENING_ID), lowercase digest rank, tie by opening ID",
            "fullPairs": FULL_PAIRS, "selectedPairs": SELECTED_PAIRS, "blockPairs": BLOCK_PAIRS, "blocks": 4, "restoredOriginalSuiteOrder": True,
            "A": selection_a["artifacts"], "B": selection_b["artifacts"],
        },
        "schedule": {
            "cells": cells, "historicalCells": historical_cells, "executionOrder": [{"position": index + 1, "cell": cell, "block": block} for index, (cell, block) in enumerate(execution_order)],
            "gamesPerBlock": BLOCK_GAMES, "pairsPerBlock": BLOCK_PAIRS, "totalNewGames": 4096, "totalNewPairs": 2048,
            "suiteSeeds": {"A": 2026091301, "B": 2026091302}, "movetimeMs": 100, "threads": 1, "concurrentGames": 1, "openingOrder": "sequential", "openingRandomPlies": 0, "maxPlies": 200,
            "commandShape": ["RUNTIME", "self-play", "--games", "256", "--threads", "1", "--a-eval", "nnue", "--b-eval", "handcrafted", "--a-nnue", "NETWORK", "--movetime-ms", "100", "--openings", "BLOCK_SFEN", "--opening-order", "sequential", "--opening-random-plies", "0", "--seed", "SUITE_SEED", "--max-plies", "200", "--report-dir", "ATTEMPT_DIR/native-raw"],
            "resumableBlocks": True, "scoreDependentStopping": False, "sampleExtension": False, "additionalCheckpoints": False,
        },
        "bootstrap": {"method": "paired-opening-score-splitmix64-v1", "seed": BOOTSTRAP_SEED, "replicates": BOOTSTRAP_REPLICATES, "interval": "two-sided-percentile-95-descriptive-unadjusted", "suiteBalancedWeights": {"A": 0.5, "B": 0.5}},
        "runner": bound(runner),
        "analyzer": bound(analyzer),
        "validation": {"blockSchema": BLOCK_SCHEMA, "historicalConfirmationValidation": "independent diagnostic validation after both new schedules complete", "outcomeDisclosureBeforeCompletion": "pass/fail, counts, active block, timing only"},
        "exclusions": {"abandoned1000msPartial": {"path": contract.get("abandoned1000msCampaign", {}).get("path"), "inspected": False, "eligible": False}, "gpuRental": False, "training": False, "deployment": False},
    }
    write_json(output_root / "manifest.json", manifest)
    manifest_hash = sha256(output_root / "manifest.json")
    status = {
        "schema": f"{DIAGNOSIS_SCHEMA}-status-v1", "state": "prepared", "createdAt": now(), "manifestSha256": manifest_hash,
        "runnerSha256": manifest["runner"]["sha256"], "analyzerSha256": manifest["analyzer"]["sha256"], "runnerPid": None, "gamePid": None, "active": None,
        "completedBlocks": [], "completedGames": 0, "totalBlocks": 16, "totalGames": 4096, "outcomesSealed": True,
    }
    write_json(output_root / "status.json", status)
    return {"manifest": artifact(output_root / "manifest.json"), "status": artifact(output_root / "status.json"), "selectionA": selection_a["artifacts"], "selectionB": selection_b["artifacts"]}


def expected_cell(manifest: dict[str, Any], cell: str) -> dict[str, Any]:
    cells = manifest["schedule"]["cells"]
    if cell not in cells:
        fail(f"unknown cell {cell}")
    return cells[cell]


def selection_for_cell(root: Path, manifest: dict[str, Any], cell: str) -> dict[str, Any]:
    definition = expected_cell(manifest, cell)
    selection_path = root / definition["selection"]
    return load_json(selection_path)


def resolve_report_path(root: Path, value: Any) -> Path:
    if not isinstance(value, str):
        fail("report command.openings must be a path")
    path = Path(value)
    return path if path.is_absolute() else root / path


def game_score(result: str) -> float:
    scores = {"a-win": 1.0, "draw": 0.5, "b-win": 0.0}
    if result not in scores:
        fail(f"invalid game result {result!r}")
    return scores[result]


def validate_game_common(game: dict[str, Any], index: int, pair_index: int, opening_id: str, sfen: str, total_games: int) -> float:
    where = f"game {index + 1}"
    exact(game.get("schema"), GAME_SCHEMA, f"{where}.schema")
    exact(game.get("schemaVersion"), GAME_SCHEMA_VERSION, f"{where}.schemaVersion")
    exact(game.get("gameIndex"), index + 1, f"{where}.gameIndex")
    exact(game.get("pairIndex"), pair_index, f"{where}.pairIndex")
    exact(game.get("aColor"), "black" if index % 2 == 0 else "white", f"{where}.aColor")
    exact(game.get("bColor"), "white" if index % 2 == 0 else "black", f"{where}.bColor")
    opening = game.get("opening")
    if not isinstance(opening, dict):
        fail(f"{where}.opening is not an object")
    exact(opening.get("source"), "suite", f"{where}.opening.source")
    exact(opening.get("suiteIndex"), pair_index, f"{where}.opening.suiteIndex")
    exact(opening.get("baseSfen"), sfen, f"{where}.opening.baseSfen")
    exact(opening.get("randomPlies"), 0, f"{where}.opening.randomPlies")
    exact(game.get("startSfen"), sfen, f"{where}.startSfen")
    exact(game.get("failureState"), None, f"{where}.failureState")
    exact(game.get("fallbacks"), 0, f"{where}.fallbacks")
    for engine in ("aBreakdown", "bBreakdown"):
        exact(game.get(engine, {}).get("fallbacks"), 0, f"{where}.{engine}.fallbacks")
    moves = game.get("moves")
    if not isinstance(moves, list) or any(not isinstance(move, str) or not move for move in moves):
        fail(f"{where}.moves invalid")
    exact(game.get("plies"), len(moves), f"{where}.plies")
    termination = game.get("terminationReason")
    exact(game.get("terminationSchema"), TERMINATION_SCHEMA, f"{where}.terminationSchema")
    if termination not in ALLOWED_TERMINATIONS:
        fail(f"{where}.terminationReason is unexpected")
    if termination == "maximum-plies":
        exact(game.get("plies"), 200, f"{where}.maximum-plies")
    result = game.get("result")
    score = game_score(result)
    expected_winner = {"a-win": "A", "b-win": "B", "draw": None}[result]
    exact(game.get("winner"), expected_winner, f"{where}.winner")
    if index >= total_games:
        fail(f"{where} outside expected schedule")
    return score


def validate_report_header(report: dict[str, Any], root: Path, block_sfen: Path, network: Path, network_hash: str, runtime_hash: str, seed: int, games: int) -> None:
    exact(report.get("schema"), "haitaka-self-play-report", "report.schema")
    exact(report.get("schemaVersion"), GAME_SCHEMA_VERSION, "report.schemaVersion")
    exact(report.get("ruleset"), "anhoku", "report.ruleset")
    exact(report.get("git", {}).get("executableSha256"), runtime_hash, "report.runtime hash")
    command = report.get("command", {})
    expected = {"games": games, "threads": 1, "aDepth": None, "bDepth": None, "movetimeMs": 100, "nodesPerMove": None, "sfen": None, "openingOrder": "sequential", "openingRandomPlies": 0, "seed": seed, "maxPlies": 200}
    for key, value in expected.items():
        exact(command.get(key), value, f"report.command.{key}")
    actual_openings = resolve_report_path(root, command.get("openings"))
    exact(actual_openings.resolve(), block_sfen.resolve(), "report.command.openings")
    engines = report.get("engines")
    if not isinstance(engines, list) or len(engines) != 2:
        fail("report must bind exactly two engines")
    exact(engines[0].get("label"), "A", "report candidate label")
    exact(engines[0].get("kind"), "nnue", "report candidate kind")
    exact(engines[0].get("nnueSha256"), network_hash, "report candidate network hash")
    actual_network = Path(engines[0].get("nnue", ""))
    if not actual_network.is_absolute():
        actual_network = root / actual_network
    exact(actual_network.resolve(), network.resolve(), "report candidate network path")
    exact(engines[1].get("label"), "B", "report opponent label")
    exact(engines[1].get("kind"), "handcrafted", "report opponent kind")


def validate_block_data(root: Path, manifest: dict[str, Any], cell: str, block: int, attempt_dir: Path, write_sealed: bool = True) -> dict[str, Any]:
    if block not in range(4):
        fail(f"invalid block {block}")
    definition = expected_cell(manifest, cell)
    selection = selection_for_cell(root, manifest, cell)
    rows = selection["rows"][block * BLOCK_PAIRS : (block + 1) * BLOCK_PAIRS]
    if len(rows) != BLOCK_PAIRS:
        fail(f"{cell} block {block} does not have 128 selected rows")
    prefix = "a" if definition["suite"] == "A" else "b"
    block_sfen = root / "schedules/blocks" / f"{prefix}-selected-block-{block}.sfen"
    report_path = attempt_dir / "native-raw/self-play-report.json"
    games_path = attempt_dir / "native-raw/self-play-games.jsonl"
    network_path = Path(definition["network"]["path"])
    if not network_path.is_absolute():
        network_path = root / network_path
    network_hash = definition["network"]["sha256"]
    runtime_hash = manifest["runtime"]["executed"]["sha256"]
    report = load_json(report_path)
    validate_report_header(report, root, block_sfen, network_path, network_hash, runtime_hash, int(definition["suiteSeed"]), BLOCK_GAMES)
    try:
        lines = games_path.read_text(encoding="utf-8").splitlines()
        games = [json.loads(line) for line in lines if line]
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        fail(f"cannot load block games: {exc}")
    exact(len(games), BLOCK_GAMES, f"{cell} block {block} game count")
    scores: list[float] = []
    wdl = Counter()
    pair_values: list[dict[str, Any]] = []
    for index, game in enumerate(games):
        if not isinstance(game, dict):
            fail(f"{cell} block {block} game {index + 1} is not an object")
        pair_index = index // 2
        row = rows[pair_index]
        score = validate_game_common(game, index, pair_index, row["openingId"], row["sfen"], BLOCK_GAMES)
        scores.append(score)
        wdl[game["result"]] += 1
    for pair_index in range(BLOCK_PAIRS):
        pair_games = games[2 * pair_index : 2 * pair_index + 2]
        pair_values.append({"selectedIndex": block * BLOCK_PAIRS + pair_index, "originalSuiteIndex": rows[pair_index]["originalSuiteIndex"], "openingId": rows[pair_index]["openingId"], "score": (scores[2 * pair_index] + scores[2 * pair_index + 1]) / 2.0, "results": [pair_games[0]["result"], pair_games[1]["result"]]})
    summary = report.get("summary", {})
    exact(summary.get("games"), BLOCK_GAMES, f"{cell} block {block} summary.games")
    exact(summary.get("aWins"), wdl["a-win"], f"{cell} block {block} summary.aWins")
    exact(summary.get("bWins"), wdl["b-win"], f"{cell} block {block} summary.bWins")
    exact(summary.get("draws"), wdl["draw"], f"{cell} block {block} summary.draws")
    exact(summary.get("pairCount"), BLOCK_PAIRS, f"{cell} block {block} summary.pairCount")
    exact(summary.get("fallbacks"), 0, f"{cell} block {block} summary.fallbacks")
    close(summary.get("aScore"), sum(scores), f"{cell} block {block} summary.aScore")
    close(summary.get("scoreRate"), sum(scores) / BLOCK_GAMES, f"{cell} block {block} summary.scoreRate")
    value = {
        "schema": BLOCK_SCHEMA,
        "createdAt": now(),
        "manifestSha256": sha256(root / "manifest.json"),
        "cell": cell,
        "block": block,
        "attempt": attempt_dir.name,
        "model": definition["model"],
        "suite": definition["suite"],
        "network": artifact(network_path, network_hash),
        "runtimeSha256": runtime_hash,
        "suiteSeed": definition["suiteSeed"],
        "blockSfen": artifact(block_sfen),
        "rawReport": artifact(report_path),
        "rawGames": artifact(games_path),
        "games": BLOCK_GAMES,
        "pairs": BLOCK_PAIRS,
        "wdl": {"wins": wdl["a-win"], "losses": wdl["b-win"], "draws": wdl["draw"]},
        "score": sum(scores),
        "scoreRate": sum(scores) / BLOCK_GAMES,
        "pairScores": pair_values,
        "validation": {"failureStateNull": True, "zeroFallbacks": True, "orderedGameIndexes": True, "alternatingColors": True, "exactSelectedSfen": True, "terminationVocabulary": sorted(ALLOWED_TERMINATIONS)},
    }
    if write_sealed:
        write_json(attempt_dir / "validation.json", value)
    return value


def set_status(root: Path, **updates: Any) -> dict[str, Any]:
    path = root / "status.json"
    status = load_json(path)
    status.update(updates)
    status["updatedAt"] = now()
    write_json(path, status)
    return status


def create_fixture_game(index: int, pair: int, row: dict[str, Any]) -> dict[str, Any]:
    a_win = True
    return {
        "schema": GAME_SCHEMA, "schemaVersion": GAME_SCHEMA_VERSION, "gameIndex": index + 1, "pairIndex": pair,
        "aColor": "black" if index % 2 == 0 else "white", "bColor": "white" if index % 2 == 0 else "black",
        "opening": {"source": "suite", "suiteIndex": pair, "baseSfen": row["sfen"], "randomPlies": 0}, "startSfen": row["sfen"],
        "failureState": None, "fallbacks": 0, "aBreakdown": {"fallbacks": 0}, "bBreakdown": {"fallbacks": 0}, "moves": ["7g7f"], "plies": 1,
        "terminationSchema": TERMINATION_SCHEMA, "terminationReason": "no-legal-move", "result": "a-win" if a_win else "b-win", "winner": "A" if a_win else "B",
    }


def create_fixture_report(block_sfen: Path, network: Path, network_hash: str, runtime_hash: str, seed: int, games: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "schema": "haitaka-self-play-report", "schemaVersion": GAME_SCHEMA_VERSION, "ruleset": "anhoku", "git": {"executableSha256": runtime_hash},
        "command": {"games": BLOCK_GAMES, "threads": 1, "aDepth": None, "bDepth": None, "movetimeMs": 100, "nodesPerMove": None, "sfen": None, "openings": str(block_sfen.resolve()), "openingOrder": "sequential", "openingRandomPlies": 0, "seed": seed, "maxPlies": 200},
        "engines": [{"label": "A", "kind": "nnue", "nnue": str(network.resolve()), "nnueSha256": network_hash}, {"label": "B", "kind": "handcrafted", "nnue": None}],
        "summary": {"games": BLOCK_GAMES, "aWins": BLOCK_GAMES, "bWins": 0, "draws": 0, "pairCount": BLOCK_PAIRS, "fallbacks": 0, "aScore": float(BLOCK_GAMES), "scoreRate": 1.0},
    }


def fixture_case(name: str, diagnosis_root: Path, fixture_work: Path, manifest: dict[str, Any], base: Path, mutate: Any) -> bool:
    target = fixture_work / name
    shutil.copytree(base, target)
    mutate(target)
    try:
        validate_block_data(diagnosis_root, manifest, "a2688-b", 0, target, write_sealed=False)
    except DiagnosisError:
        return True
    return False


def fixtures(root: Path, output: Path) -> dict[str, Any]:
    manifest = load_json(root / "manifest.json")
    definition = expected_cell(manifest, "a2688-b")
    selection = selection_for_cell(root, manifest, "a2688-b")
    rows = selection["rows"][:BLOCK_PAIRS]
    network = root / definition["network"]["path"]
    block_sfen = root / "schedules/blocks/b-selected-block-0.sfen"
    work = Path(tempfile.mkdtemp(prefix="step9-diagnosis-fixtures-", dir=str(output.parent)))
    try:
        base = work / "valid"
        raw = base / "native-raw"
        raw.mkdir(parents=True)
        games = [create_fixture_game(index, index // 2, rows[index // 2]) for index in range(BLOCK_GAMES)]
        write_json(raw / "self-play-report.json", create_fixture_report(block_sfen, network, definition["network"]["sha256"], manifest["runtime"]["executed"]["sha256"], definition["suiteSeed"], games))
        (raw / "self-play-games.jsonl").write_text("".join(json.dumps(game, sort_keys=True) + "\n" for game in games), encoding="utf-8")
        valid = True
        try:
            validate_block_data(root, manifest, "a2688-b", 0, base, write_sealed=False)
        except DiagnosisError:
            valid = False
        cases: dict[str, Any] = {"valid-fixture-accepted": valid}
        def mutate_games(path: Path, function: Any) -> None:
            game_path = path / "native-raw/self-play-games.jsonl"
            items = [json.loads(line) for line in game_path.read_text(encoding="utf-8").splitlines()]
            function(items)
            game_path.write_text("".join(json.dumps(item, sort_keys=True) + "\n" for item in items), encoding="utf-8")
        def mutate_report(path: Path, function: Any) -> None:
            report_path = path / "native-raw/self-play-report.json"
            report = load_json(report_path)
            function(report)
            write_json(report_path, report)
        mutations = {
            "missing-game-rejected": lambda path: mutate_games(path, lambda items: items.pop()),
            "duplicate-game-rejected": lambda path: mutate_games(path, lambda items: items.append(dict(items[0]))),
            "wrong-network-rejected": lambda path: mutate_report(path, lambda report: report["engines"][0].update({"nnueSha256": "0" * 64})),
            "wrong-suite-rejected": lambda path: mutate_report(path, lambda report: report["command"].update({"openings": str((root / "schedules/a-selected.sfen").resolve())})),
            "wrong-sfen-rejected": lambda path: mutate_games(path, lambda items: items[0].update({"startSfen": "bad"})),
            "wrong-color-order-rejected": lambda path: mutate_games(path, lambda items: items[1].update({"aColor": "black"})),
            "wrong-seed-rejected": lambda path: mutate_report(path, lambda report: report["command"].update({"seed": 1})),
            "wrong-command-settings-rejected": lambda path: mutate_report(path, lambda report: report["command"].update({"threads": 2})),
            "malformed-wdl-rejected": lambda path: mutate_games(path, lambda items: items[0].update({"result": "invalid", "winner": "A"})),
            "inconsistent-wdl-rejected": lambda path: mutate_report(path, lambda report: report["summary"].update({"aWins": 0})),
            "non-null-failure-rejected": lambda path: mutate_games(path, lambda items: items[0].update({"failureState": "failed"})),
            "aggregate-fallback-rejected": lambda path: mutate_report(path, lambda report: report["summary"].update({"fallbacks": 1})),
            "per-engine-fallback-rejected": lambda path: mutate_games(path, lambda items: items[0]["aBreakdown"].update({"fallbacks": 1})),
        }
        for name, mutation in mutations.items():
            cases[name] = fixture_case(name, root, work, manifest, base, mutation)
        required = ["valid-fixture-accepted"] + list(mutations)
        exact(all(cases.get(key) is True for key in required), True, "diagnostic fixture outcomes")
        result = {"schema": FIXTURE_SCHEMA, "createdAt": now(), "status": "passed", "cases": cases, "requiredRejections": list(mutations)}
        write_json(output, result)
        return result
    finally:
        shutil.rmtree(work, ignore_errors=True)


class SplitMix64:
    def __init__(self, seed: int):
        self.state = seed & ((1 << 64) - 1)

    def next(self) -> int:
        self.state = (self.state + 0x9E3779B97F4A7C15) & ((1 << 64) - 1)
        value = self.state
        value = ((value ^ (value >> 30)) * 0xBF58476D1CE4E5B9) & ((1 << 64) - 1)
        value = ((value ^ (value >> 27)) * 0x94D049BB133111EB) & ((1 << 64) - 1)
        return (value ^ (value >> 31)) & ((1 << 64) - 1)


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    position = (len(ordered) - 1) * fraction
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return ordered[lower]
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def bootstrap(left: list[float], right: list[float], seed: int, balanced: bool = False, other: tuple[list[float], list[float]] | None = None) -> dict[str, Any]:
    if len(left) != len(right) or len(left) != SELECTED_PAIRS:
        fail("bootstrap inputs must be matched 512-pair vectors")
    rng = SplitMix64(seed)
    replicates: list[float] = []
    if balanced:
        if other is None or len(other[0]) != SELECTED_PAIRS or len(other[1]) != SELECTED_PAIRS:
            fail("balanced bootstrap requires two matched suites")
        for _ in range(BOOTSTRAP_REPLICATES):
            # Draw the index once per vector, preserving all model values for
            # the same opening.  The explicit loop keeps the PRNG contract
            # independent of Python's random implementation.
            first = 0.0
            second = 0.0
            for _index in range(SELECTED_PAIRS):
                index = rng.next() % SELECTED_PAIRS
                first += left[index] - right[index]
                index = rng.next() % SELECTED_PAIRS
                second += other[0][index] - other[1][index]
            replicates.append((first + second) / (2.0 * SELECTED_PAIRS))
    else:
        for _ in range(BOOTSTRAP_REPLICATES):
            total = 0.0
            for _index in range(SELECTED_PAIRS):
                index = rng.next() % SELECTED_PAIRS
                total += left[index] - right[index]
            replicates.append(total / SELECTED_PAIRS)
    point = sum(left[index] - right[index] for index in range(SELECTED_PAIRS)) / SELECTED_PAIRS
    if balanced:
        point = (point + sum(other[0][index] - other[1][index] for index in range(SELECTED_PAIRS)) / SELECTED_PAIRS) / 2.0
    return {"point": point, "lower95": percentile(replicates, 0.025), "upper95": percentile(replicates, 0.975), "replicates": BOOTSTRAP_REPLICATES, "seed": seed, "unit": "opening pair", "interval": "two-sided-percentile-95-descriptive-unadjusted"}


def historical_scores(root: Path, manifest: dict[str, Any], cell_key: str) -> tuple[list[float], dict[str, Any]]:
    entry = manifest["schedule"]["historicalCells"][cell_key]
    suite_name = entry["suite"]
    suite_input = manifest["inputs"]["suiteA" if suite_name == "A" else "suiteB"]
    suite_path = path_from(root, suite_input["path"])
    suite_hash = EXPECTED_HASHES["suiteA"] if suite_name == "A" else EXPECTED_HASHES["suiteB"]
    suite = load_suite(suite_path, suite_hash)
    report_path = root / entry["report"]["path"]
    games_path = root / entry["games"]["path"]
    report = load_json(report_path)
    command = report.get("command", {})
    seed = 2026091301 if suite_name == "A" else 2026091302
    exact(command.get("games"), 4096, f"historical {cell_key} games")
    exact(command.get("threads"), 1, f"historical {cell_key} threads")
    exact(command.get("movetimeMs"), 100, f"historical {cell_key} movetime")
    exact(command.get("openingOrder"), "sequential", f"historical {cell_key} opening order")
    exact(command.get("openingRandomPlies"), 0, f"historical {cell_key} random plies")
    exact(command.get("seed"), seed, f"historical {cell_key} seed")
    exact(report.get("git", {}).get("executableSha256"), manifest["runtime"]["executed"]["sha256"], f"historical {cell_key} runtime")
    exact(report.get("engines", [])[0].get("nnueSha256"), entry["network"]["sha256"], f"historical {cell_key} network")
    try:
        games = [json.loads(line) for line in games_path.read_text(encoding="utf-8").splitlines() if line]
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        fail(f"cannot load historical games {cell_key}: {exc}")
    exact(len(games), FULL_PAIRS * 2, f"historical {cell_key} game count")
    scores: list[float] = []
    wdl = Counter()
    for index, game in enumerate(games):
        pair = index // 2
        score = validate_game_common(game, index, pair, suite[pair][0], suite[pair][1], FULL_PAIRS * 2)
        scores.append(score)
        wdl[game["result"]] += 1
    summary = report.get("summary", {})
    exact(summary.get("games"), FULL_PAIRS * 2, f"historical {cell_key} summary games")
    exact(summary.get("aWins"), wdl["a-win"], f"historical {cell_key} summary wins")
    exact(summary.get("bWins"), wdl["b-win"], f"historical {cell_key} summary losses")
    exact(summary.get("draws"), wdl["draw"], f"historical {cell_key} summary draws")
    exact(summary.get("pairCount"), FULL_PAIRS, f"historical {cell_key} summary pairs")
    exact(summary.get("fallbacks"), 0, f"historical {cell_key} summary fallbacks")
    close(summary.get("aScore"), sum(scores), f"historical {cell_key} summary score")
    selected = load_json(root / ("schedules/a-selection.json" if suite_name == "A" else "schedules/b-selection.json"))
    pairs = [(scores[row["originalSuiteIndex"] * 2] + scores[row["originalSuiteIndex"] * 2 + 1]) / 2.0 for row in selected["rows"]]
    return pairs, {"source": "historical-selected-subset", "suite": suite_name, "games": SELECTED_PAIRS * 2, "pairs": SELECTED_PAIRS, "wdl": {"wins": sum(scores[row["originalSuiteIndex"] * 2] == 1.0 for row in selected["rows"]) + sum(scores[row["originalSuiteIndex"] * 2 + 1] == 1.0 for row in selected["rows"]), "losses": sum(scores[row["originalSuiteIndex"] * 2] == 0.0 for row in selected["rows"]) + sum(scores[row["originalSuiteIndex"] * 2 + 1] == 0.0 for row in selected["rows"]), "draws": sum(scores[row["originalSuiteIndex"] * 2] == 0.5 for row in selected["rows"]) + sum(scores[row["originalSuiteIndex"] * 2 + 1] == 0.5 for row in selected["rows"])}, "report": artifact(report_path), "gamesArtifact": artifact(games_path)}


def new_scores(root: Path, manifest: dict[str, Any], cell: str) -> tuple[list[float], dict[str, Any]]:
    values: list[float] = []
    wdl = Counter()
    blocks: list[dict[str, Any]] = []
    for block in range(4):
        validation_path = root / "matches" / cell / f"block-{block}" / "attempt-1/validation.json"
        if not validation_path.is_file():
            attempts = sorted((root / "matches" / cell / f"block-{block}").glob("attempt-*/validation.json"))
            if not attempts:
                fail(f"missing validated block for {cell} block {block}")
            validation_path = attempts[-1]
        value = load_json(validation_path)
        exact(value.get("schema"), BLOCK_SCHEMA, f"{cell} block schema")
        exact(value.get("cell"), cell, f"{cell} block identity")
        exact(value.get("block"), block, f"{cell} block index")
        exact(value.get("pairs"), BLOCK_PAIRS, f"{cell} block pairs")
        exact(value.get("games"), BLOCK_GAMES, f"{cell} block games")
        values.extend(float(item["score"]) for item in value["pairScores"])
        wdl["wins"] += int(value["wdl"]["wins"])
        wdl["losses"] += int(value["wdl"]["losses"])
        wdl["draws"] += int(value["wdl"]["draws"])
        blocks.append({"block": block, "validation": artifact(validation_path), "wdl": value["wdl"], "scoreRate": value["scoreRate"]})
    exact(len(values), SELECTED_PAIRS, f"{cell} selected pair count")
    return values, {"source": "new-fixed-cell", "suite": expected_cell(manifest, cell)["suite"], "games": SELECTED_PAIRS * 2, "pairs": SELECTED_PAIRS, "wdl": dict(wdl), "scoreRate": sum(values) / SELECTED_PAIRS, "blocks": blocks}


def cell_markdown(label: str, value: dict[str, Any]) -> str:
    wdl = value["wdl"]
    return f"| {label} | {value['source']} | {value['games']} | {wdl['wins']}–{wdl['losses']}–{wdl['draws']} | {value['scoreRate']:.4%} |"


def analyze(root: Path, external_json: Path, external_md: Path) -> dict[str, Any]:
    manifest = load_json(root / "manifest.json")
    exact(manifest.get("schema"), DIAGNOSIS_SCHEMA, "manifest schema")
    status = load_json(root / "status.json")
    exact(status.get("state"), "complete", "diagnosis status")
    exact(status.get("completedGames"), 4096, "diagnosis completed games")
    audit_value = load_json(root / "audit/audit.json")
    exact(audit_value.get("status"), "passed", "diagnosis audit status")
    # Validate all sealed blocks again only after the runner has declared the
    # complete schedule.  This is where outcome-bearing data is first opened
    # by the final analyzer.
    cells_new = {cell: new_scores(root, manifest, cell) for cell in ("a2688-b", "b2688-a", "b768-a", "b768-b")}
    historical_a, historical_a_meta = historical_scores(root, manifest, "a2688-a")
    historical_b, historical_b_meta = historical_scores(root, manifest, "b2688-b")
    cells: dict[str, tuple[list[float], dict[str, Any]]] = {
        "A u2,688 / Suite A": (historical_a, historical_a_meta),
        "A u2,688 / Suite B": cells_new["a2688-b"],
        "B u2,688 / Suite A": cells_new["b2688-a"],
        "B u2,688 / Suite B": (historical_b, historical_b_meta),
        "B u768 / Suite A": cells_new["b768-a"],
        "B u768 / Suite B": cells_new["b768-b"],
    }
    contrasts: dict[str, Any] = {}
    a_minus_b_a = bootstrap(historical_a, cells_new["b2688-a"][0], BOOTSTRAP_SEED)
    a_minus_b_b = bootstrap(cells_new["a2688-b"][0], historical_b, BOOTSTRAP_SEED + 1)
    a_minus_b_balanced = bootstrap(historical_a, cells_new["b2688-a"][0], BOOTSTRAP_SEED + 2, balanced=True, other=(cells_new["a2688-b"][0], historical_b))
    b768_minus_b2688_a = bootstrap(cells_new["b768-a"][0], cells_new["b2688-a"][0], BOOTSTRAP_SEED + 3)
    b768_minus_b2688_b = bootstrap(cells_new["b768-b"][0], historical_b, BOOTSTRAP_SEED + 4)
    b768_minus_b2688_balanced = bootstrap(cells_new["b768-a"][0], cells_new["b2688-a"][0], BOOTSTRAP_SEED + 5, balanced=True, other=(cells_new["b768-b"][0], historical_b))
    contrasts.update({
        "A_u2688_minus_B_u2688_suite_A": a_minus_b_a,
        "A_u2688_minus_B_u2688_suite_B": a_minus_b_b,
        "A_u2688_minus_B_u2688_suite_balanced": a_minus_b_balanced,
        "B_u768_minus_B_u2688_suite_A": b768_minus_b2688_a,
        "B_u768_minus_B_u2688_suite_B": b768_minus_b2688_b,
        "B_u768_minus_B_u2688_suite_balanced": b768_minus_b2688_balanced,
    })
    a_points = [a_minus_b_a["point"], a_minus_b_b["point"]]
    b_points = [b768_minus_b2688_a["point"], b768_minus_b2688_b["point"]]
    supported: list[str] = []
    unresolved = ["corpus attribution", "initialization attribution", "recipe attribution", "session/thermal attribution", "sampling noise", "checkpoint-selection variance"]
    if all(value > 0 for value in a_points):
        supported.append("A exceeds B u2,688 descriptively on both matched suites; this supports a B-model generalization-weakness hypothesis.")
    if a_points[0] * a_points[1] < 0:
        supported.append("The A-versus-B ranking changes sign by suite, supporting a model-by-opening-distribution interaction hypothesis.")
    if all(value > 0 for value in b_points):
        supported.append("B u768 exceeds B u2,688 descriptively on both matched suites; this supports an earlier-checkpoint-generalizes-better hypothesis.")
    elif b_points[0] * b_points[1] < 0:
        supported.append("B u768 improves on only one suite, supporting a checkpoint-by-suite interaction hypothesis.")
    if not supported or any(contrast["lower95"] <= 0 <= contrast["upper95"] for contrast in contrasts.values()):
        supported.append("At least one exploratory contrast remains compatible with no difference; the diagnosis is not a causal or confirmatory conclusion.")
    if all(value > 0 for value in a_points):
        next_proposal = "If NNUE work is resumed after review, use one separately reviewed prospective lane that isolates the B-model generalization hypothesis; keep corpus, seed, recipe, and session factors explicitly unresolved and predeclare one fixed endpoint."
    elif all(value > 0 for value in b_points):
        next_proposal = "If NNUE work is resumed after review, use one separately reviewed prospective lane that fixes the checkpoint horizon at the earlier B u768 candidate; do not infer that checkpoint deterioration is causal from this comparison."
    else:
        next_proposal = "Stop this NNUE candidate line. Any further work needs a separately reviewed prospective hypothesis and budget; do not extend this diagnosis or reuse its confirmation decision."
    report = {
        "schema": REPORT_SCHEMA,
        "createdAt": now(),
        "completionStatus": "complete",
        "deploymentAuthorized": False,
        "confirmationDecisionUnchanged": True,
        "confirmationPassed": False,
        "decision": "confirmation-failed-handcrafted-remains-production",
        "scope": manifest["scope"],
        "manifest": artifact(root / "manifest.json"),
        "host": manifest["host"],
        "runtime": manifest["runtime"],
        "hashes": manifest["inputs"],
        "exclusions": manifest["exclusions"],
        "retries": {"policy": "rerun whole 128-pair block under a new attempt; no partial splice", "attempts": {cell: [{"block": block, "attempt": path.parent.name} for block in range(4) for path in (root / "matches" / cell / f"block-{block}").glob("attempt-*/validation.json")] for cell in cells_new}},
        "validation": {"auditPassed": True, "fixtureArtifact": artifact(root / "audit/diagnostic-fixtures.json"), "newCells": {cell: meta for cell, (_scores, meta) in cells_new.items()}, "allNewBlocksValidated": True, "outcomesOpenedAfterAllNewCells": True},
        "cells": {label: {**meta, "scoreRate": sum(values) / len(values), "meanPairScore": sum(values) / len(values)} for label, (values, meta) in cells.items()},
        "contrasts": contrasts,
        "audit": audit_value,
        "supportedHypotheses": supported,
        "unresolvedAlternatives": unresolved,
        "interpretationLimits": ["Opening pairs are the independent units; games within a pair are color-swapped.", "Historical diagonal cells ran in an earlier session and retain session/thermal confounding.", "Two trained models cannot causally separate corpus, initialization, recipe, or session effects.", "Intervals are descriptive, exploratory, two-sided, unadjusted for multiple comparisons.", "This diagnosis establishes neither 1,000 ms strength nor shipped-WASM qualification nor deployment authorization."],
        "nextProposal": next_proposal,
        "remainingOwnedProcesses": [],
    }
    write_json(root / "analysis/report.json", report)
    write_json(root / "report.json", report)
    md_lines = [
        "# Step 9 post-confirmation diagnosis",
        "",
        "**Decision remains unchanged: confirmation failed; handcrafted remains production. `deploymentAuthorized: false`.**",
        "",
        "This is bounded exploratory development evidence on the native MBP at 100 ms per move. It does not establish 1,000 ms strength, shipped-WASM qualification, or deployment authorization.",
        "",
        "## Scope and validation",
        "",
        f"Manifest: `{manifest['manifest']['path']}` (`{manifest['manifest']['sha256']}`). The audit passed, all four new cells completed 4,096 games, every block passed strict validation, and outcomes were opened only after the fixed schedule completed. The abandoned 1,000 ms partial remains unopened and excluded.",
        "",
        "## Six matched cells",
        "",
        "| Model / suite | Source | Games | W–L–D | Candidate pair score |",
        "| --- | --- | ---: | ---: | ---: |",
    ]
    for label in ("A u2,688 / Suite A", "A u2,688 / Suite B", "B u2,688 / Suite A", "B u2,688 / Suite B", "B u768 / Suite A", "B u768 / Suite B"):
        md_lines.append(cell_markdown(label, report["cells"][label]))
    md_lines.extend(["", "## Matched contrasts", "", "All intervals are paired-opening bootstrap 95% descriptive intervals, 10,000 replicates, exploratory and unadjusted.", "", "| Contrast | Estimate | 95% interval |", "| --- | ---: | ---: |"])
    labels = [
        ("A u2,688 − B u2,688, Suite A", "A_u2688_minus_B_u2688_suite_A"),
        ("A u2,688 − B u2,688, Suite B", "A_u2688_minus_B_u2688_suite_B"),
        ("A u2,688 − B u2,688, suite-balanced", "A_u2688_minus_B_u2688_suite_balanced"),
        ("B u768 − B u2,688, Suite A", "B_u768_minus_B_u2688_suite_A"),
        ("B u768 − B u2,688, Suite B", "B_u768_minus_B_u2688_suite_B"),
        ("B u768 − B u2,688, suite-balanced", "B_u768_minus_B_u2688_suite_balanced"),
    ]
    for label, key in labels:
        value = contrasts[key]
        md_lines.append(f"| {label} | {value['point']:+.4%} | [{value['lower95']:+.4%}, {value['upper95']:+.4%}] |")
    md_lines.extend(["", "## Audit observations", "", "The two recipes match on feature family, width, robust-clipped Huber objective, output scale, Ranger optimizer, loader semantics, learning-rate schedule, diagnostics, and serializer family. Differences are the training seed, corpus manifests/content, trainer wrapper/output history, checkpoint schedule, experiment identity, and selection outcome. The audit found no identity, export, orientation, recipe, or data-integrity defect.", "", "B u768 has a passed parity receipt bound to the exact u768 network and frozen runtime. The historical A u2,688 parity receipt passes its network checks but records a prior runtime hash; it is retained as historical evidence and is not used as exact-runtime parity evidence. Existing B u768 and B u2,688 loss, saturation, output-scale, and update-magnitude diagnostics are preserved in the machine-readable audit.", "", "## Interpretation and limits", ""])
    for item in supported:
        md_lines.append(f"- {item}")
    md_lines.extend(["", "The historical diagonal cells ran in an earlier session, so contrasts containing them retain session and thermal confounding. The crossover can describe model-by-suite interaction, but cannot causally isolate corpus, initialization, recipe, or session effects. A favorable u768 result cannot rescue Step 9 or authorize promotion.", "", f"**Bounded next proposal:** {next_proposal}", "", "Remaining owned processes: none."])
    markdown = "\n".join(md_lines) + "\n"
    (root / "report.md").write_text(markdown, encoding="utf-8")
    external_json.parent.mkdir(parents=True, exist_ok=True)
    external_md.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(root / "report.json", external_json)
    shutil.copy2(root / "report.md", external_md)
    return report


def validate_report(path: Path, root: Path) -> dict[str, Any]:
    report = load_json(path)
    exact(report.get("schema"), REPORT_SCHEMA, "report schema")
    exact(report.get("completionStatus"), "complete", "report completion")
    exact(report.get("deploymentAuthorized"), False, "report deployment authorization")
    exact(report.get("confirmationDecisionUnchanged"), True, "report unchanged decision")
    exact(report.get("confirmationPassed"), False, "report confirmation decision")
    manifest = root / "manifest.json"
    exact(report.get("manifest", {}).get("sha256"), sha256(manifest), "report manifest hash")
    exact(len(report.get("cells", {})), 6, "report cell count")
    exact(len(report.get("contrasts", {})), 6, "report contrast count")
    for name, value in report["contrasts"].items():
        exact(value.get("replicates"), BOOTSTRAP_REPLICATES, f"{name} bootstrap replicates")
        exact(value.get("unit"), "opening pair", f"{name} bootstrap unit")
        for key in ("point", "lower95", "upper95"):
            finite(value.get(key), f"{name}.{key}")
    exact(report.get("remainingOwnedProcesses"), [], "report remaining owned processes")
    return {"status": "passed", "path": display(path), "sha256": sha256(path), "deploymentAuthorized": False, "cells": 6, "contrasts": 6}


def cmd_audit(args: argparse.Namespace) -> dict[str, Any]:
    root = Path(args.root).resolve()
    output = Path(args.output_dir).resolve()
    data_root = Path(args.model_a_data_root).resolve() if args.model_a_data_root else root / "out/anhoku-reboot-r4-replacement/datasets"
    return audit(root, output, data_root)


def cmd_prepare(args: argparse.Namespace) -> dict[str, Any]:
    root = Path(args.root).resolve()
    return prepare(root, Path(args.output_root).resolve(), Path(args.audit_dir).resolve(), args.host_name, args.checkout, args.git_head, args.architecture, Path(args.runner).resolve(), Path(args.analyzer).resolve())


def cmd_validate_block(args: argparse.Namespace) -> dict[str, Any]:
    root = Path(args.root).resolve()
    manifest = load_json(root / "manifest.json")
    value = validate_block_data(root, manifest, args.cell, int(args.block), Path(args.attempt_dir).resolve(), write_sealed=True)
    return {"status": "passed", "cell": args.cell, "block": int(args.block), "attempt": Path(args.attempt_dir).name, "games": BLOCK_GAMES, "pairs": BLOCK_PAIRS}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    audit_parser = sub.add_parser("audit")
    audit_parser.add_argument("--root", default=str(ROOT))
    audit_parser.add_argument("--output-dir", required=True)
    audit_parser.add_argument("--model-a-data-root")
    prepare_parser = sub.add_parser("prepare")
    prepare_parser.add_argument("--root", default=str(ROOT))
    prepare_parser.add_argument("--output-root", required=True)
    prepare_parser.add_argument("--audit-dir", required=True)
    prepare_parser.add_argument("--host-name", required=True)
    prepare_parser.add_argument("--checkout", required=True)
    prepare_parser.add_argument("--git-head", required=True)
    prepare_parser.add_argument("--architecture", required=True)
    prepare_parser.add_argument("--runner", required=True)
    prepare_parser.add_argument("--analyzer", required=True)
    fixtures_parser = sub.add_parser("fixtures")
    fixtures_parser.add_argument("--root", required=True)
    fixtures_parser.add_argument("--output", required=True)
    block_parser = sub.add_parser("validate-block")
    block_parser.add_argument("--root", required=True)
    block_parser.add_argument("--cell", required=True)
    block_parser.add_argument("--block", required=True, type=int)
    block_parser.add_argument("--attempt-dir", required=True)
    analyze_parser = sub.add_parser("analyze")
    analyze_parser.add_argument("--root", required=True)
    analyze_parser.add_argument("--external-json", required=True)
    analyze_parser.add_argument("--external-md", required=True)
    report_parser = sub.add_parser("validate-report")
    report_parser.add_argument("--root", required=True)
    report_parser.add_argument("--report", required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "audit":
            result = cmd_audit(args)
        elif args.command == "prepare":
            result = cmd_prepare(args)
        elif args.command == "fixtures":
            result = fixtures(Path(args.root).resolve(), Path(args.output).resolve())
        elif args.command == "validate-block":
            result = cmd_validate_block(args)
        elif args.command == "analyze":
            result = analyze(Path(args.root).resolve(), Path(args.external_json).resolve(), Path(args.external_md).resolve())
        elif args.command == "validate-report":
            result = validate_report(Path(args.report).resolve(), Path(args.root).resolve())
        else:
            fail(f"unknown command {args.command}")
        print(json.dumps(result if args.command not in ("analyze",) else {"status": "complete", "deploymentAuthorized": False, "cells": 6, "contrasts": 6}, sort_keys=True))
        return 0
    except DiagnosisError as exc:
        if args.command == "validate-block":
            print("validation failed", file=sys.stderr)
        else:
            print(f"diagnosis failed: {exc}", file=sys.stderr)
        return 1
    except (OSError, subprocess.SubprocessError) as exc:
        print(f"diagnosis failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
