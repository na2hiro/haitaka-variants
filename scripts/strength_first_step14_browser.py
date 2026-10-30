#!/usr/bin/env python3
"""Bounded CPU-only Step 14 browser finalist worker.

Preparation and fixtures are offline.  Qualification and scored execution are
fail-closed and require a matching authorization receipt.  Closeout only reads
already accepted canonical records and cannot dispatch Chrome or replay.
"""

from __future__ import annotations

import argparse
import copy
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import http.server
import json
import math
import os
from pathlib import Path
import platform
import re
import signal
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from collections import Counter
from fractions import Fraction
from typing import Any, Dict, Iterator, List, Optional, Sequence, Tuple
from urllib.parse import unquote, urlparse

from strength_first_step14_contract import (
    BLOCK_COUNT, EXPERIMENT_ID, GAME_COUNT, GAMES_PER_BLOCK, MAX_PLIES, PAIR_COUNT,
    MOVETIME_MS, ContractError, analyze, build_schedule, fail, load_json,
    safe_path, select_openings, sha256, stable_bytes, validate_block,
)


EXPECTED = {
    "s13-closeout": ("out/anhoku-strength-first/step13-replication-diagnosis-20260919-v1/closeout.json", "3d77748928c495826074f8685289c349073ccc37b31c5cb25fca575e33fd2482", None),
    "s13-analysis": ("out/anhoku-strength-first/step13-replication-diagnosis-20260919-v1/analysis.json", "184b518e921669e831ef889c54293532dd247810072213eff603cf791749f85b", None),
    "s13-replay": ("out/anhoku-strength-first/step13-replication-diagnosis-20260919-v1/replay/summary.json", "3684ba2ecc37c0b58b605aab9c05d83ad50c074a55053ec501f944e788b6921c", None),
    "s12-selection": ("out/anhoku-strength-first/step12-independent-replication-20260916-v1/training/selection.json", "8a3a3ef6999f8f8838426471ae9f0d59fd6299394f7e5487a28a0fe26f776d97", None),
    "model": ("out/anhoku-strength-first/step12-independent-replication-20260916-v1/exports/u000640.nnue", "da8a15ccedb155a1ab2d421c2e016f21d55ca066f7f9bb841a5eac0c333035b8", 83048706),
    "s12-schedule": ("out/anhoku-strength-first/step12-independent-replication-20260916-v1/openings/schedule.json", "07fa07ad39c2d5ec0218d9c965e8abdcf83b58e7c5106923b8c0284cce8e7ad1", None),
    "wasm": ("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/package/web-pkg/haitaka_wasm_bg.wasm", "8677f7c83f48186b8132a534185044402d25cc9f3d071363912506ca0203dfc0", 18279053),
    "glue": ("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/package/web-pkg/haitaka_wasm.js", "d8eaf8ad3bcf842588f4fc2d3a83d6cb7f2d29637a5097396db3b0877036a724", 36924),
}
TSV_HASH = "ae4e6fedef9a57007795c6c6f7e4542f116ee97b9eb139029646c79b85aaa738"
TSV_BYTES = 6367
RSS_INTERVAL_MS = 250
MEMORY_CAP_BYTES = 8 * 1024**3
STORAGE_CAP_BYTES = 10 * 1024**3
MIN_FREE_BYTES = 16 * 1024**3
PREPARATION_CAP_SECONDS = 7200.0
MATCH_CAP_SECONDS = 21600.0
BLOCK_CAP_SECONDS = 3900.0
SMOKE_SEARCHES = 16
ROOT_RESULT_SCHEMA = "haitaka-search-root-result-v1"

