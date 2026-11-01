#!/usr/bin/env python3
"""F1 fixed-exclusion, conditionally-with-replacement opening builder.

Only the explicit Step 18 safe union and its two newly exposed suites are
read. Protected outcome trees are never discovered or opened.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Any, Iterable

import strength_first_f1_core as f1


STEP18 = f1.ROOT / "out/anhoku-strength-first/step18-model-strength-closeout-20260920-v1"
UNION = STEP18 / "preflight/current-families/union.jsonl"
SOURCE_FREEZE = STEP18 / "preflight/source-freeze.json"
HELPER = f1.ROOT / "target/step18-preparation-helper/release/haitaka_learn"
EXPECTED_UNION_HASH = "31224f20c06d25f8bf13eeac956ae6c4a7f6b6bfd02f86eecef8e5a9a246fa61"
EXPECTED_HELPER_HASH = "ac142e756ac0e3d96fe3e5badd92db4866263b0efd471d521f7f0510ad2eabb1"
EXPECTED_NORMALIZER_HASH = "e6138e0cd91755fa107cd404fd1bf23e9c3acc5742a6a2b63a515ba3b3e6d42a"
MASTER = {"development": 2026092502, "holdout": 2026092503}
PAIRS = {"development": 128, "holdout": 512}


def seed(phase: str, index: int) -> int:
    f1.require(phase in MASTER and type(index) is int and index >= 0, "invalid proposal identity")
    value = f"f1:{phase}:{MASTER[phase]}:{index}".encode("ascii")
    return int.from_bytes(hashlib.sha256(value).digest()[:8], "big")


def _family_rows(path: Path) -> Iterable[str]:
    with path.open("rb") as stream:
        for line in stream:
            row = json.loads(line)
            if row.get("kind") == "family":
                yield row["canonicalFamily"]


def _helper_inventory(input_path: Path, output_path: Path, identifier: str) -> dict[str, Any]:
    argv = [str(HELPER), "step12-exclusion-inventory", "--input", str(input_path), "--output", str(output_path), "--inventory-id", identifier]
    completed = subprocess.run(argv, text=True, capture_output=True, check=False, timeout=180)
    f1.require(completed.returncode == 0, f"normalization helper failed: {completed.stderr[-1000:]}")
    report = json.loads(completed.stdout)
    f1.require(report.get("status") == "passed" and output_path.is_file(), "normalization helper did not publish inventory")
    return {"argv": argv, "receipt": report, "output": f1.artifact(output_path)}


def explicit_source_union(root: Path) -> tuple[set[str], dict[str, Any]]:
    f1.require(UNION.stat().st_size == 333810681 and f1.sha256(UNION) == EXPECTED_UNION_HASH, "Step 18 safe source union changed")
    f1.require(f1.sha256(HELPER) == EXPECTED_HELPER_HASH, "retained helper binary changed")
    normalizer = f1.ROOT / "haitaka_learn/src/step11_opening_gate.rs"
    f1.require(f1.sha256(normalizer) == EXPECTED_NORMALIZER_HASH, "canonicalization source changed; rebuild safe inventories before reuse")
    freeze = json.loads(SOURCE_FREEZE.read_text())
    f1.require(freeze.get("status") == "passed" and freeze.get("protectedOutcomeTraversal") is False and freeze.get("recursiveDiscoveryUsed") is False, "Step 18 source freeze is unsafe")
    f1.require(freeze.get("union", {}).get("sha256") == EXPECTED_UNION_HASH, "Step 18 union upstream binding differs")
    f1.require(len(freeze.get("entries", [])) == 37, "Step 18 source inventory count differs")
    for entry in freeze["entries"]:
        for payload in entry["payloads"]:
            path = f1.ROOT / payload["path"]
            f1.require(f1.artifact(path) == payload, f"safe source payload changed: {path}")
        inventory = entry["inventory"]
        inv_path = Path(inventory["path"])
        f1.require(f1.sha256(inv_path) == inventory["sha256"] and inv_path.stat().st_size == inventory["bytes"], f"safe inventory changed: {inv_path}")
    families = set(_family_rows(UNION))
    f1.require(len(families) == 2915805, "Step 18 union family count differs")
    added = {}
    for phase in ("selection", "holdout"):
        opening = STEP18 / f"openings/{phase}.json"
        value = json.loads(opening.read_text())
        expected_count = 128 if phase == "selection" else 512
        f1.require(len(value["rows"]) == expected_count, f"Step 18 {phase} suite count differs")
        suite = root / "openings/source-suites" / f"step18-{phase}.tsv"
        suite.parent.mkdir(parents=True, exist_ok=True)
        payload = "".join(f"{row['openingId']}\t{row['sfen']}\n" for row in value["rows"]).encode()
        if suite.exists():
            f1.require(suite.read_bytes() == payload, f"Step 18 {phase} suite projection changed")
        else:
            suite.write_bytes(payload)
        inventory = root / "openings/source-suites" / f"step18-{phase}.jsonl"
        receipt = _helper_inventory(suite, inventory, f"f1-step18-{phase}") if not inventory.exists() else {"output": f1.artifact(inventory), "reused": True}
        normalized = set(_family_rows(inventory))
        declared = {row["canonicalFamily"] for row in value["rows"]}
        f1.require(normalized == declared, f"Step 18 {phase} suite normalizer differs")
        families.update(normalized)
        added[phase] = {"opening": f1.artifact(opening), "suite": f1.artifact(suite), "inventory": f1.artifact(inventory), "canonicalFamilies": len(normalized), "helper": receipt}
    return families, {"union": f1.artifact(UNION), "sourceFreeze": f1.artifact(SOURCE_FREEZE), "helper": f1.artifact(HELPER), "normalizer": f1.artifact(normalizer), "postUnionSuites": added, "totalFamilies": len(families), "protectedOutcomeTraversal": False}


def _proposal(root: Path, phase: str, index: int) -> tuple[str, str, dict[str, Any]]:
    proposal_dir = root / "openings/proposals" / phase / f"{index:05d}"
    proposal_dir.mkdir(parents=True, exist_ok=True)
    suite = proposal_dir / "proposal.tsv"
    report = proposal_dir / "generator.json"
    identifier = f"f1-{phase}-{index:05d}"
    argv = [str(HELPER), "strength-first-generate-opening-suite", "--output", str(suite), "--report", str(report), "--seed", str(seed(phase, index)), "--positions", "1", "--random-plies", "16", "--id-prefix", identifier]
    f1.require("--exclude-suite" not in argv, "inner generator exclusion is forbidden")
    pregenerated = proposal_dir / "generation-identity.json"
    generation = None
    if suite.exists() or report.exists():
        f1.require(suite.is_file() and report.is_file(), "partial existing proposal")
        if pregenerated.exists():
            generation = json.loads(pregenerated.read_text())
            f1.require(generation.get("schema") == "haitaka-anhoku-strength-first-f1-mbp-proposal-v1" and generation.get("phase") == phase and generation.get("proposalIndex") == index and generation.get("seed") == seed(phase, index), "MBP proposal identity differs")
            for binding, path in ((generation.get("output"), suite), (generation.get("report"), report)):
                f1.require(isinstance(binding, dict) and binding.get("bytes") == path.stat().st_size and binding.get("sha256") == f1.sha256(path), "MBP proposal transfer differs")
            f1.require(generation.get("producerHost") == "ITSO-WX6303" and "--exclude-suite" not in generation.get("argv", []) and generation["argv"][generation["argv"].index("--seed")+1] == str(seed(phase, index)), "MBP generator command differs")
    else:
        completed = subprocess.run(argv, capture_output=True, text=True, check=False, timeout=120)
        f1.require(completed.returncode == 0, f"generator failed at {phase} {index}: {completed.stderr[-1000:]}")
    lines = [line for line in suite.read_text().splitlines() if line and not line.startswith("#")]
    f1.require(len(lines) == 1 and "\t" in lines[0], "generator did not produce exactly one position")
    output = proposal_dir / "family.jsonl"
    if not output.exists():
        _helper_inventory(suite, output, identifier)
    rows = list(_family_rows(output))
    f1.require(len(rows) == 1, "proposal has no unique legal canonical family")
    opening_id, sfen = lines[0].split("\t", 1)
    return sfen, rows[0], {"proposalIndex": index, "seed": seed(phase, index), "generatorArgv": generation["argv"] if generation else argv, "generator": f1.artifact(suite), "generatorReport": f1.artifact(report), "generationIdentity": f1.artifact(pregenerated) if generation else None, "normalization": f1.artifact(output), "openingId": opening_id}


def _schedule(phase: str, rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    schedule = []
    for pair_index, row in enumerate(rows):
        matchups = ("T-C", "T-I") if phase == "development" and pair_index % 2 == 0 else ("T-I", "T-C") if phase == "development" else ("T-I",)
        for matchup in matchups:
            for leg in (0, 1):
                color = "black" if (pair_index + leg) % 2 == 0 else "white"
                opponent = matchup[-1]
                side = row["sfen"].split()[1]
                f1.require(side in {"b", "w"}, "opening side to move is invalid")
                schedule.append({"phase": phase, "pairIndex": pair_index, "blockIndex": pair_index // 8, "matchup": matchup, "leg": leg, "treatmentColor": color, "openingId": row["openingId"], "sfen": row["sfen"], "canonicalFamily": row["canonicalFamily"], "gameId": f"{phase}-{pair_index:04d}-{matchup}-{leg}", "gameIndex": len(schedule), "blackEngine": "T" if color == "black" else opponent, "whiteEngine": "T" if color == "white" else opponent, "initialSideToMove": "black" if side == "b" else "white", "movetimeMs": 1000, "maxPlies": 200})
    f1.require(len(schedule) == (512 if phase == "development" else 1024), "F1 schedule length differs")
    return schedule


def build(root: Path) -> dict[str, Any]:
    f1.require((root / "contract.json").is_file(), "F1 contract missing")
    excluded, source_receipt = explicit_source_union(root)
    source_path = root / "openings/source-exclusion.json"
    if source_path.exists():
        archived = json.loads(source_path.read_text())
        for key in ("union", "sourceFreeze", "helper", "normalizer", "totalFamilies", "protectedOutcomeTraversal"):
            f1.require(archived.get(key) == source_receipt.get(key), f"source exclusion {key} changed")
        for phase in ("selection", "holdout"):
            for key in ("opening", "suite", "inventory", "canonicalFamilies"):
                f1.require(archived["postUnionSuites"][phase][key] == source_receipt["postUnionSuites"][phase][key], f"source exclusion {phase}/{key} changed")
        source_receipt = archived
    f1.immutable_json(source_path, source_receipt)
    phase_reports = {}
    development_families: set[str] = set()
    for phase in ("development", "holdout"):
        phase_excluded = excluded | (development_families if phase == "holdout" else set())
        rows = []
        rejected = []
        for index in range(100000):
            if len(rows) == PAIRS[phase]:
                break
            sfen, family, evidence = _proposal(root, phase, index)
            if family in phase_excluded:
                rejected.append({**evidence, "canonicalFamily": family, "reason": "source-or-prior-phase-family"})
                continue
            # Do not insert into phase_excluded: same-phase duplicates are retained.
            rows.append({**evidence, "acceptedIndex": len(rows), "openingId": f"{phase}-{len(rows):04d}", "sfen": sfen, "canonicalFamily": family})
        f1.require(len(rows) == PAIRS[phase], f"{phase} exhausted proposal cap")
        if phase == "development":
            development_families = {row["canonicalFamily"] for row in rows}
        phase_path = root / "openings" / f"{phase}.json"
        schedule_path = root / "openings" / f"{phase}-schedule.json"
        f1.immutable_json(phase_path, {"schema": "haitaka-anhoku-strength-first-f1-openings-v1", "experimentId": f1.EXPERIMENT_ID, "phase": phase, "masterSeed": MASTER[phase], "proposalCount": len(rows)+len(rejected), "rows": rows, "rejected": rejected, "sourceExclusion": f1.artifact(root / "openings/source-exclusion.json"), "withinPhaseDeduplication": False, "holdoutExcludesDevelopment": phase == "holdout", "outcomesOpened": False})
        f1.immutable_json(schedule_path, {"schema": "haitaka-anhoku-strength-first-f1-schedule-v1", "phase": phase, "rows": _schedule(phase, rows), "outcomesOpened": False})
        phase_reports[phase] = {"openings": f1.artifact(phase_path), "schedule": f1.artifact(schedule_path), "accepted": len(rows), "rejected": len(rejected)}
    return phase_reports


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=f1.F1_ROOT)
    args = parser.parse_args()
    try:
        result = build(args.root.resolve())
    except (f1.F1Error, OSError, KeyError, ValueError, subprocess.TimeoutExpired) as exc:
        print(f"F1 openings blocked: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
