#!/usr/bin/env python3
"""Run and analyze the corrected-runtime u1920/u2048 checkpoint rescreen."""

from __future__ import annotations

import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import random
import statistics
import subprocess
import time


ROOT = Path(__file__).resolve().parents[1]
RUN = ROOT / "out/anhoku-strength-first/step7-corrected-checkpoint-rescreen-20260910-v1"
CONTRACT = ROOT / "r0/anhoku-strength-first/step7-corrected-checkpoint-rescreen-v1.json"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def workspace_path(value: str) -> Path:
    path = Path(value)
    return path if path.is_absolute() else ROOT / path


def artifact(path: Path) -> dict:
    resolved = path.resolve()
    try:
        label = str(resolved.relative_to(ROOT))
    except ValueError:
        label = str(resolved)
    return {"path": label, "bytes": resolved.stat().st_size, "sha256": sha256(resolved)}


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.tmp-{Path('/proc/self').resolve().name}")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    temporary.replace(path)


def verify_artifact(value: dict) -> Path:
    path = workspace_path(value["path"])
    if path.stat().st_size != value["bytes"] or sha256(path) != value["sha256"]:
        raise RuntimeError(f"artifact identity mismatch: {value['path']}")
    return path


def indexed_positions(report: dict, schema: str) -> dict[str, dict]:
    if report.get("schema") != schema or report.get("status") != "passed":
        raise RuntimeError(f"invalid parity report: {schema}")
    result = {row["id"]: row for row in report["positions"]}
    if len(result) != len(report["positions"]):
        raise RuntimeError(f"duplicate parity ids: {schema}")
    return result


def validate_parity(config: dict) -> dict:
    reference_path = verify_artifact(config["challenger"]["parity"]["reference"])
    native_path = verify_artifact(config["challenger"]["parity"]["native"])
    wasm_path = verify_artifact(config["challenger"]["parity"]["wasm"])
    reference = json.loads(reference_path.read_text())
    native = json.loads(native_path.read_text())
    wasm = json.loads(wasm_path.read_text())
    refs = indexed_positions(reference, "haitaka-anhoku-strength-first-reference-parity-v1")
    natives = indexed_positions(native, "haitaka-anhoku-strength-first-native-parity-v1")
    wasms = indexed_positions(wasm, "haitaka-anhoku-strength-first-wasm-parity-v1")
    if refs.keys() != natives.keys() or refs.keys() != wasms.keys():
        raise RuntimeError("parity position identities differ")
    mismatches = {
        "integerVsNative": sum(refs[key]["integerScore"] != natives[key]["score"] for key in refs),
        "integerVsWasm": sum(refs[key]["integerScore"] != wasms[key]["score"] for key in refs),
        "nativeVsWasm": sum(natives[key]["score"] != wasms[key]["score"] for key in refs),
    }
    if any(mismatches.values()) or native.get("fullRefreshEqualsIncremental") is not True:
        raise RuntimeError(f"corrected-runtime parity failed: {mismatches}")
    return {
        "positions": len(refs),
        "mismatches": mismatches,
        "nativeFullRefreshEqualsIncremental": True,
        "quantization": reference["quantization"],
    }