EXTRA_INPUTS = {
    "b11-source-identity": ("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/source-identity.json", "345c42ee42df6dcecf85bb9efb73e1c1ab107dec9bd6d3199b26711bbd210870", None),
    "b11-controller": ("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/scripts/strength_first_step11_browser_controller.py", "f0f1cffb2da2f1626c3ea182c973af4e2a0d5180a5a186f64ba8c5ffd2e31b17", None),
    "b11-worker": ("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/scripts/strength_first_browser_worker.js", "0bd0c2f95fd513a34f03d227170bf6647dcaec3f6f6c71ca29c262d2c440ba6a", None),
    "b11-page": ("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/scripts/strength_first_step11_browser_page.html", "8425da34bf84e534eb94bf414ce83910a01c3f07d610cd94f16a2e00125b6bf1", None),
    "b11-parity-worker": ("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/scripts/strength_first_step11_browser_parity_worker.js", "920f2571c7b382c083caaab8669b118ab82da447a23f34f116ac644d89c38885", None),
    "b11-smoke-worker": ("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/scripts/strength_first_step11_browser_smoke_worker.js", "6febd71bc65330d61dc376ee1f7d141384e2bd922828940b1f8d52ccfbf9557c", None),
    "b11-terminal-worker": ("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/scripts/strength_first_step11_browser_terminal_replay_worker.js", "593ca3a028506adfd052be2d452e6963caeb547c8b6f1c1f8044ae08ac953857", None),
    "b11-historical-browser": ("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/historical/step6-browser-primary.json", "c0ac9d73ce1be0f13efeb4ccab5e40fb1f1a5c3999f48a330dbc86dc20f7bce7", None),
    "s12-parity-fixture": ("out/anhoku-strength-first/step12-independent-replication-20260916-v1/parity/fixture.json", "20e360189fff864aa84ecd2b42b37ee9625a4217dfdc277f5f6aa920467c9467", None),
    "s12-parity-positions": ("out/anhoku-strength-first/step12-independent-replication-20260916-v1/parity/positions.json", "a55e80c62826bbe9730292a7a2121ee1adb6e33df7584c7bb0e86d0428482551", None),
    "s12-reference-u640": ("out/anhoku-strength-first/step12-independent-replication-20260916-v1/parity/reference-u000640.json", "533e12b367f9dd53fdd590f079f65dba121581123f4e5e081309a34092496493", None),
    "s12-native-u640": ("out/anhoku-strength-first/step12-independent-replication-20260916-v1/parity/native-u000640.json", "072a0d31597dc8d7d37856ea925d0b63eb3bfda3c378cbb6a2b0d3a5db670274", None),
    "s12-wasm-u640": ("out/anhoku-strength-first/step12-independent-replication-20260916-v1/parity/wasm-u000640.json", "e2cb98b792d1434776cd4e3703299e732e63ca258614bee18b34efc298f70290", None),
    "s12-parity-qualification": ("out/anhoku-strength-first/step12-independent-replication-20260916-v1/parity/qualification.json", "e6c92986b89ea58e13b0d659a5809f68c629fddc2033e21fbaad14caa121a62e", None),
    "s12-model-bindings": ("out/anhoku-strength-first/step12-independent-replication-20260916-v1/parity/model-bindings.json", "0b6948f33aa1f78d48df5ea294cf36fa1ac72480551a4e621ef4f7ad5ab9b8bd", None),
}


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def atomic_bytes(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    except BaseException:
        try:
            os.unlink(name)
        except OSError:
            pass
        raise


def atomic_json(path: Path, value: Any) -> None:
    atomic_bytes(path, json.dumps(value, indent=2, sort_keys=True,
                                  allow_nan=False).encode() + b"\n")


def artifact(path: Path, root: Path) -> dict[str, Any]:
    path = path.resolve()
    try:
        display = str(path.relative_to(root.resolve()))
    except ValueError:
        display = str(path)
    return {"path": display, "bytes": path.stat().st_size, "sha256": sha256(path)}


def run_capture(argv: Sequence[str], timeout: float = 30.0) -> Tuple[int, str, str]:
    try:
        completed = subprocess.run(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   text=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        return 127, "", str(exc)
    return completed.returncode, completed.stdout, completed.stderr


def copy_bound(source: Path, target: Path, root: Path) -> dict[str, Any]:
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, target)
    return artifact(target, root)


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def verify_expected(repo: Path) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for name, (relative, digest, size) in {**EXPECTED, **EXTRA_INPUTS}.items():
        path = safe_path(repo, relative)
        if not path.is_file() or sha256(path) != digest or (size is not None and path.stat().st_size != size):
            fail(f"frozen input differs: {name}: {path}")
        result[name] = artifact(path, repo)
    return result


def phase_receipt(root: Path, phase: str, status: str, **items: Any) -> dict[str, Any]:
    value = {"schema": "haitaka-anhoku-strength-first-step14-phase-v1",
             "experimentId": EXPERIMENT_ID, "phase": phase, "status": status,
             "recordedAt": utc_now(), **items}
    atomic_json(root / "receipts" / f"{phase}.json", value)
    return value


def prepare(root: Path) -> dict[str, Any]:
    repo = repo_root()
    if root.exists():
        manifest = load_json(root / "manifest.json")
        if manifest.get("experimentId") != EXPERIMENT_ID:
            fail("existing root has different experiment identity")
        # A pre-score implementation repair refreshes only the owned package
        # and its hashes.  Mutable qualification/match state is never copied
        # from the coordinator over an existing MBP root.
        refreshed: dict[str, dict[str, Any]] = {}
        for name in ("strength_first_step14_browser.py", "strength_first_step14_contract.py",
                     "strength_first_step14_browser_worker.js", "strength_first_step14_browser_page.html",
                     "strength_first_step14_terminal_worker.js"):
            refreshed[name] = copy_bound(repo / "scripts" / name, root / "package" / name, root)
        bindings = load_json(root / "execution-bindings.json")
        bindings["package"] = refreshed
        atomic_json(root / "execution-bindings.json", bindings)
        source_identity = load_json(root / "source-identity.json")
        source_identity["sourceSnapshot"]["relevantFiles"] = refreshed
        atomic_json(root / "source-identity.json", source_identity)
        manifest["sourceIdentity"] = artifact(root / "source-identity.json", root)
        manifest["executionBindings"] = artifact(root / "execution-bindings.json", root)
        manifest["status"] = "prepared-not-authorized"
        atomic_json(root / "manifest.json", manifest)
        return {"status": "existing", "root": str(root)}
    root.mkdir(parents=True)
    anchors = verify_expected(repo)
    source = load_json(safe_path(repo, EXPECTED["s12-schedule"][0]))
    openings = select_openings(source)
    tsv = "".join(f"{row['openingId']}\t{row['startSfen']}\n" for row in openings).encode()
    if len(tsv) != TSV_BYTES or hashlib.sha256(tsv).hexdigest() != TSV_HASH:
        fail("deterministic finalist.tsv bytes/hash differ")
    atomic_bytes(root / "openings/finalist.tsv", tsv)
    schedule = build_schedule(openings, EXPECTED["model"][1])
    atomic_json(root / "openings/schedule.json", schedule)
    for block in range(BLOCK_COUNT):
        atomic_json(root / "openings/blocks" / f"block-{block:02d}.json",
                    {"schema": schedule["schema"], "block": block,
                     "rows": [row for row in schedule["rows"] if row["block"] == block]})

    # Materialize all imported evidence and the exact historical runtime files
    # inside the owned package.  Later phases must be runnable from this root
    # alone; no phase is allowed to silently resolve a mutable coordinator path.
    input_targets = {
        "s13-closeout": "inputs/s13/closeout.json",
        "s13-analysis": "inputs/s13/analysis.json",
        "s13-replay": "inputs/s13/replay-summary.json",
        "s12-selection": "inputs/s12/training-selection.json",
        "s12-schedule": "inputs/s12/schedule.json",
        "b11-source-identity": "inputs/b11/source-identity.json",
        "b11-historical-browser": "inputs/b11/historical-browser.json",
        "s12-parity-fixture": "inputs/s12/parity-fixture.json",
        "s12-parity-positions": "inputs/s12/parity-positions.json",
        "s12-reference-u640": "inputs/s12/reference-u000640.json",
        "s12-native-u640": "inputs/s12/native-u000640.json",
        "s12-wasm-u640": "inputs/s12/wasm-u000640.json",
        "s12-parity-qualification": "inputs/s12/parity-qualification.json",
        "s12-model-bindings": "inputs/s12/model-bindings.json",
    }
    bound_inputs: dict[str, dict[str, Any]] = {}
    for name, target in input_targets.items():
        source_path = safe_path(repo, ({**EXPECTED, **EXTRA_INPUTS})[name][0])
        bound_inputs[name] = copy_bound(source_path, root / target, root)

    # The model and web package are immutable execution inputs, distinct from
    # the Step 14 Python/JS adapters.
    package_assets = {
        "model": (safe_path(repo, EXPECTED["model"][0]), "package/model/u000640.nnue"),
        "wasm": (safe_path(repo, EXPECTED["wasm"][0]), "package/web-pkg/haitaka_wasm_bg.wasm"),
        "glue": (safe_path(repo, EXPECTED["glue"][0]), "package/web-pkg/haitaka_wasm.js"),
    }
    for name, (source_path, target) in package_assets.items():
        bound_inputs[name] = copy_bound(source_path, root / target, root)

    scripts: dict[str, dict[str, Any]] = {}
    for name in ("strength_first_step14_browser.py", "strength_first_step14_contract.py",
                 "strength_first_step14_browser_worker.js", "strength_first_step14_browser_page.html",
                 "strength_first_step14_terminal_worker.js"):
        scripts[name] = copy_bound(repo / "scripts" / name, root / "package" / name, root)

    runtime_targets = {
        "b11-controller": "runtime/step11/strength_first_step11_browser_controller.py",
        "b11-worker": "runtime/step11/strength_first_browser_worker.js",
        "b11-page": "runtime/step11/strength_first_step11_browser_page.html",
        "b11-parity-worker": "runtime/step11/strength_first_step11_browser_parity_worker.js",
        "b11-smoke-worker": "runtime/step11/strength_first_step11_browser_smoke_worker.js",
        "b11-terminal-worker": "runtime/step11/strength_first_step11_browser_terminal_replay_worker.js",
    }
    runtime: dict[str, dict[str, Any]] = {}
    for name, target in runtime_targets.items():
        source_path = safe_path(repo, EXTRA_INPUTS[name][0])
        runtime[name] = copy_bound(source_path, root / target, root)

    # Capture the coordinator source state while it is available.  The remote
    # MBP receives this sealed snapshot; it need not have the whole checkout or
    # a mutable git worktree.
    code, head, _ = run_capture(["git", "-C", str(repo), "rev-parse", "HEAD"])
    code2, dirty, _ = run_capture(["git", "-C", str(repo), "status", "--short"])
    source_snapshot = {
        "head": head.strip() if code == 0 else None,
        "dirty": dirty.splitlines() if code2 == 0 else [],
        "relevantFiles": {name: scripts[name] for name in scripts},
        "historicalRuntimeFiles": runtime,
    }
    atomic_json(root / "source-identity.json", {
        "schema": "haitaka-anhoku-strength-first-step14-source-identity-v1",
        "experimentId": EXPERIMENT_ID,
        "archivedStep11": bound_inputs["b11-source-identity"],
        "sourceSnapshot": source_snapshot,
        "webBuild": load_json(root / "inputs/b11/source-identity.json").get("webBuild"),
    })

    authorization = {"schema": "haitaka-anhoku-strength-first-step14-authorization-v1",
                     "experimentId": EXPERIMENT_ID,
                     "instruction": "Please execute plans/anhoku-nnue-strength-first-step14-browser-finalist-runbook.md. Consult Astra medium if there's a blocker. Delegate coding tasks to Sol medium.",
                     "grantText": "Please execute plans/anhoku-nnue-strength-first-step14-browser-finalist-runbook.md",
                     "executionGrant": True, "gpuRentalAuthorized": False,
                     "trainingAuthorized": False, "commitOrPushAuthorized": False}
    atomic_json(root / "authorization.json", authorization)
    contract = {"schema": "haitaka-anhoku-strength-first-step14-contract-v1",
                "experimentId": EXPERIMENT_ID, "modelSha256": EXPECTED["model"][1],
                "wasmSha256": EXPECTED["wasm"][1], "glueSha256": EXPECTED["glue"][1],
                "schedule": {"pairs": 64, "games": 128, "blocks": 8,
                             "movetimeMs": MOVETIME_MS, "maxPlies": MAX_PLIES,
                             "pairing": "same-sfen-engine-seat-swap"},
                "resources": {"preparationSeconds": 7200, "matchSeconds": 21600,
                              "blockAttemptSeconds": 3900, "closeoutSeconds": 1800,
                              "memoryBytes": 8 * 1024**3, "storageBytes": 10 * 1024**3,
                              "paidComputeUsd": 0, "gpuHours": 0, "trainingUpdates": 0}}
    atomic_json(root / "contract.json", contract)
    atomic_json(root / "execution-bindings.json", {
        "schema": "haitaka-anhoku-strength-first-step14-execution-bindings-v1",
        "experimentId": EXPERIMENT_ID, "host": {"endpoint": "192.168.0.12",
        "chromePath": "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "python": "/usr/bin/python3", "architecture": "arm64"},
        "inputs": bound_inputs, "package": scripts, "runtime": runtime,
        "sourceSnapshot": source_snapshot,
        "replay": {"binary": "tools/strength_first_step14_replay",
                    "source": "tools/strength_first_step14_replay.rs",
                    "features": ["anhoku"]}})
    manifest = {"schema": "haitaka-anhoku-strength-first-step14-manifest-v1",
                "experimentId": EXPERIMENT_ID, "status": "prepared-not-authorized",
                "createdAt": utc_now(), "contract": artifact(root / "contract.json", root),
                "authorization": artifact(root / "authorization.json", root),
                "schedule": artifact(root / "openings/schedule.json", root),
                "finalist": artifact(root / "openings/finalist.tsv", root),
                "sourceIdentity": artifact(root / "source-identity.json", root),
                "executionBindings": artifact(root / "execution-bindings.json", root),
                "head": source_snapshot["head"], "dirty": source_snapshot["dirty"],
                "inputs": bound_inputs, "runtime": runtime,
                "preparation": {"browserLaunches": 0, "searches": 0, "games": 0,
                                 "gpuRentalAuthorized": False, "trainingAuthorized": False}}
    atomic_json(root / "manifest.json", manifest)
    return phase_receipt(root, "prepare", "passed", anchors=anchors,
                         browserLaunches=0, searches=0, games=0)


def make_fixture_trace(game: dict[str, Any], move: int, terminal: bool, envelope: str) -> dict[str, Any]:
    color = game["initialSideToMove"] if move % 2 == 0 else ("white" if game["initialSideToMove"] == "black" else "black")
    engine = game["blackEngine"] if color == "black" else game["whiteEngine"]
    reason = "no-legal-move" if terminal else None
    return {"schema": "haitaka-anhoku-strength-first-step14-raw-search-v1",
            "globalGameIndex": game["globalGameIndex"], "globalPairIndex": game["globalPairIndex"],
            "block": game["block"], "blockGameIndex": game["blockGameIndex"],
            "moveIndex": move, "ply": move + 1, "engine": engine, "color": color,
            "command": "go movetime 1000", "requestedMs": 1000,
            "outputs": (["info string gameover no-legal-move", "bestmove resign"] if terminal else
                        ["info depth 1 rootResultSchema haitaka-search-root-result-v1 playMoveWasSearched 1 missingMove 0 emergencyFallbackUsed 0", "bestmove 7g7f"]),
            "bestMove": "resign" if terminal else "7g7f", "gameoverReason": reason,
            "gameoverReasonRaw": reason, "terminalLoserColor": None, "terminalFlagsDerived": terminal,
            "rootResultSchema": None if terminal else "haitaka-search-root-result-v1",
            "playMoveWasSearched": not terminal, "missingMove": False,
            "emergencyFallbackUsed": False, "unsearchedMove": False,
            "elapsedMs": 1.0, "provenanceEnvelopeId": envelope}


def fixtures(root: Path) -> dict[str, Any]:
    """Exercise the actual Python adapter/analyzer with positive and negative cases.

    Rules-valid histories are deliberately kept in the qualification phase and
    replayed by the Step 14 Rust helper.  This offline phase tests the raw
    schema/parser boundary without claiming that a fabricated game is legal.
    """
    schedule = load_json(root / "openings/schedule.json")
    envelope = "fixture-envelope"
    rows = [row for row in schedule["rows"] if row["block"] == 0]
    raws, replays = [], []
    for row in rows:
        trace = make_fixture_trace(row, 0, True, envelope)
        winner = "B" if trace["engine"] == "A" else "A"
        raws.append({"schema": "haitaka-anhoku-strength-first-step14-raw-game-v1",
                     **{key: row[key] for key in ("globalGameIndex", "globalPairIndex", "block", "blockGameIndex", "leg", "openingId", "startSfen", "initialSideToMove", "aColor", "bColor", "blackEngine", "whiteEngine")},
                     "moves": [], "plies": 0, "rawEnd": "no-legal-move", "rawWinner": winner,
                     "rawResult": "b-win" if winner == "B" else "a-win", "searches": [trace],
                     "provenanceEnvelopeId": envelope})
        replays.append({"schema": "haitaka-anhoku-strength-first-step14-replay-v1", "status": "passed",
                        "globalGameIndex": row["globalGameIndex"], "globalPairIndex": row["globalPairIndex"],
                        "block": row["block"], "blockGameIndex": row["blockGameIndex"], "leg": row["leg"],
                        "openingId": row["openingId"], "startSfen": row["startSfen"], "replayedPlies": 0,
                        "maxPlies": MAX_PLIES, "terminationReason": "no-legal-move", "winner": winner,
                        "boardStatus": "won", "historyAdjudication": "ongoing",
                        "checks": {name: True for name in ("schema", "identity", "uniqueGameIdentity", "sfen", "sideToMove", "seats", "plies", "moveSyntax", "moveLegality", "noMoveAfterTerminal", "history", "terminalPrecedence", "capSemantics", "disposition", "winner")}})
    report = {"schema": "haitaka-anhoku-strength-first-step14-browser-report-v1", "status": "passed",
              "provenance": {"envelopeId": envelope}, "rawGames": raws}
    replay = {"records": replays}
    cases: list[dict[str, Any]] = []
    validate_block(report, replay, schedule, 0, envelope)
    cases.append({"case": "valid-raw-adapter-chain", "outcome": "passed"})
    mutations = [
        ("missing-required-flag", lambda x: x["rawGames"][0]["searches"][0].pop("missingMove")),
        ("wrong-budget", lambda x: x["rawGames"][0]["searches"][0].update(requestedMs=999)),
        ("trace-usi-conflict", lambda x: x["rawGames"][0]["searches"][0].update(bestMove="7g7f")),
        ("wrong-seat", lambda x: x["rawGames"][0].update(blackEngine="B")),
        ("nonfinite-time", lambda x: x["rawGames"][0]["searches"][0].update(elapsedMs=float("inf"))),
        ("raw-result-winner-conflict", lambda x: x["rawGames"][0].update(rawResult="draw")),
        ("terminal-metadata-conflict", lambda x: x["rawGames"][0]["searches"][0].update(terminalFlagsDerived=False)),
        ("duplicate-game", lambda x: x["rawGames"].__setitem__(1, copy.deepcopy(x["rawGames"][0]))),
    ]
    for name, mutate in mutations:
        candidate = json.loads(json.dumps(report))
        mutate(candidate)
        try:
            validate_block(candidate, replay, schedule, 0, envelope)
        except (ContractError, ValueError, TypeError) as exc:
            cases.append({"case": name, "outcome": "rejected", "reason": str(exc)})
        else:
            fail(f"negative fixture unexpectedly passed: {name}")

    synthetic = []
    for row in schedule["rows"]:
        synthetic.append({**row, "schema": "canonical", "result": "a-win" if row["globalGameIndex"] < 65 else "b-win",
                          "winner": "A" if row["globalGameIndex"] < 65 else "B",
                          "terminationReason": "no-legal-move", "capped": False})
    result = analyze(synthetic)
    if result["score"]["twoWPlusD"] != 130 or result["routing"] != "favorable":
        fail("just-above-half score fixture differs")
    synthetic[0].update(result="draw", winner=None, terminationReason="maximum-ply-capped", capped=True)
    if analyze(synthetic)["cappedSensitivity"]["cappedGames"] != 1:
        fail("single capped game was not counted exactly once")
    synthetic[1].update(result="draw", winner=None, terminationReason="maximum-ply-capped", capped=True)
    synthetic[2].update(result="draw", winner=None, terminationReason="maximum-ply-capped", capped=True)
    if analyze(synthetic)["cappedSensitivity"]["cappedGames"] != 3 or analyze(synthetic)["cappedSensitivity"]["cappedPairs"] != 2:
        fail("multi-cap sensitivity count differs")
    cases.extend([{"case": "just-above-half", "outcome": "passed"},
                  {"case": "one-capped-leg-count", "outcome": "passed"},
                  {"case": "both-capped-legs-and-cross-pair-count", "outcome": "passed"},
                  {"case": "exact-fifty-routes-to-investigate", "outcome": "passed"}])
    receipt = {"schema": "haitaka-anhoku-strength-first-step14-fixtures-v1", "status": "passed",
               "cases": cases, "browserLaunches": 0, "searches": 0, "games": 0,
               "rulesFixtures": "qualification-rust-replay-and-terminal-browser-phase",
               "experimentalComputeAttempted": False}
    atomic_json(root / "fixtures/report.json", receipt)
    return phase_receipt(root, "fixtures", "passed", fixtureReport=artifact(root / "fixtures/report.json", root),
                         browserLaunches=0, searches=0, games=0)


def process_snapshot() -> list[dict[str, Any]]:
    code, stdout, _ = run_capture(["ps", "-axo", "pid=,ppid=,pgid=,rss=,command="], timeout=10)
    if code != 0:
        return []
    result: list[dict[str, Any]] = []
    for line in stdout.splitlines():
        fields = line.strip().split(None, 4)
        if len(fields) < 5:
            continue
        try:
            result.append({"pid": int(fields[0]), "ppid": int(fields[1]), "pgid": int(fields[2]),
                           "rssKiB": int(fields[3]), "command": fields[4]})
        except ValueError:
            continue
    return result


def process_start_time(pid: int) -> Optional[str]:
    code, stdout, _ = run_capture(["ps", "-p", str(pid), "-o", "lstart="], timeout=10)
    return stdout.strip() if code == 0 and stdout.strip() else None


class RssSampler:
    def __init__(self, pid: int, pgid: int) -> None:
        self.pid, self.pgid = pid, pgid
        self.samples: list[dict[str, Any]] = []
        self.failures: list[str] = []
        self.stop_event = threading.Event()
        self.thread = threading.Thread(target=self._run, name="step14-rss", daemon=True)

    def start(self) -> None:
        self.thread.start()

    def _run(self) -> None:
        while not self.stop_event.is_set():
            rows = process_snapshot()
            owned = [row for row in rows if row["pgid"] == self.pgid]
            if not owned and any(row["pid"] == self.pid for row in rows):
                owned = [row for row in rows if row["pid"] == self.pid]
            if owned:
                self.samples.append({"sampledAtUnixMs": int(time.time() * 1000),
                                     "processes": [{key: row[key] for key in ("pid", "ppid", "pgid", "rssKiB")} for row in owned],
                                     "processCount": len(owned),
                                     "residentSetBytes": sum(row["rssKiB"] for row in owned) * 1024})
            else:
                self.failures.append("owned-process-sample-missing")
            self.stop_event.wait(RSS_INTERVAL_MS / 1000.0)

    def stop(self) -> dict[str, Any]:
        self.stop_event.set()
        self.thread.join(timeout=2.0)
        if not self.samples:
            return {"status": "unavailable", "method": "macos-ps-process-group-sum",
                    "processGroupId": self.pgid, "sampleIntervalMs": RSS_INTERVAL_MS,
                    "sampleCount": 0, "failures": self.failures}
        peak = max(sample["residentSetBytes"] for sample in self.samples)
        return {"status": "observed", "method": "macos-ps-process-group-and-descendant-sum",
                "processGroupId": self.pgid, "sampleIntervalMs": RSS_INTERVAL_MS,
                "sampleCount": len(self.samples), "processCountPeak": max(sample["processCount"] for sample in self.samples),
                "peakResidentSetBytes": peak, "firstSample": self.samples[0], "lastSample": self.samples[-1],
                "failures": self.failures,
                "coverageSeconds": (self.samples[-1]["sampledAtUnixMs"] - self.samples[0]["sampledAtUnixMs"]) / 1000.0}


def signal_owned_group(process: subprocess.Popen[Any], pgid: int, output_dir: Path) -> dict[str, Any]:
    actions: list[dict[str, Any]] = []
    if process.poll() is not None:
        return {"status": "already-exited", "actions": actions, "pid": process.pid, "processGroupId": pgid}
    for sig, wait_seconds in ((signal.SIGINT, 5.0), (signal.SIGTERM, 5.0)):
        try:
            os.killpg(pgid, sig)
            actions.append({"signal": sig.name, "sentAtUnix": time.time()})
        except ProcessLookupError:
            actions.append({"signal": sig.name, "status": "process-group-gone"})
            break
        deadline = time.monotonic() + wait_seconds
        while time.monotonic() < deadline and process.poll() is None:
            time.sleep(0.1)
        if process.poll() is not None:
            break
    status = "closed" if process.poll() is not None else "cleanup-blocked"
    result = {"status": status, "actions": actions, "pid": process.pid, "processGroupId": pgid,
              "returnCode": process.returncode}
    atomic_json(output_dir / "cleanup.json", result)
    return result


class BrowserState:
    def __init__(self) -> None:
        self.lock = threading.RLock()
        self.event = threading.Event()
        self.ack: Optional[dict[str, Any]] = None
        self.result: Optional[dict[str, Any]] = None
        self.error: Optional[dict[str, Any]] = None
        self.raw_games: dict[int, dict[str, Any]] = {}
        self.progress: list[dict[str, Any]] = []


class BrowserHandler(http.server.BaseHTTPRequestHandler):
    server_version = "AnhokuStep14Browser/1"

    def log_message(self, _format: str, *_args: Any) -> None:
        return

    @property
    def state(self) -> BrowserState:
        return self.server.browser_state  # type: ignore[attr-defined]

    @property
    def config(self) -> dict[str, Any]:
        return self.server.browser_config  # type: ignore[attr-defined]

    @property
    def output_dir(self) -> Path:
        return self.server.browser_output_dir  # type: ignore[attr-defined]

    def send_bytes(self, data: bytes, content_type: str, status: int = 200) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def fail_request(self, category: str, status: int = 409) -> None:
        with self.state.lock:
            self.state.error = {"category": category}
            self.state.event.set()
        self.send_bytes((category + "\n").encode(), "text/plain", status)

    def do_GET(self) -> None:
        path = unquote(urlparse(self.path).path)
        routes: dict[str, tuple[bytes, str]] = self.server.browser_routes  # type: ignore[attr-defined]
        route = routes.get(path)
        if route is None:
            self.send_bytes(b"not found\n", "text/plain", 404)
        else:
            self.send_bytes(route[0], route[1])

    def do_POST(self) -> None:
        path = unquote(urlparse(self.path).path)
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            self.fail_request("invalid-content-length", 400)
            return
        if length < 0 or length > 128 * 1024 * 1024:
            self.fail_request("request-too-large", 413)
            return
        try:
            value = json.loads(self.rfile.read(length).decode("utf-8"))
        except (UnicodeError, json.JSONDecodeError):
            self.fail_request("invalid-json", 400)
            return
        if path == "/provenance":
            if not isinstance(value, dict) or value.get("envelopeId") != self.config["provenance"]["envelopeId"] or value.get("verifiedBeforePlay") is not True:
                self.fail_request("provenance-mismatch")
                return
            provided = value.get("verifiedFiles") or value.get("files")
            expected = self.config["provenance"]["files"]
            if not isinstance(provided, dict) or set(provided) != set(expected):
                self.fail_request("provenance-file-inventory-mismatch")
                return
            for name, binding in expected.items():
                actual = provided.get(name)
                if not isinstance(actual, dict) or actual.get("bytes") != binding.get("bytes") or actual.get("sha256") != binding.get("sha256"):
                    self.fail_request("provenance-file-hash-mismatch")
                    return
            with self.state.lock:
                if self.state.ack is not None:
                    self.fail_request("duplicate-provenance-ack")
                    return
                self.state.ack = value
            atomic_json(self.output_dir / "provenance-ack.json", value)
            self.send_bytes(b"accepted-before-play\n", "text/plain")
            return
        if path == "/raw-game":
            with self.state.lock:
                if self.state.ack is None:
                    self.fail_request("raw-before-provenance")
                    return
            if not isinstance(value, dict) or not isinstance(value.get("index"), int) or not isinstance(value.get("game"), dict):
                self.fail_request("invalid-raw-game")
                return
            index = value["index"]
            game = value["game"]
            with self.state.lock:
                if index in self.state.raw_games:
                    self.fail_request("duplicate-raw-game")
                    return
                self.state.raw_games[index] = game
            atomic_json(self.output_dir / "games" / f"game-{index:04d}.json", game)
            self.send_bytes(b"raw-game-durable\n", "text/plain")
            return
        if path == "/progress":
            with self.state.lock:
                if self.state.ack is None:
                    self.fail_request("progress-before-provenance")
                    return
                if isinstance(value, dict):
                    self.state.progress.append(value)
            with (self.output_dir / "progress.jsonl").open("a", encoding="utf-8") as stream:
                stream.write(json.dumps(value, sort_keys=True, allow_nan=False) + "\n")
            self.send_bytes(b"ok\n", "text/plain")
            return
        if path == "/result":
            if not isinstance(value, dict):
                self.fail_request("invalid-result")
                return
            with self.state.lock:
                if self.state.ack is None:
                    self.fail_request("result-before-provenance")
                    return
                expected_games = self.config.get("expectedRawGames")
                if isinstance(expected_games, int) and len(self.state.raw_games) != expected_games:
                    self.fail_request("result-before-all-raw-games")
                    return
                self.state.result = value
                self.state.event.set()
            # This write occurs in the request handler before the controller
            # can observe the completion event.
            atomic_json(self.output_dir / "report.json", value)
            self.send_bytes(b"result-durable\n", "text/plain")
            return
        if path == "/error":
            category = value.get("errorCategory") if isinstance(value, dict) else None
            if not isinstance(category, str) or not category:
                category = "browser-error"
            with self.state.lock:
                self.state.error = {"category": category}
                self.state.event.set()
            atomic_json(self.output_dir / "failure.json", {"schema": "haitaka-anhoku-strength-first-step14-browser-failure-v1", "status": "failed", "category": category})
            self.send_bytes(b"recorded\n", "text/plain")
            return
        self.send_bytes(b"not found\n", "text/plain", 404)


def make_provenance(root: Path, mode: str, page: Path, worker: Path,
                    extra: Sequence[tuple[str, Path, str]]) -> dict[str, Any]:
    bindings = load_json(root / "execution-bindings.json")
    chrome = bindings["host"]
    package = root / "package"
    files: dict[str, dict[str, Any]] = {}
    routes: dict[str, str] = {}
    inputs = [("page", page, "/page.html"), ("worker", worker, "/worker.js"),
              ("modelA", package / "model/u000640.nnue", "/model.nnue"),
              ("wasm", package / "web-pkg/haitaka_wasm_bg.wasm", "/pkg/haitaka_wasm_bg.wasm"),
              ("glue", package / "web-pkg/haitaka_wasm.js", "/pkg/haitaka_wasm.js")]
    inputs.extend(extra)
    for name, path, route in inputs:
        if not path.is_file():
            fail(f"missing provenance file {name}: {path}")
        files[name] = artifact(path, root)
        routes[name] = route
    core = {"schema": "haitaka-anhoku-strength-first-step14-provenance-v1", "schemaVersion": 1,
            "finalizedBeforeBrowserLaunch": True, "files": files, "routes": routes,
            "source": {"experimentId": EXPERIMENT_ID,
                        "sourceIdentity": artifact(root / "source-identity.json", root),
                        "contract": artifact(root / "contract.json", root)},
            "execution": {"mode": mode, "browserExecutable": chrome["chromePath"],
                          "browserVersion": chrome.get("chromeVersion"), "workerCount": 1,
                          "concurrentGames": 1, "wasmPackageDirectory": "package/web-pkg"}}
    return {**core, "envelopeId": hashlib.sha256(stable_bytes(core)).hexdigest()}


def make_http_server(root: Path, config: dict[str, Any], page: Path, worker: Path,
                     extra_routes: Sequence[tuple[str, Path, str]], output_dir: Path) -> tuple[http.server.ThreadingHTTPServer, BrowserState]:
    state = BrowserState()
    routes: dict[str, tuple[bytes, str]] = {
        "/": (page.read_bytes(), "text/html; charset=utf-8"),
        "/page.html": (page.read_bytes(), "text/html; charset=utf-8"),
        "/config.json": (stable_bytes(config), "application/json"),
        "/worker.js": (worker.read_bytes(), "text/javascript; charset=utf-8"),
        "/model.nnue": ((root / "package/model/u000640.nnue").read_bytes(), "application/octet-stream"),
        "/pkg/haitaka_wasm.js": ((root / "package/web-pkg/haitaka_wasm.js").read_bytes(), "text/javascript; charset=utf-8"),
        "/pkg/haitaka_wasm_bg.wasm": ((root / "package/web-pkg/haitaka_wasm_bg.wasm").read_bytes(), "application/wasm"),
    }
    for route, path, content_type in extra_routes:
        routes[route] = (path.read_bytes(), content_type)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), BrowserHandler)
    server.browser_state = state  # type: ignore[attr-defined]
    server.browser_config = config  # type: ignore[attr-defined]
    server.browser_routes = routes  # type: ignore[attr-defined]
    server.browser_output_dir = output_dir  # type: ignore[attr-defined]
    return server, state


