#!/usr/bin/env python3
"""F1-owned single-block browser controller and independent replay acceptance.

The retained Step 14 HTTP transport, process sampler and search-free replay
binary are components; no historical match dispatcher is called.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import http.server
import json
import math
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import threading
import time
from typing import Any

import strength_first_f1_core as f1
import strength_first_f1_replay as replay_adapter
import strength_first_step14_browser as transport


PAGE = f1.ROOT / "scripts/strength_first_step14_browser_page.html"
WORKER = f1.ROOT / "scripts/strength_first_f1_browser_worker.js"
PARITY_PAGE = f1.ROOT / "scripts/strength_first_step11_browser_page.html"
PARITY_WORKER = f1.ROOT / "scripts/strength_first_step11_browser_parity_worker.js"
SMOKE_WORKER = f1.ROOT / "scripts/strength_first_step11_browser_smoke_worker.js"
TERMINAL_WORKER = f1.ROOT / "scripts/strength_first_step14_terminal_worker.js"
PARITY_CORPUS = f1.ROOT / "out/anhoku-strength-first/step12-independent-replication-20260916-v1/parity/fixture-corpus.jsonl"
FIXED_HASH_MB = 16
HOST_SECONDS_CAP = 32 * 3600


def _load(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text())
    f1.require(isinstance(value, dict), f"object required: {path}")
    return value


def _future_utc(value: str) -> bool:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    f1.require(parsed.tzinfo is not None, "match grant expiry lacks UTC offset")
    return parsed.astimezone(timezone.utc) > datetime.now(timezone.utc)


def _utc(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    f1.require(parsed.tzinfo is not None, "setup command timestamp lacks UTC offset")
    return parsed.astimezone(timezone.utc)


def _setup_activity(receipt: dict[str, Any], *, observed: datetime) -> tuple[float, datetime, datetime, list[tuple[datetime, datetime, str]]]:
    """Charge the wall duration of each retained MBP command, excluding idle gaps."""
    f1.require(receipt.get("schema") == "haitaka-anhoku-strength-first-f1-mbp-command-receipt-v1" and receipt.get("host") == "ITSO-WX6303", "MBP setup command receipt differs")
    commands = receipt.get("commands")
    f1.require(isinstance(commands, list) and commands, "MBP setup command receipt has no commands")
    intervals = []
    seconds = 0.0
    for item in commands:
        f1.require(isinstance(item, dict) and isinstance(item.get("commandId"), str) and item["commandId"] and isinstance(item.get("command"), str) and item["command"] and isinstance(item.get("source"), str) and item["source"], "MBP setup command evidence incomplete")
        f1.require(not any(forbidden in item["command"] for forbidden in ("freeze-runtime", "qualify-runtime", "run-block")), "separately timed runtime phase in setup receipt")
        start, end = _utc(item["startedAtUtc"]), _utc(item["endedAtUtc"])
        f1.require(start <= end <= observed, "MBP setup command timestamp differs")
        wall = (end - start).total_seconds()
        monotonic = item.get("elapsedMonotonicSeconds", wall)
        f1.require(type(monotonic) in (int, float) and math.isfinite(monotonic) and monotonic >= 0 and abs(monotonic - wall) < 5, "MBP setup command duration differs")
        seconds += max(wall, monotonic)
        intervals.append((start, end, item["commandId"]))
    f1.require(len({item[2] for item in intervals}) == len(intervals), "duplicate MBP setup command ID")
    intervals.sort()
    f1.require(all(previous[1] <= current[0] for previous, current in zip(intervals, intervals[1:])), "overlapping MBP setup commands")
    return seconds, intervals[0][0], intervals[-1][1], intervals


def record_setup_activity(root: Path, segment: int, receipt_path: Path) -> dict[str, Any]:
    """Bind retained command timestamps for one append-only active setup segment."""
    f1.require(socket.gethostname() == "ITSO-WX6303", "F1 setup activity must be recorded on MBP")
    f1.require((root / "runtime/setup-start.json").is_file() and not (root / "runtime/setup-ledger.json").exists(), "setup activity must follow start and precede ledger")
    f1.require(0 <= segment < 100 and receipt_path.is_file() and receipt_path.is_relative_to(root), "setup command receipt must be staged under F1 root")
    f1.require(receipt_path.parent == root / "runtime/setup-command-receipts", "setup receipt directory differs")
    receipt = _load(receipt_path)
    seconds, started, ended, _ = _setup_activity(receipt, observed=datetime.now(timezone.utc))
    existing = sorted((root / "runtime/setup-activity").glob("segment-*.json"))
    f1.require(segment == len(existing), "setup activity segment must be consecutive and append-only")
    value = {"schema": "haitaka-anhoku-strength-first-f1-mbp-setup-activity-v1", "experimentId": f1.EXPERIMENT_ID, "host": "ITSO-WX6303", "segment": segment, "receipt": f1.artifact(receipt_path), "startedAtUtc": started.isoformat(), "endedAtUtc": ended.isoformat(), "elapsedSeconds": seconds, "commandIds": [item["commandId"] for item in receipt["commands"]]}
    f1.immutable_json(root / f"runtime/setup-activity/segment-{segment:02d}.json", value)
    return value


def run_setup_command(root: Path, segment: int, command: list[str]) -> int:
    """Time one future MBP setup command and retain its receipt even on failure."""
    f1.require(socket.gethostname() == "ITSO-WX6303" and (root / "runtime/setup-start.json").is_file() and not (root / "runtime/setup-ledger.json").exists(), "MBP setup command must follow start and precede ledger")
    command = command[1:] if command and command[0] == "--" else command
    f1.require(command and not any(forbidden in " ".join(command) for forbidden in ("freeze-runtime", "qualify-runtime", "run-block")), "separately timed runtime phase cannot be a setup command")
    receipt_path = root / f"runtime/setup-command-receipts/segment-{segment:02d}.json"
    f1.require(not receipt_path.exists() and not (root / f"runtime/setup-activity/segment-{segment:02d}.json").exists(), "setup command segment already exists")
    started = datetime.now(timezone.utc)
    start_monotonic = time.monotonic()
    status = 127
    try:
        status = subprocess.run(command, cwd=f1.ROOT, check=False).returncode
    finally:
        ended = datetime.now(timezone.utc)
        monotonic = time.monotonic() - start_monotonic
        f1.immutable_json(receipt_path, {"schema": "haitaka-anhoku-strength-first-f1-mbp-command-receipt-v1", "host": "ITSO-WX6303", "commands": [{"commandId": f"setup-{segment:02d}", "command": " ".join(command), "source": "local-command-wrapper-monotonic-and-utc", "startedAtUtc": started.isoformat(), "endedAtUtc": ended.isoformat(), "elapsedMonotonicSeconds": monotonic, "exitCode": status}]})
        record_setup_activity(root, segment, receipt_path)
    return status


def _setup_components(root: Path, observed: datetime) -> tuple[list[dict[str, Any]], list[dict[str, Any]], float, datetime | None]:
    activities = []
    freezes = []
    intervals: list[tuple[datetime, datetime, str]] = []
    command_ids = set()
    total = 0.0
    for index, path in enumerate(sorted((root / "runtime/setup-activity").glob("segment-*.json"))):
        f1.require(path.name == f"segment-{index:02d}.json", "MBP setup activity sequence differs")
        activity = _load(path)
        receipt = _load(f1.verify_artifact(activity["receipt"]))
        seconds, start, end, commands = _setup_activity(receipt, observed=observed)
        f1.require(activity.get("schema") == "haitaka-anhoku-strength-first-f1-mbp-setup-activity-v1" and activity.get("host") == "ITSO-WX6303" and activity.get("segment") == index and activity.get("startedAtUtc") == start.isoformat() and activity.get("endedAtUtc") == end.isoformat() and abs(activity.get("elapsedSeconds", -1) - seconds) < 0.001 and activity.get("commandIds") == [item["commandId"] for item in receipt["commands"]], "MBP setup activity evidence differs")
        for command in commands:
            f1.require(command[2] not in command_ids, "duplicate MBP setup command across segments")
            command_ids.add(command[2])
        intervals.extend(commands)
        total += seconds
        activities.append(f1.artifact(path))
    for index, launch_path in enumerate(sorted((root / "runtime/freeze-attempts").glob("attempt-*/launch.json"))):
        f1.require(launch_path.parent.name == f"attempt-{index:02d}", "runtime freeze attempt sequence differs")
        duration_path = launch_path.parent / "duration.json"
        f1.require(duration_path.is_file(), "runtime freeze attempt duration absent")
        launch, duration = _load(launch_path), _load(duration_path)
        start, end = _utc(launch["startedAtUtc"]), _utc(duration["endedAtUtc"])
        seconds = duration.get("elapsedSeconds")
        f1.require(launch.get("schema") == "haitaka-anhoku-strength-first-f1-runtime-freeze-launch-v1" and duration.get("schema") == "haitaka-anhoku-strength-first-f1-runtime-freeze-duration-v1" and launch.get("attempt") == duration.get("attempt") == index and launch.get("host") == "ITSO-WX6303" and type(seconds) in (int, float) and math.isfinite(seconds) and seconds >= 0 and start <= end <= observed and abs((end - start).total_seconds() - seconds) < 5, "runtime freeze attempt timing differs")
        intervals.append((start, end, f"freeze-{index:02d}"))
        total += seconds
        freezes.append({"launch": f1.artifact(launch_path), "duration": f1.artifact(duration_path)})
    intervals.sort()
    f1.require(all(previous[1] <= current[0] for previous, current in zip(intervals, intervals[1:])), "MBP setup phases overlap")
    return activities, freezes, total, intervals[0][0] if intervals else None


def record_setup_start(root: Path) -> dict[str, Any]:
    """Preserve the first observed staging time before later rsync metadata changes."""
    f1.require(socket.gethostname() == "ITSO-WX6303", "F1 setup start must be recorded on MBP")
    path = root / "runtime/setup-start.json"
    if path.is_file():
        return _load(path)
    f1.require(not (root / "authorization/match.json").exists() and not (root / "runtime/qualification.json").exists() and not any((root / "games").glob("*/block-*/attempt-*")), "setup start must precede grant and games")
    anchors = (
        f1.ROOT / f1.INPUTS["trainingBinary"][0],
        f1.ROOT / f1.INPUTS["incumbent"][0],
    )
    evidence = []
    for anchor in anchors:
        f1.require(anchor.is_file(), "MBP staged input anchor unavailable")
        changed = datetime.fromtimestamp(anchor.stat().st_ctime, timezone.utc)
        artifact = f1.artifact(anchor)
        f1.require(artifact in ({"path": relative, "bytes": size, "sha256": sha} for relative, size, sha in f1.INPUTS.values()), "setup anchor no longer matches frozen input")
        evidence.append({"artifact": artifact, "stagedMetadataChangeUtc": changed.isoformat()})
    started = min(datetime.fromisoformat(item["stagedMetadataChangeUtc"]) for item in evidence)
    observed = datetime.now(timezone.utc)
    f1.require(started <= observed, "MBP staging timestamp is in the future")
    value = {"schema": "haitaka-anhoku-strength-first-f1-mbp-setup-start-v1", "experimentId": f1.EXPERIMENT_ID, "host": "ITSO-WX6303", "startedAtUtc": started.isoformat(), "observedAtUtc": observed.isoformat(), "source": "earliest-verified-staged-input-metadata-change", "anchors": evidence, "includesIdleTime": True}
    f1.immutable_json(path, value)
    return value


def record_setup(root: Path) -> dict[str, Any]:
    """Charge evidenced active setup and freeze attempts, retain full wall upper bound."""
    f1.require(socket.gethostname() == "ITSO-WX6303", "F1 setup ledger must be recorded on MBP")
    path = root / "runtime/setup-ledger.json"
    if path.is_file():
        return _load(path)
    marker_path = root / "runtime/setup-start.json"
    marker = _load(marker_path)
    f1.require(marker.get("schema") == "haitaka-anhoku-strength-first-f1-mbp-setup-start-v1" and marker.get("host") == "ITSO-WX6303", "MBP setup start marker differs")
    marker_started = _utc(marker["startedAtUtc"])
    f1.require(all(f1.verify_artifact(item["artifact"]) for item in marker["anchors"]), "MBP setup start inputs changed")
    freeze = _load(root / "runtime/freeze.json")
    f1.require(freeze.get("host", {}).get("name") == "ITSO-WX6303" and freeze.get("package", {}).get("sha256") == f1.sha256(root / "package.json"), "MBP runtime must be frozen before setup ledger")
    f1.require(not (root / "authorization/match.json").exists() and not (root / "runtime/qualification.json").exists() and not any((root / "games").glob("*/block-*/attempt-*")), "setup ledger must precede grant and games")
    frozen = _utc(freeze["createdAtUtc"])
    ended = datetime.now(timezone.utc)
    activities, freezes, elapsed, first_active = _setup_components(root, ended)
    f1.require(activities and freezes and marker_started <= frozen <= ended, "MBP setup command and freeze evidence required")
    started = min(marker_started, first_active) if first_active else marker_started
    wall = (ended - started).total_seconds()
    f1.require(0 <= elapsed <= HOST_SECONDS_CAP and elapsed <= wall + 5 * len(freezes), "evidenced MBP setup exhausted host cap or exceeds wall upper bound")
    value = {"schema": "haitaka-anhoku-strength-first-f1-mbp-setup-ledger-v1", "experimentId": f1.EXPERIMENT_ID, "host": "ITSO-WX6303", "startedAtUtc": started.isoformat(), "endedAtUtc": ended.isoformat(), "wallIntervalSeconds": wall, "wallIntervalRole": "upper-bound-includes-idle-and-unmetered-prior-staging", "elapsedSeconds": elapsed, "accountingMethod": "retained-command-intervals-plus-monotonic-freeze-attempts", "includesIdleTime": False, "preexistingStagingUnmetered": True, "evidenceGap": "MBP commands before the first retained timing receipt have no duration evidence; no active seconds imputed from file metadata", "setupStart": f1.artifact(marker_path), "anchors": marker["anchors"], "activities": activities, "freezeAttempts": freezes, "runtimeFreeze": f1.artifact(root / "runtime/freeze.json"), "package": f1.artifact(root / "package.json"), "qualificationOccurred": False, "scoredGamesOccurred": False}
    f1.immutable_json(path, value)
    return value


def _grant_still_current(path: Path, binding: dict[str, Any]) -> None:
    f1.require(f1.artifact(path) == binding and _future_utc(_load(path)["validUntilUtc"]), "match grant changed or expired during dispatch")


def _model(root: Path, arm: str) -> dict[str, Any]:
    selection = root / f"training/arms/{arm}/selection.json"
    value = _load(selection)
    f1.require(value.get("experimentId") == f1.EXPERIMENT_ID and value.get("selectedUpdate") in f1.UPDATES and value.get("matchEvidenceUsed") is False, f"{arm} validation-only selection absent")
    run_path = root / f"training/arms/{arm}/run.json"
    f1.require(value.get("input") == f1.artifact(run_path) and _load(run_path).get("complete") is True, f"{arm} selection input changed")
    expected = f1.select_by_common_huber(_load(run_path)["candidates"])
    f1.require(all(value.get(key) == expected[key] for key in ("selectedUpdate", "globalMinimumHuber", "selectedHuber", "selected")), f"{arm} selector differs")
    exported = value["selected"]["export"]
    f1.verify_artifact(exported)
    return {"selection": f1.artifact(selection), "run": f1.artifact(run_path), "network": exported}


def fixture_schedule(root: Path) -> dict[str, Any]:
    """Eight short games on previously exposed, excluded Step 18 families."""
    sources = []
    scored_rows = [row for phase in ("development", "holdout") for row in _load(root / f"openings/{phase}.json")["rows"]]
    scored = {row["sfen"] for row in scored_rows}
    scored_families = {row["canonicalFamily"] for row in scored_rows}
    for phase in ("selection", "holdout"):
        path = root / f"openings/source-suites/step18-{phase}.tsv"
        source = f1.artifact(path)
        inventory_path = root / f"openings/source-suites/step18-{phase}.jsonl"
        inventory = f1.artifact(inventory_path)
        excluded_families = {row["canonicalFamily"] for row in (json.loads(line) for line in inventory_path.read_text().splitlines()) if row.get("kind") == "family"}
        f1.require(len(excluded_families) == (128 if phase == "selection" else 512) and excluded_families.isdisjoint(scored_families), "pre-exposed fixture family leaked into scored suites")
        first = path.read_text().splitlines()[0]
        opening_id, sfen = first.split("\t", 1)
        f1.require(opening_id == f"{phase}-0000" and sfen.split()[1] in ("b", "w"), "runtime fixture source differs")
        sources.append((source, inventory, opening_id, sfen))
    f1.require(all(sfen not in scored for _, _, _, sfen in sources), "runtime fixture overlaps scored opening")
    rows = []
    for pair_index, (source, inventory, source_id, sfen) in enumerate(sources):
        matchups = ("T-C", "T-I") if pair_index == 0 else ("T-I", "T-C")
        for matchup in matchups:
            for leg in (0, 1):
                color = "black" if (pair_index + leg) % 2 == 0 else "white"
                opponent = matchup[-1]
                rows.append({"phase": "fixture", "pairIndex": pair_index, "blockIndex": 0, "matchup": matchup, "leg": leg, "treatmentColor": color, "openingId": f"fixture-{source_id}", "sfen": sfen, "sourceSuite": source, "sourceInventory": inventory, "gameId": f"fixture-{pair_index:04d}-{matchup}-{leg}", "gameIndex": len(rows), "blackEngine": "T" if color == "black" else opponent, "whiteEngine": "T" if color == "white" else opponent, "initialSideToMove": "black" if sfen.split()[1] == "b" else "white", "movetimeMs": 1000, "maxPlies": 2})
    f1.require(len(rows) == 8 and {row["matchup"] for row in rows} == {"T-C", "T-I"} and {row["treatmentColor"] for row in rows} == {"black", "white"}, "runtime fixture coverage differs")
    return {"schema": "haitaka-anhoku-strength-first-f1-runtime-fixture-schedule-v1", "experimentId": f1.EXPERIMENT_ID, "rows": rows, "sourcePolicy": "pre-exposed-step18-selection-and-holdout; excluded-before-f1-scored-openings", "scoredGames": 0}


def _freeze_runtime_once(root: Path, chrome: Path, replay_binary: Path, native_parity_binary: Path) -> dict[str, Any]:
    import strength_first_f1_package as package_builder
    package_builder.verify(root)
    f1.require(_load(root / "preflight/cpu-qualification-v2.json").get("passed") is True, "CPU qualification missing")
    package = _load(root / "package.json")
    f1.require(package.get("status") == "frozen" and package.get("experimentId") == f1.EXPERIMENT_ID, "package not frozen")
    fixture_path = root / "runtime/fixture-schedule.json"
    f1.require(_load(fixture_path) == fixture_schedule(root), "runtime fixture schedule differs")
    fixture_binding = f1.artifact(fixture_path)
    f1.require(package.get("runtimeFixtureSchedule") == fixture_binding and package.get("runtimeFixtureGameIds") == [row["gameId"] for row in fixture_schedule(root)["rows"]], "package fixture binding differs")
    models = {"I": f1.frozen_inputs()["incumbent"], "C": _model(root, "C"), "T": _model(root, "T")}
    schedules = {phase: f1.artifact(root / f"openings/{phase}-schedule.json") for phase in ("development", "holdout")}
    f1.require(socket.gethostname() == "ITSO-WX6303", "F1 match host must be the preselected MBP")
    version = subprocess.run([str(chrome), "--version"], capture_output=True, text=True, check=True, timeout=15).stdout.strip()
    f1.require("Chrome" in version, "desktop Chrome version unavailable")
    f1.require(replay_binary.is_file() and os.access(replay_binary, os.X_OK) and native_parity_binary.is_file() and os.access(native_parity_binary, os.X_OK), "retained replay/parity binary unavailable")
    receipt = {"schema": "haitaka-anhoku-strength-first-f1-runtime-freeze-v1", "experimentId": f1.EXPERIMENT_ID, "createdAtUtc": f1.utc_now(), "package": f1.artifact(root / "package.json"), "models": models, "schedules": schedules, "fixtureSchedule": fixture_binding, "page": f1.artifact(PAGE), "worker": f1.artifact(WORKER), "parityPage": f1.artifact(PARITY_PAGE), "parityWorker": f1.artifact(PARITY_WORKER), "smokeWorker": f1.artifact(SMOKE_WORKER), "terminalWorker": f1.artifact(TERMINAL_WORKER), "parityCorpus": f1.artifact(PARITY_CORPUS), "nativeParityBinary": f1.artifact(native_parity_binary), "wasm": f1.frozen_inputs()["wasm"], "glue": f1.frozen_inputs()["glue"], "replayBinary": f1.artifact(replay_binary), "host": {"name": socket.gethostname(), "chrome": f1.artifact(chrome), "chromeVersion": version}, "settings": {"movetimeMs": 1000, "maxPlies": 200, "hashMb": FIXED_HASH_MB, "threads": 1, "concurrentGames": 1, "history": "full-position-sfen-moves-every-search", "freshEngineInstancesPerGame": True}}
    f1.immutable_json(root / "runtime/freeze.json", receipt)
    return receipt


def freeze_runtime(root: Path, chrome: Path, replay_binary: Path, native_parity_binary: Path) -> dict[str, Any]:
    f1.require(socket.gethostname() == "ITSO-WX6303" and (root / "runtime/setup-start.json").is_file(), "MBP setup start marker required before runtime freeze")
    f1.require(not (root / "runtime/freeze.json").exists(), "runtime already frozen")
    attempts = root / "runtime/freeze-attempts"
    attempt = 0 if not (attempts / "attempt-00").exists() else 1
    f1.require(attempt <= 1 and not (attempts / "attempt-01").exists(), "runtime freeze attempt limit reached")
    base = attempts / f"attempt-{attempt:02d}"
    started = time.monotonic()
    f1.immutable_json(base / "launch.json", {"schema": "haitaka-anhoku-strength-first-f1-runtime-freeze-launch-v1", "attempt": attempt, "startedAtUtc": f1.utc_now(), "host": socket.gethostname()})
    try:
        return _freeze_runtime_once(root, chrome, replay_binary, native_parity_binary)
    finally:
        f1.immutable_json(base / "duration.json", {"schema": "haitaka-anhoku-strength-first-f1-runtime-freeze-duration-v1", "attempt": attempt, "endedAtUtc": f1.utc_now(), "elapsedSeconds": time.monotonic() - started})


def _runtime(root: Path) -> dict[str, Any]:
    value = _load(root / "runtime/freeze.json")
    f1.require(value.get("experimentId") == f1.EXPERIMENT_ID and value.get("settings", {}).get("hashMb") == 16, "runtime freeze differs")
    for role in ("page", "worker", "parityPage", "parityWorker", "smokeWorker", "terminalWorker", "parityCorpus", "nativeParityBinary", "wasm", "glue", "replayBinary", "package", "fixtureSchedule"):
        f1.verify_artifact(value[role])
    for name, item in value["models"].items():
        f1.verify_artifact(item if name == "I" else item["network"])
        if name != "I":
            f1.verify_artifact(item["selection"])
            f1.verify_artifact(item["run"])
    for item in value["schedules"].values():
        f1.verify_artifact(item)
    f1.require(socket.gethostname() == value["host"]["name"] == "ITSO-WX6303", "F1 match host changed")
    f1.verify_artifact(value["host"]["chrome"])
    return value


def _grant(root: Path, phase: str) -> dict[str, Any]:
    runtime = _runtime(root)
    qualification_path = root / "runtime/qualification.json"
    qualification = _load(qualification_path)
    f1.require(qualification.get("passed") is True and qualification.get("runtimeSha256") == f1.sha256(root / "runtime/freeze.json"), "runtime qualification absent or stale")
    path = root / "authorization/match.json"
    grant = _load(path)
    f1.require(grant.get("schema") == "haitaka-anhoku-strength-first-f1-match-grant-v1" and grant.get("authorized") is True and grant.get("userReference"), "F1 scored-game grant absent")
    f1.require(grant.get("experimentId") == f1.EXPERIMENT_ID and grant.get("runtimeSha256") == f1.sha256(root / "runtime/freeze.json") and grant.get("packageSha256") == f1.sha256(root / "package.json"), "match grant binding differs")
    f1.require(_future_utc(grant["validUntilUtc"]), "match grant expired")
    f1.require(grant.get("host") == runtime["host"]["name"] and grant.get("oneHost") is True and grant.get("scoredGamesAuthorized") is True, "match host or scored scope differs")
    f1.require(grant.get("setupLedgerSha256") == f1.sha256(root / "runtime/setup-ledger.json"), "match setup wall ledger binding differs")
    f1.require(grant.get("maximumHostHours", 99) <= 32 and grant.get("maximumScoredGames", 9999) <= 1536, "match grant cap differs")
    if phase == "holdout":
        analysis = _load(root / "analysis/development.json")
        f1.require(analysis.get("screenAdvances") is True, "holdout blocked by development screen")
    return {"grant": f1.artifact(path), "runtime": runtime}


def _qualification_grant(root: Path) -> dict[str, Any]:
    runtime = _runtime(root)
    path = root / "authorization/match.json"
    grant = _load(path)
    f1.require(grant.get("schema") == "haitaka-anhoku-strength-first-f1-match-grant-v1" and grant.get("experimentId") == f1.EXPERIMENT_ID and grant.get("authorized") is True and grant.get("qualificationAuthorized") is True and grant.get("userReference"), "F1 runtime qualification grant absent")
    f1.require(grant.get("runtimeSha256") == f1.sha256(root / "runtime/freeze.json") and grant.get("packageSha256") == f1.sha256(root / "package.json"), "qualification grant binding differs")
    f1.require(_future_utc(grant["validUntilUtc"]), "qualification grant expired")
    f1.require(grant.get("host") == runtime["host"]["name"] and grant.get("oneHost") is True, "qualification host grant differs")
    f1.require(grant.get("setupLedgerSha256") == f1.sha256(root / "runtime/setup-ledger.json"), "qualification setup wall ledger binding differs")
    f1.require(grant.get("maximumHostHours", 99) <= 32 and grant.get("maximumScoredGames", 9999) <= 1536 and _spent_seconds(root) + 2 * 3600 <= HOST_SECONDS_CAP, "qualification host wall cap lacks two-hour reserve")
    return runtime


def _simple_browser(root: Path, runtime: dict[str, Any], name: str, page: Path, worker: Path, model: dict[str, Any], source: Path, payload: dict[str, Any], output: Path, *, timeout: float) -> dict[str, Any]:
    files = {"page": f1.artifact(page), "worker": f1.artifact(worker), "wasm": runtime["wasm"], "glue": runtime["glue"], "modelA": model, "input": f1.artifact(source)}
    routes = {"page": "/page.html", "worker": "/worker.js", "wasm": "/pkg/haitaka_wasm_bg.wasm", "glue": "/pkg/haitaka_wasm.js", "modelA": "/model.nnue", "input": "/input.json"}
    provenance_core = {"schema": "haitaka-anhoku-strength-first-f1-qualification-provenance-v1", "name": name, "runtime": f1.artifact(root / "runtime/freeze.json"), "files": files, "routes": routes}
    provenance = {**provenance_core, "envelopeId": hashlib.sha256(f1.canonical_bytes(provenance_core)).hexdigest()}
    config = {**payload, "provenance": provenance}
    f1.require(not output.exists(), "qualification output already exists; use an append-only attempt directory")
    output.mkdir(parents=True)
    f1.immutable_json(output / "config.json", config)
    raw_dir = output / "raw"
    raw_dir.mkdir()
    serve = {"/": (page.read_bytes(), "text/html; charset=utf-8"), "/config.json": (f1.canonical_bytes(config), "application/json")}
    for role, route in routes.items():
        serve[route] = (f1.verify_artifact(files[role]).read_bytes(), "application/wasm" if role == "wasm" else "text/javascript" if role in ("worker", "glue") else "text/html" if role == "page" else "application/octet-stream")
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), transport.BrowserHandler)
    state = transport.BrowserState()
    server.browser_state = state  # type: ignore[attr-defined]
    server.browser_config = config  # type: ignore[attr-defined]
    server.browser_routes = serve  # type: ignore[attr-defined]
    server.browser_output_dir = raw_dir  # type: ignore[attr-defined]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    profile = output / "chrome-profile"
    profile.mkdir()
    argv = [str(f1.verify_artifact(runtime["host"]["chrome"])), "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--disable-background-networking", "--disable-component-update", f"--user-data-dir={profile}", f"http://127.0.0.1:{server.server_address[1]}/"]
    f1.immutable_json(output / "launch.json", {"argv": argv, "startedAtUtc": f1.utc_now(), "name": name})
    started = time.monotonic()
    process = None
    sampler = None
    cleanup = None
    failure = None
    try:
        with (raw_dir / "chrome.stdout.sealed").open("wb") as stdout, (raw_dir / "chrome.stderr.sealed").open("wb") as stderr:
            process = subprocess.Popen(argv, cwd=str(root), stdout=stdout, stderr=stderr, start_new_session=True)
            pgid = os.getpgid(process.pid)
            sampler = transport.RssSampler(process.pid, pgid)
            sampler.start()
            if not state.event.wait(timeout):
                failure = "qualification-timeout"
            with state.lock:
                if state.error is not None:
                    failure = state.error.get("category", "qualification-browser-error")
    finally:
        memory = sampler.stop() if sampler else {"status": "unavailable"}
        if process is not None:
            cleanup = transport.signal_owned_group(process, pgid, raw_dir)
            process.wait(timeout=10)
        server.shutdown(); server.server_close(); thread.join(timeout=2)
        f1.immutable_json(output / "telemetry.json", {"schema": "haitaka-anhoku-strength-first-f1-qualification-telemetry-v1", "elapsedSeconds": time.monotonic()-started, "memory": memory, "cleanup": cleanup, "failure": failure})
        shutil.rmtree(profile, ignore_errors=True)
    f1.require(failure is None and state.result is not None, f"qualification browser failed: {failure or 'missing-result'}")
    report_path = raw_dir / "report.json"
    f1.require(_load(report_path).get("status") == "passed", "qualification browser report failed")
    return {"report": f1.artifact(report_path), "telemetry": f1.artifact(output / "telemetry.json"), "provenance": provenance}


def _qualify_runtime_attempt(root: Path, attempt: int) -> dict[str, Any]:
    f1.require(type(attempt) is int and 0 <= attempt <= 1, "runtime qualification attempt outside bounded protocol")
    runtime = _qualification_grant(root)
    grant_binding = f1.artifact(root / "authorization/match.json")
    f1.require(not (root / "runtime/qualification.json").exists(), "runtime already qualified")
    base = root / f"runtime/qualification/attempt-{attempt:02d}"
    f1.require(not base.exists(), "runtime qualification attempt already exists")
    base.mkdir(parents=True)
    f1.immutable_json(base / "launch.json", {"schema": "haitaka-anhoku-strength-first-f1-runtime-qualification-launch-v1", "startedAtUtc": f1.utc_now(), "attempt": attempt, "host": socket.gethostname()})
    corpus = f1.verify_artifact(runtime["parityCorpus"])
    corpus_rows = [json.loads(line) for _, line in zip(range(128), corpus.open())]
    f1.require(len(corpus_rows) == 128 and {row["sfen"].split()[1] for row in corpus_rows} == {"b", "w"} and any(row.get("seedClass") == "donor-receiver" for row in corpus_rows), "128 parity positions lack required color/donor coverage")
    positions = [{"id": row["id"], "sfen": row["sfen"]} for row in corpus_rows]
    parity_input = base / "parity-input.json"
    f1.immutable_json(parity_input, {"schema": "haitaka-anhoku-strength-first-f1-parity-input-v1", "positions": positions, "source": runtime["parityCorpus"]})
    parity = {}
    smoke = {}
    terminal = {}
    native_binary = f1.verify_artifact(runtime["nativeParityBinary"])
    for name in ("I", "C", "T"):
        _grant_still_current(root / "authorization/match.json", grant_binding)
        model = runtime["models"][name] if name == "I" else runtime["models"][name]["network"]
        native_path = base / name / "native-parity.json"
        native_path.parent.mkdir(parents=True)
        completed = subprocess.run([str(native_binary), "--network", str(f1.verify_artifact(model)), "--corpus", str(corpus), "--count", "128", "--output", str(native_path)], capture_output=True, text=True, timeout=180, check=False)
        f1.require(completed.returncode == 0, f"{name} native parity failed: {completed.stderr[-500:]}")
        native = _load(native_path)
        f1.require(native.get("status") == "passed" and native.get("fullRefreshEqualsIncremental") is True and len(native.get("positions", [])) == 128, "native parity incomplete")
        _grant_still_current(root / "authorization/match.json", grant_binding)
        browser = _simple_browser(root, runtime, f"{name}-parity", PARITY_PAGE, PARITY_WORKER, model, parity_input, {"mode": "parity", "positions": positions, "budget": {"mode": "static-evaluation"}}, base / name / "browser-parity", timeout=300)
        observed = _load(f1.verify_artifact(browser["report"]))
        native_rows = {row["id"]: row for row in native["positions"]}
        f1.require(len(observed.get("positions", [])) == 128 and all(row["id"] in native_rows and row["sfen"] == native_rows[row["id"]]["sfen"] and row["score"] == native_rows[row["id"]]["score"] for row in observed["positions"]), f"{name} native/WASM integer parity mismatch")
        parity[name] = {"native": f1.artifact(native_path), "browser": browser["report"], "rows": 128, "mismatches": 0}
        smoke_input = base / "smoke-input.json"
        if not smoke_input.exists():
            f1.immutable_json(smoke_input, {"schema": "haitaka-anhoku-strength-first-f1-smoke-input-v1", "positions": positions[:8], "source": f1.artifact(parity_input)})
        _grant_still_current(root / "authorization/match.json", grant_binding)
        smoked = _simple_browser(root, runtime, f"{name}-smoke", PARITY_PAGE, SMOKE_WORKER, model, smoke_input, {"mode": "smoke", "positions": positions[:8], "budget": {"mode": "movetime", "movetimeMs": 1000, "nodes": None}}, base / name / "browser-smoke", timeout=180)
        traces = _load(f1.verify_artifact(smoked["report"])).get("searches", [])
        a_traces = [trace for trace in traces if trace.get("engine") == "A"]
        f1.require(len(a_traces) == 8 and all(trace.get("requestedMs") == 1000 and trace.get("command") == "go movetime 1000" and trace.get("gameover") is None and trace.get("rootResultSchema") == "haitaka-search-root-result-v1" and trace.get("playMoveWasSearched") is True and trace.get("emergencyFallbackUsed") is False and trace.get("missingMove") is False and type(trace.get("elapsedMs")) in (int, float) and 0 <= trace["elapsedMs"] <= 2000 for trace in a_traces), f"{name} timed searches failed")
        smoke[name] = {"browser": smoked["report"], "timedModelSearches": 8}
    step14_terminal = _load(f1.ROOT / "out/anhoku-strength-first/step14-browser-finalist-20260919-v1/qualification/terminal-input.json")["records"]
    step11_terminal = _load(f1.ROOT / "out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/preflight/terminal-replay-input.json")["records"]
    terminal_records = step14_terminal + [{"gameIndex": 4 + index, "openingId": f"step11-history-{index}", "startSfen": row["startSfen"], "moves": row["moves"], "expectedReason": row["terminationReason"]} for index, row in enumerate(step11_terminal[:2])]
    f1.require(len(terminal_records) == 6, "terminal/history fixture count differs")
    terminal_input = base / "terminal-input.json"
    f1.immutable_json(terminal_input, {"schema": "haitaka-anhoku-strength-first-step14-terminal-input-v1", "records": terminal_records, "queries": 12, "sourceStep14": f1.artifact(f1.ROOT / "out/anhoku-strength-first/step14-browser-finalist-20260919-v1/qualification/terminal-input.json"), "sourceStep11": f1.artifact(f1.ROOT / "out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/preflight/terminal-replay-input.json")})
    for name in ("I", "C", "T"):
        _grant_still_current(root / "authorization/match.json", grant_binding)
        model = runtime["models"][name] if name == "I" else runtime["models"][name]["network"]
        observed = _simple_browser(root, runtime, f"{name}-terminal", PAGE, TERMINAL_WORKER, model, terminal_input, {"schema": "haitaka-anhoku-strength-first-step14-terminal-config-v1", "records": terminal_records}, base / name / "browser-terminal", timeout=180)
        report = _load(f1.verify_artifact(observed["report"]))
        f1.require(report.get("execution", {}).get("terminalQueries") == 12 and len(report.get("replays", [])) == 6 and all(item["observed"][seat]["gameoverReason"] == item["expectedReason"] for item in report["replays"] for seat in ("A", "B")), f"{name} terminal/history queries failed")
        terminal[name] = {"browser": observed["report"], "queries": 12}
    schedule_path = root / "runtime/fixture-schedule.json"
    fixture_rows = _load(schedule_path)["rows"]
    f1.require(len(fixture_rows) == 8 and {row["matchup"] for row in fixture_rows} == {"T-C", "T-I"} and {row["treatmentColor"] for row in fixture_rows} == {"black", "white"}, "eight short fixture games do not cover pair orders/colors")
    fixtures = []
    for index, (opponent, subset) in enumerate(_ordered_parts(fixture_rows)):
        _grant_still_current(root / "authorization/match.json", grant_binding)
        output = base / f"fixture-part-{index:02d}-{opponent}"
        job = _browser(root, runtime, "fixture", 0, attempt, subset, output, fixture=True)
        raw_path = f1.verify_artifact(job["report"])
        raw = _load(raw_path)
        for game in raw["rawGames"]:
            _strict_searches(next(row for row in subset if row["gameId"] == game["gameId"]), game)
        subset_path = base / f"fixture-part-{index:02d}-{opponent}-schedule.json"
        f1.immutable_json(subset_path, {"schema": "haitaka-anhoku-strength-first-f1-fixture-schedule-v1", "rows": subset, "parent": f1.artifact(schedule_path)})
        replay_adapter.run(subset_path, raw_path, f1.verify_artifact(runtime["replayBinary"]), base / f"fixture-part-{index:02d}-{opponent}-replay", max_plies=2)
        fixtures.append({"opponent": opponent, "partIndex": index, "gameIds": [row["gameId"] for row in subset], "raw": job["report"], "replay": f1.artifact(base / f"fixture-part-{index:02d}-{opponent}-replay/replay.json"), "games": len(subset), "fixtureOnly": True})
    receipt = {"schema": "haitaka-anhoku-strength-first-f1-runtime-qualification-v1", "experimentId": f1.EXPERIMENT_ID, "createdAtUtc": f1.utc_now(), "passed": True, "runtimeSha256": f1.sha256(root / "runtime/freeze.json"), "attempt": attempt, "parity": parity, "smoke": smoke, "terminal": terminal, "fixtureGames": fixtures, "scoredGames": 0, "models": ["I", "C", "T"], "host": socket.gethostname()}
    f1.immutable_json(root / "runtime/qualification.json", receipt)
    return receipt


def qualify_runtime(root: Path, attempt: int) -> dict[str, Any]:
    started = time.monotonic()
    try:
        return _qualify_runtime_attempt(root, attempt)
    finally:
        base = root / f"runtime/qualification/attempt-{attempt:02d}"
        if (base / "launch.json").is_file():
            f1.immutable_json(base / "duration.json", {"schema": "haitaka-anhoku-strength-first-f1-runtime-qualification-duration-v1", "attempt": attempt, "endedAtUtc": f1.utc_now(), "elapsedSeconds": time.monotonic() - started})


def _spent_seconds(root: Path) -> float:
    setup = _load(root / "runtime/setup-ledger.json")
    f1.require(setup.get("schema") == "haitaka-anhoku-strength-first-f1-mbp-setup-ledger-v1" and setup.get("host") == "ITSO-WX6303" and setup.get("accountingMethod") == "retained-command-intervals-plus-monotonic-freeze-attempts" and setup.get("includesIdleTime") is False and setup.get("preexistingStagingUnmetered") is True and isinstance(setup.get("setupStart"), dict), "MBP setup activity ledger differs")
    f1.verify_artifact(setup["setupStart"])
    f1.verify_artifact(setup["runtimeFreeze"])
    f1.verify_artifact(setup["package"])
    activities, freezes, charged, first_active = _setup_components(root, datetime.now(timezone.utc))
    f1.require(activities == setup.get("activities") and freezes == setup.get("freezeAttempts") and activities and freezes, "MBP setup activity bindings differ")
    total = setup.get("elapsedSeconds")
    f1.require(type(total) in (int, float) and math.isfinite(total) and total >= 0 and abs(total - charged) < 0.001, "invalid MBP active setup duration")
    started, ended = _utc(setup["startedAtUtc"]), _utc(setup["endedAtUtc"])
    marker = _load(f1.verify_artifact(setup["setupStart"]))
    f1.require(started == min(_utc(marker["startedAtUtc"]), first_active) and ended <= datetime.now(timezone.utc) and abs((ended - started).total_seconds() - setup.get("wallIntervalSeconds", -1)) < 0.001 and setup.get("wallIntervalRole") == "upper-bound-includes-idle-and-unmetered-prior-staging" and total <= setup["wallIntervalSeconds"] + 5 * len(freezes), "MBP setup wall upper bound differs")
    for launch_path in (root / "runtime/qualification").glob("attempt-*/launch.json"):
        duration_path = launch_path.parent / "duration.json"
        if duration_path.is_file():
            duration = _load(duration_path).get("elapsedSeconds")
        else:
            launched = datetime.fromisoformat(_load(launch_path)["startedAtUtc"].replace("Z", "+00:00"))
            duration = (datetime.now(timezone.utc) - launched).total_seconds()
        f1.require(type(duration) in (int, float) and math.isfinite(duration) and duration >= 0, "invalid qualification wall ledger")
        total += duration
    for path in (root / "games").glob("*/block-*/attempt-*/launch.json"):
        duration_path = path.parent / "duration.json"
        if duration_path.is_file():
            duration = _load(duration_path).get("elapsedSeconds")
        else:
            launched = datetime.fromisoformat(_load(path)["startedAtUtc"].replace("Z", "+00:00"))
            duration = (datetime.now(timezone.utc) - launched).total_seconds()
        f1.require(type(duration) in (int, float) and math.isfinite(duration) and duration >= 0, "invalid host wall ledger")
        total += duration
    return total


def _budget_timeout(deadline: float | None, proposed: float, reserve: float) -> float:
    if deadline is None:
        return proposed
    remaining = deadline - time.monotonic() - reserve
    f1.require(math.isfinite(remaining) and remaining > 1, "F1 host wall cap leaves no safe dispatch time")
    return min(proposed, remaining)


def _provenance(root: Path, runtime: dict[str, Any], phase: str, block: int, opponent: str, rows: list[dict[str, Any]]) -> dict[str, Any]:
    schedule = runtime["fixtureSchedule"] if phase == "fixture" else runtime["schedules"][phase]
    files = {"page": runtime["page"], "worker": runtime["worker"], "wasm": runtime["wasm"], "glue": runtime["glue"], "modelT": runtime["models"]["T"]["network"], "modelOpponent": runtime["models"][opponent]["network"] if opponent != "I" else runtime["models"]["I"], "schedule": schedule, "contract": f1.artifact(root / "contract.json")}
    routes = {"page": "/page.html", "worker": "/worker.js", "wasm": "/pkg/haitaka_wasm_bg.wasm", "glue": "/pkg/haitaka_wasm.js", "modelT": "/model-t.nnue", "modelOpponent": "/model-opponent.nnue", "schedule": "/schedule.json", "contract": "/contract.json"}
    core = {"schema": "haitaka-anhoku-strength-first-f1-provenance-v1", "experimentId": f1.EXPERIMENT_ID, "phase": phase, "blockIndex": block, "opponent": opponent, "runtime": f1.artifact(root / "runtime/freeze.json"), "files": files, "routes": routes, "gameIds": [row["gameId"] for row in rows]}
    return {**core, "envelopeId": hashlib.sha256(f1.canonical_bytes(core)).hexdigest()}


def _browser(root: Path, runtime: dict[str, Any], phase: str, block: int, attempt: int, rows: list[dict[str, Any]], output: Path, *, fixture: bool = False, deadline: float | None = None) -> dict[str, Any]:
    # One browser for each matchup preserves a single opponent network in the worker.
    opponent = rows[0]["matchup"][-1]
    f1.require(all(row["matchup"] == f"T-{opponent}" for row in rows), "mixed opponent in browser process")
    provenance = _provenance(root, runtime, phase, block, opponent, rows)
    max_plies = 2 if fixture else 200
    config = {"schema": "haitaka-anhoku-strength-first-f1-browser-config-v1", "maxPlies": max_plies, "expectedRawGames": len(rows), "games": rows, "provenance": provenance}
    f1.require(not output.exists(), "append-only browser attempt already exists")
    raw_dir = output / "raw"
    raw_dir.mkdir(parents=True)
    f1.immutable_json(output / "config.json", config)
    routes = {"/": (PAGE.read_bytes(), "text/html; charset=utf-8"), "/page.html": (PAGE.read_bytes(), "text/html; charset=utf-8"), "/worker.js": (WORKER.read_bytes(), "text/javascript; charset=utf-8"), "/config.json": (f1.canonical_bytes(config), "application/json")}
    for name in ("wasm", "glue", "modelT", "modelOpponent", "schedule", "contract"):
        routes[provenance["routes"][name]] = (f1.verify_artifact(provenance["files"][name]).read_bytes(), "application/wasm" if name == "wasm" else "text/javascript" if name == "glue" else "application/octet-stream")
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), transport.BrowserHandler)
    state = transport.BrowserState()
    server.browser_state = state  # type: ignore[attr-defined]
    server.browser_config = config  # type: ignore[attr-defined]
    server.browser_routes = routes  # type: ignore[attr-defined]
    server.browser_output_dir = raw_dir  # type: ignore[attr-defined]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    url = f"http://127.0.0.1:{server.server_address[1]}/"
    profile = output / "chrome-profile"
    profile.mkdir()
    argv = [str(f1.verify_artifact(runtime["host"]["chrome"])), "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--disable-background-networking", "--disable-component-update", f"--user-data-dir={profile}", url]
    launch_time = f1.utc_now()
    f1.immutable_json(output / "launch.json", {"argv": argv, "url": url, "phase": phase, "block": block, "attempt": attempt, "fixture": fixture, "startedAtUtc": launch_time})
    started = time.monotonic()
    process = None
    sampler = None
    ownership: dict[str, Any] = {}
    cleanup = None
    failure = None
    try:
        with (raw_dir / "chrome.stdout.sealed").open("wb") as stdout, (raw_dir / "chrome.stderr.sealed").open("wb") as stderr:
            process = subprocess.Popen(argv, cwd=str(root), stdout=stdout, stderr=stderr, start_new_session=True)
            pgid = os.getpgid(process.pid)
            ownership = {"pid": process.pid, "processGroupId": pgid, "argv": argv, "profile": str(profile)}
            f1.immutable_json(output / "ownership.json", ownership)
            sampler = transport.RssSampler(process.pid, pgid)
            sampler.start()
            timeout = _budget_timeout(deadline, max(90.0, len(rows) * max_plies * 2.0 + 120.0), 900.0)
            if not state.event.wait(timeout):
                failure = "browser-timeout"
            with state.lock:
                if state.error is not None:
                    failure = state.error.get("category", "browser-error")
    except OSError as exc:
        failure = f"controller-launch:{type(exc).__name__}"
    finally:
        memory = sampler.stop() if sampler is not None else {"status": "unavailable", "sampleCount": 0}
        if process is not None:
            cleanup = transport.signal_owned_group(process, ownership["processGroupId"], raw_dir)
            process.wait(timeout=10)
        server.shutdown(); server.server_close(); thread.join(timeout=2)
        elapsed = time.monotonic() - started
        f1.immutable_json(output / "telemetry.json", {"schema": "haitaka-anhoku-strength-first-f1-browser-telemetry-v1", "elapsedSeconds": elapsed, "memory": memory, "ownership": ownership, "cleanup": cleanup, "failure": failure, "startedAtUtc": launch_time, "endedAtUtc": f1.utc_now()})
        shutil.rmtree(profile, ignore_errors=True)
    f1.require(failure is None and state.result is not None, f"browser block failed: {failure or 'missing-result'}")
    report = _load(raw_dir / "report.json")
    f1.require(report.get("status") == "passed" and len(report.get("rawGames", [])) == len(rows), "browser raw report incomplete")
    return {"report": f1.artifact(raw_dir / "report.json"), "telemetry": f1.artifact(output / "telemetry.json"), "provenance": provenance}


def _strict_searches(row: dict[str, Any], raw: dict[str, Any]) -> None:
    moves = raw.get("moves")
    searches = raw.get("searches")
    f1.require(isinstance(moves, list) and isinstance(searches, list) and raw.get("plies") == len(moves), "raw move trace count differs")
    f1.require(raw.get("freshInstances") is True and raw.get("perInstanceNetworks") is True, "fresh per-instance network loading absent")
    for name in ("T", "opponent"):
        handshake = raw.get("handshake", {}).get(name, {})
        f1.require(handshake.get("hash") == [] and "readyok" in handshake.get("ready", []) and isinstance(raw.get("modelDescriptions", {}).get(name), str), "Hash/ready/model handshake differs")
    played = 0
    for index, trace in enumerate(searches):
        f1.require(trace.get("gameId") == row["gameId"] and trace.get("moveIndex") == index and trace.get("ply") == index + 1, "search identity differs")
        color = row["initialSideToMove"] if index % 2 == 0 else ("white" if row["initialSideToMove"] == "black" else "black")
        role = row["blackEngine"] if color == "black" else row["whiteEngine"]
        f1.require(trace.get("color") == color and trace.get("engine") == role, "search side/engine differs")
        position = f"position sfen {row['sfen']}" + (" moves " + " ".join(moves[:played]) if played else "")
        f1.require(trace.get("positionCommand") == position and trace.get("command") == "go movetime 1000" and trace.get("requestedMs") == 1000, "timer/history command differs")
        elapsed = trace.get("elapsedMs")
        f1.require(type(elapsed) in (int, float) and math.isfinite(elapsed) and 0 <= elapsed <= 2000, "search actual elapsed time invalid")
        terminal = trace.get("terminalReason") is not None
        if terminal:
            f1.require(index == len(searches) - 1 and trace.get("bestMove") == "resign" and trace.get("playMoveWasSearched") is False and trace.get("reportedTimeMs") is None, "terminal search trace differs")
        else:
            f1.require(played < len(moves) and moves[played] == trace.get("bestMove"), "played move differs from search")
            f1.require(trace.get("rootResultSchema") == "haitaka-search-root-result-v1" and trace.get("playMoveWasSearched") is True and trace.get("missingMove") is False and trace.get("emergencyFallbackUsed") is False and trace.get("unsearchedMove") is False, "root search flags differ")
            f1.require(type(trace.get("reportedTimeMs")) is int and 0 <= trace["reportedTimeMs"] <= 2000, "engine-reported time differs")
            played += 1
    f1.require(played == len(moves) and (raw["rawEnd"] == "move-limit-reached") == (len(searches) == len(moves)), "terminal/cap trace differs")


def _ordered_parts(rows: list[dict[str, Any]]) -> list[tuple[str, list[dict[str, Any]]]]:
    """Keep the frozen gameIndex order while using one opponent per browser."""
    f1.require(bool(rows) and [row["gameIndex"] for row in rows] == list(range(rows[0]["gameIndex"], rows[0]["gameIndex"] + len(rows))), "scheduled game chronology differs")
    parts: list[tuple[str, list[dict[str, Any]]]] = []
    for row in rows:
        opponent = row["matchup"][-1]
        f1.require(row["matchup"] == f"T-{opponent}" and opponent in ("C", "I"), "scheduled opponent differs")
        if parts and parts[-1][0] == opponent:
            parts[-1][1].append(row)
        else:
            parts.append((opponent, [row]))
    return parts


def _canonical_replay_result(game: dict[str, Any], replay: dict[str, Any]) -> tuple[float, bool]:
    """A browser cap is provisional; replay adjudicates the final position."""
    cap = game["rawEnd"] == "move-limit-reached"
    if cap:
        f1.require(game["plies"] == 200 and game["rawWinnerColor"] is None, "provisional cap differs")
    else:
        f1.require(replay.get("terminationReason") == game["rawEnd"] and replay.get("winnerColor") == game["rawWinnerColor"], "independent terminal replay differs")
    f1.require(replay.get("terminationReason") in ("king-captured", "no-legal-move", "repetition-draw", "perpetual-check-loss", "jishogi-draw", "maximum-ply-capped"), "canonical terminal reason differs")
    if replay["terminationReason"] == "maximum-ply-capped":
        f1.require(cap and replay.get("winner") is None and replay.get("winnerColor") is None, "unreported replay cap")
    f1.require(replay.get("winner") in (None, "A", "B"), "canonical winner differs")
    score = 0.5 if replay["winner"] is None else 1 if replay["winner"] == "A" else 0
    return score, replay["terminationReason"] == "maximum-ply-capped"


def _accept(root: Path, phase: str, block: int, attempt: int, runtime: dict[str, Any], parts: list[dict[str, Any]], block_dir: Path, *, deadline: float, attempt_started: float) -> dict[str, Any]:
    schedule_path = root / f"openings/{phase}-schedule.json"
    rows = [row for row in _load(schedule_path)["rows"] if row["blockIndex"] == block]
    by_id = {row["gameId"]: row for row in rows}
    f1.require(len(rows) == (32 if phase == "development" else 16) and len(by_id) == len(rows), "block schedule differs")
    accepted = []
    observed_order = []
    wall = 0.0
    for part in parts:
        _budget_timeout(deadline, 900.0, 60.0)
        raw_path = f1.verify_artifact(part["report"])
        raw = _load(raw_path)
        raw_games = raw["rawGames"]
        observed_order.extend(game["gameId"] for game in raw_games)
        f1.require(raw.get("provenance", {}).get("envelopeId") == part["provenance"]["envelopeId"], "provenance envelope differs")
        f1.require(len(raw_games) == len({game["gameId"] for game in raw_games}), "duplicate raw F1 game")
        subset = [by_id[game["gameId"]] for game in raw_games]
        for game in raw_games:
            _strict_searches(by_id[game["gameId"]], game)
        subset_path = raw_path.parent.parent / "block-schedule.json"
        f1.immutable_json(subset_path, {"schema": "haitaka-anhoku-strength-first-f1-block-schedule-v1", "phase": phase, "blockIndex": block, "parent": f1.artifact(schedule_path), "rows": subset})
        replay_dir = raw_path.parent.parent / "replay"
        receipt = replay_adapter.run(subset_path, raw_path, f1.verify_artifact(runtime["replayBinary"]), replay_dir, max_plies=200, timeout_seconds=_budget_timeout(deadline, 900.0, 60.0))
        for matchup, group in receipt["groups"].items():
            mapping = _load(f1.verify_artifact(group["mapping"]))["rows"]
            replayed = [json.loads(line) for line in f1.verify_artifact(group["output"]).read_text().splitlines() if line]
            replay_by_index = {item["globalGameIndex"]: item for item in replayed}
            for item in mapping:
                game_id = item["f1GameId"]
                replay = replay_by_index[item["replayGameIndex"]]
                game = next(game for game in raw_games if game["gameId"] == game_id)
                f1.require(replay.get("status") == "passed" and replay.get("searchDispatched") is False and replay.get("modelLoaded") is False and replay.get("browserLaunched") is False, "replay did work or failed")
                f1.require(replay.get("replayedPlies") == game["plies"], "independent replay ply count differs")
                f1.require(all(replay.get("checks", {}).values()), "independent replay check false")
                score, genuine_cap = _canonical_replay_result(game, replay)
                accepted.append({"gameId": game_id, "schedule": by_id[game_id], "valid": True, "scoreTreatment": score, "genuinePlyCap": genuine_cap, "raw": f1.artifact(raw_path.parent / "games" / f"game-{by_id[game_id]['gameIndex']:04d}.json"), "replay": {"searchFree": True, "legal": True, "receipt": group["output"], "replayGameIndex": item["replayGameIndex"], "mapping": group["mapping"]}})
        wall += _load(f1.verify_artifact(part["telemetry"]))["elapsedSeconds"]
    f1.require(len(accepted) == len(rows) and {item["gameId"] for item in accepted} == set(by_id), "accepted block incomplete")
    f1.require(observed_order == [row["gameId"] for row in rows], "actual browser game chronology differs from frozen schedule")
    receipt = {"schema": "haitaka-anhoku-strength-first-f1-accepted-block-v1", "experimentId": f1.EXPERIMENT_ID, "phase": phase, "blockIndex": block, "attempt": attempt, "scheduleSha256": f1.sha256(schedule_path), "runtimeFreeze": f1.artifact(root / "runtime/freeze.json"), "parts": parts, "games": sorted(accepted, key=lambda item: item["schedule"]["gameIndex"]), "browserSeconds": wall, "wallSeconds": time.monotonic() - attempt_started, "createdAtUtc": f1.utc_now()}
    f1.immutable_json(block_dir / "accepted.json", receipt)
    return receipt


def run_block(root: Path, phase: str, block: int) -> dict[str, Any]:
    f1.require(phase in ("development", "holdout") and 0 <= block < (16 if phase == "development" else 64), "invalid F1 block")
    gate = _grant(root, phase)
    runtime = gate["runtime"]
    lock_path = root / "games/.active.lock"
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with lock_path.open("a+b") as lock:
        try:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise f1.F1Error("another F1 game controller is active") from exc
        _grant(root, phase)
        f1.require(shutil.disk_usage(root).free >= 16 * 1024**3, "match host archival disk reserve exhausted")
        spent = _spent_seconds(root)
        expected_games = 32 if phase == "development" else 16
        f1.require(spent + expected_games * 200 + 900 <= HOST_SECONDS_CAP, "insufficient host wall budget for worst-case block and archive reserve")
        block_dir = root / f"games/{phase}/block-{block:02d}"
        f1.require(not (block_dir / "accepted.json").exists(), "scored block already accepted")
        attempt = 0 if not (block_dir / "attempt-00").exists() else 1
        f1.require(attempt <= 1 and (attempt == 0 or (block_dir / "attempt-00/technical-invalid.json").is_file()), "block retry requires established technical invalidity")
        if attempt == 1:
            retries = sum(1 for path in (root / "games").glob("*/block-*/attempt-01") if path.is_dir())
            f1.require(retries < 4, "global retried-block cap reached")
        attempt_dir = block_dir / f"attempt-{attempt:02d}"
        started = time.monotonic()
        deadline = started + HOST_SECONDS_CAP - spent
        f1.immutable_json(attempt_dir / "launch.json", {"schema": "haitaka-anhoku-strength-first-f1-block-attempt-launch-v1", "phase": phase, "blockIndex": block, "attempt": attempt, "startedAtUtc": f1.utc_now()})
        try:
            rows = [row for row in _load(root / f"openings/{phase}-schedule.json")["rows"] if row["blockIndex"] == block]
            f1.require([row["gameIndex"] for row in rows] == list(range(block * expected_games, (block + 1) * expected_games)), "block game indices differ from frozen chronology")
            parts = []
            for index, (opponent, subset) in enumerate(_ordered_parts(rows)):
                _grant_still_current(root / "authorization/match.json", gate["grant"])
                parts.append(_browser(root, runtime, phase, block, attempt, subset, attempt_dir / f"part-{index:02d}-{opponent}", deadline=deadline))
            try:
                return _accept(root, phase, block, attempt, runtime, parts, block_dir, deadline=deadline, attempt_started=started)
            except f1.F1Error:
                failed_replays = []
                for path in attempt_dir.glob("*/replay/*-replay-output.jsonl"):
                    try:
                        replay_rows = [json.loads(line) for line in path.read_text().splitlines() if line]
                    except (OSError, ValueError):
                        continue
                    if any(row.get("status") == "failed" for row in replay_rows):
                        failed_replays.append(f1.artifact(path))
                if failed_replays:
                    f1.immutable_json(attempt_dir / "technical-invalid.json", {"schema": "haitaka-anhoku-strength-first-f1-technical-invalid-v1", "phase": phase, "blockIndex": block, "attempt": attempt, "cause": "retained-search-free-replay-failed", "failedReplayOutputs": failed_replays, "strengthResultUsed": False})
                raise
        finally:
            f1.immutable_json(attempt_dir / "duration.json", {"schema": "haitaka-anhoku-strength-first-f1-block-attempt-duration-v1", "phase": phase, "blockIndex": block, "attempt": attempt, "endedAtUtc": f1.utc_now(), "elapsedSeconds": time.monotonic() - started})


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=f1.F1_ROOT)
    sub = parser.add_subparsers(dest="command", required=True)
    freeze = sub.add_parser("freeze-runtime")
    freeze.add_argument("--chrome", type=Path, required=True)
    freeze.add_argument("--replay-binary", type=Path, required=True)
    freeze.add_argument("--native-parity-binary", type=Path, required=True)
    qualify = sub.add_parser("qualify-runtime")
    qualify.add_argument("--attempt", type=int, choices=(0, 1), default=0)
    run = sub.add_parser("run-block")
    run.add_argument("phase", choices=("development", "holdout"))
    run.add_argument("block", type=int)
    sub.add_parser("record-setup-start")
    activity = sub.add_parser("record-setup-activity")
    activity.add_argument("--segment", type=int, required=True)
    activity.add_argument("--receipt", type=Path, required=True)
    setup_command = sub.add_parser("run-setup-command")
    setup_command.add_argument("--segment", type=int, required=True)
    setup_command.add_argument("setup_command", nargs=argparse.REMAINDER)
    sub.add_parser("record-setup")
    args = parser.parse_args()
    try:
        if args.command == "freeze-runtime":
            result = freeze_runtime(args.root.resolve(), args.chrome.resolve(), args.replay_binary.resolve(), args.native_parity_binary.resolve())
        elif args.command == "qualify-runtime":
            result = qualify_runtime(args.root.resolve(), args.attempt)
        elif args.command == "record-setup-start":
            result = record_setup_start(args.root.resolve())
        elif args.command == "record-setup-activity":
            result = record_setup_activity(args.root.resolve(), args.segment, args.receipt.resolve())
        elif args.command == "run-setup-command":
            status = run_setup_command(args.root.resolve(), args.segment, args.setup_command)
            print(json.dumps({"status": "recorded", "segment": args.segment, "commandExitCode": status}, sort_keys=True))
            return status
        elif args.command == "record-setup":
            result = record_setup(args.root.resolve())
        else:
            result = run_block(args.root.resolve(), args.phase, args.block)
    except (f1.F1Error, OSError, KeyError, ValueError, TypeError, subprocess.TimeoutExpired) as exc:
        print(f"F1 live blocked: {exc}", file=sys.stderr)
        return 2
    print(json.dumps({"status": "passed", "artifact": result.get("phase", "runtime-freeze")}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