def validate_browser_report(
    path: Path,
    *,
    model_sha256: str,
    contract_sha256: str,
    openings_sha256: str,
    wasm_sha256: str,
    pairs: int,
    movetime_ms: int,
) -> tuple[dict, dict[str, float], dict[str, float]]:
    report = json.loads(path.read_text())
    if report.get("status") != "passed" or len(report.get("rawGames", [])) != pairs * 2:
        raise RuntimeError(f"incomplete browser report: {path}")
    summary = report["summary"]
    if any(summary[key] for key in ("missingMoves", "emergencyFallbacks", "unsearchedMoves")):
        raise RuntimeError(f"unqualified browser moves: {path}")
    if report["execution"]["budget"]["movetimeMs"] != movetime_ms:
        raise RuntimeError(f"browser budget mismatch: {path}")
    provenance = report["provenance"]
    expected_files = {
        "modelA": model_sha256,
        "contract": contract_sha256,
        "openings": openings_sha256,
        "wasm": wasm_sha256,
    }
    for name, expected in expected_files.items():
        if provenance["files"][name]["sha256"] != expected:
            raise RuntimeError(f"browser provenance mismatch for {name}: {path}")
    if provenance["execution"]["nodeVersion"] != "v26.8.1":
        raise RuntimeError(f"unexpected Node version: {path}")
    if provenance["execution"]["workerCount"] != 1 or provenance["execution"]["concurrentGames"] != 1:
        raise RuntimeError(f"browser concurrency mismatch: {path}")
    grouped = defaultdict(list)
    conservative = defaultdict(list)
    colors = defaultdict(set)
    for game in report["rawGames"]:
        opening = game["openingId"]
        grouped[opening].append(float(game["scoreA"]))
        conservative[opening].append(0.0 if game["capped"] else float(game["scoreA"]))
        colors[opening].add(game["aColor"])
        for search in game["searches"]:
            if search["requestedMs"] != movetime_ms:
                raise RuntimeError(f"per-search budget mismatch: {path}")
            if search["missingMove"] or search["emergencyFallbackUsed"] or search["unsearchedMove"]:
                raise RuntimeError(f"unqualified raw search: {path}")
            qualified = search["playMoveWasSearched"] or (
                search["interruptionReason"] == "terminal"
                and search["gameoverReason"] == "no-legal-move"
                and search["bestMove"] == "resign"
            )
            if not qualified:
                raise RuntimeError(f"unqualified terminal/search record: {path}")
    if len(grouped) != pairs or any(len(value) != 2 for value in grouped.values()):
        raise RuntimeError(f"browser pair cardinality mismatch: {path}")
    if any(value != {"black", "white"} for value in colors.values()):
        raise RuntimeError(f"browser color swap mismatch: {path}")
    return (
        report,
        {key: statistics.mean(value) for key, value in grouped.items()},
        {key: statistics.mean(value) for key, value in conservative.items()},
    )


def interval(groups: dict[str, float]) -> dict:
    values = list(groups.values())
    rng = random.Random(20260910)
    estimates = sorted(statistics.mean(rng.choices(values, k=len(values))) for _ in range(20_000))
    return {
        "groups": len(values),
        "mean": statistics.mean(values),
        "lower95": estimates[500],
        "upper95": estimates[19_499],
        "replicates": 20_000,
        "seed": 20260910,
    }


def load_and_validate_contract() -> tuple[dict, str, dict]:
    config = json.loads(CONTRACT.read_text())
    if config.get("schema") != "haitaka-strength-first-corrected-checkpoint-rescreen-v1":
        raise RuntimeError("unexpected rescreen contract schema")
    if config.get("status") != "frozen-before-challenger-games":
        raise RuntimeError("rescreen contract is not frozen")
    for value in config["frozenArtifacts"]:
        verify_artifact(value)
    parity = validate_parity(config)
    if parity["positions"] != config["challenger"]["parity"]["positions"]:
        raise RuntimeError("parity position count mismatch")
    contract_sha256 = sha256(CONTRACT)
    validate_browser_report(
        verify_artifact(config["incumbent"]["browserReport"]),
        model_sha256=config["incumbent"]["model"]["sha256"],
        contract_sha256=config["incumbent"]["browserContract"]["sha256"],
        openings_sha256=config["openings"]["sha256"],
        wasm_sha256=config["wasm"]["sha256"],
        pairs=config["browser"]["pairs"],
        movetime_ms=config["browser"]["movetimeMs"],
    )
    return config, contract_sha256, parity