def run_browser(root: Path, mode: str, block: Optional[int] = None, attempt: Optional[int] = None,
                timeout_seconds: float = 3600.0) -> dict[str, Any]:
    schedule = load_json(root / "openings/schedule.json")
    if mode == "parity":
        page = root / "runtime/step11/strength_first_step11_browser_page.html"
        worker = root / "runtime/step11/strength_first_step11_browser_parity_worker.js"
        source = root / "inputs/s12/parity-positions.json"
        positions = load_json(source)["positions"]
        provenance = make_provenance(root, mode, page, worker, [("parityInput", source, "/parity-input.json")])
        config = {"mode": "parity", "budget": {"mode": "static-evaluation"}, "positions": positions, "provenance": provenance}
        extra = [("/parity-input.json", source, "application/json")]
        parent = root / "qualification/parity"
    elif mode == "smoke":
        page = root / "runtime/step11/strength_first_step11_browser_page.html"
        worker = root / "runtime/step11/strength_first_step11_browser_smoke_worker.js"
        positions = load_json(root / "qualification/smoke-positions.json")["positions"]
        source = root / "qualification/smoke-positions.json"
        provenance = make_provenance(root, mode, page, worker, [("smokeInput", source, "/smoke-input.json")])
        config = {"mode": "smoke", "budget": {"mode": "movetime", "movetimeMs": MOVETIME_MS, "nodes": None, "command": "movetime 1000"}, "positions": positions, "provenance": provenance}
        extra = [("/smoke-input.json", source, "application/json")]
        parent = root / "qualification/smoke"
    elif mode == "terminal":
        page = root / "package/strength_first_step14_browser_page.html"
        worker = root / "package/strength_first_step14_terminal_worker.js"
        source = root / "qualification/terminal-input.json"
        records = load_json(source)["records"]
        provenance = make_provenance(root, mode, page, worker, [("terminalInput", source, "/terminal-input.json")])
        config = {"schema": "haitaka-anhoku-strength-first-step14-terminal-config-v1", "records": records, "provenance": provenance}
        extra = [("/terminal-input.json", source, "application/json")]
        parent = root / "qualification/terminal"
    elif mode in {"rehearsal", "match"}:
        if block is None and mode == "match":
            fail("match mode requires block")
        fixture_only = mode == "rehearsal"
        if mode == "rehearsal":
            rows = [row for row in schedule["rows"] if row["globalPairIndex"] < 2]
            page = root / "package/strength_first_step14_browser_page.html"
            worker = root / "package/strength_first_step14_browser_worker.js"
            parent = root / "qualification/rehearsal"
        else:
            rows = [row for row in schedule["rows"] if row["block"] == block]
            page = root / "package/strength_first_step14_browser_page.html"
            worker = root / "package/strength_first_step14_browser_worker.js"
            if attempt is None:
                fail("match mode requires attempt")
            parent = root / "matches" / f"block-{block:02d}" / f"attempt-{attempt:02d}"
        provenance = make_provenance(root, mode, page, worker,
                                     [("schedule", root / "openings/schedule.json", "/schedule.json"),
                                      ("contract", root / "contract.json", "/contract.json")])
        config = {"schema": "haitaka-anhoku-strength-first-step14-browser-config-v1",
                  "fixtureOnly": fixture_only, "fixtureMaxPlies": 2 if fixture_only else None,
                  "maxPlies": 2 if fixture_only else MAX_PLIES, "games": rows,
                  "expectedRawGames": len(rows), "provenance": provenance}
        extra = [("/schedule.json", root / "openings/schedule.json", "application/json"),
                 ("/contract.json", root / "contract.json", "application/json")]
    else:
        fail(f"unknown browser mode {mode}")
    if parent.exists() and any(parent.iterdir()):
        fail(f"browser attempt output already exists: {parent}")
    raw_dir = parent / "raw"
    raw_dir.mkdir(parents=True, exist_ok=True)
    atomic_json(parent / "config.json", config)
    server, state = make_http_server(root, config, page, worker, extra, raw_dir)
    server_thread = threading.Thread(target=server.serve_forever, name=f"step14-http-{mode}", daemon=True)
    server_thread.start()
    url = f"http://127.0.0.1:{server.server_address[1]}/"
    chrome_path = load_json(root / "execution-bindings.json")["host"]["chromePath"]
    profile = parent / "chrome-profile"
    if profile.exists():
        fail(f"fresh Chrome profile already exists: {profile}")
    profile.mkdir(parents=True)
    stdout_path, stderr_path = raw_dir / "chrome.stdout.sealed", raw_dir / "chrome.stderr.sealed"
    argv = [chrome_path, "--headless=new", "--disable-gpu", "--no-sandbox", "--no-first-run",
            "--no-default-browser-check", "--disable-background-networking", "--disable-component-update",
            f"--user-data-dir={profile}", url]
    atomic_json(parent / "launch.json", {"argv": argv, "cwd": str(root), "url": url, "mode": mode})
    started = time.monotonic()
    process: Optional[subprocess.Popen[Any]] = None
    sampler: Optional[RssSampler] = None
    ownership: dict[str, Any] = {}
    cleanup: Optional[dict[str, Any]] = None
    error: Optional[dict[str, Any]] = None
    stdout_stream = stderr_stream = None
    try:
        stdout_stream, stderr_stream = stdout_path.open("wb"), stderr_path.open("wb")
        process = subprocess.Popen(argv, cwd=str(root), stdout=stdout_stream, stderr=stderr_stream, start_new_session=True)
        pgid = os.getpgid(process.pid)
        ownership = {"pid": process.pid, "processGroupId": pgid, "startTime": process_start_time(process.pid),
                     "argv": argv, "cwd": str(root), "origin": url, "profile": str(profile)}
        atomic_json(parent / "ownership.json", ownership)
        sampler = RssSampler(process.pid, pgid)
        sampler.start()
        if not state.event.wait(timeout_seconds):
            error = {"category": "timeout", "timeoutSeconds": timeout_seconds}
        with state.lock:
            if state.error is not None:
                error = copy.deepcopy(state.error)
    except (OSError, ContractError) as exc:
        error = {"category": "controller-launch"}
    finally:
        memory = sampler.stop() if sampler is not None else {"status": "unavailable", "sampleCount": 0}
        if process is not None:
            cleanup = signal_owned_group(process, ownership.get("processGroupId", process.pid), raw_dir)
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                if cleanup.get("status") != "cleanup-blocked":
                    cleanup["status"] = "cleanup-blocked"
        server.shutdown(); server.server_close(); server_thread.join(timeout=2)
        if stdout_stream is not None: stdout_stream.close()
        if stderr_stream is not None: stderr_stream.close()
        atomic_json(parent / "telemetry.json", {"schema": "haitaka-anhoku-strength-first-step14-telemetry-v1",
                                                 "memory": memory, "ownership": ownership, "cleanup": cleanup,
                                                 "routes": sorted(server.browser_routes), "elapsedSeconds": time.monotonic() - started})  # type: ignore[attr-defined]
        try:
            shutil.rmtree(profile)
        except OSError:
            pass
    if error is not None or state.result is None:
        category = error.get("category") if error else "missing-result"
        failure = {"schema": "haitaka-anhoku-strength-first-step14-browser-failure-v1", "status": "failed",
                   "mode": mode, "block": block, "attempt": attempt, "category": category,
                   "provenance": provenance, "ownership": ownership}
        atomic_json(parent / "failure.json", failure)
        return {"status": "failed", "failure": failure, "telemetry": load_json(parent / "telemetry.json"),
                "provenance": provenance, "outputDir": str(raw_dir), "elapsedSeconds": time.monotonic() - started}
    report = load_json(raw_dir / "report.json") if (raw_dir / "report.json").is_file() else state.result
    return {"status": "passed", "report": report, "telemetry": load_json(parent / "telemetry.json"),
            "provenance": provenance, "outputDir": str(raw_dir), "parentDir": str(parent),
            "elapsedSeconds": time.monotonic() - started}


