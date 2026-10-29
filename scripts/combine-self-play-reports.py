#!/usr/bin/env python3
"""Combine disjoint paired self-play batches without double-counting games."""

import argparse
import json
import math
from pathlib import Path


def elo(score_rate: float) -> float:
    bounded = min(0.99, max(0.01, score_rate))
    return 400.0 * math.log10(bounded / (1.0 - bounded))


def paired_rating(bins: list[int]) -> tuple[float, list[float]]:
    pairs = sum(bins)
    if pairs == 0:
        raise SystemExit("no complete pairs")
    mean = sum(index / 4.0 * count for index, count in enumerate(bins)) / pairs
    bounded = min(0.999, max(0.001, mean))
    variance = (
        sum(
            (index / 4.0 - mean) ** 2 * count
            for index, count in enumerate(bins)
        )
        / (pairs - 1)
        if pairs > 1
        else 0.25
    )
    derivative = 400.0 / math.log(10.0) / (bounded * (1.0 - bounded))
    elo_value = elo(bounded)
    elo_se = derivative * math.sqrt(variance / pairs)
    return elo_value, [elo_value - 1.96 * elo_se, elo_value + 1.96 * elo_se]


def combined_breakdown(summaries: list[dict], key: str) -> dict:
    rows = [summary[key] for summary in summaries]
    total_nodes = sum(row["totalNodes"] for row in rows)
    total_elapsed_ms = sum(row["totalElapsedMs"] for row in rows)
    qnodes = sum(row["qnodes"] for row in rows)
    return {
        "totalNodes": total_nodes,
        "requestedBudgetNodes": sum(row["requestedBudgetNodes"] for row in rows),
        "consumedBudgetNodes": sum(row["consumedBudgetNodes"] for row in rows),
        "alphaBetaNodes": sum(row["alphaBetaNodes"] for row in rows),
        "completedDepth": max(row["completedDepth"] for row in rows),
        "incompleteIterations": sum(row["incompleteIterations"] for row in rows),
        "nodeBudgetCapHits": sum(row["nodeBudgetCapHits"] for row in rows),
        "fallbacks": sum(row["fallbacks"] for row in rows),
        "totalElapsedMs": total_elapsed_ms,
        "aggregateNps": total_nodes / (total_elapsed_ms / 1000.0),
        "qnodes": qnodes,
        "aggregateQnps": qnodes / (total_elapsed_ms / 1000.0),
        "qsearchMaxPly": max(row["qsearchMaxPly"] for row in rows),
        "qsearchCapHits": sum(row["qsearchCapHits"] for row in rows),
        "qsearchCheckMoveTries": sum(row["qsearchCheckMoveTries"] for row in rows),
        "qsearchDeltaPrunes": sum(row["qsearchDeltaPrunes"] for row in rows),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("reports", type=Path, nargs="+")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--comparison", required=True)
    args = parser.parse_args()

    reports = [json.loads(path.read_text()) for path in args.reports]
    first = reports[0]
    normalized_command = dict(first["command"])
    for field in ("games", "threads", "seed"):
        normalized_command.pop(field, None)
    for report in reports:
        if report["ruleset"] != first["ruleset"]:
            raise SystemExit("ruleset mismatch")
        if report["git"]["executableSha256"] != first["git"]["executableSha256"]:
            raise SystemExit("engine executable mismatch")
        if [engine.get("nnueSha256") for engine in report["engines"]] != [
            engine.get("nnueSha256") for engine in first["engines"]
        ]:
            raise SystemExit("model identity mismatch")
        command = dict(report["command"])
        for field in ("games", "threads", "seed"):
            command.pop(field, None)
        if command != normalized_command:
            raise SystemExit("command mismatch outside games, threads, or seed")
        if report["summary"]["games"] != report["summary"]["pairCount"] * 2:
            raise SystemExit("report contains an incomplete color-swapped pair")

    seeds = [report["command"]["seed"] for report in reports]
    if len(set(seeds)) != len(seeds):
        raise SystemExit("source reports must use distinct opening seeds")

    summaries = [report["summary"] for report in reports]
    games = sum(summary["games"] for summary in summaries)
    a_wins = sum(summary["aWins"] for summary in summaries)
    b_wins = sum(summary["bWins"] for summary in summaries)
    draws = sum(summary["draws"] for summary in summaries)
    a_score = a_wins + draws * 0.5
    rate = a_score / games
    ordinary_se = math.sqrt(rate * (1.0 - rate) / games)
    bins = [sum(summary["pairScoreBins"][index] for summary in summaries) for index in range(5)]
    paired_elo, paired_ci = paired_rating(bins)
    a_breakdown = combined_breakdown(summaries, "aBreakdown")
    b_breakdown = combined_breakdown(summaries, "bBreakdown")
    total_nodes = a_breakdown["totalNodes"] + b_breakdown["totalNodes"]
    total_elapsed_ms = a_breakdown["totalElapsedMs"] + b_breakdown["totalElapsedMs"]
    qnodes = a_breakdown["qnodes"] + b_breakdown["qnodes"]

    output = {
        "schema": "haitaka-combined-self-play-report",
        "schemaVersion": 1,
        "comparison": args.comparison,
        "ruleset": first["ruleset"],
        "git": first["git"],
        "engines": first["engines"],
        "sourceReports": [
            {
                "path": str(path),
                "games": report["summary"]["games"],
                "seed": report["command"]["seed"],
            }
            for path, report in zip(args.reports, reports)
        ],
        "summary": {
            "games": games,
            "aWins": a_wins,
            "bWins": b_wins,
            "draws": draws,
            "decidedGames": a_wins + b_wins,
            "aScore": a_score,
            "scoreRate": rate,
            "approxElo": elo(rate),
            "approxElo95Ci": [
                elo(max(0.01, rate - 1.96 * ordinary_se)),
                elo(min(0.99, rate + 1.96 * ordinary_se)),
            ],
            "pairCount": sum(bins),
            "pairScoreBins": bins,
            "pairedElo": paired_elo,
            "pairedElo95Ci": paired_ci,
            "avgPlies": sum(summary["avgPlies"] * summary["games"] for summary in summaries) / games,
            "totalNodes": total_nodes,
            "totalElapsedMs": total_elapsed_ms,
            "aggregateNps": total_nodes / (total_elapsed_ms / 1000.0),
            "qnodes": qnodes,
            "aggregateQnps": qnodes / (total_elapsed_ms / 1000.0),
            "aBreakdown": a_breakdown,
            "bBreakdown": b_breakdown,
            "warnings": sorted({warning for summary in summaries for warning in summary["warnings"]}),
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2) + "\n")


if __name__ == "__main__":
    main()