def run() -> None:
    config, contract_sha256, _parity = load_and_validate_contract()
    output = RUN / "challenger-u00002048-1000ms.json"
    started = RUN / "browser-started.json"
    if started.exists() or output.exists():
        raise RuntimeError("immutable challenger run already started")
    RUN.mkdir(parents=True, exist_ok=True)
    node = verify_artifact(config["node"])
    version = subprocess.run([node, "--version"], check=True, text=True, capture_output=True).stdout.strip()
    if version != config["node"]["version"]:
        raise RuntimeError(f"Node version mismatch: {version}")
    write_json(started, {
        "schema": "haitaka-strength-first-checkpoint-rescreen-start-v1",
        "startedUnix": time.time(),
        "contractSha256": contract_sha256,
        "nodeVersion": version,
    })
    command = [
        str(node),
        str(verify_artifact(config["harness"])),
        "--pkg-dir", str(workspace_path(config["wasmPackageDirectory"])),
        "--model", str(verify_artifact(config["challenger"]["model"])),
        "--openings", str(verify_artifact(config["openings"])),
        "--movetime-ms", str(config["browser"]["movetimeMs"]),
        "--max-plies", str(config["browser"]["maxPlies"]),
        "--timeout-ms", str(config["browser"]["timeoutMs"]),
        "--contract", str(CONTRACT),
        "--output", str(output),
    ]
    write_json(RUN / "browser-command.json", command)
    with (RUN / "browser.log").open("w", encoding="utf-8") as log:
        subprocess.run(
            command,
            cwd=ROOT,
            stdout=log,
            stderr=subprocess.STDOUT,
            check=True,
            timeout=config["browser"]["timeoutMs"] / 1000 + 60,
        )
    validate_browser_report(
        output,
        model_sha256=config["challenger"]["model"]["sha256"],
        contract_sha256=contract_sha256,
        openings_sha256=config["openings"]["sha256"],
        wasm_sha256=config["wasm"]["sha256"],
        pairs=config["browser"]["pairs"],
        movetime_ms=config["browser"]["movetimeMs"],
    )
    write_json(RUN / "browser-completed.json", {
        "schema": "haitaka-strength-first-checkpoint-rescreen-complete-v1",
        "complete": True,
        "report": artifact(output),
    })


def summarize_browser(report: dict, score: dict[str, float], conservative: dict[str, float], path: Path) -> dict:
    summary = report["summary"]
    return {
        "games": len(report["rawGames"]),
        "wins": summary["score"]["aWins"],
        "losses": summary["score"]["bWins"],
        "draws": summary["score"]["draws"],
        "capped": sum(bool(game["capped"]) for game in report["rawGames"]),
        "score": interval(score),
        "capAsLossScore": interval(conservative),
        "timing": summary["timing"],
        "nodes": summary["nodes"],
        "load": report["load"],
        "memory": report["hostTelemetry"]["memory"],
        "report": artifact(path),
    }


