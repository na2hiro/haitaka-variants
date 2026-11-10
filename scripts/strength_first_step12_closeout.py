#!/usr/bin/env python3
"""Seal and report the completed Step 12 development replication."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Any


EXPERIMENT_ID = "strength-first-step12-independent-replication-20260916-v1"
REPO_ROOT = Path(__file__).resolve().parents[1]
ANALYZER = REPO_ROOT / "scripts/strength_first_step12_analysis.py"
CELLS = ("early-selected", "u2688-control")
BLOCKS = 4


class CloseoutError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise CloseoutError(message)


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


def normalize(value: Any) -> Any:
    if isinstance(value, dict):
        return {key: normalize(item) for key, item in value.items() if key not in {"createdAt", "updatedAt"}}
    if isinstance(value, list):
        return [normalize(item) for item in value]
    return value


def verify_blocks(root: Path) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    for cell in CELLS:
        for block in range(BLOCKS):
            block_dir = root / "matches" / cell / f"block-{block:02d}"
            attempts = []
            for attempt in sorted(block_dir.glob("attempt-*")):
                validation = attempt / "validation.json"
                accepted = attempt / "accepted.json"
                if validation.is_file() and accepted.is_file():
                    value = load(validation)
                    if value.get("status") == "passed" and value.get("sealed") is True:
                        attempts.append((attempt, value))
            if len(attempts) != 1:
                fail(f"{cell} block {block} does not have exactly one sealed accepted validation")
            attempt, value = attempts[0]
            records.append({"cell": cell, "block": block, "attempt": attempt.name, "validation": {"path": str((attempt / "validation.json").relative_to(root)), "sha256": sha256(attempt / "validation.json")}, "accepted": {"path": str((attempt / "accepted.json").relative_to(root)), "sha256": sha256(attempt / "accepted.json")}, "rawGames": {"path": str((attempt / "raw/self-play-games.jsonl").relative_to(root)), "sha256": sha256(attempt / "raw/self-play-games.jsonl")}, "games": value.get("expectedGames"), "cappedGames": value.get("cappedGames", 0)})
    return records


def render_report(root: Path, analysis: dict[str, Any], blocks: list[dict[str, Any]]) -> str:
    primary = analysis["primaryContrast"]
    boot = analysis["bootstrap"]
    early = analysis["models"]["early-selected"]
    control = analysis["models"]["u2688-control"]
    routing = analysis["routing"]
    lines = [
        "# Step 12 independent replication closeout",
        "",
        f"- Experiment: `{EXPERIMENT_ID}`",
        "- Host: MBP `192.168.0.12`, one game thread, one game process at a time",
        "- Scope: development replication only; no confirmation or deployment",
        "- Production evaluator: handcrafted remains production",
        "",
        "## Sealed schedule",
        "",
        "Two cells (`early-selected` and `u2688-control`), four blocks per cell, 64 games per block, 512 total games, 100 ms/move, seed `2026091607`, pair transformation `anhoku-rotate-180-and-swap-colors-v1`.",
        "",
        "## Results",
        "",
        "| Cell | W/D/L | Mean paired score | Capped games |",
        "| --- | ---: | ---: | ---: |",
        f"| early-selected | {early['wdl']['aWins']}/{early['wdl']['draws']}/{early['wdl']['bWins']} | {early['meanPairScore']['exact']} | {early['cappedGames']} |",
        f"| u2688-control | {control['wdl']['aWins']}/{control['wdl']['draws']}/{control['wdl']['bWins']} | {control['meanPairScore']['exact']} | {control['cappedGames']} |",
        "",
        f"Primary paired contrast (early-selected minus u2688): **{primary['point']['exact']}** ({float(primary['point']['decimal']):.6f}).",
        f"Descriptive 95% paired-opening bootstrap interval: **{boot['lower95']:.6f} to {boot['upper95']:.6f}**, 10,000 replicates, SplitMix64 seed `2026091608`, linear type 7.",
        f"Primary classification: **{primary['classification']}**.",
        f"Early-selected capped sensitivity: **{analysis['cappedSensitivity']['boundsByCell']['early-selected']['lower']['exact']} to {analysis['cappedSensitivity']['boundsByCell']['early-selected']['upper']['exact']}**.",
        "",
        "## Routing",
        "",
        f"Recommendation: **{routing['recommendation']}**.",
        "The result is descriptive and conditional on this fresh corpus, initialization, opening suite, and runtime. It does not authorize training continuation, confirmation, deployment, commit, or push.",
        "",
        "## Evidence",
        "",
        "- [analysis](analysis/analysis.json)",
        "- [match status](matches/status.json)",
        "- [match resource ledger](matches/resource-ledger.json)",
        "- [qualification gate](preflight/qualification-gate.json)",
        f"- Sealed blocks: {len(blocks)}",
        "",
    ]
    return "\n".join(lines)


def closeout(root: Path) -> dict[str, Any]:
    root = root.expanduser().resolve()
    qualification = load(root / "preflight/qualification-gate.json")
    if qualification.get("status") != "passed":
        fail("qualification gate is not passed")
    blocks = verify_blocks(root)
    if len(blocks) != 8:
        fail("eight sealed blocks are required")
    status = load(root / "matches/status.json")
    if status.get("status") != "complete-sealed" or status.get("completedGames") != 512:
        fail("match status is not complete-sealed with 512 games")
    analysis_path = root / "analysis/analysis.json"
    previous_analysis = load(analysis_path) if analysis_path.is_file() else None
    command = [sys.executable, str(ANALYZER), "analyze", "--root", str(root), "--output", str(analysis_path)]
    completed = subprocess.run(command, cwd=REPO_ROOT, capture_output=True, text=True, check=False)
    (root / "analysis/analyzer.stdout").write_text(completed.stdout, encoding="utf-8")
    (root / "analysis/analyzer.stderr").write_text(completed.stderr, encoding="utf-8")
    if completed.returncode != 0:
        fail(f"analysis failed: {completed.stderr[-1000:]}")
    analysis = load(analysis_path)
    idempotent = previous_analysis is not None and normalize(previous_analysis) == normalize(analysis)
    report_path = root / "report.md"
    report_path.write_text(render_report(root, analysis, blocks), encoding="utf-8")
    artifacts = [{"path": "report.md", "sha256": sha256(report_path)}, {"path": "analysis/analysis.json", "sha256": sha256(analysis_path)}, {"path": "matches/status.json", "sha256": sha256(root / "matches/status.json")}, {"path": "matches/resource-ledger.json", "sha256": sha256(root / "matches/resource-ledger.json")}, {"path": "preflight/qualification-gate.json", "sha256": sha256(root / "preflight/qualification-gate.json")}]
    for relative in ("preflight/match-gate.json", "preflight/match-runtime-compatibility.json", "preflight/match-recovery-amendment.json", "preflight/match-runtime-regression.json", "preflight/production-archive-validation-20260919.json", "training/selection.json", "training/run/run.json"):
        path = root / relative
        if path.is_file():
            artifacts.append({"path": relative, "sha256": sha256(path)})
    index = {"schema": "haitaka-anhoku-strength-first-step12-evidence-index-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "status": "complete-sealed", "artifacts": artifacts, "sealedBlocks": blocks, "outcomesOpened": False, "gamesLaunched": 0, "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "handcraftedRemainsProduction": True}}
    atomic_json(root / "evidence-index.json", index)
    close = {"schema": "haitaka-anhoku-strength-first-step12-closeout-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "status": "complete-sealed", "idempotentSubstantiveAnalysis": idempotent, "analysis": {"path": "analysis/analysis.json", "sha256": sha256(analysis_path)}, "report": {"path": "report.md", "sha256": sha256(report_path)}, "evidenceIndex": {"path": "evidence-index.json", "sha256": sha256(root / "evidence-index.json")}, "sealedBlocks": 8, "games": 512, "matchesRerun": 0, "gamesLaunched": 0, "outcomesOpened": False, "deploymentAuthorized": False, "confirmationAuthorized": False, "handcraftedRemainsProduction": True}
    atomic_json(root / "closeout.json", close)
    return close


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("run",))
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    try:
        value = closeout(args.root)
        print(json.dumps({"status": value["status"], "idempotentSubstantiveAnalysis": value["idempotentSubstantiveAnalysis"], "gamesLaunched": 0}, sort_keys=True))
        return 0
    except (CloseoutError, OSError, KeyError, TypeError, ValueError, subprocess.SubprocessError) as exc:
        print(f"Step 12 closeout blocked: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
