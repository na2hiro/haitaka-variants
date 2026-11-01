#!/usr/bin/env python3
"""Regression fixture for Step 12 per-game opening selection.

This launches only tiny handcrafted self-play fixtures.  It does not consume
the production models, alter the frozen schedule, or publish match outcomes.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import subprocess
import sys


EXPERIMENT_ID = "strength-first-step12-independent-replication-20260916-v1"
CELLS = ("early-selected", "u2688-control")
BLOCKS = 4
ROWS = 512
GAMES_PER_BLOCK = 64
SEED = 2026091607


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def artifact(path: Path, root: Path | None = None) -> dict[str, object]:
    path = path.resolve()
    display = str(path)
    if root is not None:
        try:
            display = str(path.relative_to(root.resolve()))
        except ValueError:
            pass
    return {"path": display, "bytes": path.stat().st_size, "sha256": sha256(path)}


def load(path: Path) -> object:
    return json.loads(path.read_text(encoding="utf-8"))


def run_case(binary: Path, repo: Path, work: Path, name: str, openings: list[str], extra: list[str], games: int) -> dict[str, object]:
    case = work / name
    case.mkdir(parents=True, exist_ok=True)
    opening_path = case / "openings.sfen"
    opening_path.write_text("\n".join(openings) + "\n", encoding="utf-8")
    report_dir = case / "report"
    command = [
        str(binary),
        "self-play",
        "--games",
        str(games),
        "--threads",
        "1",
        "--a-depth",
        "1",
        "--b-depth",
        "1",
        "--a-eval",
        "handcrafted",
        "--b-eval",
        "handcrafted",
        "--openings",
        str(opening_path),
        "--opening-order",
        "sequential",
        "--seed",
        str(SEED),
        "--max-plies",
        "1",
        "--report-dir",
        str(report_dir),
        *extra,
    ]
    completed = subprocess.run(command, cwd=repo, capture_output=True, text=True, check=False)
    (case / "stdout").write_text(completed.stdout, encoding="utf-8")
    (case / "stderr").write_text(completed.stderr, encoding="utf-8")
    return {
        "command": command,
        "returncode": completed.returncode,
        "stdout": artifact(case / "stdout"),
        "stderr": artifact(case / "stderr"),
        "openingFile": artifact(opening_path),
        "report": artifact(report_dir / "self-play-report.json") if (report_dir / "self-play-report.json").is_file() else None,
        "games": artifact(report_dir / "self-play-games.jsonl") if (report_dir / "self-play-games.jsonl").is_file() else None,
        "reportDir": str(report_dir),
    }


def check_report(case: dict[str, object], expected_unit: str, expected_starts: list[str], expected_seats: list[tuple[str, str]]) -> None:
    if case["returncode"] != 0:
        raise RuntimeError(f"regression case failed: {case['command']}: {case['stderr']}")
    report_dir = Path(str(case["reportDir"]))
    report = load(report_dir / "self-play-report.json")
    games = [json.loads(line) for line in (report_dir / "self-play-games.jsonl").read_text(encoding="utf-8").splitlines() if line.strip()]
    command = report["command"]
    if command.get("openingUnit") != expected_unit or command.get("openingOrder") != "sequential" or command.get("openingRandomPlies") != 0:
        raise RuntimeError(f"unexpected opening command metadata: {command}")
    if len(games) != len(expected_starts):
        raise RuntimeError(f"unexpected fixture game count: {len(games)}")
    for index, (game, start, seats) in enumerate(zip(games, expected_starts, expected_seats)):
        if game.get("gameIndex") != index + 1 or game.get("pairIndex") != index // 2:
            raise RuntimeError(f"fixture index mismatch at {index}: {game}")
        if game.get("startSfen") != start:
            raise RuntimeError(f"fixture SFEN mismatch at {index}: {game.get('startSfen')} != {start}")
        if (game.get("aColor"), game.get("bColor")) != seats:
            raise RuntimeError(f"fixture seats mismatch at {index}: {game}")


def rows_for(schedule: dict[str, object], cell: str, block: int) -> list[dict[str, object]]:
    rows = [row for row in schedule["rows"] if row.get("cell") == cell and row.get("block") == block]
    rows.sort(key=lambda row: int(row["gameIndexInCell"]))
    if len(rows) != GAMES_PER_BLOCK:
        raise RuntimeError(f"unexpected {cell} block {block} row count: {len(rows)}")
    return rows


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = args.root.expanduser().resolve()
    repo = root / "repo"
    binary = args.runtime.expanduser().resolve()
    schedule_path = root / "openings/schedule.json"
    schedule = load(schedule_path)
    if not isinstance(schedule, dict) or not isinstance(schedule.get("rows"), list) or len(schedule["rows"]) != ROWS:
        raise RuntimeError("frozen schedule does not contain 512 rows")
    if not binary.is_file():
        raise RuntimeError(f"runtime is missing: {binary}")

    mapping_checks = []
    for cell in CELLS:
        for block in range(BLOCKS):
            rows = rows_for(schedule, cell, block)
            mapping_checks.append({"cell": cell, "block": block, "rows": len(rows), "openingIndex": "gameIndexInBlock", "exactInputOrder": True})

    work = root / "preflight/match-runtime-regression"
    if work.exists():
        attempt = 2
        while (root / f"preflight/match-runtime-regression-{attempt:02d}").exists():
            attempt += 1
        work = root / f"preflight/match-runtime-regression-{attempt:02d}"
    work.mkdir(parents=True, exist_ok=True)
    first = rows_for(schedule, CELLS[0], 0)
    pair_case = run_case(binary, repo, work, "pair-mode", [first[0]["startSfen"]], [], 2)
    check_report(pair_case, "pair", [first[0]["startSfen"], first[0]["startSfen"]], [("black", "white"), ("white", "black")])
    game_rows = first[:4]
    game_case = run_case(binary, repo, work, "game-mode", [row["startSfen"] for row in game_rows], ["--opening-unit", "game"], 4)
    check_report(game_case, "game", [row["startSfen"] for row in game_rows], [("black", "white"), ("white", "black"), ("black", "white"), ("white", "black")])

    missing_case = run_case(binary, repo, work, "game-mode-missing-opening", [first[0]["startSfen"]], ["--opening-unit", "game"], 2)
    if missing_case["returncode"] == 0:
        raise RuntimeError("game mode accepted a missing opening")
    random_case = run_case(binary, repo, work, "game-mode-random-order", [first[0]["startSfen"], first[1]["startSfen"]], ["--opening-unit", "game", "--opening-order", "random"], 2)
    if random_case["returncode"] == 0:
        raise RuntimeError("game mode accepted random opening order")
    random_plies_case = run_case(binary, repo, work, "game-mode-random-plies", [first[0]["startSfen"], first[1]["startSfen"]], ["--opening-unit", "game", "--opening-random-plies", "1"], 2)
    if random_plies_case["returncode"] == 0:
        raise RuntimeError("game mode accepted random opening plies")

    receipt = {
        "schema": "haitaka-anhoku-strength-first-step12-match-runtime-regression-v1",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "passed",
        "runtime": artifact(binary, root),
        "schedule": artifact(schedule_path, root),
        "mapping": "openingIndex=gameIndex; pairIndex=floor(gameIndex/2); a/b seats alternate by game index",
        "allFrozenRows": {"rows": ROWS, "checks": mapping_checks},
        "tests": [
            {"name": "default-pair-mode-repeats-opening", "status": "passed", "evidence": pair_case},
            {"name": "game-mode-consumes-consecutive-sfens-and-preserves-seats", "status": "passed", "evidence": game_case},
            {"name": "game-mode-rejects-missing-opening", "status": "passed", "evidence": missing_case},
            {"name": "game-mode-rejects-random-order", "status": "passed", "evidence": random_case},
            {"name": "game-mode-rejects-random-plies", "status": "passed", "evidence": random_plies_case},
        ],
        "fixtureGamesLaunched": 6,
        "outcomesOpened": False,
        "gamesLaunched": 0,
        "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "handcraftedRemainsProduction": True},
    }
    receipt["script"] = artifact(Path(__file__), root)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": "passed", "tests": len(receipt["tests"]), "rows": ROWS}, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, KeyError, TypeError, ValueError, RuntimeError, subprocess.SubprocessError) as exc:
        print(f"Step 12 match runtime regression blocked: {exc}", file=sys.stderr)
        raise SystemExit(1)
