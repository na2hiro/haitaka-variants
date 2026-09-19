#!/usr/bin/env python3
"""Build exact candidate parity and quantization evidence for strength-first screens."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from datetime import datetime, timezone
from pathlib import Path
import struct
import sys
import tempfile
import types

import numpy as np


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import r5_trainer as r5  # noqa: E402
import strength_first_trainer as strength  # noqa: E402

VERSION = 0x7AF32F20
HALFKAV2_HASH = 0x5F234CB8
DONOR_SINGLE_ANHOKU_HASH = 0x23627E42 ^ 0x3C6EF362
L1, L2, L3 = 256, 16, 32
PSQT_BUCKETS = STACKS = 8
REAL_ROWS = 150_903 + 1_620


def composite_hash(head: int, tail: int) -> int:
    return (head ^ ((tail << 1) & 0xFFFFFFFF) ^ (tail >> 1)) & 0xFFFFFFFF


def affine_hash(previous: int, outputs: int) -> int:
    return ((0xCC03DAE4 + outputs) ^ (previous >> 1) ^ ((previous << 31) & 0xFFFFFFFF)) & 0xFFFFFFFF


FEATURE_HASH = composite_hash(HALFKAV2_HASH, DONOR_SINGLE_ANHOKU_HASH)
INPUT_HASH = 0xEC42E90D ^ (L1 * 2)
H1_HASH = (0x538D24C7 + affine_hash(INPUT_HASH, L2)) & 0xFFFFFFFF
H2_HASH = (0x538D24C7 + affine_hash(H1_HASH, L3)) & 0xFFFFFFFF
OUTPUT_HASH = affine_hash(H2_HASH, 1)
NETWORK_HASH = (FEATURE_HASH ^ (L1 * 2) ^ OUTPUT_HASH) & 0xFFFFFFFF
TRANSFORMER_HASH = FEATURE_HASH ^ (L1 * 2)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def workspace_path(path: str | Path) -> Path:
    value = Path(path)
    return value if value.is_absolute() else ROOT / value


def display_path(path: Path) -> str:
    try:
        return str(path.resolve().relative_to(ROOT))
    except ValueError:
        return str(path.resolve())


def artifact(path: Path, *, include_bytes: bool = True) -> dict:
    value = {"path": display_path(path), "sha256": sha256(path)}
    if include_bytes:
        value["bytes"] = path.stat().st_size
    return value


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def load_rows(corpus_path: Path, features_path: Path, count: int):
    corpus_lines = corpus_path.read_text(encoding="utf-8").splitlines()[:count]
    feature_lines = features_path.read_text(encoding="utf-8").splitlines()[:count]
    if len(corpus_lines) != count or len(feature_lines) != count:
        raise RuntimeError(f"parity inputs must contain at least {count} rows")
    rows = []
    for index, (position_line, feature_line) in enumerate(zip(corpus_lines, feature_lines)):
        position, features = json.loads(position_line), json.loads(feature_line)
        expected = f"r1a-{index:05d}"
        if position["id"] != expected or features["id"] != expected:
            raise RuntimeError(f"unstable parity corpus order at {index}")
        black = sorted(features["black"]["base"] + features["black"]["donor"])
        white = sorted(features["white"]["base"] + features["white"]["donor"])
        if any(row < 0 or row >= REAL_ROWS for row in black + white):
            raise RuntimeError(f"feature index outside DonorSingleEff range at {expected}")
        rows.append((position, black, white))
    return rows


def read_array(data: memoryview, offset: int, dtype, shape):
    count = math.prod(shape)
    values = np.frombuffer(data, dtype=dtype, count=count, offset=offset).reshape(shape)
    return values, offset + count * np.dtype(dtype).itemsize


def parse_integer_network(path: Path):
    raw = path.read_bytes()
    data = memoryview(raw)
    version, network_hash, description_len = struct.unpack_from("<III", data, 0)
    if version != VERSION or network_hash != NETWORK_HASH:
        raise RuntimeError(f"candidate network header mismatch: {version:#x}/{network_hash:#x}")
    offset = 12 + description_len
    transformer_hash = struct.unpack_from("<I", data, offset)[0]
    offset += 4
    if transformer_hash != TRANSFORMER_HASH:
        raise RuntimeError("candidate transformer hash mismatch")
    bias, offset = read_array(data, offset, "<i2", (L1,))
    weights, offset = read_array(data, offset, "<i2", (REAL_ROWS, L1))
    psqt, offset = read_array(data, offset, "<i4", (REAL_ROWS, PSQT_BUCKETS))
    buckets = []
    for _ in range(STACKS):
        section_hash = struct.unpack_from("<I", data, offset)[0]
        offset += 4
        if section_hash != OUTPUT_HASH:
            raise RuntimeError("candidate affine section hash mismatch")
        layers = []
        for outputs, inputs in ((L2, L1 * 2), (L3, 32), (1, 32)):
            biases, offset = read_array(data, offset, "<i4", (outputs,))
            layer_weights, offset = read_array(data, offset, "i1", (outputs, inputs))
            layers.append((biases, layer_weights))
        buckets.append(layers)
    if offset != len(data):
        raise RuntimeError(f"candidate network has {len(data) - offset} trailing bytes")
    return raw, (bias, weights, psqt, buckets)


def trunc_div(value: int, divisor: int) -> int:
    return value // divisor if value >= 0 else -((-value) // divisor)


def integer_score(network, position: dict, black_rows: list[int], white_rows: list[int]) -> int:
    bias, weights, psqt_weights, buckets = network
    accumulators, psqts = [], []
    for rows in (black_rows, white_rows):
        wide = bias.astype(np.int64) + weights[rows].astype(np.int64).sum(axis=0)
        if np.any((wide < -32768) | (wide > 32767)):
            raise RuntimeError(f"candidate accumulator overflow at {position['id']}")
        accumulators.append(wide.astype("<i2"))
        psqts.append(psqt_weights[rows].astype(np.int64).sum(axis=0).astype("<i4"))
    our = 0 if position["sfen"].split()[1] == "b" else 1
    their = 1 - our
    transformed = np.concatenate([
        np.clip(accumulators[our].astype(np.int32), 0, 127),
        np.clip(accumulators[their].astype(np.int32), 0, 127),
    ]).astype("u1")
    bucket = int(position["outputBucket"])
    (h1b, h1w), (h2b, h2w), (outb, outw) = buckets[bucket]
    h1 = h1b.astype(np.int64) + h1w.astype(np.int64) @ transformed.astype(np.int64)
    h1_relu = np.clip(h1 >> 6, 0, 127).astype("u1")
    h2 = h2b.astype(np.int64) + h2w[:, :L2].astype(np.int64) @ h1_relu.astype(np.int64)
    h2_relu = np.clip(h2 >> 6, 0, 127).astype("u1")
    output = int(outb[0]) + int(outw[0].astype(np.int64) @ h2_relu.astype(np.int64))
    psqt = trunc_div(int(psqts[our][bucket]) - int(psqts[their][bucket]), 2)
    return trunc_div(psqt + output, 16)


def full_precision_scores(checkpoint: Path, rows, width: dict[str, int], cmake: str) -> list[float]:
    experiment_root = checkpoint.resolve().parents[1]
    compatibility_receipt = experiment_root / "preflight" / "parity-patch-compatibility.json"
    context_temp, context = r5.prepare_context(
        width,
        cmake,
        parity_only=True,
        compatibility_receipt=compatibility_receipt,
    )
    original_cwd = Path.cwd()
    try:
        # The pinned transformer's CUDA kernels import CuPy eagerly. This audit
        # only reads CPU tensors and executes ordinary torch layers.
        missing = object()
        prior_cupy = sys.modules.get("cupy", missing)
        cupy_stub = types.ModuleType("cupy")
        cupy_stub.RawKernel = lambda *_args, **_kwargs: (_ for _ in ()).throw(
            RuntimeError("candidate parity attempted a CUDA kernel")
        )
        sys.modules["cupy"] = cupy_stub
        try:
            torch, _features, model_module, _nnue_dataset = r5.import_trainer(context)
        finally:
            if prior_cupy is missing:
                sys.modules.pop("cupy", None)
            else:
                sys.modules["cupy"] = prior_cupy
        model = torch.load(checkpoint, map_location="cpu", weights_only=False).cpu().eval()
        if model.feature_set.name != "HalfKAv2^+DonorSingleEff":
            raise RuntimeError(f"unexpected checkpoint feature set {model.feature_set.name}")
        if int(model.input.bias.shape[0]) != L1 + PSQT_BUCKETS:
            raise RuntimeError("checkpoint width does not match the candidate runtime")
        with torch.no_grad():
            coalesced = model_module.coalesce_ft_weights(model, model.input).cpu()
            bias = model.input.bias.detach().cpu()
            layer_stacks = list(model.layer_stacks.get_coalesced_layer_stacks())
            scores = []
            for position, black_rows, white_rows in rows:
                perspectives = []
                for feature_rows in (black_rows, white_rows):
                    indices = torch.tensor(feature_rows, dtype=torch.int64)
                    perspectives.append(bias + coalesced.index_select(0, indices).sum(dim=0))
                our = 0 if position["sfen"].split()[1] == "b" else 1
                their = 1 - our
                transformed = torch.clamp(
                    torch.cat([perspectives[our][:L1], perspectives[their][:L1]]), 0.0, 1.0
                )
                bucket = int(position["outputBucket"])
                l1, l2, output = layer_stacks[bucket]
                hidden1 = torch.clamp(l1(transformed), 0.0, 1.0)
                hidden2 = torch.clamp(l2(hidden1), 0.0, 1.0)
                psqt = (perspectives[our][L1 + bucket] - perspectives[their][L1 + bucket]) * 0.5
                scores.append(float((output(hidden2)[0] + psqt) * 600.0))
        return scores
    finally:
        os.chdir(original_cwd)
        context_temp.cleanup()


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, math.ceil(fraction * len(ordered)) - 1)]


def run_reference(args) -> None:
    config = json.loads(args.config.read_text())
    rows = load_rows(args.corpus, args.features, args.count)
    _raw, network = parse_integer_network(args.network)
    integer = [integer_score(network, *row) for row in rows]
    full = full_precision_scores(args.checkpoint, rows, config["recipe"]["width"], args.cmake)
    deltas = [abs(a - b) for a, b in zip(full, integer)]
    report = {
        "schema": "haitaka-anhoku-strength-first-reference-parity-v1",
        "createdAt": datetime.now(timezone.utc).isoformat(),
        "inputs": {
            "config": artifact(args.config), "checkpoint": artifact(args.checkpoint),
            "integerNetwork": artifact(args.network), "corpus": artifact(args.corpus),
            "features": artifact(args.features),
        },
        "positions": [
            {"id": row[0]["id"], "sfen": row[0]["sfen"], "fullPrecisionScore": fp, "integerScore": iq}
            for row, fp, iq in zip(rows, full, integer)
        ],
        "quantization": {
            "count": len(deltas), "meanAbsoluteScoreDelta": float(np.mean(deltas)),
            "p95AbsoluteScoreDelta": percentile(deltas, 0.95),
            "p99AbsoluteScoreDelta": percentile(deltas, 0.99), "maximumAbsoluteScoreDelta": max(deltas),
        },
        "status": "passed",
    }
    write_json(args.output, report)
    print(f"wrote {args.count} full-precision/integer reference scores to {args.output}")


def indexed_positions(report: dict, schema: str) -> dict[str, dict]:
    if report.get("schema") != schema or report.get("status") != "passed":
        raise RuntimeError(f"invalid parity report {schema}")
    positions = report.get("positions")
    result = {row["id"]: row for row in positions}
    if len(result) != len(positions):
        raise RuntimeError(f"duplicate ids in {schema}")
    return result


def run_finalize(args) -> None:
    config = json.loads(args.config.read_text())
    reference = json.loads(args.reference.read_text())
    native = json.loads(args.native.read_text())
    wasm = json.loads(args.wasm.read_text())
    refs = indexed_positions(reference, "haitaka-anhoku-strength-first-reference-parity-v1")
    natives = indexed_positions(native, "haitaka-anhoku-strength-first-native-parity-v1")
    wasms = indexed_positions(wasm, "haitaka-anhoku-strength-first-wasm-parity-v1")
    if set(refs) != set(natives) or set(refs) != set(wasms):
        raise RuntimeError("reference/native/WASM parity ids differ")
    integer_native = sum(refs[key]["integerScore"] != natives[key]["score"] for key in refs)
    integer_wasm = sum(refs[key]["integerScore"] != wasms[key]["score"] for key in refs)
    native_wasm = sum(natives[key]["score"] != wasms[key]["score"] for key in refs)
    if integer_native or integer_wasm or native_wasm or not native.get("fullRefreshEqualsIncremental"):
        raise RuntimeError(
            f"candidate parity failed: integer/native={integer_native}, integer/WASM={integer_wasm}, native/WASM={native_wasm}"
        )
    candidate_export_source = json.loads(args.candidate_export.read_text())
    checkpoint = candidate_export_source["checkpoint"]
    update = int(checkpoint["update"])
    network = candidate_export_source["integerNetwork"]
    runtime_build = artifact(args.runtime_build)
    output = args.output_dir
    output.mkdir(parents=True, exist_ok=True)

    # Preserve a transport-local training record and rebind the candidate export
    # to it. Its nested remote receipt paths exist on the still-running source
    # instance, while candidate validation only needs its checkpoint ledger.
    training_run_path = output / f"training-run-u{update:08d}.json"
    training_run_path.write_bytes(args.training_run.read_bytes())
    localized_export = dict(candidate_export_source)
    localized_export["run"] = artifact(training_run_path, include_bytes=False)
    candidate_export_path = output / f"candidate-export-u{update:08d}.json"
    write_json(candidate_export_path, localized_export)

    quantization_path = output / f"quantization-u{update:08d}.json"
    write_json(quantization_path, {
        "schema": "haitaka-anhoku-strength-first-quantization-report-v1",
        "createdAt": datetime.now(timezone.utc).isoformat(),
        "checkpoint": checkpoint, "integerNetwork": network,
        "reference": artifact(args.reference), "metrics": reference["quantization"], "status": "passed",
    })
    detailed_path = output / f"parity-detail-u{update:08d}.json"
    write_json(detailed_path, {
        "schema": "haitaka-anhoku-strength-first-parity-detail-v1",
        "createdAt": datetime.now(timezone.utc).isoformat(), "positions": len(refs),
        "inputs": {"reference": artifact(args.reference), "native": artifact(args.native), "wasm": artifact(args.wasm)},
        "mismatches": {"integerVsNative": integer_native, "integerVsWasm": integer_wasm, "nativeVsWasm": native_wasm},
        "nativeFullRefreshEqualsIncremental": native["fullRefreshEqualsIncremental"], "status": "passed",
    })
    parity_path = output / f"parity-receipt-u{update:08d}.json"
    checks = {
        "fullPrecisionToIntegerMeasured": True, "integerReferenceMatchesNative": True,
        "integerReferenceMatchesWasm": True, "nativeMatchesWasm": True,
        "quantizationErrorReportedSeparately": True,
    }
    write_json(parity_path, {
        "schema": "haitaka-anhoku-strength-first-parity-receipt-v1",
        "experimentId": config["experimentId"], "configSha256": sha256(args.config),
        "createdAt": datetime.now(timezone.utc).isoformat(), "checkpoint": checkpoint,
        "integerNetwork": network, "runtimeBuild": runtime_build, "checks": checks, "status": "passed",
    })
    runtime_path = output / f"candidate-runtime-u{update:08d}.json"
    write_json(runtime_path, {
        "schema": "haitaka-anhoku-strength-first-candidate-runtime-v1",
        "experimentId": config["experimentId"], "configSha256": sha256(args.config),
        "createdAt": datetime.now(timezone.utc).isoformat(), "checkpoint": checkpoint,
        "candidateExport": artifact(candidate_export_path, include_bytes=False), "runtimeBuild": runtime_build,
        "parityReceipt": artifact(parity_path, include_bytes=False), "width": config["recipe"]["width"], "status": "passed",
    })
    print(json.dumps({
        "candidateRuntime": artifact(runtime_path, include_bytes=False),
        "parityReceipt": artifact(parity_path, include_bytes=False),
        "parityDetail": artifact(detailed_path), "quantization": artifact(quantization_path),
    }, indent=2))


def candidate_move_count(game: dict) -> int:
    start = game["startSfen"].split()[1]
    candidate = "b" if game["aColor"] == "black" else "w"
    count = sum(1 for ply in range(len(game["moves"])) if (start if ply % 2 == 0 else ("w" if start == "b" else "b")) == candidate)
    if count <= 0:
        raise RuntimeError(f"candidate searched no moves in source game {game['gameIndex']}")
    return count


def run_screen(args) -> None:
    config = json.loads(args.config.read_text())
    protocol = config["strengthProtocol"]
    source_report = json.loads(args.source_report.read_text())
    source_games = [json.loads(line) for line in args.source_games.read_text().splitlines() if line.strip()]
    runtime_item = artifact(args.candidate_runtime, include_bytes=False)
    runtime = json.loads(args.candidate_runtime.read_text())
    if runtime.get("status") != "passed" or runtime["checkpoint"]["update"] != args.update:
        raise RuntimeError("candidate runtime is not the requested passed checkpoint")
    if len(source_games) != 128 or source_report.get("summary", {}).get("games") != 128:
        raise RuntimeError("native source does not contain the exact 128-game screen")
    command = source_report["command"]
    if (command.get("games"), command.get("threads"), command.get("nodesPerMove"), command.get("openingRandomPlies"), command.get("maxPlies")) != (128, 1, 10000, 0, 200):
        raise RuntimeError("native source command differs from frozen scheduled screen")
    engines = source_report["engines"]
    if engines[0].get("kind") != "nnue" or engines[0].get("nnueSha256") != sha256(args.network) or engines[1].get("kind") != "handcrafted":
        raise RuntimeError("native source engines do not bind candidate A versus handcrafted B")
    converted = []
    group_scores = []
    candidate_elapsed = 0.0
    candidate_moves = 0
    for index, game in enumerate(source_games):
        pair = index // 2
        if game["gameIndex"] != index + 1 or game["pairIndex"] != pair:
            raise RuntimeError(f"native source order mismatch at game {index}")
        expected_color = ("black", "white")[index % 2]
        if game["aColor"] != expected_color:
            raise RuntimeError(f"native source color swap mismatch at game {index}")
        if game.get("failureState") is not None or int(game.get("fallbacks", 0)) != 0:
            raise RuntimeError(f"native source correctness failure at game {index}")
        score = {"a-win": 1.0, "b-win": 0.0, "draw": 0.5}.get(game["result"])
        if score is None:
            raise RuntimeError(f"unsupported result at game {index}")
        searched = candidate_move_count(game)
        elapsed = float(game["aBreakdown"]["totalElapsedMs"])
        candidate_moves += searched
        candidate_elapsed += elapsed
        converted.append({
            "schema": "haitaka-anhoku-strength-first-screen-game-v1", "gameIndex": index,
            "pairIndex": pair, "openingId": protocol["openingIds"][pair], "openingCycle": 0,
            "candidateColor": expected_color, "opponent": protocol["opponent"],
            "scoreCandidate": score, "result": {0.0: "candidate-loss", 0.5: "draw", 1.0: "candidate-win"}[score],
            "candidateSearchedMoves": searched, "candidateSearchElapsedMilliseconds": elapsed,
            "complete": True, "missingMoves": 0, "illegalMoves": 0,
            "emergencyFallbacks": 0, "unsearchedMoves": 0,
            "terminationReason": game["terminationReason"],
        })
    for pair in range(64):
        group_scores.append((converted[2 * pair]["scoreCandidate"] + converted[2 * pair + 1]["scoreCandidate"]) / 2.0)
    statistics = strength.grouped_screen_statistics(group_scores)
    summary = {
        "games": 128, "pairs": 64, "openingGroups": 64, "groupScores": group_scores,
        **statistics, "intervalMethod": protocol["uncertainty"]["intervalMethod"], "correctnessPassed": True,
    }
    output = args.output_dir
    output.mkdir(parents=True, exist_ok=True)
    source_audit_path = output / f"native-source-audit-u{args.update:08d}.json"
    write_json(source_audit_path, {
        "schema": "haitaka-anhoku-strength-first-native-source-audit-v1", "createdAt": datetime.now(timezone.utc).isoformat(),
        "source": {"report": artifact(args.source_report), "games": artifact(args.source_games),
                   "runtimeBuild": artifact(args.runtime_build), "openingSuite": artifact(args.openings), "integerNetwork": artifact(args.network)},
        "checks": {"exactFrozenCommand": True, "candidateVsHandcrafted": True, "completeColorSwappedOrder": True,
                   "zeroFailureStates": True, "zeroCandidateFallbacks": True}, "status": "passed",
    })
    raw_path = output / f"raw-match-u{args.update:08d}.json"
    write_json(raw_path, {
        "schema": "haitaka-anhoku-strength-first-raw-match-v1", "experimentId": config["experimentId"],
        "configSha256": sha256(args.config), "createdAt": datetime.now(timezone.utc).isoformat(),
        "kind": "scheduled-screen", "candidateRuntime": runtime_item,
        "schedule": strength.match_schedule(config, "scheduled-screen", args.update),
        "games": converted, "summary": summary,
        "runtimeCost": {"metric": "mean-candidate-search-elapsed-milliseconds", "value": candidate_elapsed / candidate_moves},
        "status": "passed",
    })
    scheduled_updates = protocol["scheduledScreens"]["updates"]
    if args.update not in scheduled_updates:
        raise RuntimeError(f"update {args.update} is not a scheduled screen")
    position = scheduled_updates.index(args.update)
    prior_paths = [path.resolve() for path in args.prior_screen]
    if len(prior_paths) != position:
        raise RuntimeError(f"update {args.update} requires {position} prior screen receipts")
    prior_items = [artifact(path, include_bytes=False) for path in prior_paths]
    previous_low = False
    if prior_paths:
        previous = strength.validate_screen_evidence(
            prior_paths[-1], config, args.config, scheduled_updates[position - 1]
        )
        previous_low = previous["upper95"] < protocol["uncertainty"]["screenStoppingThreshold"]
    current_low = summary["upper95"] < protocol["uncertainty"]["screenStoppingThreshold"]
    decision = "stop-two-successive-low-screens" if previous_low and current_low else "continue"
    screen_path = output / f"strength-screen-u{args.update:08d}.json"
    checkpoint = runtime["checkpoint"]
    write_json(screen_path, {
        "schema": "haitaka-anhoku-strength-first-screen-receipt-v2", "experimentId": config["experimentId"],
        "configSha256": sha256(args.config), "createdAt": datetime.now(timezone.utc).isoformat(),
        "checkpoint": checkpoint, "candidateRuntime": runtime_item, "rawMatch": artifact(raw_path, include_bytes=False),
        "summary": summary, "priorScreens": prior_items, "decision": decision,
    })
    strength.validate_screen_evidence(screen_path, config, args.config, args.update)
    receipt_path = None
    if decision == "continue":
        legs = [leg for leg in protocol["executionGates"]["continuationSequence"] if leg["fromUpdate"] == args.update]
        if len(legs) != 1:
            raise RuntimeError(f"no unique continuation leg starts at {args.update}")
        to_update = int(legs[0]["toUpdate"])
        receipt_path = output / f"continuation-{args.update}-{to_update}.json"
        write_json(receipt_path, {
            "schema": "haitaka-anhoku-strength-first-continuation-receipt-v3", "experimentId": config["experimentId"],
            "configSha256": sha256(args.config), "fromUpdate": args.update, "toUpdate": to_update,
            "decision": "continue", "createdAt": datetime.now(timezone.utc).isoformat(),
            "evidence": [
                {"role": "throughput-probe-report", **artifact(args.throughput, include_bytes=False)},
                {"role": f"strength-screen-u{args.update:08d}", **artifact(screen_path, include_bytes=False)},
            ],
        })
        strength.validate_continuation_receipt(receipt_path, config, args.config, args.update, to_update)
    print(json.dumps({"summary": summary, "sourceAudit": artifact(source_audit_path),
                      "rawMatch": artifact(raw_path, include_bytes=False), "screen": artifact(screen_path, include_bytes=False),
                      "decision": decision,
                      "continuation": artifact(receipt_path, include_bytes=False) if receipt_path else None}, indent=2))


def run_comparison(args) -> None:
    """Convert a native final match into the receipt-gated raw-match schema."""
    config = json.loads(args.config.read_text())
    protocol = config["strengthProtocol"]
    source_report = json.loads(args.source_report.read_text())
    source_games = [json.loads(line) for line in args.source_games.read_text().splitlines() if line.strip()]
    runtime_item = artifact(args.candidate_runtime, include_bytes=False)
    runtime = json.loads(args.candidate_runtime.read_text())
    if runtime.get("status") != "passed" or runtime["checkpoint"]["update"] != args.update:
        raise RuntimeError("candidate runtime is not the requested passed checkpoint")
    schedule = strength.match_schedule(config, args.kind, args.update)
    expected_games = schedule["pairs"] * 2
    command = source_report.get("command", {})
    expected_nodes = schedule["budget"]["value"] if schedule["budget"]["mode"] == "nodes" else None
    expected_movetime = schedule["budget"]["value"] if schedule["budget"]["mode"] == "movetime" else None
    command_shape = (
        command.get("games"), command.get("threads"), command.get("nodesPerMove"),
        command.get("movetimeMs"), command.get("openingRandomPlies"), command.get("maxPlies"),
    )
    # A report extended with the runner's verified merge path records the most
    # recent chunk size in command.games while summary.games records the total.
    allowed_command_games = {expected_games}
    if args.kind == "final-equal-node":
        allowed_command_games.add(protocol["scheduledScreens"]["pairsPerScreen"] * 2)
    if (command.get("games") not in allowed_command_games
            or command_shape[1:] != (1, expected_nodes, expected_movetime, 0, protocol["maximumPly"])):
        raise RuntimeError(f"native source command differs from frozen {args.kind} match: {command_shape}")
    if len(source_games) != expected_games or source_report.get("summary", {}).get("games") != expected_games:
        raise RuntimeError(f"native source does not contain exactly {expected_games} games")
    engines = source_report.get("engines", [])
    if (len(engines) != 2 or engines[0].get("kind") != "nnue"
            or engines[0].get("nnueSha256") != sha256(args.network)
            or engines[1].get("kind") != "handcrafted"):
        raise RuntimeError("native source engines do not bind candidate A versus handcrafted B")

    converted = []
    candidate_elapsed = 0.0
    candidate_moves = 0
    opening_pair_scores = {opening_id: [] for opening_id in protocol["openingIds"]}
    for index, game in enumerate(source_games):
        pair = index // 2
        if game["gameIndex"] != index + 1 or game["pairIndex"] != pair:
            raise RuntimeError(f"native source order mismatch at game {index}")
        expected_color = ("black", "white")[index % 2]
        if game["aColor"] != expected_color:
            raise RuntimeError(f"native source color swap mismatch at game {index}")
        if game.get("failureState") is not None or int(game.get("fallbacks", 0)) != 0:
            raise RuntimeError(f"native source correctness failure at game {index}")
        score = {"a-win": 1.0, "b-win": 0.0, "draw": 0.5}.get(game["result"])
        if score is None:
            raise RuntimeError(f"unsupported result at game {index}")
        searched = candidate_move_count(game)
        elapsed = float(game["aBreakdown"]["totalElapsedMs"])
        candidate_moves += searched
        candidate_elapsed += elapsed
        opening_id = protocol["openingIds"][pair % len(protocol["openingIds"])]
        converted.append({
            "schema": "haitaka-anhoku-strength-first-screen-game-v1", "gameIndex": index,
            "pairIndex": pair, "openingId": opening_id,
            "openingCycle": schedule["cycles"][pair // len(protocol["openingIds"])],
            "candidateColor": expected_color, "opponent": protocol["opponent"],
            "scoreCandidate": score, "result": {0.0: "candidate-loss", 0.5: "draw", 1.0: "candidate-win"}[score],
            "candidateSearchedMoves": searched, "candidateSearchElapsedMilliseconds": elapsed,
            "complete": True, "missingMoves": 0, "illegalMoves": 0,
            "emergencyFallbacks": 0, "unsearchedMoves": 0,
            "terminationReason": game["terminationReason"],
        })
    for pair in range(schedule["pairs"]):
        opening_id = protocol["openingIds"][pair % len(protocol["openingIds"])]
        opening_pair_scores[opening_id].append(
            (converted[2 * pair]["scoreCandidate"] + converted[2 * pair + 1]["scoreCandidate"]) / 2.0
        )
    group_scores = [float(np.mean(opening_pair_scores[opening_id])) for opening_id in protocol["openingIds"]]
    statistics = strength.grouped_screen_statistics(group_scores)
    summary = {
        "games": expected_games, "pairs": schedule["pairs"], "openingGroups": len(group_scores),
        "groupScores": group_scores, **statistics,
        "intervalMethod": protocol["uncertainty"]["intervalMethod"], "correctnessPassed": True,
    }
    output = args.output_dir
    output.mkdir(parents=True, exist_ok=True)
    audit_path = output / f"native-{args.kind}-source-audit-u{args.update:08d}.json"
    write_json(audit_path, {
        "schema": "haitaka-anhoku-strength-first-native-source-audit-v1",
        "createdAt": datetime.now(timezone.utc).isoformat(), "kind": args.kind,
        "source": {"report": artifact(args.source_report), "games": artifact(args.source_games),
                   "runtimeBuild": artifact(args.runtime_build), "openingSuite": artifact(args.openings),
                   "integerNetwork": artifact(args.network)},
        "checks": {"exactFrozenCommand": True, "candidateVsHandcrafted": True,
                   "completeColorSwappedOrder": True, "zeroFailureStates": True,
                   "zeroCandidateFallbacks": True}, "status": "passed",
    })
    raw_path = output / f"raw-match-{args.kind}-u{args.update:08d}.json"
    write_json(raw_path, {
        "schema": "haitaka-anhoku-strength-first-raw-match-v1", "experimentId": config["experimentId"],
        "configSha256": sha256(args.config), "createdAt": datetime.now(timezone.utc).isoformat(),
        "kind": args.kind, "candidateRuntime": runtime_item, "schedule": schedule,
        "games": converted, "summary": summary,
        "runtimeCost": {"metric": "mean-candidate-search-elapsed-milliseconds",
                        "value": candidate_elapsed / candidate_moves}, "status": "passed",
    })
    strength.validate_raw_match(raw_path, config, args.config, args.kind, runtime_item, runtime, args.update)
    comparison_path = None
    if args.kind == "final-equal-node":
        loss = strength.validation_loss_from_journal(args.metrics, args.update)
        comparison_path = output / f"comparison-receipt-u{args.update:08d}.json"
        write_json(comparison_path, {
            "schema": "haitaka-anhoku-strength-first-comparison-receipt-v1",
            "experimentId": config["experimentId"], "configSha256": sha256(args.config),
            "createdAt": datetime.now(timezone.utc).isoformat(), "checkpoint": runtime["checkpoint"],
            "candidateRuntime": runtime_item, "equalNodeMatch": artifact(raw_path, include_bytes=False),
            "fixedTimeMatch": None,
            "validationMetric": {"metricsJournal": artifact(args.metrics, include_bytes=False),
                                 "update": args.update, "teacherOnlyLoss": loss}, "status": "passed",
        })
        strength.validate_comparison_receipt(comparison_path, config, args.config, runtime["checkpoint"])
    print(json.dumps({"summary": summary, "sourceAudit": artifact(audit_path),
                      "rawMatch": artifact(raw_path, include_bytes=False),
                      "comparison": artifact(comparison_path, include_bytes=False) if comparison_path else None}, indent=2))


def run_selection(args) -> None:
    """Bind the two frozen candidate roles and select via the configured ranking."""
    config = json.loads(args.config.read_text())
    run = json.loads(args.training_run.read_text())
    if run.get("complete") is not True:
        raise RuntimeError("final selection requires the completed training run")
    role_inputs = [
        ("lowest-validation-loss", args.lowest_validation_comparison),
        ("highest-scheduled-screen-score", args.highest_screen_comparison),
    ]
    candidates_by_identity = {}
    validated = []
    for role, path in role_inputs:
        receipt = json.loads(path.read_text())
        checkpoint = receipt["checkpoint"]
        identity = (checkpoint["update"], checkpoint["sha256"])
        result = strength.validate_comparison_receipt(path, config, args.config, checkpoint)
        if identity in candidates_by_identity:
            candidates_by_identity[identity]["candidateRoles"].append(role)
        else:
            candidate = {
                "candidateRoles": [role], "checkpoint": checkpoint,
                "comparisonReceipt": artifact(path, include_bytes=False),
            }
            candidates_by_identity[identity] = candidate
            validated.append((candidate, result))
    use_fixed_time = all(result["fixedTimeScoreRate"] is not None for _, result in validated)
    def rank_key(item):
        candidate, result = item
        fixed_key = -float(result["fixedTimeScoreRate"]) if use_fixed_time else 0.0
        return (fixed_key, -result["equalNodeScoreRate"], result["runtimeCost"],
                result["validationLoss"], candidate["checkpoint"]["update"])
    selected = min(validated, key=rank_key)[0]["checkpoint"]
    history = {
        "schema": "haitaka-anhoku-strength-first-selection-history-v2",
        "experimentId": config["experimentId"], "configSha256": sha256(args.config),
        "createdAt": datetime.now(timezone.utc).isoformat(),
        "run": artifact(args.training_run, include_bytes=False), "terminalEvidence": None,
        "candidates": list(candidates_by_identity.values()),
        "scheduledScreens": [artifact(path, include_bytes=False) for path in args.scheduled_screen],
        "rankingOrder": config["strengthProtocol"]["finalSelection"]["rankingOrder"],
        "selectedCheckpoint": selected, "decision": "selected-by-frozen-ranking",
    }
    write_json(args.output, history)
    result = strength.validate_selection_history(args.output, config, args.config)
    print(json.dumps(result, indent=2))


def run_localize_training_run(args) -> None:
    """Rebind a transported run after fully validating every local gate receipt."""
    config = json.loads(args.config.read_text())
    run = json.loads(args.training_run.read_text())
    validated_approval = strength.validate_approval(args.approval, config, args.config)
    if validated_approval["sha256"] != run["approval"]["sha256"]:
        raise RuntimeError("local approval receipt identity differs from the training run")
    run["approval"] = {**run["approval"], "path": display_path(args.approval), "sha256": sha256(args.approval)}
    recorded_legs = run.get("continuationReceipts", [])
    if len(args.continuation) != len(recorded_legs):
        raise RuntimeError("local continuation receipt count differs from the training run")
    rebound = []
    for recorded, path in zip(recorded_legs, args.continuation):
        validated = strength.validate_continuation_receipt(
            path, config, args.config, recorded["fromUpdate"], recorded["toUpdate"]
        )
        if (validated["fromUpdate"], validated["toUpdate"], validated["decision"]) != (
                recorded["fromUpdate"], recorded["toUpdate"], recorded["decision"]):
            raise RuntimeError(f"validated local continuation leg differs at {path}")
        rebound.append({**recorded, "path": display_path(path), "sha256": sha256(path)})
    run["continuationReceipts"] = rebound
    write_json(args.output, run)
    # The checkpoint ledger and complete-run shape remain subject to the same
    # validator used by candidate export; only receipt locations are changed.
    strength.validate_recorded_checkpoint(
        args.output, config, args.config,
        {"update": run["bestValidation"]["update"], "path": run["bestValidation"]["checkpoint"]["path"],
         "sha256": run["bestValidation"]["checkpoint"]["sha256"]},
    )
    print(json.dumps(artifact(args.output), indent=2))


def run_transport_export(args) -> None:
    """Run the pinned exporter with a checkout-only alias for transported paths."""
    config = json.loads(args.config.read_text())
    original_workspace_path = strength.workspace_path
    remote_root = args.remote_root

    def transported_workspace_path(value: str) -> Path:
        path = Path(value)
        if not path.is_absolute():
            return ROOT / path
        if path.exists():
            return path
        try:
            return ROOT / path.relative_to(remote_root)
        except ValueError:
            return path

    strength.workspace_path = transported_workspace_path
    missing = object()
    prior_cupy = sys.modules.get("cupy", missing)
    cupy_stub = types.ModuleType("cupy")
    cupy_stub.RawKernel = lambda *_args, **_kwargs: (_ for _ in ()).throw(
        RuntimeError("selected export unexpectedly attempted a CUDA kernel")
    )
    sys.modules["cupy"] = cupy_stub
    prior_pythonpath = os.environ.get("PYTHONPATH")
    try:
        with tempfile.TemporaryDirectory(prefix="haitaka-cupy-import-stub-") as stub_dir:
            Path(stub_dir, "cupy.py").write_text(
                "class RawKernel:\n"
                "    def __init__(self, *args, **kwargs): pass\n"
                "    def __call__(self, *args, **kwargs):\n"
                "        raise RuntimeError('selected export unexpectedly attempted a CUDA kernel')\n",
                encoding="utf-8",
            )
            os.environ["PYTHONPATH"] = stub_dir + (os.pathsep + prior_pythonpath if prior_pythonpath else "")
            try:
                strength.run_explicit_export(args, config, args.config)
            finally:
                if prior_pythonpath is None:
                    os.environ.pop("PYTHONPATH", None)
                else:
                    os.environ["PYTHONPATH"] = prior_pythonpath
    finally:
        if prior_cupy is missing:
            sys.modules.pop("cupy", None)
        else:
            sys.modules["cupy"] = prior_cupy
        strength.workspace_path = original_workspace_path


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    sub = result.add_subparsers(dest="command", required=True)
    reference = sub.add_parser("reference")
    reference.add_argument("--config", type=Path, required=True)
    reference.add_argument("--checkpoint", type=Path, required=True)
    reference.add_argument("--network", type=Path, required=True)
    reference.add_argument("--corpus", type=Path, required=True)
    reference.add_argument("--features", type=Path, required=True)
    reference.add_argument("--count", type=int, default=1024)
    reference.add_argument("--cmake", default="cmake")
    reference.add_argument("--output", type=Path, required=True)
    reference.set_defaults(function=run_reference)
    finalize = sub.add_parser("finalize")
    for name in ("config", "reference", "native", "wasm", "candidate-export", "training-run", "runtime-build"):
        finalize.add_argument(f"--{name}", type=Path, required=True)
    finalize.add_argument("--output-dir", type=Path, required=True)
    finalize.set_defaults(function=run_finalize)
    screen = sub.add_parser("screen")
    for name in ("config", "candidate-runtime", "runtime-build", "network", "openings", "source-report", "source-games", "throughput"):
        screen.add_argument(f"--{name}", type=Path, required=True)
    screen.add_argument("--update", type=int, default=256)
    screen.add_argument("--prior-screen", type=Path, action="append", default=[])
    screen.add_argument("--output-dir", type=Path, required=True)
    screen.set_defaults(function=run_screen)
    comparison = sub.add_parser("comparison")
    for name in ("config", "candidate-runtime", "runtime-build", "network", "openings", "source-report", "source-games", "metrics"):
        comparison.add_argument(f"--{name}", type=Path, required=True)
    comparison.add_argument("--update", type=int, required=True)
    comparison.add_argument("--kind", choices=("final-equal-node", "final-fixed-time"), required=True)
    comparison.add_argument("--output-dir", type=Path, required=True)
    comparison.set_defaults(function=run_comparison)
    selection = sub.add_parser("selection")
    selection.add_argument("--config", type=Path, required=True)
    selection.add_argument("--training-run", type=Path, required=True)
    selection.add_argument("--lowest-validation-comparison", type=Path, required=True)
    selection.add_argument("--highest-screen-comparison", type=Path, required=True)
    selection.add_argument("--scheduled-screen", type=Path, action="append", required=True)
    selection.add_argument("--output", type=Path, required=True)
    selection.set_defaults(function=run_selection)
    localize = sub.add_parser("localize-training-run")
    localize.add_argument("--config", type=Path, required=True)
    localize.add_argument("--training-run", type=Path, required=True)
    localize.add_argument("--approval", type=Path, required=True)
    localize.add_argument("--continuation", type=Path, action="append", required=True)
    localize.add_argument("--output", type=Path, required=True)
    localize.set_defaults(function=run_localize_training_run)
    transport_export = sub.add_parser("transport-export")
    transport_export.add_argument("--config", type=Path, required=True)
    transport_export.add_argument("--checkpoint", type=Path, required=True)
    transport_export.add_argument("--selection-history", type=Path, required=True)
    transport_export.add_argument("--output", type=Path, required=True)
    transport_export.add_argument("--remote-root", type=Path, default=Path("/workspace/haitaka-variants-anhoku"))
    transport_export.add_argument("--cmake", default="cmake")
    transport_export.set_defaults(function=run_transport_export)
    return result


def main() -> None:
    args = parser().parse_args()
    for name, value in vars(args).items():
        if isinstance(value, Path):
            setattr(args, name.replace("-", "_"), value.resolve())
    args.function(args)


if __name__ == "__main__":
    main()
