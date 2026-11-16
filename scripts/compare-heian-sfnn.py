#!/usr/bin/env python3
"""Heian 9x9: reproduce the existing SFNN-vs-handcrafted paired USI protocol.

Uses the same sfnn_usi/CLI endpoints as docs/tuyouraou11-rule-comparison.md.
Artifacts include complete games, commands, executable/model hashes and paired CIs.
"""
import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path
import random
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
HASHES = {
    "micro": "7f9545dbf73b87acf312146daabf59c4f17d0845c87d587fa19caed51707dddb",
    "mini": "45323c65135da65d6c2c364eea93a5af6a285c27ebfb07eab4afbe737ef74508",
    "progress": "a2d18f0564639c945b5d6469e9b437cd64a526ad4fbcef02ab1d8f547d08e5f1",
}


def save(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2, allow_nan=False) + "\n")


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def execute(command, log):
    print(datetime.now(timezone.utc).isoformat(), "RUN", " ".join(map(str, command)), flush=True)
    started = time.monotonic()
    with log.open("w") as stream:
        subprocess.run(list(map(str, command)), cwd=ROOT, stdout=stream,
                       stderr=subprocess.STDOUT, check=True)
    print("DONE", log.name, round(time.monotonic() - started, 2), "seconds", flush=True)


def percentile(values, p):
    values = sorted(values)
    x = (len(values) - 1) * p
    i = int(x)
    return values[i] + (values[min(i + 1, len(values) - 1)] - values[i]) * (x - i)