def analyze() -> None:
    config, contract_sha256, parity = load_and_validate_contract()
    if not (RUN / "browser-completed.json").exists():
        raise RuntimeError("challenger browser run is incomplete")
    incumbent_path = verify_artifact(config["incumbent"]["browserReport"])
    challenger_path = RUN / "challenger-u00002048-1000ms.json"
    incumbent, incumbent_score, incumbent_conservative = validate_browser_report(
        incumbent_path,
        model_sha256=config["incumbent"]["model"]["sha256"],
        contract_sha256=config["incumbent"]["browserContract"]["sha256"],
        openings_sha256=config["openings"]["sha256"],
        wasm_sha256=config["wasm"]["sha256"],
        pairs=config["browser"]["pairs"],
        movetime_ms=config["browser"]["movetimeMs"],
    )
    challenger, challenger_score, challenger_conservative = validate_browser_report(
        challenger_path,
        model_sha256=config["challenger"]["model"]["sha256"],
        contract_sha256=contract_sha256,
        openings_sha256=config["openings"]["sha256"],
        wasm_sha256=config["wasm"]["sha256"],
        pairs=config["browser"]["pairs"],
        movetime_ms=config["browser"]["movetimeMs"],
    )
    if incumbent_score.keys() != challenger_score.keys():
        raise RuntimeError("incumbent and challenger opening groups differ")
    delta = {key: challenger_conservative[key] - incumbent_conservative[key] for key in incumbent_score}
    incumbent_summary = summarize_browser(incumbent, incumbent_score, incumbent_conservative, incumbent_path)
    challenger_summary = summarize_browser(challenger, challenger_score, challenger_conservative, challenger_path)
    selected = "u2048" if challenger_summary["capAsLossScore"]["mean"] > incumbent_summary["capAsLossScore"]["mean"] else "u1920"
    reason = (
        "u2048 has the higher conservative 1-second score on the frozen common openings"
        if selected == "u2048"
        else "u1920 retains the higher or tied conservative 1-second score; the frozen tie-break also favors its higher prior equal-node score and lower validation loss"
    )
    result = {
        "schema": "haitaka-strength-first-corrected-checkpoint-rescreen-result-v1",
        "complete": True,
        "scope": "reused-development-openings-checkpoint-selection-only",
        "contract": artifact(CONTRACT),
        "parity": parity,
        "incumbent": {"id": "u1920", **incumbent_summary},
        "challenger": {"id": "u2048", **challenger_summary},
        "challengerMinusIncumbent": interval(delta),
        "selection": {
            "selected": selected,
            "rule": config["selectionRule"],
            "reason": reason,
        },
        "productionRecommendation": "retain-handcrafted",
        "productionChanged": False,
        "gpuUsed": False,
        "limitations": [
            "Both checkpoints and all openings are reused development evidence.",
            "The two browser matches were run sequentially on the same host rather than simultaneously.",
            "Selection uses the frozen conservative fixed-time point estimate; its interval is reported and does not become a confirmation claim.",
        ],
    }
    output = RUN / "analysis"
    write_json(output / "result.json", result)
    score_delta = result["challengerMinusIncumbent"]
    lines = [
        "# Anhoku corrected-runtime checkpoint rescreen",
        "",
        "This CPU-only development rescreen compares the two compact checkpoints selected in advance: update 1,920 for lowest validation loss and update 2,048 for highest scheduled game score. Both use the serializer-consistent runtime, the same 64 opening groups, 1,000 ms per move, one browser worker, and handcrafted as the opponent.",
        "",
        "| Checkpoint | Games | W–L–D | Conservative score | Opening-group 95% interval |",
        "| --- | ---: | ---: | ---: | ---: |",
    ]
    for cell in (result["incumbent"], result["challenger"]):
        score = cell["capAsLossScore"]
        lines.append(f"| {cell['id']} | {cell['games']} | {cell['wins']}–{cell['losses']}–{cell['draws']} | {score['mean']:.1%} | {score['lower95']:.1%}–{score['upper95']:.1%} |")
    lines.extend([
        "",
        f"The opening-paired u2048 minus u1920 conservative score difference is {score_delta['mean']*100:+.1f} percentage points, with a 95% interval [{score_delta['lower95']*100:+.1f}, {score_delta['upper95']*100:+.1f}].",
        "",
        f"The frozen development rule selects **{selected}**: {reason}.",
        "",
        f"Update 2,048 passed corrected full-precision/integer/native/WASM qualification on {parity['positions']:,} positions with zero mismatches. Its corrected export quantization MAE is {parity['quantization']['meanAbsoluteScoreDelta']:.2f} score units.",
        "",
        "This is checkpoint selection on reused development openings. Handcrafted remains the production recommendation; no promotion, deployment, training, or GPU use is authorized by this result.",
    ])
    markdown = "\n".join(lines) + "\n"
    (output / "result.md").write_text(markdown, encoding="utf-8")
    write_json(ROOT / "r0/anhoku-strength-first/step7-result-report.json", result)
    (ROOT / "r0/anhoku-strength-first/step7-result-report.md").write_text(markdown, encoding="utf-8")
    print(json.dumps({
        "selected": selected,
        "incumbent": incumbent_summary["score"],
        "challenger": challenger_summary["score"],
        "challengerMinusIncumbent": result["challengerMinusIncumbent"],
    }, indent=2))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("preflight", "run", "analyze"))
    args = parser.parse_args()
    if args.command == "preflight":
        config, contract_sha256, parity = load_and_validate_contract()
        print(json.dumps({"contractSha256": contract_sha256, "parity": parity, "status": "passed"}, indent=2))
    elif args.command == "run":
        run()
    else:
        analyze()


if __name__ == "__main__":
    main()
