#!/usr/bin/env python3
"""Freeze, assemble, and analyze the bounded strength-first Step 4 diagnostic."""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path
import statistics
import sys
from typing import Any, Iterable

import numpy as np


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "haitaka-anhoku-strength-first-step4-diagnostic-v1"
SELECTION_DOMAIN = b"haitaka-strength-first-step4-selection-v1\0"
SCORE_CLIP = 12_000.0
MATE_BASE = 30_000.0


def fail(message: str) -> None:
    raise RuntimeError(message)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def display(path: Path) -> str:
    try:
        return str(path.resolve().relative_to(ROOT))
    except ValueError:
        return str(path.resolve())


def artifact(path: Path) -> dict[str, Any]:
    return {"path": display(path), "bytes": path.stat().st_size, "sha256": sha256(path)}


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_jsonl(path: Path, values: Iterable[Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as stream:
        for value in values:
            stream.write(json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n")


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def checked_output_dir(path: Path) -> None:
    if path.exists():
        fail(f"immutable output directory already exists: {path}")
    path.mkdir(parents=True)


def load_contract(path: Path) -> dict[str, Any]:
    contract = json.loads(path.read_text(encoding="utf-8"))
    if contract.get("schema") != SCHEMA:
        fail("unexpected Step 4 contract schema")
    if contract.get("status") != "frozen-authorized-cpu-diagnostic-awaiting-mbp":
        fail("Step 4 contract is not frozen in the expected state")
    authorization = contract.get("authorization", {})
    if authorization.get("cpuDiagnosticAuthorized") is not True or authorization.get("gpuAllowed") is not False:
        fail("Step 4 authorization must allow CPU diagnosis and forbid GPU use")
    if contract["execution"]["cpuHost"] != "192.168.0.12" or contract["execution"]["cpuHostLabel"] != "mbp":
        fail("Step 4 CPU host is not the frozen corrected mbp endpoint")
    if contract["parents"]["maximumSelected"] > 128 or contract["labels"]["maximumMovesPerParent"] > 4:
        fail("Step 4 parent or move ceiling exceeds the plan")
    if contract["labels"]["nodesPerChild"] > 250_000:
        fail("Step 4 child budget exceeds 250k nodes")
    if contract["labels"]["maximumChildSearchNodes"] > 128_000_000:
        fail("Step 4 child-search ceiling exceeds 128 million nodes")
    if contract["execution"]["maximumWallHours"] > 48:
        fail("Step 4 wall ceiling exceeds two days")
    if (
        contract["rootSearch"]["maximumRootSearchNodes"]
        + contract["labels"]["maximumChildSearchNodes"]
        != contract["execution"]["maximumTotalSearchNodes"]
    ):
        fail("Step 4 total search ceiling does not equal its root and child ceilings")
    for section, name in (
        ("implementation", "runner"),
        ("implementation", "orchestrator"),
        ("implementation", "remoteExecutor"),
        ("implementation", "controller"),
        ("sources", "games"),
        ("sources", "gameReport"),
        ("sources", "width256Result"),
        ("sources", "width512Result"),
        ("models", "c16"),
        ("models", "candidateExport"),
        ("models", "candidateCheckpoint"),
        ("analysis", "r3CalibrationEvidence"),
    ):
        item = contract[section][name]
        resolved = ROOT / item["path"]
        if not resolved.is_file() or sha256(resolved) != item["sha256"] or resolved.stat().st_size != item["bytes"]:
            fail(f"contract artifact mismatch: {section}.{name}")
    phase_total = sum(contract["parents"]["probePhaseCounts"].values())
    if phase_total != contract["parents"]["probePool"]:
        fail("probe phase counts do not sum to the pool size")
    selection_total = sum(
        sum(phases.values()) for phases in contract["parents"]["selectionQuotas"].values()
    )
    if selection_total != contract["parents"]["maximumSelected"]:
        fail("selection quotas do not sum to the parent ceiling")
    return contract


def validate_receipt_for_probe(path: Path) -> None:
    receipt_path = path.parent / "receipt.json"
    receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
    if receipt.get("schema") != "haitaka-strength-first-step4-root-probe-receipt-v1":
        fail(f"bad probe receipt: {receipt_path}")
    bound = receipt["output"]
    if Path(bound["path"]).name != path.name or sha256(path) != bound["sha256"] or path.stat().st_size != bound["bytes"]:
        fail(f"probe receipt output mismatch: {path}")
    if receipt.get("passed") is not True:
        fail(f"probe did not pass: {path}")


def probe_complete(row: dict[str, Any]) -> bool:
    return bool(
        row.get("legalBestMove")
        and row.get("bestMove")
        and row.get("rootScore") is not None
        and row.get("completedDepth", 0) >= 1
    )


def merge_probes(
    paths: list[Path],
    expected_evaluator: str,
    expected_nodes: int,
    expected_model_sha256: str | None = None,
) -> dict[str, dict[str, Any]]:
    if not paths:
        fail(f"no {expected_evaluator} probe files supplied")
    merged: dict[str, dict[str, Any]] = {}
    for path in paths:
        validate_receipt_for_probe(path)
        for row in read_jsonl(path):
            if row.get("schema") != "haitaka-strength-first-step4-root-probe-v1":
                fail(f"bad probe row in {path}")
            if row["evaluator"] != expected_evaluator or row["nodeBudget"] != expected_nodes:
                fail(f"probe identity mismatch in {path}")
            if row.get("modelSha256") != expected_model_sha256:
                fail(f"probe model mismatch in {path}")
            if row["parentId"] in merged:
                fail(f"duplicate {expected_evaluator} parent {row['parentId']}")
            if row["alphaBetaNodes"] + row["qsearchNodes"] != row["totalNodes"]:
                fail(f"node accounting mismatch in {path}")
            merged[row["parentId"]] = row
    return merged


def selection_rank(parent: dict[str, Any], classification: str) -> str:
    digest = hashlib.sha256()
    digest.update(SELECTION_DOMAIN)
    digest.update(classification.encode())
    digest.update(b"\0")
    digest.update(parent["id"].encode())
    digest.update(b"\0")
    digest.update(parent["sfen"].encode())
    return digest.hexdigest()


def select_rows(
    contract: dict[str, Any],
    parents: list[dict[str, Any]],
    handcrafted: dict[str, dict[str, Any]],
    c16: dict[str, dict[str, Any]],
    candidate: dict[str, dict[str, Any]],
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    parent_ids = {row["id"] for row in parents}
    if len(parent_ids) != len(parents) or any(set(probe) != parent_ids for probe in (handcrafted, c16, candidate)):
        fail("parent/probe identity sets differ")
    available: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    excluded_incomplete: Counter[str] = Counter()
    for parent in parents:
        parent_id = parent["id"]
        probes = {
            "handcrafted-10000": handcrafted[parent_id],
            "c16-10000": c16[parent_id],
            "candidate-10000": candidate[parent_id],
        }
        if any(row["parentSfen"] != parent["sfen"] for row in probes.values()):
            fail(f"probe SFEN mismatch for {parent_id}")
        incomplete = [name for name, row in probes.items() if not probe_complete(row)]
        if incomplete:
            excluded_incomplete.update(incomplete)
            continue
        moves = {row["bestMove"] for row in probes.values()}
        classification = "control" if len(moves) == 1 else "disagreement"
        selected = dict(parent)
        selected["selectionClass"] = classification
        selected["selectionRankSha256"] = selection_rank(parent, classification)
        selected["rootChoices"] = {name: row["bestMove"] for name, row in probes.items()}
        available[(classification, parent["phase"])].append(selected)
    chosen: list[dict[str, Any]] = []
    shortages: dict[str, int] = {}
    for classification, phases in contract["parents"]["selectionQuotas"].items():
        for phase, quota in phases.items():
            rows = sorted(available[(classification, phase)], key=lambda row: row["selectionRankSha256"])
            take = min(quota, len(rows))
            chosen.extend(rows[:take])
            if take < quota:
                shortages[f"{classification}:{phase}"] = quota - take
    chosen.sort(key=lambda row: row["id"])
    if len({row["id"] for row in chosen}) != len(chosen):
        fail("selected parent ids are not unique")
    available_counts = Counter(
        (row["selectionClass"], row["phase"])
        for rows in available.values()
        for row in rows
    )
    summary = {
        "schema": "haitaka-strength-first-step4-selection-summary-v1",
        "candidateParents": len(parents),
        "commonCompleteProbeParents": sum(len(rows) for rows in available.values()),
        "excludedIncompleteByEvaluator": dict(sorted(excluded_incomplete.items())),
        "availableByClassPhase": {
            classification: {
                phase: available_counts[(classification, phase)]
                for phase in ("early", "middle", "late")
            }
            for classification in ("disagreement", "control")
        },
        "selectedParents": len(chosen),
        "selectedByClass": dict(sorted(Counter(row["selectionClass"] for row in chosen).items())),
        "selectedByPhase": dict(sorted(Counter(row["phase"] for row in chosen).items())),
        "quotaShortfallsWithoutRefill": shortages,
        "maximumSelected": contract["parents"]["maximumSelected"],
        "passed": len(chosen) <= contract["parents"]["maximumSelected"] and bool(chosen),
    }
    return chosen, summary


def command_select(args: argparse.Namespace) -> None:
    contract = load_contract(args.contract)
    parents = read_jsonl(args.parents)
    expected_pool = contract["parents"]["probePool"]
    if len(parents) != expected_pool:
        fail(f"expected {expected_pool} pool parents, got {len(parents)}")
    nodes = contract["rootSearch"]["developmentNodes"]
    handcrafted = merge_probes(args.handcrafted_probe, "handcrafted-10000", nodes)
    c16 = merge_probes(
        args.c16_probe, "c16-10000", nodes, contract["models"]["c16"]["sha256"]
    )
    candidate = merge_probes(
        args.candidate_probe,
        "candidate-10000",
        nodes,
        contract["models"]["candidateExport"]["sha256"],
    )
    selected, summary = select_rows(contract, parents, handcrafted, c16, candidate)
    checked_output_dir(args.output_dir)
    output = args.output_dir / "selected-parents.jsonl"
    write_jsonl(output, selected)
    summary.update({
        "createdAt": datetime.now(timezone.utc).isoformat(),
        "contract": artifact(args.contract),
        "parents": artifact(args.parents),
        "probes": {
            "handcrafted": [artifact(path) for path in args.handcrafted_probe],
            "c16": [artifact(path) for path in args.c16_probe],
            "candidate": [artifact(path) for path in args.candidate_probe],
        },
        "selected": artifact(output),
    })
    write_json(args.output_dir / "summary.json", summary)
    print(f"selected {len(selected)} fixed parents into {output}")


def command_union(args: argparse.Namespace) -> None:
    contract = load_contract(args.contract)
    selected = read_jsonl(args.selected)
    ids = {row["id"] for row in selected}
    probes = {
        "handcrafted-10000": merge_probes(args.handcrafted_probe, "handcrafted-10000", contract["rootSearch"]["developmentNodes"]),
        "c16-10000": merge_probes(
            args.c16_probe,
            "c16-10000",
            contract["rootSearch"]["developmentNodes"],
            contract["models"]["c16"]["sha256"],
        ),
        "candidate-10000": merge_probes(
            args.candidate_probe,
            "candidate-10000",
            contract["rootSearch"]["developmentNodes"],
            contract["models"]["candidateExport"]["sha256"],
        ),
        "handcrafted-deep": merge_probes(args.deep_handcrafted_probe, "handcrafted-deep", contract["rootSearch"]["deepHandcraftedNodes"]),
        "candidate-low": merge_probes(
            args.candidate_low_probe,
            "candidate-low",
            contract["rootSearch"]["sensitivityCandidateNodes"][0],
            contract["models"]["candidateExport"]["sha256"],
        ),
        "candidate-high": merge_probes(
            args.candidate_high_probe,
            "candidate-high",
            contract["rootSearch"]["sensitivityCandidateNodes"][1],
            contract["models"]["candidateExport"]["sha256"],
        ),
    }
    if any(not ids.issubset(set(rows)) for rows in probes.values()):
        fail("a union probe does not cover every selected parent")
    union_sources = contract["labels"]["unionMoveSources"]
    rows = []
    excluded_incomplete: Counter[str] = Counter()
    for parent in selected:
        parent_id = parent["id"]
        observations = {name: probe[parent_id] for name, probe in probes.items()}
        if any(row["parentSfen"] != parent["sfen"] for row in observations.values()):
            fail(f"union probe SFEN mismatch for {parent_id}")
        incomplete = [name for name in union_sources if not probe_complete(observations[name])]
        if incomplete:
            excluded_incomplete.update(incomplete)
            continue
        moves = sorted({observations[name]["bestMove"] for name in union_sources})
        if not 1 <= len(moves) <= contract["labels"]["maximumMovesPerParent"]:
            fail(f"invalid union size for {parent_id}: {len(moves)}")
        row = dict(parent)
        row["unionMoves"] = moves
        row["rootObservations"] = {
            name: {
                "bestMove": value["bestMove"],
                "rootScore": value["rootScore"],
                "completedDepth": value["completedDepth"],
                "partialIteration": value["partialIteration"],
                "totalNodes": value["totalNodes"],
                "legalBestMove": value["legalBestMove"],
                "staticParentScore": value["staticParentScore"],
                "childStaticScores": value["childStaticScores"],
            }
            for name, value in observations.items()
        }
        rows.append(row)
    planned_labels = sum(len(row["unionMoves"]) for row in rows)
    maximum_nodes = planned_labels * contract["labels"]["nodesPerChild"]
    if maximum_nodes > contract["labels"]["maximumChildSearchNodes"]:
        fail("union plan exceeds child-search ceiling")
    checked_output_dir(args.output_dir)
    output = args.output_dir / "union-plan.jsonl"
    write_jsonl(output, rows)
    summary = {
        "schema": "haitaka-strength-first-step4-union-plan-summary-v1",
        "createdAt": datetime.now(timezone.utc).isoformat(),
        "contract": artifact(args.contract),
        "selected": artifact(args.selected),
        "inputSelectedParents": len(selected),
        "parents": len(rows),
        "excludedIncompleteUnionSourceByEvaluator": dict(sorted(excluded_incomplete.items())),
        "unionSizeCounts": dict(sorted(Counter(len(row["unionMoves"]) for row in rows).items())),
        "plannedChildLabels": planned_labels,
        "maximumChildSearchNodes": maximum_nodes,
        "configuredChildSearchCeiling": contract["labels"]["maximumChildSearchNodes"],
        "union": artifact(output),
        "passed": True,
    }
    write_json(args.output_dir / "summary.json", summary)
    print(f"wrote {planned_labels} fixed child labels over {len(rows)} parents to {output}")


def command_full_precision(args: argparse.Namespace) -> None:
    contract = load_contract(args.contract)
    for path, expected, label in (
        (args.checkpoint, contract["models"]["candidateCheckpoint"], "candidate checkpoint"),
        (args.network, contract["models"]["candidateExport"], "candidate export"),
    ):
        if path.stat().st_size != expected["bytes"] or sha256(path) != expected["sha256"]:
            fail(f"{label} does not match the frozen contract")
    sys.path.insert(0, str(ROOT / "scripts"))
    import strength_first_candidate_parity as parity

    positions = read_jsonl(args.positions)
    rows = []
    for position in positions:
        black = sorted(position["black"]["base"] + position["black"]["donor"])
        white = sorted(position["white"]["base"] + position["white"]["donor"])
        rows.append((position, black, white))
    _raw, integer_network = parity.parse_integer_network(args.network)
    integer = [parity.integer_score(integer_network, *row) for row in rows]
    full = parity.full_precision_scores(
        args.checkpoint, rows, contract["models"]["candidateWidth"], args.cmake
    )
    output_rows = [
        {
            "id": position["id"],
            "parentId": position["parentId"],
            "move": position["move"],
            "sfen": position["sfen"],
            "fullPrecisionScore": fp,
            "integerScore": iq,
            "fullPrecisionMinusInteger": fp - iq,
        }
        for (position, _black, _white), fp, iq in zip(rows, full, integer)
    ]
    checked_output_dir(args.output_dir)
    output = args.output_dir / "scores.jsonl"
    write_jsonl(output, output_rows)
    deltas = [abs(row["fullPrecisionMinusInteger"]) for row in output_rows]
    write_json(args.output_dir / "summary.json", {
        "schema": "haitaka-strength-first-step4-full-precision-summary-v1",
        "createdAt": datetime.now(timezone.utc).isoformat(),
        "contract": artifact(args.contract),
        "positions": artifact(args.positions),
        "checkpoint": artifact(args.checkpoint),
        "integerNetwork": artifact(args.network),
        "scores": artifact(output),
        "count": len(output_rows),
        "meanAbsoluteDelta": statistics.mean(deltas),
        "p95AbsoluteDelta": float(np.percentile(deltas, 95)),
        "maximumAbsoluteDelta": max(deltas),
        "passed": True,
    })
    print(f"wrote {len(output_rows)} full-precision/integer scores to {output}")


def teacher_order(label: dict[str, Any]) -> float:
    cls = label["rootOrientedMateClass"]
    score = label["rawRootOrientedScore"]
    distance = label["mateDistance"]
    score_value = -32_000 if score is None else score
    if cls == "win":
        return 3_000_000.0 - float(30_000 if distance is None else distance) * 1_000.0 + float(score_value)
    if cls == "none":
        return 2_000_000.0 + float(score_value)
    if cls == "loss":
        return 1_000_000.0 + float(0 if distance is None else distance) * 1_000.0 + float(score_value)
    return -1_000_000.0


def robust_huber(prediction: float, target: float) -> float:
    error = abs((prediction - target) / SCORE_CLIP)
    return 0.5 * error * error if error <= 0.1 else 0.1 * (error - 0.05)


def logistic(score: float, scale: float) -> float:
    value = max(-60.0, min(60.0, score / scale))
    return 1.0 / (1.0 + math.exp(-value))


def metric_summary(values: list[float]) -> dict[str, Any]:
    if not values:
        return {"count": 0, "mean": None, "median": None, "p95": None}
    return {
        "count": len(values),
        "mean": statistics.mean(values),
        "median": statistics.median(values),
        "p95": float(np.percentile(values, 95)),
    }


def regression(pairs: list[tuple[float, float]], scale: float) -> dict[str, Any]:
    if len(pairs) < 2:
        return {
            "count": len(pairs),
            "slopeTeacherOnModel": None,
            "interceptTeacherOnModel": None,
            "pearson": None,
            "meanAbsoluteError": None,
            "signDisagreementRate": None,
            "robustClippedHuberLoss": None,
            "calibratedSoftScoreBrier": None,
        }
    prediction = np.asarray([pair[0] for pair in pairs], dtype=np.float64)
    target = np.asarray([pair[1] for pair in pairs], dtype=np.float64)
    slope, intercept = np.polyfit(prediction, target, 1)
    pearson = float(np.corrcoef(prediction, target)[0, 1]) if prediction.std() > 0 and target.std() > 0 else None
    return {
        "count": len(pairs),
        "slopeTeacherOnModel": float(slope),
        "interceptTeacherOnModel": float(intercept),
        "pearson": pearson,
        "meanAbsoluteError": float(np.mean(np.abs(prediction - target))),
        "signDisagreementRate": float(np.mean(np.sign(prediction) != np.sign(target))),
        "robustClippedHuberLoss": float(np.mean([robust_huber(p, t) for p, t in pairs])),
        "calibratedSoftScoreBrier": float(np.mean([(logistic(p, scale) - logistic(t, scale)) ** 2 for p, t in pairs])),
    }


def bootstrap_cluster_difference(rows: list[dict[str, Any]], seed: int, replicates: int) -> dict[str, Any]:
    by_group: dict[str, list[float]] = defaultdict(list)
    for row in rows:
        by_group[row["group"]].append(row["difference"])
    groups = [values for _group, values in sorted(by_group.items())]
    if not groups:
        return {"groups": 0, "mean": None, "lower95": None, "upper95": None}
    sums = np.asarray([sum(values) for values in groups], dtype=np.float64)
    counts = np.asarray([len(values) for values in groups], dtype=np.float64)
    rng = np.random.Generator(np.random.PCG64DXSM(seed))
    draws = rng.integers(0, len(groups), size=(replicates, len(groups)))
    samples = sums[draws].sum(axis=1) / counts[draws].sum(axis=1)
    return {
        "groups": len(groups),
        "parents": len(rows),
        "mean": float(sums.sum() / counts.sum()),
        "lower95": float(np.percentile(samples, 2.5)),
        "upper95": float(np.percentile(samples, 97.5)),
        "replicates": replicates,
        "seed": seed,
        "method": "opening-group-cluster-resample-whole-clusters-percentile-bootstrap-development-only-v1",
    }


def command_analyze(args: argparse.Namespace) -> None:
    contract = load_contract(args.contract)
    union = read_jsonl(args.union)
    union_by_id = {row["id"]: row for row in union}
    if len(union_by_id) != len(union):
        fail("duplicate union parent id")
    labels: dict[tuple[str, str], dict[str, Any]] = {}
    label_artifacts = []
    for path in args.labels:
        receipt = json.loads((path.parent / "receipt.json").read_text(encoding="utf-8"))
        if receipt.get("schema") != "haitaka-strength-first-step4-restricted-label-receipt-v1":
            fail(f"bad label receipt for {path}")
        if (
            sha256(path) != receipt["output"]["sha256"]
            or path.stat().st_size != receipt["output"]["bytes"]
            or receipt.get("passed") is not True
            or receipt.get("nodeBudgetPerChild") != contract["labels"]["nodesPerChild"]
        ):
            fail(f"label receipt mismatch for {path}")
        label_artifacts.append(artifact(path))
        for row in read_jsonl(path):
            if row.get("schema") != "haitaka-strength-first-step4-restricted-child-label-v1":
                fail(f"bad child-label row in {path}")
            if row["alphaBetaNodes"] + row["qsearchNodes"] != row["totalNodes"]:
                fail(f"child-label node accounting mismatch in {path}")
            key = (row["parentId"], row["move"])
            if key in labels:
                fail(f"duplicate child label {key}")
            labels[key] = row
    planned = {(row["id"], move) for row in union for move in row["unionMoves"]}
    if set(labels) != planned:
        fail(f"label coverage differs: missing={len(planned-set(labels))} extra={len(set(labels)-planned)}")
    for (parent_id, _move), label in labels.items():
        if label["parentSfen"] != union_by_id[parent_id]["sfen"]:
            fail(f"child-label parent SFEN mismatch for {parent_id}")
        if label["totalNodes"] > contract["labels"]["nodesPerChild"]:
            fail(f"child-label node ceiling exceeded for {parent_id}")
    full_rows = read_jsonl(args.full_precision)
    full_summary_path = args.full_precision.parent / "summary.json"
    full_summary = json.loads(full_summary_path.read_text(encoding="utf-8"))
    if (
        full_summary.get("schema") != "haitaka-strength-first-step4-full-precision-summary-v1"
        or full_summary.get("passed") is not True
        or full_summary["contract"]["sha256"] != sha256(args.contract)
        or full_summary["scores"]["sha256"] != sha256(args.full_precision)
        or full_summary["scores"]["bytes"] != args.full_precision.stat().st_size
    ):
        fail("full-precision summary does not bind the supplied scores")
    full = {(row["parentId"], row["move"]): row for row in full_rows}
    if len(full) != len(full_rows) or not planned.issubset(set(full)):
        fail("full-precision scores are duplicated or do not cover every planned child")
    scale = contract["analysis"]["calibratedSoftScoreScale"]
    model_names = ["handcrafted-10000", "c16-10000", "candidate-10000"]
    regrets: dict[str, list[float]] = defaultdict(list)
    regret_rows: dict[str, list[dict[str, Any]]] = defaultdict(list)
    static_pairs: dict[str, list[tuple[float, float]]] = defaultdict(list)
    full_pairs: list[tuple[float, float]] = []
    complete_parents = 0
    incomplete_labels = sum(not row["completed"] for row in labels.values())
    near_equal: dict[str, list[float]] = defaultdict(list)
    mate_parent_count = 0
    mate_best_parent_count = 0
    mate_class_matches: Counter[str] = Counter()
    mate_best_move_matches: Counter[str] = Counter()
    full_integer_choice_agreement = 0
    full_search_choice_agreement = 0
    full_choice_parents = 0
    pruning_all_agree = 0
    pruning_eligible = 0
    analyzed = []
    for parent in union:
        parent_id = parent["id"]
        parent_labels = {move: labels[(parent_id, move)] for move in parent["unionMoves"]}
        completed = {move: row for move, row in parent_labels.items() if row["completed"] and row["rawRootOrientedScore"] is not None}
        if not completed:
            analyzed.append({"parentId": parent_id, "completeUnionLabels": 0, "status": "no-complete-label"})
            continue
        complete_parents += 1
        best_move, best = max(completed.items(), key=lambda item: (teacher_order(item[1]), item[0]))
        ordered = sorted(completed.items(), key=lambda item: (teacher_order(item[1]), item[0]), reverse=True)
        gap = teacher_order(ordered[0][1]) - teacher_order(ordered[1][1]) if len(ordered) > 1 else None
        if any(row["rootOrientedMateClass"] != "none" for row in completed.values()):
            mate_parent_count += 1
        best_is_mate = best["rootOrientedMateClass"] != "none"
        mate_best_parent_count += int(best_is_mate)
        parent_regrets = {}
        for name in model_names:
            choice = parent["rootObservations"][name]["bestMove"]
            chosen = completed.get(choice)
            if chosen is None:
                continue
            value = max(0.0, float(best["boundedRootOrientedScore"] - chosen["boundedRootOrientedScore"])) / SCORE_CLIP
            regrets[name].append(value)
            parent_regrets[name] = value
            regret_rows[name].append({"group": parent["sourceOpeningGroupId"], "value": value})
            if gap is not None and gap <= 250:
                near_equal[name].append(value)
            if best_is_mate:
                mate_class_matches[name] += int(
                    chosen["rootOrientedMateClass"] == best["rootOrientedMateClass"]
                )
                mate_best_move_matches[name] += int(choice == best_move)
        for model_name in ("c16-10000", "candidate-10000"):
            scores = {
                row["move"]: row["rootOrientedScore"]
                for row in parent["rootObservations"][model_name]["childStaticScores"]
            }
            for move, label in completed.items():
                if move in scores and not label["terminal"]:
                    static_pairs[model_name].append((float(scores[move]), float(label["rawRootOrientedScore"])))
        full_scores = {}
        integer_scores = {}
        for move, label in completed.items():
            score = full.get((parent_id, move))
            if score is not None and label["terminal"]:
                full_scores[move] = MATE_BASE
                integer_scores[move] = MATE_BASE
            elif score is not None:
                full_root = -float(score["fullPrecisionScore"])
                integer_root = -float(score["integerScore"])
                full_scores[move] = full_root
                integer_scores[move] = integer_root
                full_pairs.append((full_root, float(label["rawRootOrientedScore"])))
        if full_scores:
            full_choice_parents += 1
            full_choice = max(full_scores, key=lambda move: (full_scores[move], move))
            integer_choice = max(integer_scores, key=lambda move: (integer_scores[move], move))
            full_integer_choice_agreement += int(full_choice == integer_choice)
            full_search_choice_agreement += int(full_choice == parent["rootObservations"]["candidate-10000"]["bestMove"])
        sensitivity_rows = [
            parent["rootObservations"][name]
            for name in ("candidate-low", "candidate-10000", "candidate-high")
        ]
        sensitivity_moves = [row["bestMove"] for row in sensitivity_rows]
        if all(probe_complete(row) for row in sensitivity_rows):
            pruning_eligible += 1
            pruning_all_agree += int(len(set(sensitivity_moves)) == 1)
        analyzed.append({
            "parentId": parent_id,
            "selectionClass": parent["selectionClass"],
            "phase": parent["phase"],
            "openingGroup": parent["sourceOpeningGroupId"],
            "unionMoves": parent["unionMoves"],
            "completeUnionLabels": len(completed),
            "bestRestrictedTeacherMove": best_move,
            "bestRestrictedTeacherScore": best["rawRootOrientedScore"],
            "bestSecondTeacherOrderGap": gap,
            "regret": parent_regrets,
            "sensitivityMoves": sensitivity_moves,
            "status": "analyzed",
        })
    paired = []
    candidate_by_parent = {row["parentId"]: row for row in analyzed if "regret" in row and "candidate-10000" in row["regret"]}
    for row in analyzed:
        if "regret" not in row or "c16-10000" not in row["regret"] or row["parentId"] not in candidate_by_parent:
            continue
        paired.append({
            "group": row["openingGroup"],
            "difference": row["regret"]["candidate-10000"] - row["regret"]["c16-10000"],
        })
    bootstrap = contract["analysis"]["clusterBootstrap"]
    result = {
        "schema": "haitaka-anhoku-strength-first-step4-result-v1",
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "status": "complete-development-diagnostic" if complete_parents else "inconclusive-no-complete-parents",
        "claimScope": "reused-development-evidence-only-not-confirmation-or-promotion",
        "contract": artifact(args.contract),
        "inputs": {
            "union": artifact(args.union),
            "labels": label_artifacts,
            "fullPrecision": artifact(args.full_precision),
        },
        "coverage": {
            "selectedParents": len(union),
            "plannedLabels": len(planned),
            "completeLabels": len(labels) - incomplete_labels,
            "incompleteLabels": incomplete_labels,
            "parentsWithAtLeastOneCompleteLabel": complete_parents,
            "noQuotaRefill": True,
        },
        "restrictedRegret": {name: metric_summary(values) for name, values in sorted(regrets.items())},
        "candidateMinusC16Regret": bootstrap_cluster_difference(
            paired, bootstrap["seed"], bootstrap["replicates"]
        ),
        "scoreCalibration": {
            "c16Integer": regression(static_pairs["c16-10000"], scale),
            "candidateInteger": regression(static_pairs["candidate-10000"], scale),
            "candidateFullPrecision": regression(full_pairs, scale),
            "calibratedSoftScoreScale": scale,
        },
        "nearEqual": {
            "teacherOrderGapThreshold": 250,
            "regret": {name: metric_summary(values) for name, values in sorted(near_equal.items())},
        },
        "tacticalMate": {
            "parentsWithAnyRestrictedMateLabel": mate_parent_count,
            "parentsWhoseBestRestrictedLabelIsMate": mate_best_parent_count,
            "selectedSameMateClassAsBest": dict(sorted(mate_class_matches.items())),
            "selectedExactBestMateMove": dict(sorted(mate_best_move_matches.items())),
        },
        "pruningSensitivity": {
            "candidateNodeBudgets": contract["rootSearch"]["sensitivityCandidateNodes"][:1]
                + [contract["rootSearch"]["developmentNodes"]]
                + contract["rootSearch"]["sensitivityCandidateNodes"][1:],
            "allThreeChoicesAgree": pruning_all_agree,
            "eligibleParents": pruning_eligible,
            "incompleteOrMissingParents": len(union) - pruning_eligible,
            "agreementRate": pruning_all_agree / pruning_eligible if pruning_eligible else None,
        },
        "fullPrecisionExport": {
            "restrictedStaticChoiceAgreement": full_integer_choice_agreement,
            "fullPrecisionStaticVsCandidateSearchAgreement": full_search_choice_agreement,
            "eligibleRestrictedParents": full_choice_parents,
        },
        "parents": analyzed,
    }
    checked_output_dir(args.output_dir)
    write_json(args.output_dir / "result.json", result)
    lines = [
        "# Anhoku strength-first Step 4 diagnostic",
        "",
        f"Generated {result['generatedAt']}. Scope: **reused development evidence only**.",
        "",
        f"The fixed diagnostic retained {len(union)} parents and {len(planned)} restricted child labels. "
        f"{len(labels)-incomplete_labels} labels completed; {incomplete_labels} were retained as incomplete without quota refill.",
        "",
        "| Evaluator | Parents | Mean restricted regret | Median | p95 |",
        "| --- | ---: | ---: | ---: | ---: |",
    ]
    for name, values in sorted(regrets.items()):
        metric = metric_summary(values)
        lines.append(f"| {name} | {metric['count']} | {metric['mean']:.6f} | {metric['median']:.6f} | {metric['p95']:.6f} |")
    diff = result["candidateMinusC16Regret"]
    lines.extend([
        "",
        f"Candidate minus C/16 mean restricted regret was {diff['mean']} with the development opening-group bootstrap interval "
        f"[{diff['lower95']}, {diff['upper95']}]. Negative values favor the candidate.",
        "",
        f"Candidate search chose the same move at all three node budgets on {pruning_all_agree}/{pruning_eligible} eligible parents. "
        f"Full-precision and integer static choices agreed on {full_integer_choice_agreement}/{full_choice_parents} eligible restricted parents.",
        "",
        "See `result.json` for score calibration, near-equal, mate, per-parent, and completeness details.",
    ])
    (args.output_dir / "result.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote Step 4 analysis to {args.output_dir}")


def command_validate(args: argparse.Namespace) -> None:
    contract = load_contract(args.contract)
    print(json.dumps({"status": "passed", "experimentId": contract["experimentId"], "contractSha256": sha256(args.contract)}, indent=2))


def command_self_test(_args: argparse.Namespace) -> None:
    contract = {
        "parents": {
            "maximumSelected": 4,
            "selectionQuotas": {
                "disagreement": {"early": 1, "middle": 0, "late": 0},
                "control": {"early": 1, "middle": 0, "late": 0},
            },
        }
    }
    parents = [
        {"id": "a", "sfen": "a", "phase": "early"},
        {"id": "b", "sfen": "b", "phase": "early"},
    ]
    def probes(moves: list[str]) -> dict[str, dict[str, Any]]:
        return {
            row["id"]: {
                "parentSfen": row["sfen"],
                "bestMove": move,
                "rootScore": 0,
                "completedDepth": 1,
                "legalBestMove": True,
            }
            for row, move in zip(parents, moves)
        }
    selected, summary = select_rows(contract, parents, probes(["x", "x"]), probes(["y", "x"]), probes(["x", "x"]))
    assert len(selected) == 2 and summary["selectedByClass"] == {"control": 1, "disagreement": 1}
    assert robust_huber(0, 0) == 0 and math.isclose(robust_huber(1200, 0), 0.005)
    regression_result = regression([(0, 10), (10, 30), (20, 50)], 6528.0)
    assert math.isclose(regression_result["slopeTeacherOnModel"], 2.0)
    boot = bootstrap_cluster_difference(
        [{"group": "a", "difference": -1.0}, {"group": "b", "difference": 1.0}], 7, 1000
    )
    assert boot["groups"] == 2 and math.isclose(boot["mean"], 0.0)
    print("strength-first Step 4 orchestration self-test passed")


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    sub = result.add_subparsers(dest="command", required=True)
    validate = sub.add_parser("validate")
    validate.add_argument("--contract", type=Path, required=True)
    validate.set_defaults(function=command_validate)
    select = sub.add_parser("select")
    select.add_argument("--contract", type=Path, required=True)
    select.add_argument("--parents", type=Path, required=True)
    for name in ("handcrafted", "c16", "candidate"):
        select.add_argument(f"--{name}-probe", type=Path, action="append", required=True)
    select.add_argument("--output-dir", type=Path, required=True)
    select.set_defaults(function=command_select)
    union = sub.add_parser("union")
    union.add_argument("--contract", type=Path, required=True)
    union.add_argument("--selected", type=Path, required=True)
    for name in ("handcrafted", "c16", "candidate", "deep-handcrafted", "candidate-low", "candidate-high"):
        union.add_argument(f"--{name}-probe", type=Path, action="append", required=True)
    union.add_argument("--output-dir", type=Path, required=True)
    union.set_defaults(function=command_union)
    full = sub.add_parser("full-precision")
    full.add_argument("--contract", type=Path, required=True)
    full.add_argument("--positions", type=Path, required=True)
    full.add_argument("--checkpoint", type=Path, required=True)
    full.add_argument("--network", type=Path, required=True)
    full.add_argument("--cmake", default="cmake")
    full.add_argument("--output-dir", type=Path, required=True)
    full.set_defaults(function=command_full_precision)
    analyze = sub.add_parser("analyze")
    analyze.add_argument("--contract", type=Path, required=True)
    analyze.add_argument("--union", type=Path, required=True)
    analyze.add_argument("--labels", type=Path, action="append", required=True)
    analyze.add_argument("--full-precision", type=Path, required=True)
    analyze.add_argument("--output-dir", type=Path, required=True)
    analyze.set_defaults(function=command_analyze)
    test = sub.add_parser("self-test")
    test.set_defaults(function=command_self_test)
    return result


def main() -> None:
    args = parser().parse_args()
    for name, value in vars(args).items():
        if isinstance(value, Path):
            setattr(args, name.replace("-", "_"), value.resolve())
        elif isinstance(value, list) and value and isinstance(value[0], Path):
            setattr(args, name.replace("-", "_"), [path.resolve() for path in value])
    args.function(args)


if __name__ == "__main__":
    main()