def summarize(directory, conditions):
    rows = []
    for profile in ("micro", "mini"):
        d = directory / profile / "main"
        report = json.loads((d / "self-play-report.json").read_text())
        games = [json.loads(line) for line in (d / "self-play-games.jsonl").read_text().splitlines()]
        assert len(games) == conditions["games"]
        assert {g["gameIndex"] for g in games} == set(range(1, len(games) + 1))
        assert report["ruleset"] == "heian-9x9"
        assert report["git"]["commit"] == conditions["sourceCommit"]
        assert report["git"]["executableSha256"] == conditions["cliSha256"]
        assert report["engines"][0]["command"] == report["engines"][1]["command"]
        pairs = defaultdict(list)
        for game in games:
            assert game["failureState"] is None
            assert game["winner"] in ("A", "B", None)
            assert len(game["moves"]) == game["plies"] <= conditions["maxPlies"]
            assert game["pairIndex"] == (game["gameIndex"] - 1) // 2
            assert game["aColor"] == ("black" if game["gameIndex"] % 2 else "white")
            pairs[game["pairIndex"]].append(game)
        scores = []
        for pair in pairs.values():
            assert len(pair) == 2 and pair[0]["startSfen"] == pair[1]["startSfen"]
            scores.append(sum(1 if g["winner"] == "A" else 0 if g["winner"] == "B" else .5 for g in pair) / 2)
        counts = Counter(g["winner"] for g in games)
        score = (counts["A"] + .5 * counts[None]) / len(games)
        summary = report["summary"]
        assert (counts["A"], counts["B"], counts[None]) == (summary["aWins"], summary["bWins"], summary["draws"])
        assert math.isclose(score, summary["scoreRate"])
        rng = random.Random(conditions["seed"])
        samples = [sum(rng.choices(scores, k=len(scores))) / len(scores) for _ in range(10000)]
        rows.append({"profile": profile, "games": len(games), "sfnnWins": counts["A"],
                     "handcraftedWins": counts["B"], "draws": counts[None], "scoreRate": score,
                     "scoreCi95": [percentile(samples, p) for p in (.025, .975)],
                     "terminationReasons": dict(Counter(g["terminationReason"] for g in games)),
                     "fallbacks": {"sfnn": summary["aBreakdown"]["fallbacks"],
                                   "handcrafted": summary["bBreakdown"]["fallbacks"]},
                     "avgPlies": summary["avgPlies"],
                     "byColor": {color: dict(Counter(g["winner"] or "draw" for g in games if g["aColor"] == color)) for color in ("black", "white")},
                     "gamesSha256": sha(d / "self-play-games.jsonl"),
                     "reportSha256": sha(d / "self-play-report.json")})
    result = {"conditions": conditions,
              "interval": "10,000 percentile bootstrap resamples of color-swapped opening pairs; seed as above",
              "limitations": "Native CPU, transferred standard-shogi models, fixed time and ply cap; exploratory comparison of two profiles without multiplicity adjustment.",
              "rows": rows}
    save(directory / "summary.json", result)
    print(json.dumps(result, ensure_ascii=False, indent=2), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=["prepare", "run", "summarize", "all"])
    parser.add_argument("--micro-model", type=Path, required=True)
    parser.add_argument("--mini-model", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=ROOT / "target/heian-sfnn")
    parser.add_argument("--games", type=int, default=100)
    parser.add_argument("--movetime-ms", type=int, default=100)
    parser.add_argument("--max-plies", type=int, default=512)
    parser.add_argument("--threads", type=int, default=4)
    args = parser.parse_args()
    assert args.games > 0 and args.games % 2 == 0
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    models = {p: getattr(args, p + "_model").resolve() for p in ("micro", "mini")}
    for profile, directory in models.items():
        assert sha(directory / "nn.bin") == HASHES[profile]
        assert sha(directory / "progress.bin") == HASHES["progress"]
    if args.phase in ("prepare", "all"):
        assert not subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT).strip(), "commit the exact tested source before building"
        assert not (out / "conditions.json").exists(), "use a fresh output directory"
        execute(["cargo", "build", "--offline", "--release", "-p", "haitaka_cli", "-p", "haitaka_wasm",
                 "--bin", "haitaka_cli", "--example", "sfnn_usi", "--features", "heian-9x9"], out / "build.log")
        (out / "bin").mkdir(exist_ok=True)
        for src, dst in [("target/release/haitaka_cli", "cli"), ("target/release/examples/sfnn_usi", "engine")]:
            shutil.copy2(ROOT / src, out / "bin" / dst)
        conditions = {"sourceCommit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                      "startedUtc": datetime.now(timezone.utc).isoformat(), "games": args.games,
                      "movetimeMs": args.movetime_ms, "maxPlies": args.max_plies, "threads": args.threads,
                      "seed": 330011, "openingRandomPlies": 8, "runtime": "native CPU", "maxDepth": 64,
                      "modelSha256": HASHES, "cliSha256": sha(out / "bin/cli"), "engineSha256": sha(out / "bin/engine"),
                      "rustc": subprocess.check_output(["rustc", "-V"], text=True).strip(),
                      "platform": subprocess.check_output(["uname", "-a"], text=True).strip()}
        save(out / "conditions.json", conditions)
    conditions = json.loads((out / "conditions.json").read_text())
    assert sha(out / "bin/cli") == conditions["cliSha256"]
    assert sha(out / "bin/engine") == conditions["engineSha256"]
    if args.phase in ("run", "all"):
        for label, games, ms, cap in [("smoke", 2, 20, 64), ("main", conditions["games"], conditions["movetimeMs"], conditions["maxPlies"])]:
            for profile in ("micro", "mini"):
                directory = out / profile
                directory.mkdir(exist_ok=True)
                assert not (directory / label).exists(), "completed or partial run exists; preserve it and use a fresh directory"
                command = [out / "bin/cli", "self-play", "--games", games, "--threads", conditions["threads"],
                           "--movetime-ms", ms, "--max-plies", cap, "--opening-random-plies", conditions["openingRandomPlies"],
                           "--seed", conditions["seed"], "--a-engine", out / "bin/engine",
                           "--a-engine-arg=--model-dir", "--a-engine-arg", models[profile]]
                if profile == "mini": command.append("--a-engine-arg=--mini")
                command += ["--b-engine", out / "bin/engine", "--report-dir", directory / label]
                save(directory / (label + "-command.json"), list(map(str, command)))
                execute(command, directory / (label + ".log"))
                report = json.loads((directory / label / "self-play-report.json").read_text())
                assert report["summary"]["games"] == games
    if args.phase in ("summarize", "all", "run"):
        summarize(out, conditions)


if __name__ == "__main__":
    main()
