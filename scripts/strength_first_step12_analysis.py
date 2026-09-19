#!/usr/bin/env python3
"""Pure deterministic analysis for the sealed Step 12 development matches."""

from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime, timezone
from fractions import Fraction
import hashlib
import json
import math
import os
from pathlib import Path
import sys
from typing import Any


EXPERIMENT_ID = "strength-first-step12-independent-replication-20260916-v1"
CELLS = ("early-selected", "u2688-control")
BLOCKS = 4
PAIRS_PER_BLOCK = 32
PAIRS = 128
GAMES_PER_BLOCK = 64
GAMES_PER_CELL = 256
BOOTSTRAP_SEED = 2026091608
BOOTSTRAP_REPLICATES = 10_000
THRESHOLD = Fraction(1, 50)
HALF = Fraction(1, 2)


class AnalysisError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise AnalysisError(message)


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def load(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        fail(f"cannot load {path}: {exc}")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def rational(value: Fraction) -> dict[str, Any]:
    return {"numerator": value.numerator, "denominator": value.denominator, "decimal": float(value), "exact": str(value)}


def score(result: str) -> Fraction:
    values = {"a-win": Fraction(1), "draw": HALF, "b-win": Fraction(0)}
    if result not in values:
        fail(f"invalid result {result!r}")
    return values[result]


class SplitMix64:
    def __init__(self, seed: int):
        self.state = seed & ((1 << 64) - 1)

    def next(self) -> int:
        self.state = (self.state + 0x9E3779B97F4A7C15) & ((1 << 64) - 1)
        value = self.state
        value = ((value ^ (value >> 30)) * 0xBF58476D1CE4E5B9) & ((1 << 64) - 1)
        value = ((value ^ (value >> 27)) * 0x94D049BB133111EB) & ((1 << 64) - 1)
        return value ^ (value >> 31)

    def index(self, bound: int) -> int:
        if bound <= 0:
            fail("SplitMix64 bound must be positive")
        limit = (1 << 64) - ((1 << 64) % bound)
        while True:
            value = self.next()
            if value < limit:
                return value % bound


def percentile(values: list[float], probability: float) -> float:
    if not values:
        fail("percentile input is empty")
    ordered = sorted(values)
    position = probability * (len(ordered) - 1)
    low = math.floor(position)
    high = math.ceil(position)
    if low == high:
        return ordered[low]
    return ordered[low] * (high - position) + ordered[high] * (position - low)


def accepted(root: Path, cell: str, block: int) -> tuple[Path, dict[str, Any], list[dict[str, Any]]]:
    block_dir = root / "matches" / cell / f"block-{block:02d}"
    candidates: list[tuple[Path, dict[str, Any]]] = []
    for attempt in sorted(block_dir.glob("attempt-*")):
        validation_path = attempt / "validation.json"
        accepted_path = attempt / "accepted.json"
        if validation_path.is_file() and accepted_path.is_file():
            value = load(validation_path)
            if value.get("status") == "passed" and value.get("sealed") is True:
                candidates.append((attempt, value))
    if len(candidates) != 1:
        fail(f"{cell} block {block} has {len(candidates)} accepted validations")
    attempt, validation = candidates[0]
    games_path = attempt / "raw/self-play-games.jsonl"
    try:
        games = [json.loads(line) for line in games_path.read_text(encoding="utf-8").splitlines() if line.strip()]
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        fail(f"cannot load raw games for {cell} block {block}: {exc}")
    if len(games) != GAMES_PER_BLOCK:
        fail(f"raw game count differs for {cell} block {block}")
    return attempt, validation, games


def collect(root: Path) -> tuple[dict[str, list[Fraction]], dict[str, list[dict[str, Any]]], dict[str, Any], list[dict[str, Any]]]:
    scores: dict[str, list[Fraction]] = {cell: [] for cell in CELLS}
    pairs: dict[str, list[dict[str, Any]]] = {cell: [] for cell in CELLS}
    bounds: dict[str, dict[str, list[Fraction]]] = {cell: {"lower": [], "upper": []} for cell in CELLS}
    validations: dict[str, Any] = {}
    validation_refs: list[dict[str, Any]] = []
    for cell in CELLS:
        wdl = Counter()
        capped_games = 0
        blocks: list[dict[str, Any]] = []
        game_by_pair: dict[int, list[dict[str, Any]]] = {pair: [] for pair in range(PAIRS)}
        for block in range(BLOCKS):
            attempt, validation, games = accepted(root, cell, block)
            validation_path = attempt / "validation.json"
            validation_refs.append({"cell": cell, "block": block, "attempt": attempt.name, "path": str(validation_path.relative_to(root)), "sha256": sha256(validation_path)})
            blocks.append({"block": block, "attempt": attempt.name, "validation": {"path": str(validation_path.relative_to(root)), "sha256": sha256(validation_path)}, "rawGames": {"path": str((attempt / "raw/self-play-games.jsonl").relative_to(root)), "sha256": sha256(attempt / "raw/self-play-games.jsonl")}})
            for local_index, game in enumerate(games):
                pair = block * PAIRS_PER_BLOCK + local_index // 2
                if game.get("pairIndex") != local_index // 2:
                    fail(f"local pair index differs in {cell} block {block}")
                value = score(game.get("result"))
                capped = game.get("terminationReason") == "maximum-plies"
                game_by_pair[pair].append({"score": value, "capped": capped, "result": game["result"], "gameIndex": game.get("gameIndex")})
                wdl[game["result"]] += 1
                capped_games += int(capped)
        for pair in range(PAIRS):
            games = game_by_pair[pair]
            if len(games) != 2:
                fail(f"{cell} pair {pair} is incomplete")
            actual = sum((game["score"] for game in games), Fraction(0)) / 2
            lower = sum((Fraction(0) if game["capped"] else game["score"] for game in games), Fraction(0)) / 2
            upper = sum((Fraction(1) if game["capped"] else game["score"] for game in games), Fraction(0)) / 2
            scores[cell].append(actual)
            bounds[cell]["lower"].append(lower)
            bounds[cell]["upper"].append(upper)
            pairs[cell].append({"pairIndex": pair, "score": rational(actual), "games": [{"gameIndex": game["gameIndex"], "result": game["result"], "score": rational(game["score"]), "capped": game["capped"]} for game in games], "cappedGames": sum(int(game["capped"]) for game in games)})
        validations[cell] = {"blocks": blocks, "games": GAMES_PER_CELL, "pairs": PAIRS, "wdl": {"aWins": wdl["a-win"], "draws": wdl["draw"], "bWins": wdl["b-win"]}, "cappedGames": capped_games}
    return scores, pairs, bounds, validation_refs


def bootstrap(differences: list[Fraction]) -> dict[str, Any]:
    if len(differences) != PAIRS:
        fail("primary analysis requires exactly 128 paired openings")
    rng = SplitMix64(BOOTSTRAP_SEED)
    samples: list[float] = []
    vector_prefix: list[int] = []
    for replicate in range(BOOTSTRAP_REPLICATES):
        selected: list[int] = []
        total = Fraction(0)
        for _ in range(PAIRS):
            index = rng.index(PAIRS)
            selected.append(index)
            total += differences[index]
        samples.append(float(total / PAIRS))
        if replicate == 0:
            vector_prefix = selected[:16]
    point = sum(differences, Fraction(0)) / PAIRS
    return {"method": "paired-opening-index-resample-splitmix64-v1", "seed": BOOTSTRAP_SEED, "replicates": BOOTSTRAP_REPLICATES, "unit": "opening pair", "percentile": "linear-type-7-v1", "point": rational(point), "lower95": percentile(samples, 0.025), "upper95": percentile(samples, 0.975), "vectorPrefix": vector_prefix, "decision": "supported-material-advantage" if percentile(samples, 0.025) > 0.02 else "material-inferiority" if percentile(samples, 0.975) < -0.02 else "inconclusive"}


def capped_summary(scores: dict[str, list[Fraction]], bounds: dict[str, dict[str, list[Fraction]]]) -> dict[str, Any]:
    early_lower = sum(bounds["early-selected"]["lower"], Fraction(0)) / PAIRS
    early_upper = sum(bounds["early-selected"]["upper"], Fraction(0)) / PAIRS
    control_lower = sum(bounds["u2688-control"]["lower"], Fraction(0)) / PAIRS
    control_upper = sum(bounds["u2688-control"]["upper"], Fraction(0)) / PAIRS
    contrast_lower = sum((bounds["early-selected"]["lower"][i] - bounds["u2688-control"]["upper"][i] for i in range(PAIRS)), Fraction(0)) / PAIRS
    contrast_upper = sum((bounds["early-selected"]["upper"][i] - bounds["u2688-control"]["lower"][i] for i in range(PAIRS)), Fraction(0)) / PAIRS
    return {"policy": "maximum-plies games are bounded to [0,1]; genuine results remain fixed", "boundsByCell": {"early-selected": {"lower": rational(early_lower), "upper": rational(early_upper)}, "u2688-control": {"lower": rational(control_lower), "upper": rational(control_upper)}}, "contrastBounds": {"lower": rational(contrast_lower), "upper": rational(contrast_upper)}, "earlySelectedAboveHalfActual": scores["early-selected"] and sum(scores["early-selected"], Fraction(0)) / PAIRS > HALF, "earlySelectedAboveHalfLower": early_lower > HALF, "earlySelectedAboveHalfUpper": early_upper > HALF, "earlySelectedAboveHalfDependsOnCappedGames": early_lower <= HALF < early_upper}


def analyze(root: Path, output: Path) -> dict[str, Any]:
    root = root.expanduser().resolve()
    gate = load(root / "preflight/qualification-gate.json")
    if gate.get("status") != "passed":
        fail("qualification gate is not passed")
    status = load(root / "matches/status.json")
    if status.get("status") != "complete-sealed" or status.get("completedBlocks") != 8 or status.get("completedGames") != 512:
        fail("matches are not complete and sealed")
    scores, pairs, bounds, validation_refs = collect(root)
    differences = [scores["early-selected"][i] - scores["u2688-control"][i] for i in range(PAIRS)]
    point = sum(differences, Fraction(0)) / PAIRS
    boot = bootstrap(differences)
    caps = capped_summary(scores, bounds)
    favorable = bool(boot["lower95"] > 0.02 and caps["earlySelectedAboveHalfActual"] is True and caps["earlySelectedAboveHalfLower"] is True and gate.get("status") == "passed")
    if boot["lower95"] > 0.02:
        contrast_class = "supported-material-advantage"
    elif boot["upper95"] < -0.02:
        contrast_class = "material-inferiority"
    else:
        contrast_class = "inconclusive"
    result = {
        "schema": "haitaka-anhoku-strength-first-step12-analysis-v1",
        "createdAt": now(),
        "status": "complete-sealed",
        "experimentId": EXPERIMENT_ID,
        "scope": "exploratory-development-only",
        "schedule": {"cells": list(CELLS), "blocks": BLOCKS, "pairs": PAIRS, "games": 512, "matchSeed": 2026091607, "movetimeMs": 100, "maxPlies": 200},
        "models": {cell: {"games": GAMES_PER_CELL, "pairs": PAIRS, "meanPairScore": rational(sum(scores[cell], Fraction(0)) / PAIRS), "wdl": {"aWins": sum(1 for pair in pairs[cell] for game in pair["games"] if game["result"] == "a-win"), "draws": sum(1 for pair in pairs[cell] for game in pair["games"] if game["result"] == "draw"), "bWins": sum(1 for pair in pairs[cell] for game in pair["games"] if game["result"] == "b-win")}, "cappedGames": sum(pair["cappedGames"] for pair in pairs[cell]), "pairScores": pairs[cell]} for cell in CELLS},
        "primaryContrast": {"name": "early-selected-minus-u2688-control", "point": rational(point), "interval95": {"lower": boot["lower95"], "upper": boot["upper95"]}, "classification": contrast_class, "threshold": rational(THRESHOLD)},
        "bootstrap": boot,
        "cappedSensitivity": caps,
        "qualification": {"path": "preflight/qualification-gate.json", "sha256": sha256(root / "preflight/qualification-gate.json")},
        "validationReferences": validation_refs,
        "routing": {"favorableDevelopmentReplication": favorable, "recommendBrowserOneSecondCheck": favorable, "recommendInvestigation": not favorable, "recommendation": "prepare separately delegated 1,000 ms browser/WASM check of early-selected" if favorable else "keep handcrafted production and investigate the replication before any browser follow-up", "absoluteScoreGate": "early-selected nominal and capped lower bound strictly above 0.5 required", "primaryGate": "two-sided 95% lower endpoint strictly above +0.02 required"},
        "deploymentAuthorized": False,
        "confirmationAuthorized": False,
        "handcraftedRemainsProduction": True,
        "outcomesSealedAfterAllCellsValidated": True,
        "gamesLaunched": 0,
    }
    atomic_json(output, result)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("analyze", choices=("analyze",))
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        root = args.root.expanduser().resolve()
        result = analyze(root, args.output or root / "analysis/analysis.json")
        print(json.dumps({"status": result["status"], "classification": result["primaryContrast"]["classification"], "favorable": result["routing"]["favorableDevelopmentReplication"]}, sort_keys=True))
        return 0
    except (AnalysisError, OSError, KeyError, TypeError, ValueError) as exc:
        print(f"Step 12 analysis blocked: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