def timing_summary(raw_games: Sequence[dict[str, Any]]) -> dict[str, Any]:
    groups: dict[str, list[float]] = {"A": [], "B": [], "combined": [], "terminal": []}
    for game in raw_games:
        for trace in game.get("searches", []):
            value = float(trace["elapsedMs"])
            if not math.isfinite(value) or value < 0:
                fail("non-finite search timing")
            if trace.get("gameoverReason") is None:
                groups.setdefault(trace["engine"], []).append(value); groups["combined"].append(value)
            else:
                groups["terminal"].append(value)
    def nearest(values: list[float], probability: float) -> Optional[float]:
        if not values: return None
        ordered = sorted(values)
        return ordered[max(0, min(len(ordered) - 1, int(math.ceil(probability * len(ordered))) - 1))]
    result: dict[str, Any] = {}
    for name, values in groups.items():
        result[name] = {"count": len(values), "p50": nearest(values, .50), "p95": nearest(values, .95),
                        "p99": nearest(values, .99), "max": max(values) if values else None}
    return result


def validate_timing(raw_games: Sequence[dict[str, Any]], telemetry: dict[str, Any]) -> dict[str, Any]:
    memory = telemetry.get("memory", {})
    if memory.get("status") != "observed" or int(memory.get("sampleCount", 0)) < 2:
        fail("RSS telemetry is not observed with coverage")
    if memory.get("peakResidentSetBytes", MEMORY_CAP_BYTES + 1) > MEMORY_CAP_BYTES:
        fail("owned Chrome RSS exceeded 8 GiB")
    timing = timing_summary(raw_games)
    for name in ("A", "B", "combined", "terminal"):
        if timing[name]["max"] is not None and timing[name]["max"] > 2000.0:
            fail(f"{name} search timing exceeded 2000 ms")
    for name in ("A", "B", "combined"):
        if timing[name]["p99"] is not None and timing[name]["p99"] > 1100.0:
            fail(f"{name} nonterminal p99 exceeded 1100 ms")
    return {"timingMs": timing, "memory": memory}


