#!/usr/bin/env python3
"""Search-free F1 to retained Step 14 replay identity adapter.

Replay A is always T. The retained replay leg is a seat/color identity,
not F1's chronological leg. Original F1 IDs remain in the mapping receipt.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys
from typing import Any, Mapping

import strength_first_f1_core as f1


def _load(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text())
    f1.require(isinstance(value, dict), f"object required: {path}")
    return value


def adapt(row: Mapping[str, Any], raw: Mapping[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
    """Translate only replay identity and engine names; preserve play data."""
    for key in ("gameId", "phase", "matchup", "pairIndex", "leg", "sfen", "initialSideToMove", "blackEngine", "whiteEngine", "treatmentColor"):
        f1.require(raw.get(key) == row.get(key), f"raw F1 {key} differs")
    f1.require(raw.get("freshInstances") is True and raw.get("perInstanceNetworks") is True, "engine isolation evidence absent")
    f1.require(raw.get("plies") == len(raw.get("moves", [])) and isinstance(raw.get("moves"), list), "raw moves/plies differ")
    winner_color = raw.get("rawWinnerColor")
    f1.require(winner_color in (None, "black", "white"), "invalid raw winner color")
    result = "draw" if winner_color is None else "a-win" if winner_color == row["treatmentColor"] else "b-win"
    winner = None if winner_color is None else "A" if result == "a-win" else "B"
    replay_leg = 0 if row["treatmentColor"] == "black" else 1
    pair = row["pairIndex"]
    f1.require(type(pair) is int and pair >= 0 and row["matchup"] in {"T-C", "T-I"}, "invalid F1 replay pair/matchup")
    f1.require(row["blackEngine"] == ("T" if replay_leg == 0 else row["matchup"][-1]), "black engine differs")
    f1.require(row["whiteEngine"] == (row["matchup"][-1] if replay_leg == 0 else "T"), "white engine differs")
    replay = {
        "schema": "haitaka-anhoku-strength-first-step14-replay-input-v1",
        "globalGameIndex": 2 * pair + replay_leg,
        "globalPairIndex": pair,
        "block": pair // 8,
        "blockGameIndex": 2 * (pair % 8) + replay_leg,
        "leg": replay_leg,
        "openingId": row["openingId"],
        "startSfen": row["sfen"],
        "initialSideToMove": row["initialSideToMove"],
        "aColor": row["treatmentColor"],
        "bColor": "white" if row["treatmentColor"] == "black" else "black",
        "blackEngine": "A" if replay_leg == 0 else "B",
        "whiteEngine": "B" if replay_leg == 0 else "A",
        "moves": raw["moves"],
        "plies": raw["plies"],
        "rawEnd": raw["rawEnd"],
        "rawWinner": winner,
        "rawResult": result,
    }
    mapping = {"f1GameId": row["gameId"], "f1GameIndex": row["gameIndex"], "f1Leg": row["leg"], "matchup": row["matchup"], "replayGameIndex": replay["globalGameIndex"], "replayLeg": replay_leg, "pairIndex": pair}
    return replay, mapping


def run(schedule_path: Path, raw_path: Path, binary: Path, output: Path, *, max_plies: int, timeout_seconds: float = 900) -> dict[str, Any]:
    f1.require(max_plies in (2, 200), "unqualified replay cap")
    f1.require(0 < timeout_seconds <= 900, "replay host-wall timeout differs")
    schedule = _load(schedule_path)
    raw_report = _load(raw_path)
    expected = schedule.get("rows", [])
    raw_games = raw_report.get("rawGames")
    f1.require(isinstance(raw_games, list) and len(raw_games) == len(expected), "raw report count differs")
    by_id = {row["gameId"]: row for row in expected}
    f1.require(len(by_id) == len(expected), "duplicate scheduled F1 identity")
    raw_by_id = {game.get("gameId"): game for game in raw_games}
    f1.require(len(raw_by_id) == len(expected) and set(raw_by_id) == set(by_id), "raw F1 IDs differ")
    f1.require(binary.is_file() and not binary.is_symlink(), "retained replay executable missing")
    output.mkdir(parents=True, exist_ok=True)
    groups: dict[str, list[tuple[dict[str, Any], dict[str, Any]]]] = {}
    for row in expected:
        replay, mapping = adapt(row, raw_by_id[row["gameId"]])
        groups.setdefault(row["matchup"], []).append((replay, mapping))
    results = {}
    for matchup, records in groups.items():
        records.sort(key=lambda item: item[0]["globalGameIndex"])
        indices = [item[0]["globalGameIndex"] for item in records]
        f1.require(len(indices) == len(set(indices)), "duplicate replay game identity")
        stem = matchup.lower().replace("-", "-")
        input_path = output / f"{stem}-replay-input.jsonl"
        output_path = output / f"{stem}-replay-output.jsonl"
        map_path = output / f"{stem}-identity-map.json"
        input_bytes = b"".join(f1.canonical_bytes(item[0]) for item in records)
        if input_path.exists():
            f1.require(input_path.read_bytes() == input_bytes, "immutable replay input conflicts")
        else:
            input_path.write_bytes(input_bytes)
        f1.immutable_json(map_path, {"schema": "haitaka-anhoku-strength-first-f1-replay-map-v1", "matchup": matchup, "rawReport": f1.artifact(raw_path), "schedule": f1.artifact(schedule_path), "rows": [item[1] for item in records]})
        f1.require(not output_path.exists(), "replay output already exists; use an append-only attempt directory")
        completed = subprocess.run([str(binary), "--input", str(input_path), "--output", str(output_path), "--max-plies", str(max_plies)], capture_output=True, text=True, timeout=timeout_seconds, check=False)
        f1.require(completed.returncode == 0, f"retained replay process failed: {completed.stderr[-500:]}")
        replayed = [json.loads(line) for line in output_path.read_text().splitlines() if line]
        f1.require(len(replayed) == len(records) and all(item.get("status") == "passed" and item.get("searchDispatched") is False and item.get("modelLoaded") is False and item.get("browserLaunched") is False for item in replayed), "independent replay failed")
        by_index = {item["globalGameIndex"]: item for item in replayed}
        f1.require(len(by_index) == len(records) and set(by_index) == set(indices), "replay output identity differs")
        results[matchup] = {"input": f1.artifact(input_path), "output": f1.artifact(output_path), "mapping": f1.artifact(map_path), "count": len(records)}
    receipt = {"schema": "haitaka-anhoku-strength-first-f1-replay-batch-v1", "schedule": f1.artifact(schedule_path), "rawReport": f1.artifact(raw_path), "binary": f1.artifact(binary), "maxPlies": max_plies, "groups": results, "searchDispatched": False, "browserLaunched": False, "modelLoaded": False, "passed": True}
    f1.immutable_json(output / "replay.json", receipt)
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--schedule", type=Path, required=True)
    parser.add_argument("--raw", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--max-plies", type=int, choices=(2, 200), default=200)
    args = parser.parse_args()
    try:
        result = run(args.schedule.resolve(), args.raw.resolve(), args.binary.resolve(), args.output.resolve(), max_plies=args.max_plies)
    except (f1.F1Error, OSError, KeyError, TypeError, ValueError, subprocess.TimeoutExpired) as exc:
        print(f"F1 replay blocked: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
