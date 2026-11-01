#!/usr/bin/env python3
"""Run the bounded Step 12 MBP development match schedule.

The runner is deliberately serial: one process, one game thread, one frozen
opening block at a time.  A block is accepted only after the independent
validator seals its raw report.  Existing accepted evidence is resumable;
failed blocks are never silently retried.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
from typing import Any


EXPERIMENT_ID = "strength-first-step12-independent-replication-20260916-v1"
CELLS = ("early-selected", "u2688-control")
BLOCKS = 4
GAMES_PER_BLOCK = 64
PAIRS_PER_BLOCK = 32
MATCH_SEED = 2026091607
MAX_PLIES = 200
MOVETIME_MS = 100
RECOVERY_SCHEMA = "haitaka-anhoku-strength-first-step12-match-recovery-amendment-v1"
COMPATIBILITY_SCHEMA = "haitaka-anhoku-strength-first-step12-match-runtime-compatibility-v1"
REPO_ROOT = Path(__file__).resolve().parents[1]
VALIDATOR = REPO_ROOT / "scripts/strength_first_step12_match_validation.py"


class MatchError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise MatchError(message)


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


def artifact_matches(root: Path, item: Any) -> bool:
    if not isinstance(item, dict) or not isinstance(item.get("path"), str) or not isinstance(item.get("bytes"), int) or not isinstance(item.get("sha256"), str):
        return False
    path = root / item["path"]
    return path.is_file() and not path.is_symlink() and path.stat().st_size == item["bytes"] and sha256(path) == item["sha256"]


def recovery_error(root: Path, cell: str, block: int, failed: Path) -> str | None:
    """Validate the one explicit replacement allowed for the preserved mismatch."""
    path = root / "preflight/match-recovery-amendment.json"
    if not path.is_file():
        return "match-recovery-amendment.json is missing"
    try:
        value = load(path)
    except (MatchError, OSError, UnicodeError, json.JSONDecodeError) as exc:
        return f"cannot load match recovery amendment: {exc}"
    if value.get("schema") != RECOVERY_SCHEMA or value.get("status") != "passed" or value.get("experimentId") != EXPERIMENT_ID:
        return "match recovery amendment schema/status/experiment differs"
    target = value.get("replacement", {})
    failed_rel = str(failed.resolve().relative_to(root.resolve()))
    if target.get("cell") != cell or target.get("block") != block or target.get("failedAttempt") != failed_rel or target.get("newAttempt") != f"matches/{cell}/block-{block:02d}/attempt-02":
        return "match recovery amendment targets a different failed block"
    if target.get("replacementOnly") is not True or target.get("excludeFailedAttemptFromAnalysis") is not True:
        return "match recovery amendment does not define an isolated replacement"
    failure = failed / "failure.json"
    if not failure.is_file() or load(failure).get("retryableExternalInterruption") is not False:
        return "preserved attempt is not a non-interruption failure"
    files = value.get("failedAttempt", {}).get("files")
    if not isinstance(files, list) or not files or any(not artifact_matches(root, item) for item in files):
        return "preserved failed-attempt bindings are missing or stale"
    schedule_ref = value.get("frozenSchedule")
    if schedule_ref is None or schedule_ref.get("path") != "openings/schedule.json" or not artifact_matches(root, schedule_ref):
        return "recovery amendment does not bind the frozen schedule"
    for name, expected in (("scheduleChanged", False), ("failedAttemptModified", False), ("selectionChanged", False), ("additionalOptimizerUpdates", 0), ("outcomesOpened", False), ("confirmationAuthorized", False), ("deploymentAuthorized", False)):
        if value.get(name) != expected:
            return f"match recovery amendment changes {name}"
    return None


def compatibility_error(root: Path, engine_identity: dict[str, Any] | None, schedule_identity: dict[str, Any] | None) -> str | None:
    path = root / "preflight/match-runtime-compatibility.json"
    if not path.is_file():
        return "match-runtime-compatibility.json is missing"
    try:
        value = load(path)
    except (MatchError, OSError, UnicodeError, json.JSONDecodeError) as exc:
        return f"cannot load runtime compatibility receipt: {exc}"
    if value.get("schema") != COMPATIBILITY_SCHEMA or value.get("status") != "passed" or value.get("experimentId") != EXPERIMENT_ID:
        return "runtime compatibility receipt schema/status/experiment differs"
    if value.get("scheduleChanged") is not False or value.get("failedAttemptModified") is not False or value.get("selectionChanged") is not False or value.get("additionalOptimizerUpdates") != 0:
        return "runtime compatibility receipt changes the scientific inputs"
    new_runtime = value.get("newRuntime")
    if engine_identity is None or not isinstance(new_runtime, dict) or new_runtime.get("path") not in ("runtime/haitaka_cli", engine_identity.get("path")) or new_runtime.get("bytes") != engine_identity.get("bytes") or new_runtime.get("sha256") != engine_identity.get("sha256"):
        return "runtime compatibility receipt does not bind the active runtime"
    schedule = value.get("frozenSchedule")
    if schedule_identity is None or not isinstance(schedule, dict) or schedule.get("path") != schedule_identity.get("path") or schedule.get("bytes") != schedule_identity.get("bytes") or schedule.get("sha256") != schedule_identity.get("sha256"):
        return "runtime compatibility receipt does not bind the frozen schedule"
    qualification = value.get("renewedQualification")
    qualification_path = root / "preflight/qualification-gate.json"
    if not isinstance(qualification, dict) or qualification.get("path") != "preflight/qualification-gate.json" or not qualification_path.is_file() or qualification.get("bytes") != qualification_path.stat().st_size or qualification.get("sha256") != sha256(qualification_path):
        return "runtime compatibility receipt does not bind renewed qualification"
    regression = value.get("regressionReceipt")
    if not isinstance(regression, dict) or regression.get("path") != "preflight/match-runtime-regression.json" or not artifact_matches(root, regression):
        return "runtime compatibility receipt does not bind the runtime regression"
    return None


def nonretryable_failures(root: Path) -> list[tuple[str, int, Path]]:
    found: list[tuple[str, int, Path]] = []
    for cell in CELLS:
        for block in range(BLOCKS):
            block_dir = root / "matches" / cell / f"block-{block:02d}"
            for failed in previous_failures(block_dir):
                failure = failed / "failure.json"
                try:
                    value = load(failure)
                except (MatchError, OSError, UnicodeError, json.JSONDecodeError):
                    found.append((cell, block, failed))
                    continue
                if value.get("retryableExternalInterruption") is not True:
                    found.append((cell, block, failed))
    return found


def atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def schedule(root: Path) -> dict[str, Any]:
    value = load(root / "openings/schedule.json")
    if value.get("cells") != list(CELLS) or value.get("blocks") != BLOCKS or value.get("gamesPerCell") != 256 or value.get("pairsPerCell") != 128:
        fail("fixed Step 12 schedule header differs")
    rows = value.get("rows")
    if not isinstance(rows, list) or len(rows) != 512:
        fail("fixed Step 12 schedule must contain 512 rows")
    expected = {"matchSeed": MATCH_SEED, "movetimeMs": MOVETIME_MS, "maxPlies": MAX_PLIES}
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            fail(f"schedule row {index} is not an object")
        for name, required in expected.items():
            if type(row.get(name)) is not int or row[name] != required:
                fail(f"schedule row {index}: {name} differs")
    for name, required in expected.items():
        if name in value and (type(value[name]) is not int or value[name] != required):
            fail(f"schedule header: {name} conflicts with fixed schedule")
    if any(row.get("threads") != 1 or row.get("concurrentGames") != 1 or row.get("openingOrder") != "sequential" or row.get("openingRandomPlies") != 0 for row in rows):
        fail("fixed Step 12 execution schedule differs")
    return value


def model_update(root: Path, cell: str) -> tuple[int, Path]:
    selection = load(root / "training/selection.json")
    selected = int(selection["selectedUpdate"])
    update = selected if cell == "early-selected" else 2688
    network = root / "training/run/exports" / f"u{update:06d}.nnue"
    if not network.is_file():
        network = root / "exports" / f"u{update:06d}.nnue"
    if not network.is_file() or network.is_symlink():
        fail(f"model export is missing or symlinked: {network}")
    return update, network


def runtime(root: Path) -> Path:
    path = root / "runtime/haitaka_cli"
    if not path.is_file():
        path = REPO_ROOT / "target/release/haitaka_cli"
    if not path.is_file() or path.is_symlink():
        fail("native match runtime is missing")
    return path


def rows_for(value: dict[str, Any], cell: str, block: int) -> list[dict[str, Any]]:
    rows = [row for row in value["rows"] if row.get("cell") == cell and row.get("block") == block]
    rows.sort(key=lambda row: int(row.get("gameIndexInCell", -1)))
    if len(rows) != GAMES_PER_BLOCK:
        fail(f"schedule has {len(rows)} rows for {cell} block {block}")
    return rows


def materialize_openings(root: Path, rows: list[dict[str, Any]], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    text = "\n".join(str(row["startSfen"]) for row in rows) + "\n"
    if path.exists():
        if path.read_text(encoding="utf-8") != text:
            fail(f"existing opening file differs: {path}")
    else:
        path.write_text(text, encoding="utf-8")


def gate(root: Path) -> dict[str, Any]:
    root = root.expanduser().resolve()
    reasons: list[str] = []
    schedule_path = root / "openings/schedule.json"
    try:
        sched = schedule(root)
    except MatchError as exc:
        reasons.append(str(exc))
        sched = {}
    for rel in ("contract.json", "training/selection.json", "preflight/qualification-gate.json", "authorization/cpu-match.json"):
        if not (root / rel).is_file():
            reasons.append(f"missing {rel}")
    qualification = load(root / "preflight/qualification-gate.json") if (root / "preflight/qualification-gate.json").is_file() else {}
    if qualification.get("status") != "passed":
        reasons.append("qualification gate is not passed")
    auth = load(root / "authorization/cpu-match.json") if (root / "authorization/cpu-match.json").is_file() else {}
    if auth.get("granted") is not True:
        reasons.append("cpu-match authorization is not granted")
    for cell in CELLS:
        try:
            _update, network = model_update(root, cell)
            if not network.is_file():
                reasons.append(f"missing model for {cell}")
        except (MatchError, KeyError, TypeError, ValueError) as exc:
            reasons.append(str(exc))
    try:
        engine = runtime(root)
        engine_identity = {"path": str(engine), "bytes": engine.stat().st_size, "sha256": sha256(engine)}
    except MatchError as exc:
        reasons.append(str(exc))
        engine_identity = None
    schedule_identity = {"path": "openings/schedule.json", "bytes": schedule_path.stat().st_size, "sha256": sha256(schedule_path)} if schedule_path.is_file() else None
    layout_path = root / "preflight/schedule-layout-compatibility.json"
    layout = load(layout_path) if layout_path.is_file() else {}
    if layout.get("schema") != "haitaka-anhoku-strength-first-step12-schedule-layout-compatibility-v1" or layout.get("status") != "passed" or layout.get("sourceLayout") != "per-row" or layout.get("rowsVerified") != 512 or layout.get("scheduleModified") is not False or layout.get("scheduleRegenerated") is not False:
        reasons.append("schedule layout compatibility receipt is missing or invalid")
    if schedule_identity is None or layout.get("schedule", {}).get("sha256") != schedule_identity["sha256"] or layout.get("schedule", {}).get("bytes") != schedule_identity["bytes"]:
        reasons.append("schedule layout compatibility does not bind the frozen schedule bytes")
    helper_binding = layout.get("patchedHelper", {})
    if helper_binding.get("path") != "scripts/strength_first_step12_matches.py" or helper_binding.get("sha256") != sha256(Path(__file__)):
        reasons.append("schedule layout compatibility helper binding differs")
    regression_binding = layout.get("regressionTests", {})
    if not isinstance(regression_binding, dict) or not isinstance(regression_binding.get("sha256"), str) or not (root / str(regression_binding.get("path", ""))).is_file() or sha256(root / str(regression_binding.get("path"))) != regression_binding.get("sha256"):
        reasons.append("schedule layout regression receipt is missing or unbound")
    recovery_path = root / "preflight/match-recovery-amendment.json"
    failures = nonretryable_failures(root)
    for failed_cell, failed_block, failed_attempt in failures:
        error = recovery_error(root, failed_cell, failed_block, failed_attempt)
        if error is not None:
            reasons.append(f"non-retryable match failure requires a valid replacement amendment: {error}")
    if failures:
        error = compatibility_error(root, engine_identity, schedule_identity)
        if error is not None:
            reasons.append(f"non-retryable match failure requires runtime compatibility evidence: {error}")
    resolution_path = root / "preflight/training-authorization-resolution.json"
    value = {
        "schema": "haitaka-anhoku-strength-first-step12-match-gate-v1",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "passed" if not reasons else "blocked",
        "reasons": reasons,
        "schedule": {"cells": list(CELLS), "blocks": BLOCKS, "gamesPerBlock": GAMES_PER_BLOCK, "games": 512, "matchSeed": MATCH_SEED, "movetimeMs": MOVETIME_MS, "maxPlies": MAX_PLIES, "threads": 1, "concurrency": 1},
        "qualification": {"path": "preflight/qualification-gate.json", "sha256": sha256(root / "preflight/qualification-gate.json")} if (root / "preflight/qualification-gate.json").is_file() else None,
        "schedule": schedule_identity,
        "scheduleLayoutCompatibility": {"path": "preflight/schedule-layout-compatibility.json", "sha256": sha256(layout_path)} if layout_path.is_file() else None,
        "trainingAuthorizationResolution": {"path": "preflight/training-authorization-resolution.json", "sha256": sha256(resolution_path)} if resolution_path.is_file() else None,
        "matchRecoveryAmendment": {"path": "preflight/match-recovery-amendment.json", "sha256": sha256(recovery_path)} if recovery_path.is_file() else None,
        "matchRuntimeCompatibility": {"path": "preflight/match-runtime-compatibility.json", "sha256": sha256(root / "preflight/match-runtime-compatibility.json")} if (root / "preflight/match-runtime-compatibility.json").is_file() else None,
        "engine": engine_identity,
        "launchesRealWork": False,
        "gamesLaunched": 0,
        "outcomesOpened": False,
        "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "handcraftedRemainsProduction": True},
    }
    atomic_json(root / "preflight/match-gate.json", value)
    return value


def previous_failures(block_dir: Path) -> list[Path]:
    return sorted(path for path in block_dir.glob("attempt-*") if path.is_dir() and not (path / "accepted.json").is_file())


def valid_accepted(block_dir: Path) -> Path | None:
    for path in sorted(block_dir.glob("attempt-*")):
        if path.is_dir() and (path / "accepted.json").is_file() and (path / "validation.json").is_file():
            try:
                if load(path / "validation.json").get("status") == "passed" and load(path / "validation.json").get("sealed") is True:
                    return path
            except (MatchError, KeyError, TypeError):
                continue
    return None


def write_status(root: Path, status: str, cell: str | None = None, block: int | None = None, **extra: Any) -> None:
    path = root / "matches/status.json"
    value = load(path) if path.is_file() else {"schema": "haitaka-anhoku-strength-first-step12-match-status-v1", "experimentId": EXPERIMENT_ID}
    value.update({"updatedAt": now(), "status": status, "cell": cell, "block": block, **extra})
    atomic_json(path, value)


def run_block(root: Path, sched: dict[str, Any], cell: str, block: int, engine: Path, network: Path) -> dict[str, Any]:
    block_dir = root / "matches" / cell / f"block-{block:02d}"
    block_dir.mkdir(parents=True, exist_ok=True)
    accepted = valid_accepted(block_dir)
    if accepted is not None:
        return {"cell": cell, "block": block, "status": "already-accepted", "attempt": accepted.name, "gamesLaunched": 0}
    failures = previous_failures(block_dir)
    for failed in failures:
        failure = failed / "failure.json"
        if not failure.is_file():
            fail(f"refusing to retry attempt without a failure receipt in {failed}")
        failure_value = load(failure)
        if failure_value.get("retryableExternalInterruption") is not True:
            error = recovery_error(root, cell, block, failed)
            if error is not None:
                fail(f"refusing to retry non-interruption failure in {failed}: {error}")
    if len(failures) >= 2:
        fail(f"interruption retry budget exhausted for {cell} block {block}")
    attempt = block_dir / f"attempt-{len(failures) + 1:02d}"
    attempt.mkdir(parents=True, exist_ok=False)
    rows = rows_for(sched, cell, block)
    openings = attempt / "openings.sfen"
    materialize_openings(root, rows, openings)
    raw = attempt / "raw"
    raw.mkdir()
    update = int(load(root / "training/selection.json")["selectedUpdate"] if cell == "early-selected" else 2688)
    command = [str(engine), "self-play", "--games", str(GAMES_PER_BLOCK), "--threads", "1", "--a-eval", "nnue", "--b-eval", "handcrafted", "--a-nnue", str(network), "--movetime-ms", str(MOVETIME_MS), "--openings", str(openings), "--opening-order", "sequential", "--opening-unit", "game", "--opening-random-plies", "0", "--seed", str(MATCH_SEED), "--max-plies", str(MAX_PLIES), "--report-dir", str(raw)]
    atomic_json(attempt / "planned-command.json", {"schema": "haitaka-anhoku-strength-first-step12-match-command-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "cell": cell, "block": block, "modelUpdate": update, "command": command, "runtime": {"path": str(engine), "sha256": sha256(engine)}, "network": {"path": str(network), "sha256": sha256(network)}, "openings": {"path": str(openings), "sha256": sha256(openings)}, "games": GAMES_PER_BLOCK, "threads": 1, "movetimeMs": MOVETIME_MS, "openingOrder": "sequential", "openingUnit": "game", "openingRandomPlies": 0, "seed": MATCH_SEED, "maxPlies": MAX_PLIES, "gamesLaunched": 0, "outcomesOpened": False})
    started = time.monotonic()
    run_command = command
    if sys.platform == "darwin" and shutil.which("caffeinate"):
        run_command = ["caffeinate", "-i", *command]
    completed = subprocess.run(run_command, cwd=REPO_ROOT, capture_output=True, text=True, check=False)
    (attempt / "runner.stdout").write_text(completed.stdout, encoding="utf-8")
    (attempt / "runner.stderr").write_text(completed.stderr, encoding="utf-8")
    elapsed = time.monotonic() - started
    if completed.returncode != 0:
        retryable = completed.returncode < 0
        atomic_json(attempt / "failure.json", {"schema": "haitaka-anhoku-strength-first-step12-match-failure-v1", "createdAt": now(), "status": "failed", "returncode": completed.returncode, "retryableExternalInterruption": retryable, "command": run_command, "elapsedSeconds": elapsed, "gamesLaunched": 0, "outcomesOpened": False})
        fail(f"self-play failed for {cell} block {block} with return code {completed.returncode}")
    validate_command = [sys.executable, str(VALIDATOR), "validate", "--root", str(root), "--cell", cell, "--block", str(block), "--attempt", str(attempt)]
    checked = subprocess.run(validate_command, cwd=REPO_ROOT, capture_output=True, text=True, check=False)
    (attempt / "validator.stdout").write_text(checked.stdout, encoding="utf-8")
    (attempt / "validator.stderr").write_text(checked.stderr, encoding="utf-8")
    if checked.returncode != 0:
        atomic_json(attempt / "failure.json", {"schema": "haitaka-anhoku-strength-first-step12-match-failure-v1", "createdAt": now(), "status": "validation-failed", "returncode": checked.returncode, "retryableExternalInterruption": False, "command": command, "validator": validate_command, "elapsedSeconds": elapsed, "gamesLaunched": GAMES_PER_BLOCK, "outcomesOpened": False})
        fail(f"strict validation failed for {cell} block {block}: {checked.stderr[-1000:]}")
    validation = load(attempt / "validation.json")
    atomic_json(attempt / "accepted.json", {"schema": "haitaka-anhoku-strength-first-step12-accepted-block-v1", "createdAt": now(), "status": "accepted", "cell": cell, "block": block, "attempt": attempt.name, "validation": {"path": "validation.json", "bytes": (attempt / "validation.json").stat().st_size, "sha256": sha256(attempt / "validation.json")}, "rawReport": {"path": "raw/self-play-report.json", "bytes": (raw / "self-play-report.json").stat().st_size, "sha256": sha256(raw / "self-play-report.json")}, "gamesLaunched": GAMES_PER_BLOCK, "outcomesOpened": False, "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "handcraftedRemainsProduction": True}})
    return {"cell": cell, "block": block, "status": "accepted", "attempt": attempt.name, "modelUpdate": update, "gamesLaunched": GAMES_PER_BLOCK, "elapsedSeconds": elapsed, "wdl": validation.get("wdl"), "cappedGames": validation.get("cappedGames", 0)}


def run(root: Path, resume: bool) -> dict[str, Any]:
    root = root.expanduser().resolve()
    # Revalidate artifact bindings and authorization on every invocation.
    gate_value = gate(root)
    if gate_value.get("status") != "passed":
        fail("match gate is not passed")
    sched = schedule(root)
    engine = runtime(root)
    lock_path = root / "matches/controller.lock"
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with lock_path.open("a+") as lock:
        try:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as exc:
            fail(f"match controller lock is held: {exc}")
        write_status(root, "running", gamesLaunched=0, completedBlocks=0, completedGames=0)
        records: list[dict[str, Any]] = []
        total_launched = 0
        total_completed = 0
        for cell in CELLS:
            update, network = model_update(root, cell)
            for block in range(BLOCKS):
                write_status(root, "running", cell=cell, block=block, modelUpdate=update, gamesLaunched=total_launched, completedBlocks=len(records), completedGames=total_completed)
                record = run_block(root, sched, cell, block, engine, network)
                records.append(record)
                total_launched += int(record.get("gamesLaunched", 0))
                # Both new and previously accepted blocks contain 64 sealed games.
                total_completed += GAMES_PER_BLOCK
                write_status(root, "running", cell=cell, block=block, modelUpdate=update, gamesLaunched=total_launched, completedBlocks=len(records), completedGames=total_completed)
        summary = {"schema": "haitaka-anhoku-strength-first-step12-match-status-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "status": "complete-sealed", "completedBlocks": len(records), "completedGames": total_completed, "blocks": records, "schedule": {"cells": list(CELLS), "blocks": BLOCKS, "games": 512, "seed": MATCH_SEED, "threads": 1, "concurrency": 1}, "outcomesSealed": True, "outcomesOpened": False, "gamesLaunched": total_launched, "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "handcraftedRemainsProduction": True}}
        atomic_json(root / "matches/status.json", summary)
        atomic_json(root / "matches/resource-ledger.json", {"schema": "haitaka-anhoku-strength-first-step12-match-resource-ledger-v1", "createdAt": now(), "status": "complete", "host": "192.168.0.12", "runtime": {"path": str(engine), "sha256": sha256(engine)}, "blocks": records, "games": total_completed, "gamesLaunched": total_launched, "threads": 1, "concurrency": 1, "matchSeed": MATCH_SEED, "outcomesOpened": False})
        return summary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("gate", "run"))
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args()
    try:
        value = gate(args.root) if args.command == "gate" else run(args.root, args.resume)
        print(json.dumps({"command": args.command, "status": value.get("status"), "completedBlocks": value.get("completedBlocks", 0), "completedGames": value.get("completedGames", 0), "launchesRealWork": args.command == "run"}, sort_keys=True))
        return 0
    except (MatchError, OSError, KeyError, TypeError, ValueError, subprocess.SubprocessError) as exc:
        print(f"Step 12 matches blocked: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