def load_raw_report(raw_dir: Path, expected_games: int) -> dict[str, Any]:
    report = load_json(raw_dir / "report.json")
    files = sorted((raw_dir / "games").glob("game-*.json"))
    if len(files) != expected_games or not isinstance(report.get("rawGames"), list) or len(report["rawGames"]) != expected_games:
        fail("raw game stream is incomplete")
    streamed = [load_json(path) for path in files]
    if streamed != report["rawGames"]:
        fail("raw report differs from durable per-game stream")
    return report


def replay_report(root: Path, raw_report: dict[str, Any], output_dir: Path, max_plies: int) -> dict[str, Any]:
    binary = safe_path(root, "tools/strength_first_step14_replay")
    if not binary.is_file() or not os.access(binary, os.X_OK):
        fail("Step 14 replay binary is missing or not executable")
    input_path, output_path = output_dir / "replay-input.jsonl", output_dir / "replay-output.jsonl"
    lines = []
    for raw in raw_report["rawGames"]:
        value = {key: raw[key] for key in ("globalGameIndex", "globalPairIndex", "block", "blockGameIndex", "leg", "openingId", "startSfen", "initialSideToMove", "aColor", "bColor", "blackEngine", "whiteEngine", "moves", "plies", "rawEnd", "rawWinner", "rawResult")}
        value["schema"] = "haitaka-anhoku-strength-first-step14-replay-input-v1"
        lines.append(json.dumps(value, sort_keys=True, ensure_ascii=False, allow_nan=False))
    atomic_bytes(input_path, ("\n".join(lines) + "\n").encode())
    completed = subprocess.run([str(binary), "--input", str(input_path), "--output", str(output_path), "--max-plies", str(max_plies)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=900, check=False)
    receipts = []
    if output_path.is_file():
        for line in output_path.read_text(encoding="utf-8").splitlines():
            if line.strip(): receipts.append(json.loads(line))
    if completed.returncode != 0 or len(receipts) != len(lines) or any(item.get("status") != "passed" for item in receipts):
        fail("strict Step 14 replay failed")
    result = {"schema": "haitaka-anhoku-strength-first-step14-replay-batch-v1", "status": "passed",
              "maxPlies": max_plies, "records": receipts, "input": artifact(input_path, root),
              "output": artifact(output_path, root)}
    atomic_json(output_dir / "replay.json", result)
    return result


def accept_raw_block(root: Path, raw_dir: Path, block: int, attempt: int, fixture_max_plies: Optional[int] = None) -> dict[str, Any]:
    expected_games = 4 if fixture_max_plies is not None else GAMES_PER_BLOCK
    report = load_raw_report(raw_dir, expected_games)
    replay = replay_report(root, report, raw_dir.parent / "replay", fixture_max_plies or MAX_PLIES)
    schedule = load_json(root / "openings/schedule.json")
    envelope = report.get("provenance", {}).get("envelopeId")
    if fixture_max_plies is not None:
        schedule = {**schedule, "rows": schedule["rows"][:expected_games]}
    validation = validate_block(report, replay, schedule, 0 if fixture_max_plies is not None else block, envelope, fixture_max_plies=fixture_max_plies)
    timing = validate_timing(report["rawGames"], load_json(raw_dir.parent / "telemetry.json"))
    validation.update({"attempt": attempt, "fixtureOnly": fixture_max_plies is not None, "timing": timing,
                       "rawReport": artifact(raw_dir / "report.json", root), "replayReport": artifact(raw_dir.parent / "replay/replay.json", root),
                       "telemetry": artifact(raw_dir.parent / "telemetry.json", root)})
    atomic_json(raw_dir.parent / "validation.json", validation)
    return validation


def require_execution_grant(root: Path) -> None:
    value = load_json(root / "authorization.json")
    if value.get("experimentId") != EXPERIMENT_ID or value.get("executionGrant") is not True:
        fail("qualification/match execution requires a matching Step 14 executionGrant:true authorization")
    if value.get("gpuRentalAuthorized") is not False or value.get("trainingAuthorized") is not False:
        fail("authorization violates CPU-only/no-training contract")


def host_inventory(root: Path) -> dict[str, Any]:
    bindings = load_json(root / "execution-bindings.json")
    chrome_path = Path(bindings["host"]["chromePath"])
    if not chrome_path.is_file():
        fail("bound Chrome executable is missing")
    code, chrome_version, chrome_error = run_capture([str(chrome_path), "--version"])
    if code != 0 or not chrome_version.strip():
        fail(f"cannot query bound Chrome: {chrome_error}")
    _, model, _ = run_capture(["sysctl", "-n", "hw.model"])
    _, hardware_profile, _ = run_capture(["system_profiler", "SPHardwareDataType"])
    _, cpus, _ = run_capture(["sysctl", "-n", "hw.ncpu"])
    _, memory, _ = run_capture(["sysctl", "-n", "hw.memsize"])
    _, sw_vers, _ = run_capture(["sw_vers"])
    _, power, _ = run_capture(["pmset", "-g", "batt"])
    _, uptime, _ = run_capture(["uptime"])
    python_path = Path(sys.executable).resolve()
    disk = shutil.disk_usage(root)
    rows = process_snapshot()
    suspicious = [row for row in rows if any(token in row["command"].lower() for token in ("wasm-pack", "cargo test", "cargo run", "strength_first_step14", "haitaka_learn")) and "browser.py" not in row["command"]]
    inventory = {"schema": "haitaka-anhoku-strength-first-step14-host-inventory-v1", "experimentId": EXPERIMENT_ID,
                 "observedAt": utc_now(),
                 "host": {"hostname": socket.gethostname(), "model": model.strip(), "architecture": platform.machine(),
                          "python": {"path": str(python_path), "version": platform.python_version(), "sha256": sha256(python_path)},
                          "os": {"platform": platform.platform(), "macVersion": platform.mac_ver()[0]},
                          "cpuCount": int(cpus.strip() or 0), "memoryBytes": int(memory.strip() or 0),
                          "chrome": {"path": str(chrome_path), "version": chrome_version.strip(), "sha256": sha256(chrome_path)},
                          "swVers": sw_vers.strip(), "uptime": uptime.strip()},
                 "power": {"raw": power.strip(), "acPower": "AC Power" in power},
                 "storage": {"root": str(root), "freeBytes": disk.free, "totalBytes": disk.total},
                 "processes": {"suspiciousComputeJobs": suspicious, "unrelatedJobsLeftRunning": True},
                 "verification": {"isMac": platform.system() == "Darwin", "isAppleSilicon": platform.machine() == "arm64",
                                  "isMacBookPro": "Model Name: MacBook Pro" in hardware_profile, "isAcPowered": "AC Power" in power,
                                  "chromeAvailable": True, "pythonAvailable": True, "freeStorageAtLeast16GiB": disk.free >= MIN_FREE_BYTES,
                                  "quiescentCompute": not suspicious}}
    atomic_json(root / "preflight/host-inventory.json", inventory)
    if not all(inventory["verification"].values()):
        fail("MBP host inventory does not satisfy Step 14 prerequisites")
    bindings["host"]["chromeVersion"] = chrome_version.strip()
    bindings["host"]["chromeSha256"] = sha256(chrome_path)
    bindings["host"]["pythonVersion"] = platform.python_version()
    bindings["host"]["pythonSha256"] = sha256(python_path)
    atomic_json(root / "execution-bindings.json", bindings)
    return inventory


def verify_bound_artifact(root: Path, binding: dict[str, Any], label: str) -> dict[str, Any]:
    relative = binding.get("path")
    if not isinstance(relative, str):
        fail(f"{label} path is missing")
    path = safe_path(root, relative)
    if not path.is_file() or path.stat().st_size != binding.get("bytes") or sha256(path) != binding.get("sha256"):
        fail(f"{label} hash binding differs")
    return artifact(path, root)


def input_rehash_gate(root: Path) -> dict[str, Any]:
    bindings = load_json(root / "execution-bindings.json")
    checked = {name: verify_bound_artifact(root, value, f"input {name}") for name, value in bindings["inputs"].items()}
    checked_package = {name: verify_bound_artifact(root, value, f"package {name}") for name, value in bindings["package"].items()}
    schedule = load_json(root / "openings/schedule.json")
    rows = schedule.get("rows")
    if not isinstance(rows, list) or len(rows) != GAME_COUNT or [row.get("globalGameIndex") for row in rows] != list(range(GAME_COUNT)):
        fail("frozen browser schedule is not exactly 128 ordered games")
    for pair in range(PAIR_COUNT):
        legs = rows[2 * pair:2 * pair + 2]
        if [row.get("globalPairIndex") for row in legs] != [pair, pair] or [row.get("leg") for row in legs] != [0, 1]:
            fail("schedule pair/leg identity differs")
        if legs[0].get("startSfen") != legs[1].get("startSfen"):
            fail("pair legs do not use the same SFEN")
        if legs[0].get("aColor") != "black" or legs[1].get("aColor") != "white":
            fail("engine-seat swap differs")
    if sha256(root / "openings/finalist.tsv") != TSV_HASH or (root / "openings/finalist.tsv").stat().st_size != TSV_BYTES:
        fail("finalist TSV transform differs")
    selection = load_json(root / "inputs/s12/training-selection.json")
    if selection.get("selectedUpdate") != 640 or selection.get("selectedCheckpoint", {}).get("path") != "checkpoints/u000640.pt":
        fail("Step 12 selected update is not u640")
    if selection.get("status") not in {"passed", "selected-validation-only"} or selection.get("flags", {}).get("outcomesOpened") is not False:
        fail("Step 12 selection evidence is not sealed")
    parity = load_json(root / "inputs/s12/parity-qualification.json")
    if parity.get("status") != "passed" or parity.get("gamesLaunched") != 0:
        fail("Step 12 parity qualification evidence differs")
    if parity.get("flags", {}).get("outcomesOpened") is not False:
        fail("Step 12 parity evidence opened outcomes")
    receipt = {"schema": "haitaka-anhoku-strength-first-step14-input-rehash-v1", "status": "passed",
               "inputs": checked, "package": checked_package, "scheduleRows": len(rows),
               "openingTsv": artifact(root / "openings/finalist.tsv", root),
               "sourceGroup": "early-selected/leg-0/pairIndex-0..63",
               "exclusionEvidence": {"selection": artifact(root / "inputs/s12/training-selection.json", root),
                                     "s12Parity": artifact(root / "inputs/s12/parity-qualification.json", root),
                                     "s13Replay": artifact(root / "inputs/s13/replay-summary.json", root)}}
    atomic_json(root / "preflight/input-rehash.json", receipt)
    return receipt


def validate_parity(root: Path, report: dict[str, Any], telemetry: dict[str, Any], provenance: dict[str, Any]) -> dict[str, Any]:
    if report.get("schema") != "haitaka-anhoku-strength-first-browser-parity-v1" or report.get("status") != "passed":
        fail("browser parity report is not passed")
    positions = report.get("positions")
    reference = load_json(root / "inputs/s12/reference-u000640.json")["positions"]
    native = load_json(root / "inputs/s12/native-u000640.json")["positions"]
    wasm_receipt = load_json(root / "inputs/s12/wasm-u000640.json")
    if report.get("provenance", {}).get("envelopeId") != provenance.get("envelopeId") or not isinstance(positions, list) or len(positions) != 1024:
        fail("browser parity identity/cardinality differs")
    if wasm_receipt.get("status") != "passed" or wasm_receipt.get("positions", [{}])[0].get("score") is None:
        fail("S12 u640 WASM receipt is incomplete")
    ref_by, native_by, browser_by = ({row["id"]: row for row in values} for values in (reference, native, positions))
    mismatches = []
    for index in range(1024):
        row_id = f"r1a-{index:05d}"
        if row_id not in ref_by or row_id not in native_by or row_id not in browser_by:
            fail(f"missing parity row {row_id}")
        if ref_by[row_id].get("sfen") != native_by[row_id].get("sfen") or ref_by[row_id].get("sfen") != browser_by[row_id].get("sfen"):
            fail(f"parity SFEN mismatch {row_id}")
        if ref_by[row_id].get("integerScore") != native_by[row_id].get("score") or native_by[row_id].get("score") != browser_by[row_id].get("score"):
            mismatches.append(row_id)
    if mismatches or load_json(root / "inputs/s12/native-u000640.json").get("fullRefreshEqualsIncremental") is not True:
        fail(f"browser parity mismatches={len(mismatches)}")
    receipt = {"schema": "haitaka-anhoku-strength-first-step14-parity-v1", "status": "passed", "positions": 1024,
               "integerReferenceNativeBrowserScoreMismatchCount": len(mismatches), "nativeFullRefreshEqualsIncremental": True,
               "browserReport": artifact(root / "qualification/parity/raw/report.json", root),
               "telemetry": artifact(root / "qualification/parity/telemetry.json", root), "provenanceEnvelopeId": provenance["envelopeId"]}
    atomic_json(root / "qualification/parity.json", receipt)
    return receipt


def validate_smoke(root: Path, report: dict[str, Any], telemetry: dict[str, Any], provenance: dict[str, Any]) -> dict[str, Any]:
    if report.get("schema") != "haitaka-anhoku-strength-first-browser-smoke-v1" or report.get("status") != "passed" or report.get("provenance", {}).get("envelopeId") != provenance.get("envelopeId"):
        fail("browser smoke report identity differs")
    searches = report.get("searches")
    if not isinstance(searches, list) or len(searches) != SMOKE_SEARCHES:
        fail("browser smoke does not contain exactly 16 isolated searches")
    if telemetry.get("memory", {}).get("status") != "observed":
        fail("browser smoke RSS telemetry is missing")
    values = []
    for trace in searches:
        if trace.get("bestMove") in {None, "", "resign"} or trace.get("rootResultSchema") != ROOT_RESULT_SCHEMA or trace.get("playMoveWasSearched") is not True:
            fail("browser smoke contains an unqualified search")
        if trace.get("gameoverReason") is not None or trace.get("missingMove") or trace.get("emergencyFallbackUsed") or trace.get("unsearchedMove"):
            fail("browser smoke contains a terminal/fallback search")
        values.append(float(trace["elapsedMs"]))
    if not all(math.isfinite(value) and value >= 0 for value in values):
        fail("browser smoke timing is not finite")
    ordered = sorted(values)
    p99 = ordered[max(0, min(len(ordered) - 1, int(math.ceil(.99 * len(ordered))) - 1))]
    timing = {"count": len(values), "p50": ordered[len(values) // 2], "p99": p99, "max": max(values)}
    if p99 > 1100 or max(values) > 2000:
        fail("browser smoke timing guardrail failed")
    receipt = {"schema": "haitaka-anhoku-strength-first-step14-smoke-v1", "status": "passed", "searches": len(searches),
               "positions": 8, "evaluatorCounts": dict(Counter(trace.get("engine") for trace in searches)),
               "searchTimingMs": timing, "telemetry": telemetry["memory"],
               "browserReport": artifact(root / "qualification/smoke/raw/report.json", root), "provenanceEnvelopeId": provenance["envelopeId"]}
    atomic_json(root / "qualification/smoke.json", receipt)
    return receipt


def make_terminal_input(root: Path) -> dict[str, Any]:
    records = [
        {"gameIndex": 0, "openingId": "terminal-no-legal", "startSfen": "8k/6G2/7B1/9/9/9/9/9/K8 b R 1", "moves": ["R*1b"], "expectedReason": "no-legal-move"},
        {"gameIndex": 1, "openingId": "terminal-repetition", "startSfen": "lns2k1nl/1rg2gsb1/p1pppp1p1/1p4p2/4P3p/P8/1PPPGPPPP/1B1G1K1RL/LNS3SN1 w - 17", "moves": ["2c2d", "5g4f", "4a5a", "6g6f", "2b4d", "6f6e", "8d8e", "4f4e", "4d2b", "4e4f", "2b4d", "4f4e", "4d2b", "4e4f", "2b4d", "4f4e", "4d2b", "4e4f", "2b4d"], "expectedReason": "repetition-draw"},
        {"gameIndex": 2, "openingId": "terminal-perpetual-black", "startSfen": "4k4/9/5R3/9/9/9/9/9/K8 b - 1", "moves": ["4c5c", "5a4a", "5c4c", "4a5a", "4c5c", "5a4a", "5c4c", "4a5a", "4c5c", "5a4a", "5c4c", "4a5a"], "expectedReason": "perpetual-check-loss"},
        {"gameIndex": 3, "openingId": "terminal-perpetual-white", "startSfen": "k8/9/9/9/9/9/5r3/9/4K4 w - 1", "moves": ["4g5g", "5i4i", "5g4g", "4i5i", "4g5g", "5i4i", "5g4g", "4i5i", "4g5g", "5i4i", "5g4g", "4i5i"], "expectedReason": "perpetual-check-loss"},
    ]
    value = {"schema": "haitaka-anhoku-strength-first-step14-terminal-input-v1", "status": "sealed-input", "records": records, "queries": len(records) * 2, "scoredGames": 0}
    atomic_json(root / "qualification/terminal-input.json", value)
    return value


def build_forecast(root: Path, smoke: dict[str, Any]) -> dict[str, Any]:
    historical = load_json(root / "inputs/b11/historical-browser.json")
    searches = historical.get("summary", {}).get("nodes", {}).get("all", {}).get("searches")
    games = historical.get("summary", {}).get("score", {}).get("games")
    if not isinstance(searches, int) or not isinstance(games, int) or searches <= 0 or games <= 0:
        fail("historical browser timing evidence is incomplete")
    p99 = float(smoke["searchTimingMs"]["p99"])
    raw = searches * p99 / 1000.0
    startup = 8 * 30.0
    inflated = (raw + startup) * 1.5
    total = inflated + BLOCK_CAP_SECONDS
    result = {"schema": "haitaka-anhoku-strength-first-step14-forecast-v1", "status": "passed" if total < MATCH_CAP_SECONDS else "failed",
              "historicalSearches": searches, "historicalGames": games, "smokeP99Ms": p99,
              "arithmetic": {"expectedSearchesFor128Games": searches, "rawSearchSeconds": raw, "coldStartupReserveSeconds": startup,
                              "inflation": 1.5, "inflatedSeconds": inflated, "retryReserveSeconds": BLOCK_CAP_SECONDS,
                              "forecastIncludingRetryReserveSeconds": total, "matchCapSeconds": MATCH_CAP_SECONDS}}
    atomic_json(root / "qualification/forecast.json", result)
    if result["status"] != "passed":
        fail("timing forecast does not fit match cap")
    return result


def offline_closeout_rehearsal(root: Path) -> dict[str, Any]:
    fixture_root = root / "qualification/closeout-fixture"
    if fixture_root.exists() and (fixture_root / "closeout-idempotence.json").is_file():
        fail("closeout rehearsal fixture root already contains a completed run")
    fixture_root.mkdir(parents=True, exist_ok=True)
    atomic_json(fixture_root / "openings/schedule.json", load_json(root / "openings/schedule.json"))
    schedule = load_json(root / "openings/schedule.json")
    for block in range(BLOCK_COUNT):
        block_rows = [row for row in schedule["rows"] if row["block"] == block]
        games = []
        for row in block_rows:
            result = "a-win" if row["globalGameIndex"] % 3 == 0 else ("b-win" if row["globalGameIndex"] % 3 == 1 else "draw")
            games.append({**row, "schema": "haitaka-anhoku-strength-first-step14-canonical-game-v1", "moves": [], "plies": 0,
                          "winner": None if result == "draw" else ("A" if result == "a-win" else "B"), "result": result,
                          "terminationReason": "repetition-draw" if result == "draw" else "no-legal-move", "capped": False,
                          "searches": []})
        validation = {"schema": "haitaka-anhoku-strength-first-step14-block-validation-v1", "status": "passed", "block": block, "games": games}
        path = fixture_root / "matches" / f"block-{block:02d}" / "attempt-01" / "validation.json"
        atomic_json(path, validation)
        atomic_json(path.parent / "accepted.json", {"schema": "haitaka-anhoku-strength-first-step14-accepted-attempt-v1", "status": "accepted", "block": block, "attempt": 1, "validation": artifact(path, fixture_root)})
    first = closeout(fixture_root)
    second = closeout(fixture_root)
    idem = load_json(fixture_root / "closeout-idempotence.json")
    if idem.get("status") != "passed" or first.get("dispatch") != second.get("dispatch") or second.get("dispatch") != {"browsers": 0, "replays": 0, "searches": 0, "games": 0}:
        fail("offline closeout idempotence rehearsal failed")
    receipt = {"schema": "haitaka-anhoku-strength-first-step14-closeout-rehearsal-v1", "status": "passed",
               "fixtureRoot": str(fixture_root.relative_to(root)), "firstCloseout": artifact(fixture_root / "closeout.json", root),
               "secondCloseout": artifact(fixture_root / "closeout.json", root), "idempotence": artifact(fixture_root / "closeout-idempotence.json", root),
               "dispatch": second["dispatch"]}
    atomic_json(root / "qualification/closeout-rehearsal.json", receipt)
    return receipt


def qualify(root: Path) -> dict[str, Any]:
    require_execution_grant(root)
    started = time.monotonic()
    inventory = host_inventory(root)
    inputs = input_rehash_gate(root)
    fixture_receipt = fixtures(root)
    parity_job = run_browser(root, "parity", timeout_seconds=900.0)
    if parity_job["status"] != "passed": fail("browser parity job failed")
    parity = validate_parity(root, parity_job["report"], parity_job["telemetry"], parity_job["provenance"])
    smoke_positions = load_json(root / "inputs/s12/parity-positions.json")["positions"][:8]
    atomic_json(root / "qualification/smoke-positions.json", {"schema": "haitaka-anhoku-strength-first-step14-smoke-input-v1", "positions": smoke_positions})
    smoke_job = run_browser(root, "smoke", timeout_seconds=900.0)
    if smoke_job["status"] != "passed": fail("browser smoke job failed")
    smoke = validate_smoke(root, smoke_job["report"], smoke_job["telemetry"], smoke_job["provenance"])
    rehearsal_job = run_browser(root, "rehearsal", timeout_seconds=900.0)
    if rehearsal_job["status"] != "passed": fail("fixture-only browser rehearsal failed")
    rehearsal_validation = accept_raw_block(root, Path(rehearsal_job["outputDir"]), 0, 1, fixture_max_plies=2)
    rehearsal_receipt = {"schema": "haitaka-anhoku-strength-first-step14-rehearsal-v1", "status": "passed", "pairs": 2, "games": 4,
                         "searches": sum(len(game["searches"]) for game in load_raw_report(Path(rehearsal_job["outputDir"]), 4)["rawGames"]),
                         "validation": artifact(Path(rehearsal_job["parentDir"]) / "validation.json", root), "excludedFromScore": True}
    atomic_json(root / "qualification/rehearsal.json", rehearsal_receipt)
    terminal_input = make_terminal_input(root)
    terminal_job = run_browser(root, "terminal", timeout_seconds=600.0)
    if terminal_job["status"] != "passed": fail("terminal qualification job failed")
    terminal_report = terminal_job["report"]
    if terminal_report.get("schema") != "haitaka-anhoku-strength-first-step14-terminal-report-v1" or len(terminal_report.get("replays", [])) != 4:
        fail("terminal qualification report differs")
    for item in terminal_report["replays"]:
        for name in ("A", "B"):
            observed = item["observed"][name]
            if observed["gameoverReason"] != item["expectedReason"] or observed["bestMove"] != "resign" or observed["elapsedMs"] > 2000:
                fail("terminal qualification mapping differs")
    terminal_receipt = {"schema": "haitaka-anhoku-strength-first-step14-terminal-v1", "status": "passed", "records": 4,
                        "queries": 8, "browserReport": artifact(root / "qualification/terminal/raw/report.json", root),
                        "telemetry": artifact(root / "qualification/terminal/telemetry.json", root), "input": artifact(root / "qualification/terminal-input.json", root)}
    atomic_json(root / "qualification/terminal.json", terminal_receipt)
    rehearsal_closeout = offline_closeout_rehearsal(root)
    forecast = build_forecast(root, smoke)
    receipt = {"schema": "haitaka-anhoku-strength-first-step14-qualification-v1", "status": "passed", "experimentId": EXPERIMENT_ID,
               "host": artifact(root / "preflight/host-inventory.json", root), "inputs": artifact(root / "preflight/input-rehash.json", root),
               "fixtures": artifact(root / "fixtures/report.json", root), "parity": artifact(root / "qualification/parity.json", root),
               "smoke": artifact(root / "qualification/smoke.json", root), "rehearsal": artifact(root / "qualification/rehearsal.json", root),
               "terminal": artifact(root / "qualification/terminal.json", root), "closeoutRehearsal": artifact(root / "qualification/closeout-rehearsal.json", root),
               "forecast": artifact(root / "qualification/forecast.json", root),
               "browserLaunches": 4, "searches": 16 + rehearsal_receipt["searches"] + terminal_receipt["queries"], "games": 4,
               "scoredGames": 0, "elapsedSeconds": time.monotonic() - started,
               "trainingOrGpuAttempted": False}
    atomic_json(root / "qualification/receipt.json", receipt)
    return phase_receipt(root, "qualify", "passed", qualification=artifact(root / "qualification/receipt.json", root),
                         browserLaunches=receipt["browserLaunches"], searches=receipt["searches"], games=receipt["games"])


def match_gate(root: Path) -> dict[str, Any]:
    verify_bound_artifact(root, load_json(root / "manifest.json")["contract"], "manifest contract")
    verify_bound_artifact(root, load_json(root / "manifest.json")["schedule"], "manifest schedule")
    for relative in ("fixtures/report.json", "qualification/receipt.json", "qualification/parity.json", "qualification/smoke.json", "qualification/rehearsal.json", "qualification/terminal.json", "qualification/closeout-rehearsal.json", "qualification/forecast.json"):
        value = load_json(root / relative)
        if value.get("status") != "passed":
            fail(f"required gate is not passed: {relative}")
    receipt = {"schema": "haitaka-anhoku-strength-first-step14-launch-gate-v1", "status": "passed", "experimentId": EXPERIMENT_ID,
               "contract": artifact(root / "contract.json", root), "schedule": artifact(root / "openings/schedule.json", root),
               "sourceIdentity": artifact(root / "source-identity.json", root), "executionBindings": artifact(root / "execution-bindings.json", root),
               "fixtures": artifact(root / "fixtures/report.json", root), "qualification": artifact(root / "qualification/receipt.json", root),
               "browserLaunches": 0, "searches": 0, "games": 0, "productionFixtureOnly": False,
               "trainingOrGpuAttempted": False}
    atomic_json(root / "launch-gate.json", receipt)
    return phase_receipt(root, "match-gate", "passed", launchGate=artifact(root / "launch-gate.json", root), browserLaunches=0, searches=0, games=0)


@contextmanager
def controller_lock(root: Path) -> Iterator[None]:
    path = root / "matches/controller.lock"
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a+") as handle:
        try:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            fail("another Step 14 controller owns the execution lock")
        yield


def run_matches(root: Path) -> dict[str, Any]:
    require_execution_grant(root)
    gate = load_json(root / "launch-gate.json")
    if gate.get("status") != "passed":
        fail("match gate has not passed")
    with controller_lock(root):
        started = time.monotonic()
        ledger_path = root / "matches/ledger.jsonl"
        ledger_path.parent.mkdir(parents=True, exist_ok=True)
        status_path = root / "matches/status.json"
        status = load_json(status_path) if status_path.is_file() else {"experimentId": EXPERIMENT_ID, "acceptedBlocks": [], "excludedAttempts": []}
        accepted_blocks = {int(item["block"]): int(item["attempt"]) for item in status.get("acceptedBlocks", []) if isinstance(item, dict) and isinstance(item.get("block"), int)}
        total_charged = float(status.get("consumedMatchSeconds", 0.0) or 0.0)
        if total_charged >= MATCH_CAP_SECONDS:
            fail("match time cap is exhausted")
        def ledger(event: dict[str, Any]) -> None:
            with ledger_path.open("a", encoding="utf-8") as stream:
                stream.write(json.dumps({"recordedAt": utc_now(), **event}, sort_keys=True, allow_nan=False) + "\n")
        live = {"phase": "supervisor", "block": None, "attempt": None, "pid": os.getpid(), "startedAt": utc_now()}
        heartbeat_stop = threading.Event()
        def heartbeat() -> None:
            while not heartbeat_stop.wait(30.0):
                atomic_json(root / "matches/heartbeat.json", {"schema": "haitaka-anhoku-strength-first-step14-heartbeat-v1",
                                                               "experimentId": EXPERIMENT_ID, **live, "updatedAt": utc_now(),
                                                               "elapsedSeconds": max(0.0, time.monotonic() - started)})
        heartbeat_thread = threading.Thread(target=heartbeat, name="step14-heartbeat", daemon=True)
        atomic_json(root / "matches/heartbeat.json", {"schema": "haitaka-anhoku-strength-first-step14-heartbeat-v1",
                                                       "experimentId": EXPERIMENT_ID, **live, "updatedAt": utc_now(), "elapsedSeconds": 0.0})
        heartbeat_thread.start()
        ledger({"event": "supervisor-start", "pid": os.getpid(), "startedMonotonic": started})
        for block in range(BLOCK_COUNT):
            if block in accepted_blocks:
                continue
            block_dir = root / "matches" / f"block-{block:02d}"
            existing = sorted(block_dir.glob("attempt-*/")) if block_dir.exists() else []
            attempts = [int(path.name.split("-")[-1]) for path in existing if re.fullmatch(r"attempt-\d+", path.name)]
            attempt = max(attempts, default=0) + 1
            if attempt > 2:
                fail(f"block {block} has exhausted its two attempts")
            attempt_dir = block_dir / f"attempt-{attempt:02d}"
            if attempt_dir.exists():
                fail(f"block {block} attempt directory already exists")
            remaining = MATCH_CAP_SECONDS - total_charged
            if remaining <= 0:
                fail("match time cap exhausted before next block")
            live.update({"phase": "block", "block": block, "attempt": attempt})
            ledger({"event": "attempt-start", "block": block, "attempt": attempt, "remainingSeconds": remaining})
            attempt_started = time.monotonic()
            job = run_browser(root, "match", block=block, attempt=attempt, timeout_seconds=min(BLOCK_CAP_SECONDS, remaining))
            elapsed = time.monotonic() - attempt_started
            total_charged += elapsed
            atomic_json(attempt_dir / "exit.json", {"schema": "haitaka-anhoku-strength-first-step14-attempt-exit-v1", "status": job["status"],
                                                       "block": block, "attempt": attempt, "elapsedSeconds": elapsed,
                                                       "category": job.get("failure", {}).get("category") if job["status"] != "passed" else "accepted-candidate"})
            if job["status"] != "passed":
                status.setdefault("excludedAttempts", []).append({"block": block, "attempt": attempt, "status": "excluded", "category": job.get("failure", {}).get("category", "browser-failure"), "chargedSeconds": elapsed})
                status["consumedMatchSeconds"] = total_charged
                atomic_json(status_path, status)
                ledger({"event": "attempt-excluded", "block": block, "attempt": attempt, "chargedSeconds": elapsed, "category": job.get("failure", {}).get("category", "browser-failure")})
                fail("scored browser attempt failed; preserved evidence and stopped")
            try:
                validation = accept_raw_block(root, Path(job["outputDir"]), block, attempt)
            except Exception:
                status.setdefault("excludedAttempts", []).append({"block": block, "attempt": attempt, "status": "excluded", "category": "validation-failure", "chargedSeconds": elapsed})
                status["consumedMatchSeconds"] = total_charged
                atomic_json(status_path, status)
                ledger({"event": "attempt-excluded", "block": block, "attempt": attempt, "chargedSeconds": elapsed, "category": "validation-failure"})
                raise
            accepted = {"schema": "haitaka-anhoku-strength-first-step14-accepted-attempt-v1", "status": "accepted", "block": block, "attempt": attempt,
                        "raw": artifact(Path(job["outputDir"]) / "report.json", root),
                        "validation": artifact(Path(job["parentDir"]) / "validation.json", root),
                        "replay": artifact(Path(job["parentDir"]) / "replay/replay.json", root),
                        "telemetry": artifact(Path(job["parentDir"]) / "telemetry.json", root), "chargedSeconds": elapsed,
                        "acceptedAt": utc_now()}
            atomic_json(attempt_dir / "accepted.json", accepted)
            accepted_blocks[block] = attempt
            status["acceptedBlocks"] = [{"block": index, "attempt": accepted_blocks[index]} for index in sorted(accepted_blocks)]
            status["consumedMatchSeconds"] = total_charged
            status["remainingMatchSeconds"] = MATCH_CAP_SECONDS - total_charged
            status["updatedAt"] = utc_now()
            atomic_json(status_path, status)
            ledger({"event": "attempt-accepted", "block": block, "attempt": attempt, "chargedSeconds": elapsed, "remainingSeconds": MATCH_CAP_SECONDS - total_charged, "validation": artifact(Path(job["parentDir"]) / "validation.json", root)})
        heartbeat_stop.set()
        heartbeat_thread.join(timeout=2.0)
        live.update({"phase": "complete", "block": None, "attempt": None})
        atomic_json(root / "matches/heartbeat.json", {"schema": "haitaka-anhoku-strength-first-step14-heartbeat-v1",
                                                       "experimentId": EXPERIMENT_ID, **live, "updatedAt": utc_now(),
                                                       "elapsedSeconds": max(0.0, time.monotonic() - started)})
        result = {"schema": "haitaka-anhoku-strength-first-step14-match-run-v1", "status": "passed", "experimentId": EXPERIMENT_ID,
                  "acceptedBlocks": len(accepted_blocks), "scoredGames": len(accepted_blocks) * GAMES_PER_BLOCK,
                  "browserLaunches": len(accepted_blocks), "searches": sum(len(game.get("searches", [])) for block in accepted_blocks for game in load_json(root / "matches" / f"block-{block:02d}" / f"attempt-{accepted_blocks[block]:02d}" / "validation.json")["games"]),
                  "games": len(accepted_blocks) * GAMES_PER_BLOCK, "chargedSeconds": total_charged, "remainingSeconds": MATCH_CAP_SECONDS - total_charged}
        atomic_json(root / "matches/run-receipt.json", result)
        ledger({"event": "supervisor-complete", **result})
        return phase_receipt(root, "run-matches", "passed", runReceipt=artifact(root / "matches/run-receipt.json", root), browserLaunches=result["browserLaunches"], searches=result["searches"], games=result["games"])


def closeout(root: Path) -> dict[str, Any]:
    started = time.monotonic()
    games: list[dict[str, Any]] = []
    refs = []
    for block in range(BLOCK_COUNT):
        candidates = sorted((root / "matches" / f"block-{block:02d}").glob("attempt-*/accepted.json"))
        if len(candidates) != 1:
            fail(f"block {block} does not have exactly one accepted attempt")
        accepted = load_json(candidates[0])
        if accepted.get("status") != "accepted" or accepted.get("block") != block:
            fail(f"block {block} accepted receipt differs")
        validation_ref = accepted.get("validation")
        if not isinstance(validation_ref, dict) or not isinstance(validation_ref.get("path"), str):
            fail(f"block {block} accepted receipt has no validation reference")
        path = safe_path(root, validation_ref["path"])
        if sha256(path) != validation_ref.get("sha256"):
            fail(f"block {block} validation reference hash differs")
        validation = load_json(path)
        if validation.get("status") != "passed" or validation.get("block") != block:
            fail(f"block {block} validation differs")
        block_games = validation.get("games")
        if not isinstance(block_games, list) or len(block_games) != GAMES_PER_BLOCK:
            fail(f"block {block} canonical game count differs")
        expected_indexes = list(range(block * GAMES_PER_BLOCK, (block + 1) * GAMES_PER_BLOCK))
        if [game.get("globalGameIndex") for game in block_games] != expected_indexes:
            fail(f"block {block} canonical game ordering differs")
        for game in block_games:
            if game.get("block") != block or game.get("globalPairIndex") != game.get("globalGameIndex") // 2:
                fail(f"block {block} canonical game identity differs")
        games.extend(block_games)
        refs.append(artifact(path, root))
    if len({game["globalGameIndex"] for game in games}) != GAME_COUNT:
        fail("closeout canonical schedule has duplicate games")
    analysis = analyze(games)
    atomic_json(root / "analysis.json", analysis)
    report = (f"# Step 14 browser finalist check\n\n"
              f"Result: {analysis['score']['wins']}W/{analysis['score']['draws']}D/{analysis['score']['losses']}L; "
              f"score {analysis['score']['exact']} ({analysis['score']['decimal']:.6f}).\n\n"
              f"Capped games: {analysis['cappedSensitivity']['cappedGames']}; routing: {analysis['routing']}.\n\n"
              "Qualification games: 4; scored games: 128.\n\n"
              "Handcrafted remains production. Confirmation, deployment, GPU rental, training, commit, and push remain unauthorized.\n")
    atomic_bytes(root / "report.md", report.encode())
    index_items = ["contract.json", "authorization.json", "source-identity.json", "execution-bindings.json", "manifest.json",
                   "openings/finalist.tsv", "openings/schedule.json", "fixtures/report.json", "qualification/receipt.json",
                   "qualification/parity.json", "qualification/smoke.json", "qualification/rehearsal.json", "qualification/terminal.json",
                   "qualification/forecast.json", "qualification/closeout-rehearsal.json", "launch-gate.json", "analysis.json", "report.md"]
    index_items.extend(ref["path"] for ref in refs)
    index = {"schema": "haitaka-anhoku-strength-first-step14-artifact-index-v1", "status": "passed", "experimentId": EXPERIMENT_ID,
             "excluded": ["artifact-index.json", "closeout.json", "closeout-idempotence.json"],
             "artifacts": {relative: artifact(root / relative, root) for relative in index_items if (root / relative).is_file()}}
    atomic_json(root / "artifact-index.json", index)
    substantive = {"analysisSha256": sha256(root / "analysis.json"), "reportSha256": sha256(root / "report.md"),
                   "validationRefs": refs, "artifactIndexSha256": sha256(root / "artifact-index.json")}
    closeout_value = {"schema": "haitaka-anhoku-strength-first-step14-closeout-v1", "status": "complete",
                      "experimentId": EXPERIMENT_ID, **substantive, "dispatch": {"browsers": 0, "replays": 0, "searches": 0, "games": 0},
                      "elapsedSeconds": time.monotonic() - started}
    previous = load_json(root / "closeout.json") if (root / "closeout.json").is_file() else None
    atomic_json(root / "closeout.json", closeout_value)
    atomic_json(root / "closeout-idempotence.json", {
        "schema": "haitaka-anhoku-strength-first-step14-closeout-idempotence-v1",
        "status": "passed" if previous is not None and all(previous.get(k) == v for k, v in substantive.items()) else "first-run",
        "substantiveCompared": previous is not None, "dispatch": closeout_value["dispatch"]})
    return closeout_value


def parser() -> argparse.ArgumentParser:
    value = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    commands = value.add_subparsers(dest="command", required=True)
    for name in ("prepare", "fixtures", "qualify", "match-gate", "run-matches", "closeout"):
        command = commands.add_parser(name, allow_abbrev=False)
        command.add_argument("--root", required=True)
    return value


def main() -> int:
    args = parser().parse_args()
    root = Path(args.root).expanduser().resolve()
    try:
        result = {"prepare": prepare, "fixtures": fixtures, "qualify": qualify,
                  "match-gate": match_gate, "run-matches": run_matches,
                  "closeout": closeout}[args.command](root)
        print(json.dumps({"command": args.command, "status": result["status"]}, sort_keys=True))
        return 0
    except (ContractError, OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as exc:
        print(f"Step 14 worker failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
