#!/usr/bin/env python3
"""Phase 0 helper for plans/anhoku-nnue-small-net-plan.md (stdlib only).

scores  : teacher |score| and ply distribution of packed 72-byte training files
games   : game length and per-side NPS of self-play-games.jsonl match records

Usage:
  python3 scripts/small_net_phase0_stats.py scores <training.bin>...
  python3 scripts/small_net_phase0_stats.py games <self-play-games.jsonl>...
"""

import json
import statistics
import struct
import sys
from collections import Counter, defaultdict
from pathlib import Path

PLY_BANDS = [(0, 20), (20, 40), (40, 80), (80, 120), (120, 200)]


def quantiles(values):
    ordered = sorted(values)
    return {f"p{int(q * 100):02d}": ordered[min(len(ordered) - 1, int(len(ordered) * q))]
            for q in (0.05, 0.25, 0.5, 0.75, 0.95)}


def scores(paths):
    out = {}
    for path in paths:
        data = Path(path).read_bytes()
        assert len(data) % 72 == 0, path
        abs_scores, plies = [], []
        for offset in range(0, len(data), 72):
            score, _move, ply = struct.unpack_from("<hHH", data, offset + 64)
            abs_scores.append(abs(score))
            plies.append(ply)
        n = len(abs_scores)
        bands = []
        for lo, hi in PLY_BANDS:
            sub = [s for s, p in zip(abs_scores, plies) if lo <= p < hi]
            bands.append({
                "plyFrom": lo, "plyTo": hi, "records": len(sub),
                "share": len(sub) / n,
                "medianAbsScore": statistics.median(sub) if sub else None,
                "shareAbsScoreBelow300": sum(s < 300 for s in sub) / len(sub) if sub else None,
            })
        out[path] = {
            "records": n,
            "shareAbsScoreBelow300": sum(s < 300 for s in abs_scores) / n,
            "shareAbsScoreBelow1000": sum(s < 1000 for s in abs_scores) / n,
            "shareAbsScoreAtLeast3000": sum(s >= 3000 for s in abs_scores) / n,
            "plyBands": bands,
        }
    return out


def games(paths):
    played, final_ply, reasons = [], [], Counter()
    nodes, elapsed = defaultdict(int), defaultdict(float)
    for path in paths:
        report = json.loads(Path(path).with_name("self-play-report.json").read_text())
        kinds = {e["label"]: e["kind"] for e in report["engines"]}
        for line in Path(path).read_text().splitlines():
            game = json.loads(line)
            start_ply = int(game["startSfen"].split()[-1])
            played.append(game["plies"])
            final_ply.append(start_ply - 1 + game["plies"])
            reasons[game.get("terminationReason")] += 1
            for side in ("a", "b"):
                breakdown = game[side + "Breakdown"]
                kind = kinds[side.upper()]
                nodes[kind] += breakdown["totalNodes"]
                elapsed[kind] += breakdown["totalElapsedMs"]
    n = len(played)
    return {
        "files": len(paths),
        "games": n,
        "playedPlies": quantiles(played),
        "finalAbsolutePly": quantiles(final_ply),
        "shareReachingPly80": sum(p >= 80 for p in final_ply) / n,
        "shareReachingPly120": sum(p >= 120 for p in final_ply) / n,
        "terminationReasons": dict(reasons),
        "npsByEvaluator": {k: nodes[k] / elapsed[k] * 1000 for k in nodes},
    }


def main():
    if len(sys.argv) < 3 or sys.argv[1] not in ("scores", "games"):
        sys.exit(__doc__)
    fn = scores if sys.argv[1] == "scores" else games
    print(json.dumps(fn(sys.argv[2:]), indent=1))


if __name__ == "__main__":
    main()
