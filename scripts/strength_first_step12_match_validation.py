#!/usr/bin/env python3
"""Validate one sealed Step 12 64-game match block.

The validator consumes only the frozen schedule and the raw self-play report
produced by ``haitaka_cli``.  It writes a separate validation receipt so raw
outcomes remain evidence and are never silently rewritten by analysis.
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
from typing import Any


EXPERIMENT_ID = "strength-first-step12-independent-replication-20260916-v1"
CELLS = ("early-selected", "u2688-control")
BLOCKS = 4
GAMES_PER_BLOCK = 64
PAIRS_PER_BLOCK = 32
SEED = 2026091607
MAX_PLIES = 200
CAP_TERMINATIONS = {"maximum-ply-capped", "maximum-plies"}
DRAW_TERMINATIONS = {"repetition-draw", "jishogi-draw"}
TERMINATIONS = {"king-captured", "no-legal-move", "perpetual-check-loss"} | DRAW_TERMINATIONS | CAP_TERMINATIONS
RESULT_SCORES = {"a-win": 1, "b-win": 0, "draw": 0.5, "capped": 0.5}
RESULTS = set(RESULT_SCORES)


class ValidationError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise ValidationError(message)


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


def finite(value: Any, where: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(float(value)):
        fail(f"{where} is not finite")
    return float(value)


def schedule_rows(root: Path, cell: str, block: int) -> list[dict[str, Any]]:
    if cell not in CELLS or block not in range(BLOCKS):
        fail(f"invalid cell/block: {cell}/{block}")
    schedule = load(root / "openings/schedule.json")
    rows = schedule.get("rows")
    if not isinstance(rows, list):
        fail("opening schedule has no rows")
    selected = [row for row in rows if row.get("cell") == cell and row.get("block") == block]
    selected.sort(key=lambda row: int(row.get("gameIndexInCell", -1)))
    if len(selected) != GAMES_PER_BLOCK:
        fail(f"schedule has {len(selected)} rows for {cell} block {block}")
    if [int(row.get("gameIndexInCell", -1)) for row in selected] != list(range(block * GAMES_PER_BLOCK, (block + 1) * GAMES_PER_BLOCK)):
        fail("schedule gameIndexInCell is not contiguous")
    return selected


def expected_network(root: Path, cell: str) -> tuple[int, Path]:
    selection = load(root / "training/selection.json")
    selected = int(selection["selectedUpdate"])
    update = selected if cell == "early-selected" else 2688
    network = root / "training/run/exports" / f"u{update:06d}.nnue"
    if not network.is_file():
        network = root / "exports" / f"u{update:06d}.nnue"
    if not network.is_file():
        fail(f"missing bound network: {network}")
    return update, network


def validate(root: Path, cell: str, block: int, attempt_dir: Path) -> dict[str, Any]:
    root = root.expanduser().resolve()
    rows = schedule_rows(root, cell, block)
    update, network = expected_network(root, cell)
    raw = attempt_dir / "raw"
    report_path = raw / "self-play-report.json"
    games_path = raw / "self-play-games.jsonl"
    report = load(report_path)
    if report.get("schema") != "haitaka-self-play-report" or report.get("schemaVersion") != 3:
        fail("self-play report schema/version mismatch")
    if report.get("ruleset") != "anhoku":
        fail("self-play report ruleset mismatch")
    command = report.get("command", {})
    expected_command = {
        "games": GAMES_PER_BLOCK,
        "threads": 1,
        "movetimeMs": 100,
        "openingOrder": "sequential",
        "openingUnit": "game",
        "openingRandomPlies": 0,
        "seed": SEED,
        "maxPlies": MAX_PLIES,
    }
    for key, expected in expected_command.items():
        if command.get(key) != expected:
            fail(f"report.command.{key}: expected {expected!r}, got {command.get(key)!r}")
    engines = report.get("engines")
    if not isinstance(engines, list) or len(engines) != 2:
        fail("report must contain exactly two engines")
    if engines[0].get("label") != "A" or engines[0].get("kind") != "nnue":
        fail("engine A is not the NNUE model role")
    if engines[0].get("nnueSha256") != sha256(network):
        fail("engine A network hash does not match the selected model")
    if engines[1].get("label") != "B" or engines[1].get("kind") != "handcrafted":
        fail("engine B is not the handcrafted role")
    try:
        games = [json.loads(line) for line in games_path.read_text(encoding="utf-8").splitlines() if line.strip()]
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        fail(f"cannot load game JSONL: {exc}")
    if len(games) != GAMES_PER_BLOCK:
        fail(f"expected {GAMES_PER_BLOCK} games, got {len(games)}")
    seen: set[int] = set()
    wdl = Counter()
    capped = 0
    pair_scores: list[dict[str, Any]] = []
    for index, (row, game) in enumerate(zip(rows, games)):
        game_index = game.get("gameIndex")
        if not isinstance(game_index, int) or game_index != index + 1 or game_index in seen:
            fail(f"game index mismatch at line {index + 1}")
        seen.add(game_index)
        if game.get("pairIndex") != row.get("pairIndex") - block * PAIRS_PER_BLOCK:
            fail(f"pair index mismatch at game {game_index}")
        if game.get("aColor") != row.get("aColor") or game.get("bColor") != row.get("bColor"):
            fail(f"seat assignment mismatch at game {game_index}")
        if game.get("startSfen") != row.get("startSfen"):
            fail(f"start SFEN mismatch at game {game_index}")
        if game.get("result") not in RESULTS:
            fail(f"invalid result at game {game_index}")
        termination = game.get("terminationReason")
        if termination not in TERMINATIONS:
            fail(f"invalid termination reason at game {game_index}: {termination!r}")
        if game.get("failureState") is not None:
            fail(f"failureState is non-null at game {game_index}")
        if game.get("fallbacks") != 0:
            fail(f"fallbacks is nonzero at game {game_index}")
        moves = game.get("moves")
        if not isinstance(moves, list) or not moves:
            fail(f"missing moves at game {game_index}")
        plies = game.get("plies")
        if not isinstance(plies, int) or plies < 1 or plies > MAX_PLIES or plies != len(moves):
            fail(f"plies mismatch or cap overrun at game {game_index}")
        if termination in CAP_TERMINATIONS:
            expected_result = "capped" if termination == "maximum-ply-capped" else "draw"
            if plies != MAX_PLIES or game["result"] != expected_result:
                fail(f"capped result or ply count mismatch at game {game_index}")
            capped += 1
        elif termination in DRAW_TERMINATIONS:
            if game["result"] != "draw":
                fail(f"draw termination has a decisive result at game {game_index}")
        elif game["result"] not in {"a-win", "b-win"}:
            fail(f"decisive termination has no winner at game {game_index}")
        wdl[game["result"]] += 1
    if seen != set(range(1, GAMES_PER_BLOCK + 1)):
        fail("game indexes are incomplete")
    for pair in range(PAIRS_PER_BLOCK):
        pair_games = games[pair * 2:pair * 2 + 2]
        if len(pair_games) != 2 or pair_games[0].get("pairIndex") != pair or pair_games[1].get("pairIndex") != pair:
            fail(f"pair {pair} is incomplete")
        values = [{"result": item["result"], "score": RESULT_SCORES[item["result"]], "capped": item.get("terminationReason") in CAP_TERMINATIONS} for item in pair_games]
        pair_scores.append({"pairIndex": block * PAIRS_PER_BLOCK + pair, "games": values, "score": sum(item["score"] for item in values) / 2})
    report_summary = report.get("summary", {})
    if report_summary.get("games") != GAMES_PER_BLOCK or report_summary.get("fallbacks") != 0:
        fail("report summary does not bind to the validated raw games")
    if report_summary.get("aWins") != wdl["a-win"] or report_summary.get("bWins") != wdl["b-win"] or report_summary.get("draws") != wdl["draw"] + wdl["capped"]:
        fail("report WDL summary differs from raw games")
    result = {
        "schema": "haitaka-anhoku-strength-first-step12-block-validation-v1",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "passed",
        "sealed": True,
        "cell": cell,
        "block": block,
        "attempt": attempt_dir.name,
        "modelUpdate": update,
        "expectedGames": GAMES_PER_BLOCK,
        "expectedPairs": PAIRS_PER_BLOCK,
        "schedule": {"seed": SEED, "pairTransformation": "anhoku-rotate-180-and-swap-colors-v1", "rowsSha256": hashlib.sha256(json.dumps(rows, sort_keys=True, separators=(",", ":")).encode()).hexdigest()},
        "network": {"path": str(network.relative_to(root)), "bytes": network.stat().st_size, "sha256": sha256(network)},
        "report": {"path": str(report_path.relative_to(root)), "bytes": report_path.stat().st_size, "sha256": sha256(report_path)},
        "games": {"path": str(games_path.relative_to(root)), "bytes": games_path.stat().st_size, "sha256": sha256(games_path)},
        "wdl": {"aWins": wdl["a-win"], "draws": wdl["draw"] + wdl["capped"], "bWins": wdl["b-win"]},
        "cappedGames": capped,
        "pairScores": pair_scores,
        "strictChecks": {"orderedGames": True, "seatSwaps": True, "scheduleIdentity": True, "legalResults": True, "failureStateNull": True, "zeroFallbacks": True, "movesPresent": True, "maxPliesRespected": True, "summaryRecomputed": True},
        "gamesLaunched": GAMES_PER_BLOCK,
        "outcomesOpened": False,
        "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "handcraftedRemainsProduction": True},
    }
    atomic_json(attempt_dir / "validation.json", result)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("validate", choices=("validate",))
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--cell", choices=CELLS, required=True)
    parser.add_argument("--block", type=int, required=True)
    parser.add_argument("--attempt", type=Path, required=True)
    args = parser.parse_args()
    try:
        value = validate(args.root, args.cell, args.block, args.attempt)
        print(json.dumps({"status": value["status"], "cell": args.cell, "block": args.block, "attempt": args.attempt.name}, sort_keys=True))
        return 0
    except (ValidationError, OSError, KeyError, TypeError, ValueError) as exc:
        print(f"Step 12 match validation blocked: {exc}", flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
