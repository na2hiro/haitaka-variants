#!/usr/bin/env python3
"""Validate the superseding 100 ms Step 9 confirmation without interim looks."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path
import statistics
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "haitaka-anhoku-strength-first-independent-confirmation-100ms-result-v1"
GAME_SCHEMA = "haitaka-self-play-game"
GAME_SCHEMA_VERSION = 3
ONE_SIDED_T_95_DF_2047 = 1.6455983573763029


class ConfirmationError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise ConfirmationError(message)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def artifact(path: Path) -> dict[str, Any]:
    resolved = path.resolve()
    try:
        display = str(resolved.relative_to(ROOT))
    except ValueError:
        display = str(resolved)
    return {"path": display, "bytes": resolved.stat().st_size, "sha256": sha256(resolved)}


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot load {path}: {exc}")
    if not isinstance(value, dict):
        fail(f"{path} must contain one JSON object")
    return value


def exact(actual: Any, expected: Any, where: str) -> None:
    if actual != expected:
        fail(f"{where} differs: expected {expected!r}, got {actual!r}")


def same_float(actual: Any, expected: float, where: str) -> None:
    if not isinstance(actual, (int, float)) or not math.isfinite(float(actual)):
        fail(f"{where} is not finite")
    if not math.isclose(float(actual), expected, rel_tol=1e-12, abs_tol=1e-12):
        fail(f"{where} differs: expected {expected!r}, got {actual!r}")


def load_suite(path: Path, expected_sha256: str) -> list[tuple[str, str]]:
    exact(sha256(path), expected_sha256, f"suite SHA-256 {path}")
    rows: list[tuple[str, str]] = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 2 or not fields[0] or not fields[1]:
            fail(f"invalid suite row {path}:{number}")
        rows.append((fields[0], fields[1]))
    exact(len(rows), 2048, f"suite rows {path}")
    if len({opening_id for opening_id, _ in rows}) != len(rows):
        fail(f"suite {path} contains duplicate opening IDs")
    return rows


def validate_model(
    *,
    role: str,
    report_path: Path,
    games_path: Path,
    suite_path: Path,
    suite_sha256: str,
    network_path: Path,
    network_sha256: str,
    expected_seed: int,
) -> dict[str, Any]:
    suite = load_suite(suite_path, suite_sha256)
    exact(sha256(network_path), network_sha256, f"{role} network SHA-256")
    report = load_json(report_path)
    command = report.get("command", {})
    expected_command = {
        "games": 4096,
        "threads": 1,
        "aDepth": None,
        "bDepth": None,
        "movetimeMs": 100,
        "nodesPerMove": None,
        "sfen": None,
        "openingOrder": "sequential",
        "openingRandomPlies": 0,
        "seed": expected_seed,
        "maxPlies": 200,
    }
    for key, value in expected_command.items():
        exact(command.get(key), value, f"{role} command.{key}")
    engines = report.get("engines")
    if not isinstance(engines, list) or len(engines) != 2:
        fail(f"{role} report must bind exactly two engines")
    exact(engines[0].get("label"), "A", f"{role} candidate label")
    exact(engines[0].get("kind"), "nnue", f"{role} candidate kind")
    exact(engines[0].get("nnueSha256"), network_sha256, f"{role} report network SHA-256")
    exact(engines[1].get("label"), "B", f"{role} opponent label")
    exact(engines[1].get("kind"), "handcrafted", f"{role} opponent kind")

    try:
        game_lines = [line for line in games_path.read_text(encoding="utf-8").splitlines() if line]
        games = [json.loads(line) for line in game_lines]
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot load {role} games: {exc}")
    exact(len(games), 4096, f"{role} game count")

    scores: list[float] = []
    wins = losses = draws = maximum_ply_games = 0
    termination_counts: dict[str, int] = {}
    for index, game in enumerate(games):
        pair = index // 2
        seat = index % 2
        opening_id, opening_sfen = suite[pair]
        where = f"{role} game {index + 1}"
        exact(game.get("schema"), GAME_SCHEMA, f"{where}.schema")
        exact(game.get("schemaVersion"), GAME_SCHEMA_VERSION, f"{where}.schemaVersion")
        exact(game.get("gameIndex"), index + 1, f"{where}.gameIndex")
        exact(game.get("pairIndex"), pair, f"{where}.pairIndex")
        exact(game.get("aColor"), ("black", "white")[seat], f"{where}.aColor")
        exact(game.get("bColor"), ("white", "black")[seat], f"{where}.bColor")
        opening = game.get("opening", {})
        exact(opening.get("source"), "suite", f"{where}.opening.source")
        exact(opening.get("suiteIndex"), pair, f"{where}.opening.suiteIndex")
        exact(opening.get("baseSfen"), opening_sfen, f"{where}.opening.baseSfen")
        exact(opening.get("randomPlies"), 0, f"{where}.opening.randomPlies")
        exact(game.get("startSfen"), opening_sfen, f"{where}.startSfen")
        exact(game.get("failureState"), None, f"{where}.failureState")
        exact(game.get("fallbacks"), 0, f"{where}.fallbacks")
        exact(game.get("aBreakdown", {}).get("fallbacks"), 0, f"{where}.aBreakdown.fallbacks")
        exact(game.get("bBreakdown", {}).get("fallbacks"), 0, f"{where}.bBreakdown.fallbacks")
        moves = game.get("moves")
        if not isinstance(moves, list) or any(not isinstance(move, str) or not move for move in moves):
            fail(f"{where}.moves is invalid")
        exact(game.get("plies"), len(moves), f"{where}.plies")
        termination = game.get("terminationReason")
        if not isinstance(termination, str) or not termination:
            fail(f"{where}.terminationReason is missing")
        termination_counts[termination] = termination_counts.get(termination, 0) + 1
        if termination == "maximum-plies":
            maximum_ply_games += 1
        result = game.get("result")
        score = {"a-win": 1.0, "b-win": 0.0, "draw": 0.5}.get(result)
        if score is None:
            fail(f"{where}.result is invalid: {result!r}")
        scores.append(score)
        if score == 1.0:
            wins += 1
        elif score == 0.0:
            losses += 1
        else:
            draws += 1

    pair_scores = [(scores[2 * pair] + scores[2 * pair + 1]) / 2.0 for pair in range(2048)]
    mean = statistics.fmean(pair_scores)
    standard_deviation = statistics.stdev(pair_scores)
    standard_error = standard_deviation / math.sqrt(len(pair_scores))
    lower = mean - ONE_SIDED_T_95_DF_2047 * standard_error
    summary = report.get("summary", {})
    exact(summary.get("games"), 4096, f"{role} summary.games")
    exact(summary.get("aWins"), wins, f"{role} summary.aWins")
    exact(summary.get("bWins"), losses, f"{role} summary.bWins")
    exact(summary.get("draws"), draws, f"{role} summary.draws")
    exact(summary.get("pairCount"), 2048, f"{role} summary.pairCount")
    exact(summary.get("fallbacks"), 0, f"{role} summary.fallbacks")
    same_float(summary.get("aScore"), sum(scores), f"{role} summary.aScore")
    same_float(summary.get("scoreRate"), statistics.fmean(scores), f"{role} summary.scoreRate")
    return {
        "role": role,
        "network": artifact(network_path),
        "openingSuite": artifact(suite_path),
        "nativeReport": artifact(report_path),
        "nativeGames": artifact(games_path),
        "games": 4096,
        "pairs": 2048,
        "wins": wins,
        "losses": losses,
        "draws": draws,
        "score": sum(scores),
        "meanPairScore": mean,
        "sampleStandardDeviation": standard_deviation,
        "standardError": standard_error,
        "criticalValue": ONE_SIDED_T_95_DF_2047,
        "oneSidedLower95": lower,
        "passesLowerBoundAboveHalf": lower > 0.5,
        "maximumPlyGames": maximum_ply_games,
        "terminationCounts": dict(sorted(termination_counts.items())),
        "zeroFallbacks": True,
        "completeSchedule": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract", type=Path, required=True)
    parser.add_argument("--execution", type=Path, required=True)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--model-a-network", type=Path, required=True)
    parser.add_argument("--model-a-suite", type=Path, required=True)
    parser.add_argument("--model-a-report", type=Path, required=True)
    parser.add_argument("--model-a-games", type=Path, required=True)
    parser.add_argument("--model-b-network", type=Path, required=True)
    parser.add_argument("--model-b-suite", type=Path, required=True)
    parser.add_argument("--model-b-report", type=Path, required=True)
    parser.add_argument("--model-b-games", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    # Check every sealed output exists before opening any one of them.
    sealed = (args.model_a_report, args.model_a_games, args.model_b_report, args.model_b_games)
    missing = [str(path) for path in sealed if not path.is_file()]
    if missing:
        fail("both complete schedules are required before results may be opened; missing: " + ", ".join(missing))

    contract = load_json(args.contract)
    exact(contract.get("schema"), "haitaka-anhoku-strength-first-independent-confirmation-100ms-contract-v1", "contract.schema")
    exact(contract.get("status"), "preregistered", "contract.status")
    exact(contract.get("deploymentAuthorized"), False, "contract.deploymentAuthorized")
    protocol = contract.get("matchProtocol", {})
    exact(protocol.get("concurrentGames"), 1, "contract.matchProtocol.concurrentGames")
    exact(protocol.get("moveTimeMilliseconds"), 100, "contract.matchProtocol.moveTimeMilliseconds")
    exact(protocol.get("gamesPerModel"), 4096, "contract.matchProtocol.gamesPerModel")
    exact(protocol.get("gamesPerOpeningSourceGroup"), 2, "contract.matchProtocol.gamesPerOpeningSourceGroup")
    decision = contract.get("statisticalDecision", {})
    exact(decision.get("pairCountPerModel"), 2048, "contract.statisticalDecision.pairCountPerModel")
    exact(decision.get("alphaOneSided"), 0.05, "contract.statisticalDecision.alphaOneSided")
    exact(decision.get("perModelPass"), "lower-bound-strictly-greater-than-0.5", "contract.statisticalDecision.perModelPass")
    exact(decision.get("overallPass"), "both-model-A-and-model-B-pass", "contract.statisticalDecision.overallPass")
    execution = load_json(args.execution)
    exact(execution.get("schema"), "haitaka-anhoku-strength-first-independent-confirmation-100ms-execution-v1", "execution.schema")
    exact(execution.get("experimentId"), contract["experimentId"], "execution.experimentId")
    exact(execution.get("confirmationResultsOpened"), False, "execution.confirmationResultsOpened")
    exact(execution.get("contract", {}).get("sha256"), sha256(args.contract), "execution.contract.sha256")
    exact(execution.get("runtime", {}).get("sha256"), sha256(args.runtime), "execution.runtime.sha256")
    exact(execution.get("analyzer", {}).get("sha256"), sha256(Path(__file__)), "execution.analyzer.sha256")
    exact(execution.get("modelA", {}).get("integerNetworkSha256"), contract["modelA"]["integerNetworkSha256"], "execution.modelA.integerNetworkSha256")
    model_b_network_sha256 = execution.get("modelB", {}).get("integerNetworkSha256")
    if not isinstance(model_b_network_sha256, str) or len(model_b_network_sha256) != 64:
        fail("execution.modelB.integerNetworkSha256 is invalid")

    model_a = validate_model(
        role="model-a-suite-a", report_path=args.model_a_report, games_path=args.model_a_games,
        suite_path=args.model_a_suite, suite_sha256=contract["suites"]["A"]["sha256"],
        network_path=args.model_a_network, network_sha256=contract["modelA"]["integerNetworkSha256"],
        expected_seed=contract["suites"]["A"]["sourceGameSeed"],
    )
    model_b = validate_model(
        role="model-b-suite-b", report_path=args.model_b_report, games_path=args.model_b_games,
        suite_path=args.model_b_suite, suite_sha256=contract["suites"]["B"]["sha256"],
        network_path=args.model_b_network, network_sha256=model_b_network_sha256,
        expected_seed=contract["suites"]["B"]["sourceGameSeed"],
    )
    passed = model_a["passesLowerBoundAboveHalf"] and model_b["passesLowerBoundAboveHalf"]
    result = {
        "schema": SCHEMA,
        "createdAt": datetime.now(timezone.utc).isoformat(),
        "experimentId": contract["experimentId"],
        "contract": artifact(args.contract),
        "execution": artifact(args.execution),
        "runtime": artifact(args.runtime),
        "models": {"A": model_a, "B": model_b},
        "statisticalDecision": {
            "analysisUnit": "opening-source color-swapped pair",
            "interval": decision["interval"],
            "alphaOneSided": 0.05,
            "multiplicity": decision["multiplicity"],
            "bothModelsPass": passed,
        },
        "confirmationPassed": passed,
        "deploymentAuthorized": False,
        "decision": "confirmation-passed-deployment-still-not-authorized" if passed else "confirmation-failed-handcrafted-remains-production",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"result": artifact(args.output), "confirmationPassed": passed, "decision": result["decision"]}, indent=2))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ConfirmationError as exc:
        print(f"Step 9 confirmation FAILED: {exc}", file=__import__("sys").stderr)
        raise SystemExit(1)
