#!/usr/bin/env python3
"""Offline F1 development screen and single independent holdout analysis."""

from __future__ import annotations

import argparse
from collections import defaultdict
from datetime import datetime, timezone
from fractions import Fraction
import json
import math
from pathlib import Path
import sys
from typing import Any

import strength_first_f1_core as f1


def _load(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text())
    f1.require(isinstance(value, dict), f"object required: {path}")
    return value


def _phase(root: Path, phase: str) -> dict[str, Any]:
    f1.require(phase in {"development", "holdout"}, "invalid F1 phase")
    schedule_path = root / f"openings/{phase}-schedule.json"
    schedule = _load(schedule_path)
    rows = schedule["rows"]
    expected = 512 if phase == "development" else 1024
    f1.require(len(rows) == expected, "frozen schedule length differs")
    by_id = {row["gameId"]: row for row in rows}
    f1.require(len(by_id) == expected, "duplicate scheduled game ID")
    blocks = 16 if phase == "development" else 64
    accepted: list[dict[str, Any]] = []
    evidence = []
    replay_cache: dict[str, dict[int, dict[str, Any]]] = {}
    total_wall = 0.0
    for block in range(blocks):
        path = root / f"games/{phase}/block-{block:02d}/accepted.json"
        receipt = _load(path)
        f1.require(receipt.get("schema") == "haitaka-anhoku-strength-first-f1-accepted-block-v1" and receipt.get("phase") == phase and receipt.get("blockIndex") == block, f"invalid accepted block {block}")
        f1.require(receipt.get("scheduleSha256") == f1.sha256(schedule_path), "accepted block schedule binding differs")
        f1.require(f1.verify_artifact(receipt["runtimeFreeze"]) == root / "runtime/freeze.json", "accepted runtime freeze differs")
        runtime = _load(root / "runtime/freeze.json")
        f1.verify_artifact(runtime["package"])
        for name, item in runtime["models"].items():
            f1.verify_artifact(item if name == "I" else item["network"])
        for part in receipt["parts"]:
            f1.verify_artifact(part["report"])
            f1.verify_artifact(part["telemetry"])
        games = receipt.get("games")
        f1.require(isinstance(games, list) and len(games) == (32 if phase == "development" else 16), f"block {block} game count differs")
        wall = receipt.get("wallSeconds")
        f1.require(type(wall) in (int, float) and math.isfinite(wall) and wall >= 0, "invalid wall time")
        total_wall += wall
        for game in games:
            game_id = game.get("gameId")
            f1.require(game_id in by_id and game.get("schedule") == by_id[game_id], f"game schedule identity differs: {game_id}")
            raw = _load(f1.verify_artifact(game["raw"]))
            f1.require(raw.get("gameId") == game_id and raw.get("moves") is not None and raw.get("plies") == len(raw["moves"]), f"raw game evidence differs: {game_id}")
            replay_binding = game.get("replay", {})
            f1.require(game.get("valid") is True and replay_binding.get("searchFree") is True and replay_binding.get("legal") is True, f"invalid game evidence: {game_id}")
            mapping = _load(f1.verify_artifact(replay_binding["mapping"]))
            f1.require(any(item.get("f1GameId") == game_id and item.get("replayGameIndex") == replay_binding["replayGameIndex"] and item.get("f1Leg") == game["schedule"]["leg"] and item.get("matchup") == game["schedule"]["matchup"] for item in mapping.get("rows", [])), f"replay identity map differs: {game_id}")
            replay_path = f1.verify_artifact(replay_binding["receipt"])
            cache_key = str(replay_path)
            if cache_key not in replay_cache:
                replay_rows = [json.loads(line) for line in replay_path.read_text().splitlines() if line]
                replay_cache[cache_key] = {item["globalGameIndex"]: item for item in replay_rows}
                f1.require(len(replay_cache[cache_key]) == len(replay_rows), "replay identity duplicated")
            replayed = replay_cache[cache_key].get(replay_binding["replayGameIndex"])
            f1.require(replayed is not None and replayed.get("status") == "passed" and replayed.get("searchDispatched") is False and replayed.get("modelLoaded") is False and replayed.get("browserLaunched") is False and all(replayed.get("checks", {}).values()), f"replay evidence differs: {game_id}")
            f1.require(replayed.get("replayedPlies") == raw["plies"], f"replayed ply count differs: {game_id}")
            score, genuine_cap = f1.canonical_replay_result(raw, replayed)
            f1.require(game.get("scoreTreatment") == score and game.get("genuinePlyCap") == genuine_cap, f"game score differs from replay: {game_id}")
            accepted.append(game)
        evidence.append(f1.artifact(path))
    f1.require(len(accepted) == expected and {game["gameId"] for game in accepted} == set(by_id), "accepted game IDs incomplete or repeated")
    matchups = ("T-C", "T-I") if phase == "development" else ("T-I",)
    analysis = {}
    for matchup in matchups:
        by_pair: dict[int, list[dict[str, Any]]] = defaultdict(list)
        for game in accepted:
            if game["schedule"]["matchup"] == matchup:
                by_pair[game["schedule"]["pairIndex"]].append(game)
        expected_pairs = 128 if phase == "development" else 512
        f1.require(set(by_pair) == set(range(expected_pairs)), f"{matchup} pair indices incomplete")
        pair_legs = []
        for pair_index in range(expected_pairs):
            games = sorted(by_pair[pair_index], key=lambda game: game["schedule"]["leg"])
            f1.require([item["schedule"]["leg"] for item in games] == [0, 1], "pair legs missing or duplicate")
            f1.require(games[0]["schedule"]["sfen"] == games[1]["schedule"]["sfen"] and games[0]["schedule"]["treatmentColor"] != games[1]["schedule"]["treatmentColor"], "pair lacks exact opening/color swap")
            pair_legs.append(games)
        analysis[matchup] = f1.pair_statistics(pair_legs)
    result = {"schema": "haitaka-anhoku-strength-first-f1-analysis-v1", "experimentId": f1.EXPERIMENT_ID, "phase": phase, "createdAtUtc": datetime.now(timezone.utc).isoformat(), "schedule": f1.artifact(schedule_path), "acceptedBlocks": evidence, "matchups": analysis, "wallSeconds": total_wall, "games": expected, "searchesDispatchedByAnalysis": 0, "jobsDispatchedByAnalysis": 0}
    if phase == "development":
        result["screenAdvances"] = all(analysis[name]["ordinaryAtLeast052"] and analysis[name]["conservativeAtLeast052"] for name in matchups)
        result["significanceClaim"] = False
    else:
        development = _load(root / "analysis/development.json")
        f1.require(development.get("screenAdvances") is True, "holdout may only be analyzed after development pass")
        result["development"] = f1.artifact(root / "analysis/development.json")
        result["superioritySupported"] = analysis["T-I"]["holdoutRejectsHalf"] is True
        result["null"] = "expected-conservative-pair-score-at-most-one-half-under-frozen-eligible-opening-runtime-distribution"
        result["test"] = "fixed-bet-lambda-one-eighth-exact-strict-integer-threshold-alpha-point-zero-five"
        result["deploymentAuthorized"] = False
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("development", "holdout"))
    parser.add_argument("--root", type=Path, default=f1.F1_ROOT)
    args = parser.parse_args()
    try:
        result = _phase(args.root.resolve(), args.phase)
        output = args.root / f"analysis/{args.phase}.json"
        if output.exists():
            prior = _load(output)
            result["createdAtUtc"] = prior["createdAtUtc"]
        f1.immutable_json(output, result)
    except (f1.F1Error, OSError, KeyError, ValueError, TypeError) as exc:
        print(f"F1 analysis blocked: {exc}", file=sys.stderr)
        return 2
    print(json.dumps({"phase": args.phase, "matchups": result["matchups"], "screenAdvances": result.get("screenAdvances"), "superioritySupported": result.get("superioritySupported")}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
