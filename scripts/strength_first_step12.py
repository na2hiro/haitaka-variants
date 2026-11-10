#!/usr/bin/env python3
"""Fail-closed preparation adapter for the Step 12 replication runbook.

This adapter deliberately makes the preparation boundary executable before it
makes the scientific phases executable.  ``prepare`` and ``fixtures`` inspect
explicitly named inputs and write auditable receipts.  Generation, training,
matching, and closeout commands exist as bounded interfaces but refuse to run
until their phase implementations, bindings, fixtures, and authorization
receipts are present.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shlex
import shutil
import subprocess
import sys
from typing import Any, Callable


REPO_ROOT = Path(__file__).resolve().parents[1]
EXPERIMENT_ID = "strength-first-step12-independent-replication-20260916-v1"
ROOT_NAME = "out/anhoku-strength-first/step12-independent-replication-20260916-v1"
PROTECTED_TOKEN = "step9-independent-confirmation-20260913-abandoned-1000ms-partial-unopened"
PROTECTED_PATH = f"out/anhoku-strength-first/{PROTECTED_TOKEN}"
SOURCE_CONFIG_REL = "source/step12-source.toml"
SOURCE_OUTPUT_REL = "source/generated"
SOURCE_FULL_OUTPUT_REL = "source/automatic-parallelism-calibration-generated"
HOST_CONNECTIVITY_REL = "preflight/host-connectivity.json"
SOURCE_BINARY = "target/release/haitaka_learn"
SOURCE_GENERATOR = "haitaka_learn/src/dataset.rs"
CANONICALIZATION_SOURCE = "haitaka_learn/src/step11_opening_gate.rs"
INVENTORY_HELPER = "haitaka_learn/src/step12_exclusion_inventory.rs"
CORPUS_FREEZE_IMPLEMENTATION = "haitaka_learn/src/step12_corpus_freeze.rs"
OPENING_IMPLEMENTATION = "haitaka_learn/src/step12_openings.rs"
OPENING_VALIDATION_IMPLEMENTATION = "haitaka_learn/src/step12_opening_validation.rs"
OPENING_PAIRING_IMPLEMENTATION = "haitaka_learn/src/openings.rs"
OPENING_PAIRING_DOCUMENTATION = "docs/nnue-training-anhoku-v0.6-phase2.md"
CORPUS_FREEZE_SCHEMA = "haitaka-anhoku-strength-first-step12-corpus-freeze-v1"
CORPUS_SELECTION_SCHEMA = "haitaka-anhoku-strength-first-step12-corpus-selection-v1"
CORPUS_VALIDATION_SCHEMA = "haitaka-anhoku-strength-first-step12-corpus-independent-validation-v1"
SOURCE_RECORD_MAP_SCHEMA = "haitaka-anhoku-strength-first-step12-source-record-map-v1"
REMOTE_HOST = "192.168.0.12"
REMOTE_ROOT = "/Users/na2hiro/anhoku-strength-first-step12-20260916-v1"
REMOTE_REPO = f"{REMOTE_ROOT}/repo"
REMOTE_BINARY = f"{REMOTE_REPO}/target/release/haitaka_learn"
REMOTE_SOURCE_OUTPUT = f"{REMOTE_ROOT}/source/automatic-parallelism-calibration-generated"
REMOTE_INVENTORIES = f"{REMOTE_ROOT}/preflight/exclusion-families"
REMOTE_OPENING_ROOT = f"{REMOTE_ROOT}/opening-freeze"
REMOTE_OPENING_INPUTS = f"{REMOTE_OPENING_ROOT}/inputs"
REMOTE_CARGO = "/Users/na2hiro/.cargo/bin/cargo"

CANDIDATE_UPDATES = [128, 256, 384, 512, 640, 768, 896, 1024]
CONTROL_UPDATE = 2688
SELECTION_TOLERANCE = 1e-12
SEEDS = {
    "sourceGames": 2026091601,
    "sourceSplit": 2026091602,
    "sourceShuffle": 2026091603,
    "corpusSelection": 2026091604,
    "initialization": 2026091605,
    "developmentOpenings": 2026091606,
    "match": 2026091607,
    "bootstrap": 2026091608,
}

SCHEMAS = {
    "contract": "haitaka-anhoku-strength-first-step12-contract-v1",
    "authorization": "haitaka-anhoku-strength-first-step12-authorization-v1",
    "bindings": "haitaka-anhoku-strength-first-step12-execution-bindings-v1",
    "commands": "haitaka-anhoku-strength-first-step12-command-bindings-v1",
    "unresolved": "haitaka-anhoku-strength-first-step12-unresolved-v1",
    "fixtures": "haitaka-anhoku-strength-first-step12-preflight-fixtures-v1",
    "launchGate": "haitaka-anhoku-strength-first-step12-launch-gate-v1",
    "trainingGate": "haitaka-anhoku-strength-first-step12-training-gate-v1",
    "allowlist": "haitaka-anhoku-strength-first-step12-exclusion-allowlist-v1",
    "status": "haitaka-anhoku-strength-first-step12-status-v1",
    "manifest": "haitaka-anhoku-strength-first-step12-manifest-v1",
    "sourceIdentity": "haitaka-anhoku-strength-first-step12-source-identity-v1",
    "exclusionFreeze": "haitaka-anhoku-strength-first-step12-exclusion-freeze-v1",
    "hostConnectivity": "haitaka-anhoku-strength-first-step12-host-connectivity-v1",
    "dataFreeze": CORPUS_FREEZE_SCHEMA,
    "dataSelection": CORPUS_SELECTION_SCHEMA,
    "dataValidation": CORPUS_VALIDATION_SCHEMA,
    "sourceRecordMap": SOURCE_RECORD_MAP_SCHEMA,
    "openingSpec": "haitaka-anhoku-strength-first-step12-opening-spec-v1",
    "openingGeneration": "haitaka-anhoku-strength-first-step12-opening-generation-v1",
    "openingValidation": "haitaka-anhoku-strength-first-step12-opening-validation-v1",
    "openingFreeze": "haitaka-anhoku-strength-first-step12-opening-freeze-v1",
    "openingGate": "haitaka-anhoku-strength-first-step12-opening-freeze-gate-v1",
}


class Step12Error(RuntimeError):
    """A sanitized, expected preparation or gate failure."""


def fail(message: str) -> None:
    raise Step12Error(message)


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def digest(path: Path) -> str:
    value = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                value.update(chunk)
    except OSError as exc:
        fail(f"cannot hash {path}: {exc}")
    return value.hexdigest()


def digest_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        fail(f"cannot load JSON {path}: {exc}")
    if not isinstance(value, dict):
        fail(f"JSON root is not an object: {path}")
    return value


def protected(path_value: str) -> bool:
    return PROTECTED_TOKEN in path_value


def repo_path(path_value: str) -> Path:
    """Resolve one explicitly named repository path without traversing the protected path."""

    if protected(path_value):
        fail(f"protected historical path is forbidden: {PROTECTED_TOKEN}")
    path = (REPO_ROOT / path_value).resolve()
    if PROTECTED_TOKEN in str(path):
        fail(f"protected historical path is forbidden: {PROTECTED_TOKEN}")
    return path


def root_path(root: Path, relative: str) -> Path:
    if protected(str(root)) or protected(relative):
        fail(f"protected historical path is forbidden: {PROTECTED_TOKEN}")
    path = (root / relative).resolve()
    if protected(str(path)):
        fail(f"protected historical path is forbidden: {PROTECTED_TOKEN}")
    return path


def artifact(path_value: str, expected_sha256: str | None = None) -> dict[str, Any]:
    """Return an observed or unresolved artifact without inventing a hash."""

    path = repo_path(path_value)
    item: dict[str, Any] = {"path": path_value, "bytes": None, "sha256": None, "status": "missing"}
    if not path.is_file():
        return item
    actual = digest(path)
    item.update({"bytes": path.stat().st_size, "sha256": actual, "status": "observed"})
    if expected_sha256 is not None:
        item["expectedSha256"] = expected_sha256
        if actual != expected_sha256:
            item["status"] = "mismatch"
    return item


def root_artifact(root: Path, relative: str) -> dict[str, Any]:
    path = root_path(root, relative)
    if not path.is_file():
        return {"path": relative, "bytes": None, "sha256": None, "status": "missing"}
    return {"path": relative, "bytes": path.stat().st_size, "sha256": digest(path), "status": "observed"}


def training_authorization_resolution(root: Path) -> dict[str, Any] | None:
    """Validate the post-training reconciliation of the preparation blocker."""
    path = root_path(root, "preflight/training-authorization-resolution.json")
    if not path.is_file():
        return None
    value = load_json(path)
    if value.get("schema") != "haitaka-anhoku-strength-first-step12-training-authorization-resolution-v1" or value.get("status") != "resolved" or value.get("experimentId") != EXPERIMENT_ID or value.get("resolves") != "training-authorization":
        fail("training-authorization resolution receipt is invalid")
    if value.get("kind") != "reconcile-stale-preparation-blocker":
        fail("training-authorization resolution kind differs")
    for section, names in {
        "grantEvidence": ("trainingAuthorization", "gpuAuthorization"),
        "productionEvidence": ("package", "config", "run", "finalizationRepair"),
    }.items():
        fields = value.get(section)
        if not isinstance(fields, dict):
            fail(f"training-authorization resolution {section} is missing")
        for name in names:
            item = fields.get(name)
            if not isinstance(item, dict) or not isinstance(item.get("path"), str) or not isinstance(item.get("bytes"), int) or not isinstance(item.get("sha256"), str):
                fail(f"training-authorization resolution {section}.{name} is not an artifact reference")
            bound = root_path(root, item["path"])
            if not bound.is_file() or bound.stat().st_size != item["bytes"] or digest(bound) != item["sha256"]:
                fail(f"training-authorization resolution artifact differs: {item['path']}")
    for name in ("priorUnresolved", "qualificationGate"):
        item = value.get(name)
        if not isinstance(item, dict) or not isinstance(item.get("path"), str) or not isinstance(item.get("bytes"), int) or not isinstance(item.get("sha256"), str):
            fail(f"training-authorization resolution {name} is not an artifact reference")
        bound = root_path(root, item["path"])
        if not bound.is_file() or bound.stat().st_size != item["bytes"] or digest(bound) != item["sha256"]:
            fail(f"training-authorization resolution artifact differs: {item['path']}")
    checks = value.get("checks")
    if not isinstance(checks, dict) or any(checks.get(name) is not True for name in ("experimentIdentityMatches", "explicitGrantCoversCompletedRun", "productionBindingsVerified", "resourceScopeVerified", "allNineModelsQualified")) or checks.get("optimizerUpdates") != 2688 or checks.get("repairAdditionalOptimizerUpdates") != 0:
        fail("training-authorization resolution checks are incomplete")
    if value.get("newAuthorizationGranted") is not False or value.get("additionalOptimizerUpdates") != 0 or value.get("gamesLaunched") != 0:
        fail("training-authorization resolution changes scope")
    if value.get("confirmationAuthorized") is not False or value.get("deploymentAuthorized") is not False:
        fail("training-authorization resolution changes confirmation or deployment scope")
    return root_artifact(root, "preflight/training-authorization-resolution.json")


def git_identity() -> dict[str, Any]:
    def run(*args: str) -> str | None:
        try:
            result = subprocess.run(
                ["git", *args],
                cwd=REPO_ROOT,
                check=True,
                capture_output=True,
                text=True,
            )
        except (OSError, subprocess.CalledProcessError):
            return None
        return result.stdout

    status = run("status", "--porcelain=v1", "--untracked-files=all")
    diff = run("diff", "--no-ext-diff", "--binary")
    return {
        "head": (run("rev-parse", "HEAD") or "").strip() or None,
        "dirty": bool(status),
        "statusSha256": digest_bytes((status or "").encode()),
        "trackedDiffSha256": digest_bytes((diff or "").encode()),
    }


def ref_list(paths: list[str], expected: dict[str, str] | None = None) -> list[dict[str, Any]]:
    expected = expected or {}
    return [artifact(path, expected.get(path)) for path in paths]


def atomic_text(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(value, encoding="utf-8")
    os.replace(temporary, path)


def archive_prior_opening_attempt(root: Path) -> str | None:
    """Preserve local receipts before a fresh opening attempt can replace them."""

    gate_path = root_path(root, "preflight/opening-freeze-gate.json")
    if not gate_path.is_file():
        return None
    gate = load_json(gate_path)
    remote_attempt = gate.get("remoteAttemptRoot")
    if not isinstance(remote_attempt, str) or not remote_attempt:
        return None
    attempt_name = Path(remote_attempt).name
    if not attempt_name.startswith("attempt-"):
        fail("opening gate has an invalid remote attempt root")
    archive_rel = f"preflight/opening-attempts/{attempt_name}"
    archive_dir = root_path(root, archive_rel)
    if archive_dir.exists():
        fail(f"opening attempt archive already exists: {archive_rel}")
    archive_dir.mkdir(parents=True, exist_ok=True)
    names = [
        "opening-spec.json",
        "opening-build.json",
        "opening-freeze-gate.json",
        "opening-producer.stdout",
        "opening-producer.stderr",
        "opening-validator.stdout",
        "opening-validator.stderr",
        "opening-implementation-amendment.json",
    ]
    copied: list[dict[str, Any]] = []
    for name in names:
        source = root_path(root, f"preflight/{name}")
        if not source.is_file():
            continue
        destination = archive_dir / name
        shutil.copy2(source, destination)
        copied.append({
            "path": str(destination.relative_to(root)),
            "bytes": destination.stat().st_size,
            "sha256": digest(destination),
        })
    atomic_json(archive_dir / "archive.json", {
        "schema": "haitaka-anhoku-strength-first-step12-opening-attempt-archive-v1",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "remoteAttemptRoot": remote_attempt,
        "sourceGate": "preflight/opening-freeze-gate.json",
        "artifacts": copied,
        "preservedBeforeRetry": True,
    })
    return archive_rel


def source_config_text(root: Path) -> str:
    """Return the complete fresh Step 12 source/label generator TOML."""

    output_dir = str(root_path(root, SOURCE_OUTPUT_REL))
    opening_suite = str(REPO_ROOT / "haitaka_learn/openings/anhoku-v3.tsv")
    validation_ids = [f"anhoku-v3-{index:03}" for index in range(53, 65)]
    return f'''[rules]
ruleset = "anhoku"

[paths]
output_dir = {json.dumps(output_dir)}

[data]
train_games = 30000
validation_games = 2016
max_plies = 180
position_policy = "root-position"
incomplete_label_policy = "reject-position"
label_retry_policy = "root-position-adaptive-retry-v1"
max_label_attempts_per_game = 96
self_play_move_policy = "searched-stochastic-rollout-v1"
rollout_candidate_limit = 4
rollout_score_margin = 80
rollout_temperature = 40.0
rollout_rng_version = "splitmix64-v1"
opening_random_plies = 0
opening_policy = "suite"
opening_suite = {json.dumps(opening_suite)}
opening_suite_id = "anhoku-v3"
validation_opening_ids = {json.dumps(validation_ids)}
validation_opening_schedule = "equal-color-swapped-pairs-v1"
validation_opening_pairs_per_id = 84
split_policy = "opening-group-hash-v1"
split_seed = 2026091602
shuffle_policy = "chunk-v1"
shuffle_seed = 2026091603
shuffle_chunk_records = 65536
sample_start_ply = 8
sample_every_ply = 2
sampling_policy = "per-game-random-v1"
max_positions_per_game = 64
minimum_train_boards = 920000
seed = 2026091601
jobs = 10
shard_games = 10
progress_every_percent = 5
resume = true

[generation]
record_format = "haitaka-packed-training-record-v3-72-byte"

[generation.trajectory_evaluator]
search_depth = 1

[generation.trajectory_evaluator.evaluator]
kind = "handcrafted"

[generation.label_evaluator]
search_budget = "nodes"
search_nodes = 10000
max_depth = 64
target_semantics = "bounded-nonmate-root-plus-explicit-mate-class-distance"
score_transform_version = "identity-with-robust-clip-v1"

[generation.label_evaluator.evaluator]
kind = "handcrafted"
'''


def ensure_source_config(root: Path) -> Path:
    path = root_path(root, SOURCE_CONFIG_REL)
    expected = source_config_text(root)
    if path.exists():
        try:
            actual = path.read_text(encoding="utf-8")
        except (OSError, UnicodeError) as exc:
            fail(f"cannot read existing Step 12 source config {path}: {exc}")
        if actual != expected:
            fail(f"Step 12 source config is immutable but differs from the bound recipe: {path}")
    else:
        atomic_text(path, expected)
    return path


def probe_binary_help(path: Path) -> dict[str, Any]:
    if not path.is_file():
        return {"status": "missing", "supportsStep12Inventory": False}
    try:
        result = subprocess.run(
            [str(path), "--help"],
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
            text=True,
            timeout=20,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return {"status": "probe-failed", "supportsStep12Inventory": False, "error": str(exc)}
    help_text = f"{result.stdout}\n{result.stderr}"
    return {
        "status": "observed" if result.returncode == 0 else "probe-failed",
        "returncode": result.returncode,
        "supportsStep12Inventory": "step12-exclusion-inventory" in help_text,
    }


def source_binding_observation(root: Path) -> dict[str, Any]:
    config_path = ensure_source_config(root)
    binary_path = repo_path(SOURCE_BINARY)
    binary_probe = probe_binary_help(binary_path)
    config_ref = root_artifact(root, SOURCE_CONFIG_REL)
    binary_ref = artifact(SOURCE_BINARY)
    generator_ref = artifact(SOURCE_GENERATOR)
    canonical_ref = artifact(CANONICALIZATION_SOURCE)
    inventory_ref = artifact(INVENTORY_HELPER)
    ready = all(
        item.get("status") == "observed"
        for item in (config_ref, binary_ref, generator_ref, canonical_ref, inventory_ref)
    ) and binary_probe.get("supportsStep12Inventory") is True
    binding = {
        "status": "bound" if ready else "resolve",
        "config": config_ref,
        "generatorImplementation": generator_ref,
        "canonicalizationImplementation": canonical_ref,
        "inventoryImplementation": inventory_ref,
        "binary": binary_ref,
        "binaryProbe": binary_probe,
        "generatorCommand": {
            "executable": SOURCE_BINARY,
            "argv": [
                SOURCE_BINARY,
                "generate-data",
                "--config",
                str(config_path),
                "--jobs",
                "10",
            ],
            "cwd": str(REPO_ROOT),
            "environmentAllowlist": [],
            "host": "192.168.0.12",
            "output": str(root_path(root, SOURCE_OUTPUT_REL)),
            "deterministicShardMap": {
                "shardGames": 10,
                "resume": True,
                "sourceGameSeed": SEEDS["sourceGames"],
                "splitSeed": SEEDS["sourceSplit"],
                "shuffleSeed": SEEDS["sourceShuffle"],
            },
        },
        "preparationOnly": True,
        "realWorkLaunched": False,
    }
    # Once the measured all-core path is approved, its exact command becomes
    # the operational binding.  Keep the original probe receipt immutable and
    # prefer a planned/completed full-generation receipt over that stopped
    # probe when refreshing the preparation artifacts.
    for relative, accepted_statuses in (
        ("source/full-generation.json", {"prepared", "running", "recovery-prepared", "recovery-running", "recovery-completed", "completed", "completed-awaiting-independent-freeze"}),
        ("source/generation.json", {"stopped-resource-cap"}),
    ):
        generation_path = root_path(root, relative)
        if not generation_path.is_file():
            continue
        generation = load_json(generation_path)
        recorded_command = generation.get("command")
        if generation.get("status") in accepted_statuses and isinstance(recorded_command, dict):
            if isinstance(recorded_command.get("argv"), list) and recorded_command["argv"]:
                binding["generatorCommand"] = recorded_command
                binding["remoteExecution"] = generation.get(
                    "remoteExecution", recorded_command.get("remoteExecution", {})
                )
                binding["phaseReceipt"] = relative
                binding["preparationOnly"] = False
                binding["realWorkLaunched"] = generation.get("status") in {
                    "running",
                    "recovery-prepared",
                    "recovery-running",
                    "recovery-completed",
                    "completed",
                    "completed-awaiting-independent-freeze",
                }
                break
    return binding


def source_binding_ready(root: Path) -> bool:
    return source_binding_observation(root).get("status") == "bound"


DATA_INPUTS = [
    {
        "id": "model-a-training",
        "manifest": ["out/anhoku-reboot-r4-replacement/datasets/training.json"],
        "payload": [
            "out/anhoku-reboot-r4-replacement/datasets/training.bin",
        ],
        "sidecar": [
            "out/anhoku-reboot-r4-replacement/datasets/training.jsonl",
        ],
        "records": 880000,
        "rationale": "Exclude all Model A training families used by the original recipe.",
        "expected": {
            "out/anhoku-reboot-r4-replacement/datasets/training.json": "9684f34a8c5eb9dc8edc91140466990b347fbc9a8da866bcfece902c799ec5da",
            "out/anhoku-reboot-r4-replacement/datasets/training.bin": "9ce80b25becf329f16135dd7c861374c0d51351e286b0f13396ffceb701fdc33",
            "out/anhoku-reboot-r4-replacement/datasets/training.jsonl": "f3928363815244a79eb06e83fd2b0da397fe32b34d7d380c761b47670bf081d8",
        },
    },
    {
        "id": "model-a-validation",
        "manifest": ["out/anhoku-reboot-r4-replacement/datasets/validation.json"],
        "payload": [
            "out/anhoku-reboot-r4-replacement/datasets/validation.bin",
        ],
        "sidecar": [
            "out/anhoku-reboot-r4-replacement/datasets/validation.jsonl",
        ],
        "records": 60000,
        "rationale": "Exclude all Model A validation families used for prior selection.",
        "expected": {
            "out/anhoku-reboot-r4-replacement/datasets/validation.json": "51bcc2ea425858ba849ab0690747644d186161512db2a46b14c019d1d09331f5",
            "out/anhoku-reboot-r4-replacement/datasets/validation.bin": "fcb2b62bc7974499adbc71ec241ec344d9fbe6b8a4b964bb4d19fd74ab20b81b",
            "out/anhoku-reboot-r4-replacement/datasets/validation.jsonl": "10ffbcb690a0fcb4781fdd7e8d4efaf079dc3875d6577d77409617b52ff35a4a",
        },
    },
    {
        "id": "model-a-development",
        "manifest": ["out/anhoku-reboot-r4-replacement/datasets/development.json"],
        "payload": [
            "out/anhoku-reboot-r4-replacement/datasets/development.bin",
        ],
        "sidecar": [
            "out/anhoku-reboot-r4-replacement/datasets/development.jsonl",
        ],
        "records": None,
        "rationale": "Exclude all Model A development families used for prior selection and diagnosis.",
        "expected": {
            "out/anhoku-reboot-r4-replacement/datasets/development.json": "cb86e467a914729d343b67f211b727aa33b24625de21d25e40052b8c079c3c33",
            "out/anhoku-reboot-r4-replacement/datasets/development.bin": "21e7f9ca0cbd4b6b8256816f770f5dc6e66939aab7591cf3a3bc55072288cd00",
            "out/anhoku-reboot-r4-replacement/datasets/development.jsonl": "121a65844092d5e399b3db1f427c22bd2d41339012083d3290348c5ac8b60388",
        },
    },
    {
        "id": "model-b-training",
        "manifest": [
            "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/model-b-corpus.json",
            "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/training.json",
        ],
        "payload": [
            "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/training.bin",
        ],
        "sidecar": [
            "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/training.jsonl",
        ],
        "records": 880000,
        "rationale": "Exclude all Step 9 B and Step 10 training families.",
        "expected": {
            "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/model-b-corpus.json": "2e7b32aa0103b033e627a272c20f135f6a5aa11d1c22d6c6d0b75b7c4412b54d",
            "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/training.json": "89f976b5fdfde9198fa9b671e8a6024758b1d638cceedecf9316371865cb0c55",
            "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/training.jsonl": "f213ae17df30478d12f9fcacdde057e85161cf40fd2a046cdb173cb1185a5fa0",
        },
    },
    {
        "id": "model-b-validation",
        "manifest": [
            "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/model-b-corpus.json",
            "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/validation.json",
        ],
        "payload": [
            "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/validation.bin",
        ],
        "sidecar": [
            "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/validation.jsonl",
        ],
        "records": 60000,
        "rationale": "Exclude all Step 9 B validation families.",
        "expected": {
            "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/model-b-corpus.json": "2e7b32aa0103b033e627a272c20f135f6a5aa11d1c22d6c6d0b75b7c4412b54d",
            "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/validation.json": "e72d1119554bc7bcfb2b56487217d5af6801e0892c7532197b539a6b330755bf",
            "out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/validation.jsonl": "4e576f0d4e0a0e80509f6ab695a71d1fe10754a231b76b7d20f30491050a4fb3",
        },
    },
]


OPENING_SOURCES = [
    ("step1-development", ["out/anhoku-strength-first/step1/development-openings.sfen"], {"out/anhoku-strength-first/step1/development-openings.sfen": "f17ef22e4c1ff203ff0dacd1f1968dabf74a6603580a04a99d35ff87a28b0f78"}, None),
    ("step3-compact-development", ["out/anhoku-strength-first/step3-compact-scratch-seed-20260906-v1/runtime/development-openings.sfen"], {"out/anhoku-strength-first/step3-compact-scratch-seed-20260906-v1/runtime/development-openings.sfen": "f17ef22e4c1ff203ff0dacd1f1968dabf74a6603580a04a99d35ff87a28b0f78"}, "step1-development"),
    ("step3-width512-development", ["out/anhoku-strength-first/step3-width512-capacity-seed-20260906-v1/runtime/development-openings.sfen"], {"out/anhoku-strength-first/step3-width512-capacity-seed-20260906-v1/runtime/development-openings.sfen": "f17ef22e4c1ff203ff0dacd1f1968dabf74a6603580a04a99d35ff87a28b0f78"}, "step1-development"),
    ("step5-scale-time", ["out/anhoku-strength-first/step5-scale-time-20260908-v1/openings.tsv"], {"out/anhoku-strength-first/step5-scale-time-20260908-v1/openings.tsv": "595d06ada03a8be7e5c444c6f7997dda3c33995f0b40824dd7926aa1ca81c359"}, None),
    ("step6-native", ["out/anhoku-strength-first/step6-browser-export-20260908-v1/native-openings.sfen"], {"out/anhoku-strength-first/step6-browser-export-20260908-v1/native-openings.sfen": "cf161237ab7d39e976e6a6d6aeed6fa6843053b4089ac3fee26eabae304dbf0a"}, None),
    ("step6-browser", ["out/anhoku-strength-first/step6-browser-export-20260908-v1/openings.tsv"], {"out/anhoku-strength-first/step6-browser-export-20260908-v1/openings.tsv": "1cc44568c8ee5a374d2a9ad0af2730c8564b8111aaab33da496c5c2a5fb4c92f"}, "anhoku-v3.tsv"),
    ("step6-secondary", ["out/anhoku-strength-first/step6-browser-export-20260908-v1/secondary-openings.tsv"], {"out/anhoku-strength-first/step6-browser-export-20260908-v1/secondary-openings.tsv": "44a79c923851e83820fcd7c783fa317a25e1841d7f1163a14b7a68eb9380a45b"}, None),
    ("step8-development", ["out/anhoku-strength-first/step8-compact-seed-replication-20260910-v1/development-openings.sfen"], {"out/anhoku-strength-first/step8-compact-seed-replication-20260910-v1/development-openings.sfen": "f17ef22e4c1ff203ff0dacd1f1968dabf74a6603580a04a99d35ff87a28b0f78"}, "step1-development"),
    ("step9-model-b-development", ["out/anhoku-strength-first/step9-model-b-training-20260912-v1/development-openings.sfen"], {"out/anhoku-strength-first/step9-model-b-training-20260912-v1/development-openings.sfen": "f17ef22e4c1ff203ff0dacd1f1968dabf74a6603580a04a99d35ff87a28b0f78"}, "step1-development"),
    ("step9-confirmation-a", ["out/anhoku-strength-first/step9-independent-confirmation-100ms-20260913-v1/model-a-suite-a/openings.sfen"], {"out/anhoku-strength-first/step9-independent-confirmation-100ms-20260913-v1/model-a-suite-a/openings.sfen": "d27bfbbd6e555c2f7263e78aa8189553f1e984284b4800b32f2aae6943843d41"}, None),
    ("step9-confirmation-b", ["out/anhoku-strength-first/step9-independent-confirmation-100ms-20260913-v1/model-b-suite-b/openings.sfen"], {"out/anhoku-strength-first/step9-independent-confirmation-100ms-20260913-v1/model-b-suite-b/openings.sfen": "8b3b3c854b9482a6b69d13cb5b257e59b2bcdff023694a825be8dea0bf31114e"}, None),
    ("step9-diagnosis-a", ["out/anhoku-strength-first/step9-post-confirmation-diagnosis-20260914-v1/schedules/a-selected.tsv", "out/anhoku-strength-first/step9-post-confirmation-diagnosis-20260914-v1/schedules/a-selected.sfen"], {"out/anhoku-strength-first/step9-post-confirmation-diagnosis-20260914-v1/schedules/a-selected.tsv": "353eb2c9d81d917a844e7ed22722a5e7cad5541ff3685bb96049a662a0e089ae", "out/anhoku-strength-first/step9-post-confirmation-diagnosis-20260914-v1/schedules/a-selected.sfen": "387425fe3b7d21b4870e768fa8ec3c9550b64d7cbe2fd91e11f751b83955b6b2"}, None),
    ("step9-diagnosis-b", ["out/anhoku-strength-first/step9-post-confirmation-diagnosis-20260914-v1/schedules/b-selected.tsv", "out/anhoku-strength-first/step9-post-confirmation-diagnosis-20260914-v1/schedules/b-selected.sfen"], {"out/anhoku-strength-first/step9-post-confirmation-diagnosis-20260914-v1/schedules/b-selected.tsv": "eead2efc7d1a63021ffdb7f60ee322af182c1659399fcc1b0f8593a5eead333e", "out/anhoku-strength-first/step9-post-confirmation-diagnosis-20260914-v1/schedules/b-selected.sfen": "7fa9408e0a25507c400623a58faefaa286e72d66c64eded1dfd53e449508ae1d"}, None),
    ("step10-v1", ["out/anhoku-strength-first/step10-checkpoint-selection-20260914-v1/openings/development.tsv", "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v1/openings/development.sfen"], {"out/anhoku-strength-first/step10-checkpoint-selection-20260914-v1/openings/development.tsv": "4270af869b75371e1792bbb84590cde4eca63b310b6f91b2a9314ebef4624c02", "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v1/openings/development.sfen": "5e2fddb5669eb99e83f2339f453e9df90c7d31e815e00fc14c9280380c0287c6"}, None),
    ("step10-v2", ["out/anhoku-strength-first/step10-checkpoint-selection-20260914-v2/openings/development.tsv", "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v2/openings/development.sfen"], {"out/anhoku-strength-first/step10-checkpoint-selection-20260914-v2/openings/development.tsv": "4270af869b75371e1792bbb84590cde4eca63b310b6f91b2a9314ebef4624c02", "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v2/openings/development.sfen": "5e2fddb5669eb99e83f2339f453e9df90c7d31e815e00fc14c9280380c0287c6"}, None),
    ("step10-v3-approx", ["out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-approx-100ms/openings/development.tsv", "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-approx-100ms/openings/development.sfen"], {"out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-approx-100ms/openings/development.tsv": "4270af869b75371e1792bbb84590cde4eca63b310b6f91b2a9314ebef4624c02", "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-approx-100ms/openings/development.sfen": "5e2fddb5669eb99e83f2339f453e9df90c7d31e815e00fc14c9280380c0287c6"}, None),
    ("step10-v3-recovery", ["out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-recovery/openings/development.tsv", "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-recovery/openings/development.sfen"], {"out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-recovery/openings/development.tsv": "4270af869b75371e1792bbb84590cde4eca63b310b6f91b2a9314ebef4624c02", "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-recovery/openings/development.sfen": "5e2fddb5669eb99e83f2339f453e9df90c7d31e815e00fc14c9280380c0287c6"}, None),
    ("step11-selection", ["out/anhoku-strength-first/step11-retained-model-selection-20260915-v1/openings/development.tsv"], {"out/anhoku-strength-first/step11-retained-model-selection-20260915-v1/openings/development.tsv": "2349eb374ee354c096d57d0f66c84b3e867b4cf03e01d71bcbe4941a56719a51"}, None),
    ("step11-browser", ["out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/openings/finalist.tsv"], {"out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/openings/finalist.tsv": "b9889a81f2b62116b2e635d1f992c60e926e43bf192d3eb96ff86eb4decb872b"}, None),
    ("anhoku-v1", ["haitaka_learn/openings/anhoku-v1.tsv"], {"haitaka_learn/openings/anhoku-v1.tsv": "7150a2a5871c4d302b63ab99ea31abe086471fa38a213dc184f42ce5d05721a7"}, None),
    ("anhoku-v2", ["haitaka_learn/openings/anhoku-v2.tsv"], {"haitaka_learn/openings/anhoku-v2.tsv": "bc576bbe57c05b8b2112b416c1907845d38d5e087e8e3b71b44c19c4e1593307"}, None),
    ("anhoku-v3", ["haitaka_learn/openings/anhoku-v3.tsv"], {"haitaka_learn/openings/anhoku-v3.tsv": "1cc44568c8ee5a374d2a9ad0af2730c8564b8111aaab33da496c5c2a5fb4c92f"}, None),
    ("anhoku-confirmation-a", ["haitaka_learn/openings/anhoku-strength-confirmation-a-v1.tsv"], {"haitaka_learn/openings/anhoku-strength-confirmation-a-v1.tsv": "d58c3adfdb7ba96f0cf5e21e36c97fe48d1d3df337293b92b33665744f497f39"}, None),
    ("anhoku-confirmation-b", ["haitaka_learn/openings/anhoku-strength-confirmation-b-v1.tsv"], {"haitaka_learn/openings/anhoku-strength-confirmation-b-v1.tsv": "4824a0c63bae39d86148cc1bc4bd6471b4c8f1296a460ec5b150fba0cce54578"}, None),
    ("anhoku-confirmation-100ms-a", ["haitaka_learn/openings/anhoku-strength-confirmation-100ms-a-v1.tsv"], {"haitaka_learn/openings/anhoku-strength-confirmation-100ms-a-v1.tsv": "66fd0e8c365d85f965d02e214738a48bed23d7905f7bb3e2b45dfc3821c10e61"}, None),
    ("anhoku-confirmation-100ms-b", ["haitaka_learn/openings/anhoku-strength-confirmation-100ms-b-v1.tsv"], {"haitaka_learn/openings/anhoku-strength-confirmation-100ms-b-v1.tsv": "53f1cd689da3615317144ef958ba731986191fbb6847e6b3e6df3f79231377cf"}, None),
]

RECEIPT_INPUTS = [
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v1/manifest.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v1/openings/generation.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v1/openings/independent-validation.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v1/openings/corpus-family-exclusion.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v1/openings/freeze.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v2/manifest.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v2/openings/generation.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v2/openings/independent-validation.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v2/openings/corpus-family-exclusion.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v2/openings/freeze.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-approx-100ms/manifest.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-approx-100ms/openings/generation.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-approx-100ms/openings/independent-validation.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-approx-100ms/openings/corpus-family-exclusion.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-approx-100ms/openings/freeze.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-recovery/manifest.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-recovery/openings/generation.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-recovery/openings/independent-validation.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-recovery/openings/corpus-family-exclusion.json",
    "out/anhoku-strength-first/step10-checkpoint-selection-20260914-v3-recovery/openings/freeze.json",
    "out/anhoku-strength-first/step11-retained-model-selection-20260915-v1/manifest.json",
    "out/anhoku-strength-first/step11-retained-model-selection-20260915-v1/openings/generation.json",
    "out/anhoku-strength-first/step11-retained-model-selection-20260915-v1/openings/independent-validation.json",
    "out/anhoku-strength-first/step11-retained-model-selection-20260915-v1/openings/corpus-family-exclusion.json",
    "out/anhoku-strength-first/step11-retained-model-selection-20260915-v1/openings/freeze.json",
    "out/anhoku-strength-first/step11-retained-model-selection-20260915-v1/openings/exclusions/manifest.json",
    "out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/manifest.json",
    "out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/prior/manifest.json",
    "out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/prior/openings/generation.json",
    "out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/prior/openings/independent-validation.json",
    "out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/prior/openings/corpus-family-exclusion.json",
    "out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/prior/openings/freeze.json",
]


def path_status(item: dict[str, Any]) -> str:
    return str(item.get("status", "missing"))


def exclusion_allowlist(root: Path) -> dict[str, Any]:
    entries: list[dict[str, Any]] = []
    for data in DATA_INPUTS:
        paths = data["manifest"] + data["payload"] + data["sidecar"]
        refs = ref_list(paths, data["expected"])
        entries.append(
            {
                "id": data["id"],
                "kind": "dataset-family-exclusion",
                "manifest": ref_list(data["manifest"], data["expected"]),
                "payload": ref_list(data["payload"], data["expected"]),
                "sidecar": ref_list(data["sidecar"], data["expected"]),
                "familyInventory": {
                    "path": f"preflight/exclusion-families/{data['id']}.jsonl",
                    "bytes": None,
                    "sha256": None,
                    "status": "resolve",
                },
                "recordsOrRows": data["records"],
                "sha256": {
                    "status": "observed" if all(path_status(x) == "observed" for x in refs) else "resolve",
                    "artifacts": refs,
                },
                "canonicalization": "anhoku-canonical-color-swap-family-v1",
                "coverageRationale": data["rationale"],
                "verification": {
                    "status": "pending",
                    "allNamedInputsExist": all(path_status(x) == "observed" for x in refs),
                    "familyInventoryGenerated": False,
                    "zeroOverlapWithNewData": False,
                },
            }
        )

    for suite_id, paths, expected, alias in OPENING_SOURCES:
        refs = ref_list(paths, expected)
        entries.append(
            {
                "id": suite_id,
                "kind": "opening-suite-family-exclusion",
                "manifest": [],
                "payload": refs,
                "sidecar": [],
                "familyInventory": {
                    "path": f"preflight/exclusion-families/{suite_id}.jsonl",
                    "bytes": None,
                    "sha256": None,
                    "status": "resolve",
                },
                "recordsOrRows": None,
                "sha256": {
                    "status": "observed" if all(path_status(x) == "observed" for x in refs) else "resolve",
                    "artifacts": refs,
                },
                "canonicalization": "anhoku-canonical-color-swap-family-v1",
                "coverageRationale": "Exclude every opening family previously opened or used for development.",
                "sameFamilyAs": alias,
                "verification": {
                    "status": "pending",
                    "allNamedInputsExist": all(path_status(x) == "observed" for x in refs),
                    "familyInventoryGenerated": False,
                    "zeroOverlapWithNewOpenings": False,
                },
            }
        )

    receipts = ref_list(RECEIPT_INPUTS)
    missing = []
    mismatched = []
    for entry in entries:
        for collection in (entry["manifest"], entry["payload"], entry["sidecar"]):
            for item in collection:
                if path_status(item) == "missing":
                    missing.append(item["path"])
                elif path_status(item) == "mismatch":
                    mismatched.append(item["path"])
    return {
        "schema": SCHEMAS["allowlist"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "open",
        "entries": entries,
        "receiptInputs": receipts,
        "protectedPaths": [PROTECTED_PATH],
        "coverage": {
            "entryCount": len(entries),
            "missingNamedInputs": sorted(set(missing)),
            "mismatchedNamedInputs": sorted(set(mismatched)),
            "familyInventoriesPending": len(entries),
            "zeroOverlapVerified": False,
        },
        "verification": {
            "forbiddenPathTraversed": False,
            "recursiveDiscoveryUsed": False,
            "historicalFilesModified": False,
        },
    }


def post_training_adapters_ready(root: Path) -> bool:
    """Return true only after Astra's downstream adapter review is bound."""

    review_path = root_path(root, "preflight/post-training-adapter-review.json")
    fixtures_path = root_path(root, "preflight/post-training-adapter-fixtures.json")
    fixture_path = root_path(root, "parity/fixture.json")
    positions_path = root_path(root, "parity/positions.json")
    if not all(path.is_file() for path in (review_path, fixtures_path, fixture_path, positions_path)):
        return False
    try:
        review = load_json(review_path)
        fixtures = load_json(fixtures_path)
        fixture = load_json(fixture_path)
        positions = load_json(positions_path)
    except (Step12Error, OSError, UnicodeError, json.JSONDecodeError, KeyError, TypeError):
        return False
    if review.get("status") != "passed" or fixtures.get("status") != "passed":
        return False
    if fixture.get("status") != "frozen-before-production-training" or fixture.get("count") != 1024:
        return False
    if positions.get("count") != 1024 or not isinstance(positions.get("positions"), list) or len(positions["positions"]) != 1024:
        return False
    implementation = review.get("implementation")
    expected = (
        "strength_first_step12_qualification.py",
        "strength_first_step12_matches.py",
        "strength_first_step12_match_validation.py",
        "strength_first_step12_analysis.py",
        "strength_first_step12_closeout.py",
    )
    if not isinstance(implementation, dict):
        return False
    return all(
        isinstance(implementation.get(name), dict)
        and isinstance(implementation[name].get("sha256"), str)
        and len(implementation[name]["sha256"]) == 64
        and isinstance(implementation[name].get("bytes"), int)
        and implementation[name]["bytes"] > 0
        for name in expected
    )


def unresolved_entries(allowlist: dict[str, Any], root: Path) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    for item in allowlist["entries"]:
        statuses = []
        for collection in (item["manifest"], item["payload"], item["sidecar"]):
            statuses.extend(path_status(ref) for ref in collection)
        if any(status != "observed" for status in statuses):
            entries.append(
                {
                    "id": f"named-inputs:{item['id']}",
                    "phase": "exclusion-preparation",
                    "owner": "Step 12 worker",
                    "reason": "one or more explicitly named historical inputs is missing or has an observed-hash mismatch",
                    "evidenceNeeded": "reverify or create an exact source receipt; do not substitute",
                    "resolution": None,
                }
            )
        family = item.get("familyInventory", {})
        if allowlist.get("status") != "frozen" or path_status(family) != "observed":
            entries.append(
                {
                    "id": f"family-inventory:{item['id']}",
                    "phase": "exclusion-preparation",
                    "owner": "Step 12 worker",
                    "reason": "canonical color-swap family inventory has not yet been generated and independently checked",
                    "evidenceNeeded": "ROOT/preflight/exclusion-families/*.jsonl with producer/canonicalization hash and coverage count",
                    "resolution": None,
                }
            )

    host_receipt_path = root_path(root, HOST_CONNECTIVITY_REL)
    host_receipt = load_json(host_receipt_path) if host_receipt_path.is_file() else {}
    host_attempts = host_receipt.get("attempts", [])
    host_reason = "the current MBP inventory and measured storage/time forecast have not been recorded for this root"
    host_evidence = "host identity, free space, process inventory, forecast and reserve receipt"
    if host_receipt.get("status") == "unreachable" and len(host_attempts) >= 3:
        host_reason = f"MBP 192.168.0.12 remained unreachable after {len(host_attempts)} bounded read-only SSH probes"
        host_evidence = "a reachable MBP probe or an explicit operational host amendment; no generation may start while this remains unresolved"

    phase_entries: list[dict[str, Any]] = []
    if not post_training_adapters_ready(root):
        phase_entries.extend([
            {
                "id": "qualification-adapter",
                "phase": "qualification",
                "owner": "Step 12 worker",
                "reason": "runtime/parity qualification remains fail-closed until its exact helper, schema, recovery fixtures and host binding are complete",
                "evidenceNeeded": "implemented qualification helper, exact argv and hashes, native/integer/WASM parity receipt and telemetry fixture",
                "resolution": None,
            },
            {
                "id": "matches-adapter",
                "phase": "matches",
                "owner": "Step 12 worker",
                "reason": "development matching remains fail-closed until its exact runner, opening schedule, recovery fixture and outcome ledger are bound",
                "evidenceNeeded": "implemented match helper, exact argv and hashes, fixed-time schedule and raw-result fixture",
                "resolution": None,
            },
            {
                "id": "closeout-adapter",
                "phase": "closeout",
                "owner": "Step 12 worker",
                "reason": "closeout remains fail-closed until its evidence graph, analysis and deployment handoff schemas are bound",
                "evidenceNeeded": "implemented closeout helper, complete evidence graph and deployment exclusion receipt",
                "resolution": None,
            },
            {
                "id": "runtime-and-parity-binding",
                "phase": "qualification",
                "owner": "Step 12 worker",
                "reason": "native ARM64 runtime, integer reference, WASM/glue and loader bindings are not yet phase-bound",
                "evidenceNeeded": "exact bytes, build identities, 1,024-position qualification and telemetry receipt",
                "resolution": None,
            },
        ])
    if training_authorization_resolution(root) is None:
        phase_entries.append({
            "id": "training-authorization",
            "phase": "training",
            "owner": "user / worker",
            "reason": "the runbook requires a separate explicit training grant; the current execution request grants preparation and bounded CPU phases only",
            "evidenceNeeded": "authorization/training.json identifying the frozen training package and applicable resource cap",
            "resolution": None,
        })
    if not valid_published_openings(root):
        phase_entries.insert(
            1,
            {
                "id": "opening-generator-binding",
                "phase": "opening-generation",
                "owner": "Step 12 worker",
                "reason": "the 128-opening model-blind generator and family validator are not resolved",
                "evidenceNeeded": "exact generator/validator implementation, argv, source IDs and freeze receipt",
                "resolution": None,
            },
        )
    entries.extend(phase_entries)
    legacy_forecast = host_receipt.get("forecast")
    parallel_forecast_path = root_path(root, "preflight/parallel-forecast.json")
    parallel_forecast = load_json(parallel_forecast_path) if parallel_forecast_path.is_file() else {}
    forecast = (
        parallel_forecast
        if parallel_forecast.get("status") == "passed"
        else legacy_forecast
    )
    if isinstance(forecast, dict) and forecast.get("status") == "failed":
        host_reason = "the measured MBP source-generation forecast exceeds the frozen source-generation host-wall ceiling"
        host_evidence = "preflight/forecast.json with measured probe arithmetic and an explicit resource-cap amendment; no further shard may start while this remains unresolved"
    host_ready = (
        host_receipt.get("status") == "inventory-complete"
        and isinstance(forecast, dict)
        and forecast.get("status") == "passed"
    )
    if not host_ready:
        entries.append(
            {
                "id": "mbp-host-forecast",
                "phase": "host-preparation",
                "owner": "Step 12 worker",
                "reason": host_reason,
                "evidenceNeeded": host_evidence,
                "evidencePath": HOST_CONNECTIVITY_REL,
                "resolution": None,
            }
        )
    parallel_calibration_path = root_path(root, "preflight/parallel-calibration.json")
    parallel_calibration = load_json(parallel_calibration_path) if parallel_calibration_path.is_file() else {}
    parallel_forecast_passed = parallel_forecast.get("status") == "passed"
    parallel_calibration_completed = parallel_calibration.get("status") == "completed"
    source_generation_path = root_path(root, "source/generation.json")
    if source_generation_path.is_file():
        source_generation = load_json(source_generation_path)
        if source_generation.get("status") == "stopped-resource-cap" and not (
            parallel_calibration_completed and parallel_forecast_passed
        ):
            entries.append(
                {
                    "id": "source-generation-resource-cap",
                    "phase": "source-generation",
                    "owner": "user / worker",
                    "reason": "the retained MBP source-generation probe projects beyond the frozen source-generation host-wall and process-CPU ceilings",
                    "evidenceNeeded": "an explicit measured-cap amendment; do not truncate, reseed, change shard mapping, or continue silently",
                    "evidencePath": "source/generation.json",
                    "resolution": None,
                }
            )
    if not source_binding_ready(root):
        entries.append(
            {
                "id": "source-generator-binding",
                "phase": "source-generation",
                "owner": "Step 12 worker",
                "reason": "the Step 12 source/label generator binding is not yet resolved against the repository Rust generator and inventory helper",
                "evidenceNeeded": "exact generator implementation, TOML, argv, binary/helper hashes, shard map and fixture receipt",
                "resolution": None,
            }
        )
    return entries


def write_authorizations(root: Path) -> None:
    common = {
        "schema": SCHEMAS["authorization"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "grantor": "user",
        "basis": "explicit user instruction to execute the Step 12 runbook",
        "outcomesMayBeOpened": False,
        "confirmationAuthorized": False,
        "deploymentAuthorized": False,
        "handcraftedRemainsProduction": True,
    }
    records = {
        "cpu-preparation.json": {
            **common,
            "scope": "preparation, input inspection, receipt writing and bounded synthetic fixtures",
            "granted": True,
            "resource": "local coordinator / CPU only",
        },
        "cpu-generation.json": {
            **common,
            "scope": "bounded source and opening generation after all preparation gates pass",
            "granted": True,
            "resource": "MBP 192.168.0.12, CPU only",
        },
        "cpu-match.json": {
            **common,
            "scope": "fixed 512-game development comparison after all phase gates pass",
            "granted": True,
            "resource": "MBP 192.168.0.12, one game/thread",
        },
        "training.json": {
            **common,
            "scope": "one scratch training run through update 2688",
            "granted": False,
            "resource": None,
            "reason": "the runbook requires a separate explicit training grant",
        },
        "gpu-rental.json": {
            **common,
            "scope": "Vast.ai rental for Step 12 training",
            "granted": False,
            "resource": None,
            "reason": "AGENTS.md requires explicit user permission with a concrete quote and cap",
        },
    }
    for filename, record in records.items():
        atomic_json(root_path(root, f"authorization/{filename}"), record)


def build_contract(root: Path) -> dict[str, Any]:
    return {
        "schema": SCHEMAS["contract"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "preparation-blocked",
        "parentPlan": "plans/anhoku-nnue-strength-first-plan.md",
        "runbook": "plans/anhoku-nnue-strength-first-step12-independent-replication-runbook.md",
        "objective": {
            "primary": "paired early-selected minus same-run u2688 score against handcrafted on fresh openings",
            "routingGate": "early-selected absolute point score versus handcrafted strictly above 0.5 and robust to capped extremes",
            "developmentOnly": True,
            "confirmation": False,
        },
        "source": {
            "trainingRecords": 880000,
            "validationRecords": 60000,
            "recordBytes": 72,
            "freshScratchInitialization": True,
            "sharedStrataLimitation": "fresh trajectories and initialization; broad anhoku-v3 strata and teacher policy remain shared",
        },
        "recipe": {
            "featureFamily": "HalfKAv2^+DonorSingleEff",
            "width": {"l1": 256, "l2": 16, "l3": 32},
            "candidateUpdates": CANDIDATE_UPDATES,
            "controlUpdate": CONTROL_UPDATE,
            "selection": {
                "metric": "finite-validation-teacher-only-loss",
                "globalMinimumTolerance": SELECTION_TOLERANCE,
                "tieBreak": "earliest-update",
                "matchResultsForbidden": True,
            },
            "trainingHorizonUpdates": 4096,
            "executionStopUpdate": CONTROL_UPDATE,
        },
        "schedule": {
            "cells": ["early-selected", "u2688-control"],
            "pairsPerCell": 128,
            "gamesPerCell": 256,
            "totalGames": 512,
            "pairsPerBlock": 32,
            "blocksPerCell": 4,
            "gamesPerBlock": 64,
            "movetimeMs": 100,
            "threads": 1,
            "concurrentGames": 1,
            "maxPlies": 200,
        },
        "seeds": SEEDS,
        "resources": {
            "matchHost": "192.168.0.12",
            "matchWallHours": 4.0,
            "storageGiB": 64,
            "trainingRuns": 1,
            "gpuRentalAuthorized": False,
        },
        "flags": {
            "confirmationAuthorized": False,
            "deploymentAuthorized": False,
            "handcraftedRemainsProduction": True,
        },
        "protectedPaths": [PROTECTED_PATH],
    }


def build_commands() -> dict[str, Any]:
    commands = [
        ("prepare", "preparation", "implemented"),
        ("fixtures", "preparation", "implemented"),
        ("freeze-exclusions", "exclusion-preparation", "implemented-preparation"),
        ("host-preflight", "host-preparation", "implemented-read-only"),
        ("prepare-training", "training-preparation", "implemented-preparation"),
        ("training-gate", "gate", "implemented-fail-closed"),
        ("match-gate", "gate", "implemented-fail-closed"),
        ("generate-data", "source-generation", "implemented-gated"),
        ("freeze-data", "data-freeze", "implemented-gated"),
        ("freeze-openings", "opening-generation", "implemented-gated"),
        ("train", "training", "implemented-gated"),
        ("select-and-qualify", "qualification", "implemented-gated"),
        ("run-matches", "matches", "implemented-gated"),
        ("closeout", "closeout", "implemented-gated"),
    ]
    return {
        "schema": SCHEMAS["commands"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "script": "scripts/strength_first_step12.py",
        "commands": [
            {
                "name": name,
                "phase": phase,
                "status": status,
                "argv": ["python3", "scripts/strength_first_step12.py", name, "--root", "ROOT"],
                "launchesRealWork": name not in {"prepare", "fixtures", "freeze-exclusions", "host-preflight", "prepare-training", "training-gate", "match-gate"},
            }
            for name, phase, status in commands
        ],
        "placeholderPolicy": "blocked commands must not launch real generation, training, matches or closeout",
    }


def data_freeze_binding(root: Path, allowlist: dict[str, Any]) -> dict[str, Any]:
    """Describe the MBP-only native producer/validator contract."""

    inventory_paths = [
        f"{REMOTE_INVENTORIES}/{Path(entry['familyInventory']['path']).name}"
        for entry in allowlist.get("entries", [])
    ]
    source_dir = root_path(root, SOURCE_FULL_OUTPUT_REL)
    source_train = str(source_dir / "datasets/train.bin")
    source_validation = str(source_dir / "datasets/validation.bin")
    train_shards = str(source_dir / "datasets/shards/train")
    validation_shards = str(source_dir / "datasets/shards/validation")
    stage_remote = f"{REMOTE_ROOT}/data/.staging-corpus-freeze-v1"
    producer_argv = [
        "ssh", "-F", "/dev/null", "-o", "ConnectTimeout=10", "-o", "BatchMode=yes",
        "-o", "ConnectionAttempts=1", REMOTE_HOST, "caffeinate", "-i", "/usr/bin/time", "-l",
        REMOTE_BINARY, "strength-first-step12-freeze-corpus",
    ]
    for path in inventory_paths:
        producer_argv.extend(["--exclusion-inventory", path])
    producer_argv.extend([
        "--source-train", f"{REMOTE_SOURCE_OUTPUT}/datasets/train.bin",
        "--source-train-shards", f"{REMOTE_SOURCE_OUTPUT}/datasets/shards/train",
        "--source-validation", f"{REMOTE_SOURCE_OUTPUT}/datasets/validation.bin",
        "--source-validation-shards", f"{REMOTE_SOURCE_OUTPUT}/datasets/shards/validation",
        "--output-dir", stage_remote,
        "--train-records", "880000",
        "--validation-records", "60000",
        "--selection-seed", str(SEEDS["corpusSelection"]),
    ])
    validator_argv = [
        "ssh", "-F", "/dev/null", "-o", "ConnectTimeout=10", "-o", "BatchMode=yes",
        "-o", "ConnectionAttempts=1", REMOTE_HOST, "caffeinate", "-i", "/usr/bin/time", "-l",
        REMOTE_BINARY, "strength-first-step12-validate-corpus",
    ]
    for path in inventory_paths:
        validator_argv.extend(["--exclusion-inventory", path])
    validator_argv.extend([
        "--source-train", f"{REMOTE_SOURCE_OUTPUT}/datasets/train.bin",
        "--source-train-shards", f"{REMOTE_SOURCE_OUTPUT}/datasets/shards/train",
        "--source-validation", f"{REMOTE_SOURCE_OUTPUT}/datasets/validation.bin",
        "--source-validation-shards", f"{REMOTE_SOURCE_OUTPUT}/datasets/shards/validation",
        "--output-dir", stage_remote,
        "--report", f"{stage_remote}/independent-validation.json",
        "--train-records", "880000",
        "--validation-records", "60000",
        "--selection-seed", str(SEEDS["corpusSelection"]),
    ])
    receipt = root_path(root, "data/freeze.json")
    if receipt.is_file():
        try:
            frozen = load_json(receipt)
        except Step12Error:
            frozen = {}
        if frozen.get("schema") == CORPUS_FREEZE_SCHEMA and frozen.get("status") == "frozen":
            return {
                "status": "bound",
                "phaseReceipt": "data/freeze.json",
                "producer": frozen.get("producer"),
                "validator": frozen.get("validator"),
                "command": frozen.get("producer", {}).get("command", producer_argv),
                "validatorCommand": frozen.get("validator", {}).get("command", validator_argv),
                "host": REMOTE_HOST,
                "source": {
                    "train": source_train,
                    "validation": source_validation,
                    "trainShards": train_shards,
                    "validationShards": validation_shards,
                },
                "exclusionInventories": inventory_paths,
                "selectionSeed": SEEDS["corpusSelection"],
                "preparationOnly": False,
                "realWorkLaunched": True,
            }
    return {
        "status": "resolve",
        "phase": "data-freeze",
        "producerCommand": producer_argv,
        "validatorCommand": validator_argv,
        "host": REMOTE_HOST,
        "source": {
            "train": source_train,
            "validation": source_validation,
            "trainShards": train_shards,
            "validationShards": validation_shards,
        },
        "exclusionInventories": inventory_paths,
        "selectionSeed": SEEDS["corpusSelection"],
        "implementation": artifact(CORPUS_FREEZE_IMPLEMENTATION),
        "preparationOnly": True,
        "realWorkLaunched": False,
    }


def opening_binding(root: Path, allowlist: dict[str, Any]) -> dict[str, Any]:
    """Describe the MBP-only producer/validator for the fresh opening suite."""

    if valid_published_openings(root):
        return {
            "status": "bound",
            "phaseReceipt": "openings/freeze.json",
            "host": REMOTE_HOST,
            "generator": root_artifact(root, "openings/generation.json"),
            "validator": root_artifact(root, "openings/independent-validation.json"),
            "freeze": root_artifact(root, "openings/freeze.json"),
            "implementation": artifact(OPENING_IMPLEMENTATION),
            "validationImplementation": artifact(OPENING_VALIDATION_IMPLEMENTATION),
            "pairingImplementation": artifact(OPENING_PAIRING_IMPLEMENTATION),
            "pairingTransformation": "anhoku-rotate-180-and-swap-colors-v1",
            "historicalInventoryCount": len(allowlist.get("entries", [])),
            "modelsBound": False,
            "preparationOnly": False,
            "realWorkLaunched": True,
        }
    return {
        "status": "resolve",
        "phase": "opening-generation",
        "host": REMOTE_HOST,
        "generatorImplementation": artifact(OPENING_IMPLEMENTATION),
        "validatorImplementation": artifact(OPENING_VALIDATION_IMPLEMENTATION),
        "pairingImplementation": artifact(OPENING_PAIRING_IMPLEMENTATION),
        "pairingDocumentation": artifact(OPENING_PAIRING_DOCUMENTATION),
        "pairingTransformation": "anhoku-rotate-180-and-swap-colors-v1",
        "historicalInventoryCount": len(allowlist.get("entries", [])),
        "seed": SEEDS["developmentOpenings"],
        "positions": 128,
        "randomPlies": 16,
        "candidateCap": 100000,
        "preparationOnly": True,
        "realWorkLaunched": False,
    }


def build_bindings(root: Path, allowlist: dict[str, Any], unresolved: list[dict[str, Any]]) -> dict[str, Any]:
    helper_paths = [
        "scripts/strength_first_step12.py",
        "scripts/strength_first_step10_trainer.py",
        "scripts/strength_first_step10_trainer_backend.py",
        "scripts/strength_first_step10_trainer_fixture.py",
        "scripts/strength_first_step11_analysis.py",
        "scripts/strength_first_step11_browser_controller.py",
        "scripts/strength_first_step11_semantic_analysis.py",
        "scripts/strength_first_step11_browser_terminal_replay_worker.js",
        "scripts/strength_first_step12_trainer.py",
        "scripts/strength_first_step12_trainer_fixture.py",
        "scripts/strength_first_step12_trainer_backend.py",
        "scripts/strength_first_step12_context.py",
        "trainer-patches/evaluate.py",
        OPENING_IMPLEMENTATION,
        OPENING_VALIDATION_IMPLEMENTATION,
        OPENING_PAIRING_IMPLEMENTATION,
        OPENING_PAIRING_DOCUMENTATION,
    ]
    source_binding = source_binding_observation(root)
    return {
        "schema": SCHEMAS["bindings"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "preparation-blocked",
        "coordinatorRoot": str(root),
        "repository": str(REPO_ROOT),
        "hostPreference": {"match": "192.168.0.12", "fallbacks": ["192.168.0.5", "localhost"]},
        "helpers": {path: artifact(path) for path in helper_paths},
        "recipeReferences": {
            "step10Contract": artifact("r0/anhoku-strength-first/step10-checkpoint-selection-contract-v1.json"),
            "step10Trainer": artifact("scripts/strength_first_step10_trainer.py"),
            "step9DataContract": artifact("r0/anhoku-strength-first/step9-model-b-data-contract-v2.json"),
            "step9SourceToml": artifact("haitaka_learn.anhoku-strength-first-step9-model-b-source-v2.toml"),
            "step9SourceImplementation": artifact("haitaka_learn/src/strength_first_model_b.rs"),
            "step11Report": artifact("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/report.md"),
            "step11Analysis": artifact("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/analysis.json"),
        },
        "pinnedTrainingCoreRevision": "61666d9e3653e4df9881b14c23f8fdcc4bf7779b",
        "sourceGeneration": source_binding,
        "dataFreeze": data_freeze_binding(root, allowlist),
        "openingGeneration": opening_binding(root, allowlist),
        "trainingWorker": {
            "host": REMOTE_HOST,
            "runner": artifact("scripts/strength_first_step12_trainer.py"),
            "fixture": artifact("scripts/strength_first_step12_trainer_fixture.py"),
            "backend": artifact("scripts/strength_first_step12_trainer_backend.py"),
            "contextBuilder": artifact("scripts/strength_first_step12_context.py"),
            "config": root_artifact(root, "training/config.json"),
            "inputs": root_artifact(root, "training/inputs.json"),
            "package": root_artifact(root, "training/package.json"),
            "cpuFixtureSummary": root_artifact(root, "preflight/training-fixtures-cpu.json"),
            "contextAssembly": root_artifact(root, "preflight/training-context-assembly.json"),
            "hostBindingValidation": root_artifact(root, "preflight/training-host-binding-validation.json"),
            "fixtureCoverage": root_artifact(root, "preflight/training-fixture-coverage.json"),
            "resourceForecast": root_artifact(root, "preflight/training-resource-forecast.json"),
            "resourceProposal": root_artifact(root, "training/resource-proposal.json"),
            "trainingAuthorization": root_artifact(root, "authorization/training.json"),
            "gpuAuthorization": root_artifact(root, "authorization/gpu-rental.json"),
            "selectionInput": root_artifact(root, "training/selection-input.json"),
            "selection": root_artifact(root, "training/selection.json"),
        },
        "hostConnectivity": root_artifact(root, HOST_CONNECTIVITY_REL),
        "commands": build_commands()["commands"],
        "exclusionAllowlist": {
            "path": "preflight/exclusion-allowlist.json",
            "status": allowlist["status"],
            "entryCount": len(allowlist["entries"]),
        },
        "unresolved": {
            "path": "preflight/unresolved.json",
            "status": "open",
            "count": len(unresolved),
        },
        "protectedPaths": [PROTECTED_PATH],
        "flags": {
            "confirmationAuthorized": False,
            "deploymentAuthorized": False,
            "handcraftedRemainsProduction": True,
        },
    }


def build_unresolved(unresolved: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "schema": SCHEMAS["unresolved"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "open" if unresolved else "empty",
        "entries": unresolved,
        "protectedPaths": [PROTECTED_PATH],
        "flags": {
            "confirmationAuthorized": False,
            "deploymentAuthorized": False,
            "handcraftedRemainsProduction": True,
        },
    }


PHASE_DEPENDENCIES = {
    "source-generation": {"exclusion-preparation", "source-generation", "host-preparation"},
    "data-freeze": {"exclusion-preparation", "source-generation", "data-freeze", "host-preparation"},
    "opening-generation": {"exclusion-preparation", "source-generation", "data-freeze", "opening-generation", "host-preparation"},
    "training": {"exclusion-preparation", "source-generation", "data-freeze", "opening-generation", "training", "host-preparation"},
    "qualification": {"exclusion-preparation", "source-generation", "data-freeze", "opening-generation", "training", "qualification", "host-preparation"},
    "matches": {"exclusion-preparation", "source-generation", "data-freeze", "opening-generation", "training", "qualification", "matches", "host-preparation"},
    "closeout": {"exclusion-preparation", "source-generation", "data-freeze", "opening-generation", "training", "qualification", "matches", "closeout", "host-preparation"},
}


def phase_unresolved(unresolved: list[dict[str, Any]], phase: str) -> list[dict[str, Any]]:
    phases = PHASE_DEPENDENCIES.get(phase)
    if phases is None:
        fail(f"unknown Step 12 phase for scoped gate: {phase}")
    return [entry for entry in unresolved if entry.get("phase") in phases]


def build_launch_gate(root: Path, unresolved: list[dict[str, Any]], fixture_status: str) -> dict[str, Any]:
    reasons = [entry["reason"] for entry in unresolved]
    if fixture_status != "passed-preparation-fixtures":
        reasons.append("required preparation fixtures have not passed")
    phase_gates = {}
    for phase in PHASE_DEPENDENCIES:
        scoped = phase_unresolved(unresolved, phase)
        phase_gates[phase] = {
            "launchReady": not scoped and fixture_status == "passed-preparation-fixtures",
            "unresolvedIds": [entry["id"] for entry in scoped],
        }
    return {
        "schema": SCHEMAS["launchGate"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "blocked",
        "launchReady": False,
        "preparationOnly": True,
        "reasons": reasons,
        "phaseGates": phase_gates,
        "requiredBeforeLaunch": [
            "preflight/unresolved.json status empty with entries []",
            "preflight/exclusion-allowlist.json status frozen with all family inventories checked",
            "all phase adapters and helpers exist and are hash-bound",
            "all required positive and negative fixtures pass",
            "applicable authorization receipts are present",
            "resource and storage forecasts fit without GPU unless explicitly granted",
        ],
        "flags": {
            "confirmationAuthorized": False,
            "deploymentAuthorized": False,
            "handcraftedRemainsProduction": True,
        },
        "root": str(root),
    }


def build_status(root: Path, phase: str, state: str) -> dict[str, Any]:
    return {
        "schema": SCHEMAS["status"],
        "createdAt": now(),
        "updatedAt": now(),
        "experimentId": EXPERIMENT_ID,
        "root": str(root),
        "phase": phase,
        "state": state,
        "acceptedBlocks": 0,
        "acceptedPairs": 0,
        "acceptedGames": 0,
        "outcomesOpened": False,
        "trainingOrGpuAttempted": False,
        "confirmationAuthorized": False,
        "deploymentAuthorized": False,
        "handcraftedRemainsProduction": True,
    }


def write_manifest(root: Path, allowlist: dict[str, Any], unresolved: list[dict[str, Any]], fixture_status: str) -> None:
    status_path = root_path(root, "status.json")
    status = load_json(status_path)
    refs = {
        relative: root_artifact(root, relative)
        for relative in [
            "contract.json",
            "source-identity.json",
            SOURCE_CONFIG_REL,
            "execution-bindings.json",
            "commands.json",
            "status.json",
            "preflight/unresolved.json",
            "preflight/fixtures.json",
            "preflight/launch-gate.json",
            "preflight/exclusion-allowlist.json",
            "preflight/exclusion-freeze.json",
            HOST_CONNECTIVITY_REL,
        ]
    }
    for relative in [
        "preflight/forecast.json",
        "preflight/parallel-calibration.json",
        "preflight/parallel-forecast.json",
        "preflight/source-generation-gate.json",
        "preflight/source-generation-reforecast-gate.json",
        "preflight/recovery-baseline.json",
        "preflight/recovery-inventory.json",
        "preflight/recovery-attempt-1.json",
        "preflight/recovery-supervisor.json",
        "preflight/recovery-supervisor-failure-1.json",
        "preflight/source-generation-completion.json",
        "preflight/train-audit.json",
        "preflight/validation-audit.json",
        "preflight/data-freeze-gate.json",
        "preflight/data-freeze-build.json",
        "preflight/data-freeze-producer.stdout",
        "preflight/data-freeze-producer.stderr",
        "preflight/data-freeze-validator.stdout",
        "preflight/data-freeze-validator.stderr",
        "data/selection.json",
        "data/independent-validation.json",
        "data/freeze.json",
        "data/frozen-corpus-v1/source-record-map.json",
        "data/frozen-corpus-v1/source-record-map-training.bin",
        "data/frozen-corpus-v1/source-record-map-validation.bin",
        "data/frozen-corpus-v1/training.bin",
        "data/frozen-corpus-v1/training.jsonl",
        "data/frozen-corpus-v1/training.json",
        "data/frozen-corpus-v1/validation.bin",
        "data/frozen-corpus-v1/validation.jsonl",
        "data/frozen-corpus-v1/validation.json",
        "preflight/opening-spec.json",
        "preflight/opening-build.json",
        "preflight/opening-freeze-gate.json",
        "preflight/opening-producer.stdout",
        "preflight/opening-producer.stderr",
        "preflight/opening-validator.stdout",
        "preflight/opening-validator.stderr",
        "preflight/opening-implementation-amendment.json",
        "openings/development.tsv",
        "openings/development.sfen",
        "openings/candidates.jsonl",
        "openings/generation.json",
        "openings/exclusions.json",
        "openings/schedule.json",
        "openings/independent-validation.json",
        "openings/producer-freeze.json",
        "openings/freeze.json",
        "source/automatic-parallelism-calibration.toml",
        "source/generation.json",
        "source/generation.stdout",
        "source/generation.stderr",
        "source/parallel-calibration.json",
        "source/parallel-calibration.stdout",
        "source/parallel-calibration.stderr",
        "preflight/full-generation.json",
        "source/full-generation.json",
        "source/full-generation.stdout",
        "source/full-generation.stderr",
        "source/recovery/20260916T111614Z-v3/exit.json",
        "source/recovery/20260916T111614Z-v3/generation.stdout",
        "source/recovery/20260916T111614Z-v3/generation.stderr",
        "source/recovery/20260916T111614Z-v3/heartbeat.log",
        "source/recovery/20260916T111614Z-v3-launcher.log",
    ]:
        if root_path(root, relative).is_file():
            refs[relative] = root_artifact(root, relative)
    for path in sorted(root_path(root, "preflight").glob("opening-spec-*.json")):
        relative = str(path.relative_to(root))
        refs[relative] = root_artifact(root, relative)
    attempts_dir = root_path(root, "preflight/opening-attempts")
    if attempts_dir.is_dir():
        for path in sorted(attempts_dir.rglob("*")):
            if path.is_file():
                relative = str(path.relative_to(root))
                refs[relative] = root_artifact(root, relative)
    for block in range(4):
        relative = f"openings/blocks/block-{block:02}.sfen"
        if root_path(root, relative).is_file():
            refs[relative] = root_artifact(root, relative)
    manifest = {
        "schema": SCHEMAS["manifest"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": str(status.get("state", "preparation-blocked")),
        "phase": str(status.get("phase", "prepare")),
        "artifacts": refs,
        "exclusionEntryCount": len(allowlist["entries"]),
        "unresolvedCount": len(unresolved),
        "fixtureStatus": fixture_status,
        "flags": {
            "confirmationAuthorized": False,
            "deploymentAuthorized": False,
            "handcraftedRemainsProduction": True,
        },
    }
    atomic_json(root_path(root, "manifest.json"), manifest)
    status["manifest"] = root_artifact(root, "manifest.json")
    status["updatedAt"] = now()
    atomic_json(status_path, status)


def refresh_bound_identity(root: Path) -> None:
    path = root_path(root, "source-identity.json")
    identity = load_json(path)
    identity["updatedAt"] = now()
    identity["coordinator"] = {"repository": str(REPO_ROOT), **git_identity()}
    identity["boundDocuments"] = {
        "plan": artifact("plans/anhoku-nnue-strength-first-plan.md"),
        "runbook": artifact("plans/anhoku-nnue-strength-first-step12-independent-replication-runbook.md"),
    }
    worker_runbook = repo_path("plans/anhoku-nnue-strength-first-step12-opening-freeze-worker-runbook.md")
    if worker_runbook.is_file():
        identity["boundDocuments"]["openingFreezeWorkerRunbook"] = artifact(
            "plans/anhoku-nnue-strength-first-step12-opening-freeze-worker-runbook.md"
        )
    atomic_json(path, identity)


def refresh_preflight(root: Path, phase: str, state: str) -> list[dict[str, Any]]:
    refresh_bound_identity(root)
    allowlist = load_json(root_path(root, "preflight/exclusion-allowlist.json"))
    unresolved = unresolved_entries(allowlist, root)
    atomic_json(root_path(root, "preflight/unresolved.json"), build_unresolved(unresolved))
    fixtures = load_json(root_path(root, "preflight/fixtures.json"))
    atomic_json(
        root_path(root, "preflight/launch-gate.json"),
        build_launch_gate(root, unresolved, str(fixtures.get("status", "not-run"))),
    )
    atomic_json(
        root_path(root, "execution-bindings.json"),
        build_bindings(root, allowlist, unresolved),
    )
    status_path = root_path(root, "status.json")
    status_doc = load_json(status_path)
    status_doc.update({
        "updatedAt": now(),
        "phase": phase,
        "state": state,
        "unresolvedCount": len(unresolved),
    })
    atomic_json(status_path, status_doc)
    write_manifest(root, allowlist, unresolved, str(fixtures.get("status", "not-run")))
    return unresolved


def family_inventory_inputs(entry: dict[str, Any]) -> list[str]:
    if entry.get("kind") == "dataset-family-exclusion":
        inputs = [item["path"] for item in entry.get("payload", []) if item["path"].endswith(".bin")]
    else:
        inputs = [
            item["path"]
            for item in entry.get("payload", [])
            if item["path"].endswith((".sfen", ".tsv"))
        ]
    if not inputs:
        fail(f"no explicit family-inventory payload is bound for {entry.get('id')}")
    if any(protected(path) for path in inputs):
        fail(f"protected historical path is forbidden: {PROTECTED_PATH}")
    return inputs


def run_freeze_exclusions(root: Path) -> int:
    allowlist_path = root_path(root, "preflight/exclusion-allowlist.json")
    if not root_path(root, "contract.json").is_file():
        fail("run prepare before freeze-exclusions")
    allowlist = load_json(allowlist_path)
    if allowlist.get("status") == "frozen":
        unresolved = refresh_preflight(root, "freeze-exclusions", "exclusions-frozen")
        print(json.dumps({
            "command": "freeze-exclusions",
            "status": "already-frozen",
            "entries": len(allowlist["entries"]),
            "unresolved": len(unresolved),
            "realWorkLaunched": False,
        }, sort_keys=True))
        return 0

    named_unresolved = [
        item["path"]
        for entry in allowlist["entries"]
        for collection in (entry["manifest"], entry["payload"], entry["sidecar"])
        for item in collection
        if path_status(item) != "observed"
    ]
    if named_unresolved:
        receipt = {
            "schema": SCHEMAS["exclusionFreeze"],
            "createdAt": now(),
            "experimentId": EXPERIMENT_ID,
            "status": "blocked",
            "reason": "one or more explicitly named allowlist inputs is missing or hash-mismatched",
            "inputs": sorted(set(named_unresolved)),
            "protectedPaths": [PROTECTED_PATH],
            "recursiveDiscoveryUsed": False,
            "realWorkLaunched": False,
        }
        atomic_json(root_path(root, "preflight/exclusion-freeze.json"), receipt)
        print(json.dumps({"command": "freeze-exclusions", "status": "blocked", "unresolved": len(named_unresolved), "realWorkLaunched": False}, sort_keys=True), file=sys.stderr)
        return 2

    binary_path = repo_path(SOURCE_BINARY)
    probe = probe_binary_help(binary_path)
    if probe.get("supportsStep12Inventory") is not True:
        receipt = {
            "schema": SCHEMAS["exclusionFreeze"],
            "createdAt": now(),
            "experimentId": EXPERIMENT_ID,
            "status": "blocked",
            "reason": "the rebuilt Rust Step 12 inventory subcommand is not available",
            "producer": {"binary": artifact(SOURCE_BINARY), "probe": probe},
            "protectedPaths": [PROTECTED_PATH],
            "recursiveDiscoveryUsed": False,
            "realWorkLaunched": False,
        }
        atomic_json(root_path(root, "preflight/exclusion-freeze.json"), receipt)
        print(json.dumps({"command": "freeze-exclusions", "status": "blocked", "reason": "inventory producer unavailable", "realWorkLaunched": False}, sort_keys=True), file=sys.stderr)
        return 2

    results: list[dict[str, Any]] = []
    failures: list[dict[str, Any]] = []
    for entry in allowlist["entries"]:
        family = entry["familyInventory"]
        if family.get("status") == "observed":
            results.append({"id": entry["id"], "status": "reused", "familyInventory": family})
            continue
        inputs = family_inventory_inputs(entry)
        output_rel = str(family["path"])
        output_path = root_path(root, output_rel)
        command = [
            str(binary_path),
            "step12-exclusion-inventory",
            "--inventory-id",
            entry["id"],
            "--output",
            str(output_path),
        ]
        for path in inputs:
            command.extend(["--input", path])
        try:
            completed = subprocess.run(
                command,
                cwd=REPO_ROOT,
                check=False,
                capture_output=True,
                text=True,
                timeout=7200,
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            failures.append({"id": entry["id"], "status": "failed", "error": str(exc)})
            break
        if completed.returncode != 0:
            failures.append({
                "id": entry["id"],
                "status": "failed",
                "returncode": completed.returncode,
                "stderr": completed.stderr[-2000:],
            })
            break
        try:
            producer_receipt = json.loads(completed.stdout.strip().splitlines()[-1])
        except (IndexError, json.JSONDecodeError) as exc:
            failures.append({"id": entry["id"], "status": "failed", "error": f"invalid inventory receipt: {exc}"})
            break
        if producer_receipt.get("status") != "passed":
            failures.append({"id": entry["id"], "status": "failed", "error": "inventory producer did not report passed"})
            break
        output_ref = root_artifact(root, output_rel)
        family.update({
            "status": output_ref["status"],
            "bytes": output_ref["bytes"],
            "sha256": output_ref["sha256"],
            "count": producer_receipt.get("uniqueCanonicalFamilies"),
            "rows": producer_receipt.get("rows"),
            "familyDefinition": producer_receipt.get("familyDefinition"),
            "producer": producer_receipt.get("producer"),
        })
        entry["verification"].update({
            "status": "frozen-inputs-family-inventory-ready",
            "familyInventoryGenerated": True,
        })
        atomic_json(allowlist_path, allowlist)
        results.append({"id": entry["id"], "status": "generated", "familyInventory": family})

    complete = not failures and all(item["familyInventory"].get("status") == "observed" for item in allowlist["entries"])
    if complete:
        allowlist["status"] = "frozen"
        allowlist["coverage"]["familyInventoriesPending"] = 0
        allowlist["coverage"]["familyInventoryCounts"] = {
            entry["id"]: entry["familyInventory"].get("count", 0)
            for entry in allowlist["entries"]
        }
        allowlist["verification"].update({
            "forbiddenPathTraversed": False,
            "recursiveDiscoveryUsed": False,
            "historicalFilesModified": False,
        })
        atomic_json(allowlist_path, allowlist)

    receipt = {
        "schema": SCHEMAS["exclusionFreeze"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "frozen" if complete else "blocked-partial",
        "entries": results,
        "failures": failures,
        "allowlist": root_artifact(root, "preflight/exclusion-allowlist.json"),
        "producer": artifact(SOURCE_BINARY),
        "canonicalizationSource": artifact(CANONICALIZATION_SOURCE),
        "protectedPaths": [PROTECTED_PATH],
        "recursiveDiscoveryUsed": False,
        "historicalFilesModified": False,
        "realWorkLaunched": False,
    }
    atomic_json(root_path(root, "preflight/exclusion-freeze.json"), receipt)
    unresolved = refresh_preflight(root, "freeze-exclusions", "exclusions-frozen" if complete else "exclusions-partial")
    payload = {
        "command": "freeze-exclusions",
        "status": receipt["status"],
        "entries": len(results),
        "failures": len(failures),
        "unresolved": len(unresolved),
        "realWorkLaunched": False,
    }
    print(json.dumps(payload, sort_keys=True), file=sys.stderr if failures else sys.stdout)
    return 0 if complete else 2


def run_host_preflight(root: Path) -> int:
    if not root_path(root, "contract.json").is_file():
        fail("run prepare before host-preflight")
    host = "192.168.0.12"
    command = [
        "ssh",
        "-F",
        "/dev/null",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectionAttempts=1",
        host,
        "pwd",
    ]
    try:
        completed = subprocess.run(
            command,
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
            text=True,
            timeout=20,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        completed = None
        error = str(exc)
    else:
        error = None
    receipt_path = root_path(root, HOST_CONNECTIVITY_REL)
    if receipt_path.is_file():
        receipt = load_json(receipt_path)
        attempts = list(receipt.get("attempts", []))
    else:
        attempts = []
    attempt = {
        "attempt": len(attempts) + 1,
        "startedAt": now(),
        "host": host,
        "command": command,
        "status": "timeout" if completed is None else ("reachable" if completed.returncode == 0 else "unreachable"),
        "returncode": None if completed is None else completed.returncode,
        "stdout": None if completed is None else completed.stdout[-2000:],
        "stderr": error if completed is None else completed.stderr[-2000:],
        "readOnly": True,
    }
    attempts.append(attempt)
    status = "reachable-probe-only" if attempt["status"] == "reachable" else "unreachable"
    receipt = {
        "schema": SCHEMAS["hostConnectivity"],
        "createdAt": receipt.get("createdAt", now()) if receipt_path.is_file() else now(),
        "updatedAt": now(),
        "experimentId": EXPERIMENT_ID,
        "preferredHost": host,
        "status": status,
        "attempts": attempts,
        "fullInventoryRequiredAfterReachability": True,
        "forecast": None,
        "fallbackHostUsed": None,
        "readOnly": True,
        "realWorkLaunched": False,
    }
    atomic_json(receipt_path, receipt)
    unresolved = refresh_preflight(root, "host-preflight", "host-probe-recorded")
    payload = {
        "command": "host-preflight",
        "status": status,
        "attempt": attempt["attempt"],
        "unresolved": len(unresolved),
        "realWorkLaunched": False,
    }
    print(json.dumps(payload, sort_keys=True), file=sys.stderr if status != "reachable-probe-only" else sys.stdout)
    return 0 if status == "reachable-probe-only" else 2


def run_prepare(root: Path, resume: bool) -> int:
    if protected(str(root)):
        fail(f"protected historical path is forbidden: {PROTECTED_TOKEN}")
    if root.exists() and any(root.iterdir()) and not resume:
        fail(f"Step 12 root is already populated; use --resume only after reviewing it: {root}")
    root.mkdir(parents=True, exist_ok=True)
    for directory in (
        "authorization",
        "preflight",
        "preflight/exclusion-families",
        "source",
        "data",
        "openings",
        "training",
        "checkpoints",
        "exports",
        "parity",
        "matches",
    ):
        root_path(root, directory).mkdir(parents=True, exist_ok=True)

    ensure_source_config(root)
    allowlist = exclusion_allowlist(root)
    unresolved = unresolved_entries(allowlist, root)
    contract = build_contract(root)
    identity = {
        "schema": SCHEMAS["sourceIdentity"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "coordinator": {"repository": str(REPO_ROOT), **git_identity()},
        "boundDocuments": {
            "plan": artifact("plans/anhoku-nnue-strength-first-plan.md"),
            "runbook": artifact("plans/anhoku-nnue-strength-first-step12-independent-replication-runbook.md"),
        },
        "priorEvidence": {
            "report": artifact("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/report.md"),
            "analysis": artifact("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/analysis.json"),
            "closeoutRepair": artifact("out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/preflight/closeout-repair.json"),
        },
        "forbiddenPathPolicy": {"token": PROTECTED_TOKEN, "traversed": False, "reopened": False},
    }
    atomic_json(root_path(root, "contract.json"), contract)
    atomic_json(root_path(root, "source-identity.json"), identity)
    write_authorizations(root)
    atomic_json(root_path(root, "commands.json"), build_commands())
    atomic_json(root_path(root, "preflight/exclusion-allowlist.json"), allowlist)
    atomic_json(root_path(root, "preflight/unresolved.json"), build_unresolved(unresolved))
    bindings = build_bindings(root, allowlist, unresolved)
    atomic_json(root_path(root, "execution-bindings.json"), bindings)
    fixture_status = "not-run"
    atomic_json(root_path(root, "preflight/fixtures.json"), {
        "schema": SCHEMAS["fixtures"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": fixture_status,
        "tests": [],
        "launchReady": False,
    })
    atomic_json(root_path(root, "preflight/launch-gate.json"), build_launch_gate(root, unresolved, fixture_status))
    atomic_json(root_path(root, "status.json"), build_status(root, "prepare", "preparation-blocked"))
    write_manifest(root, allowlist, unresolved, fixture_status)
    print(json.dumps({
        "command": "prepare",
        "status": "preparation-blocked",
        "root": str(root),
        "exclusionEntries": len(allowlist["entries"]),
        "unresolved": len(unresolved),
        "trainingAuthorized": False,
        "generationTrainingMatchesStarted": False,
        "sourceConfig": SOURCE_CONFIG_REL,
    }, sort_keys=True))
    return 0


def choose_earliest(losses: dict[int, float]) -> int:
    if set(losses) != set(CANDIDATE_UPDATES):
        fail("selection fixture must contain exactly the eight candidate updates")
    if any(not isinstance(value, (int, float)) or isinstance(value, bool) or not math.isfinite(float(value)) for value in losses.values()):
        fail("selection rejects missing or nonfinite validation loss")
    minimum = min(float(value) for value in losses.values())
    eligible = [update for update in CANDIDATE_UPDATES if float(losses[update]) <= minimum + SELECTION_TOLERANCE]
    return min(eligible)


def run_fixtures(root: Path) -> int:
    contract_path = root_path(root, "contract.json")
    if not contract_path.is_file():
        fail("run prepare before fixtures")
    contract = load_json(contract_path)
    if contract.get("experimentId") != EXPERIMENT_ID:
        fail("contract experiment identity mismatch")
    # Keep the command ledger synchronized with the currently implemented,
    # still-gated downstream adapters before exercising its assertions.
    atomic_json(root_path(root, "commands.json"), build_commands())
    tests: list[dict[str, Any]] = []

    def test(name: str, fn: Callable[[], None], kind: str) -> None:
        try:
            fn()
        except (AssertionError, Step12Error, ValueError, TypeError) as exc:
            tests.append({"name": name, "kind": kind, "status": "failed", "message": str(exc)})
        else:
            tests.append({"name": name, "kind": kind, "status": "passed"})

    def assert_flags() -> None:
        assert contract["flags"] == {
            "confirmationAuthorized": False,
            "deploymentAuthorized": False,
            "handcraftedRemainsProduction": True,
        }

    def schedule_shape() -> None:
        assert 2 * 4 * 32 * 2 == 512
        assert contract["schedule"]["pairsPerCell"] == 128
        assert contract["schedule"]["totalGames"] == 512

    def selection_boundary() -> None:
        losses = {update: 0.4 + update / 1000000 for update in CANDIDATE_UPDATES}
        losses[768] = 0.1
        losses[1024] = 0.1 + 0.5e-12
        assert choose_earliest(losses) == 768

    def selection_earliest_tie() -> None:
        losses = {update: 0.2 for update in CANDIDATE_UPDATES}
        assert choose_earliest(losses) == 128

    def selection_rejects_nan() -> None:
        losses = {update: 0.2 for update in CANDIDATE_UPDATES}
        losses[768] = float("nan")
        try:
            choose_earliest(losses)
        except Step12Error:
            return
        raise AssertionError("NaN candidate was accepted")

    def protected_path_rejected() -> None:
        try:
            repo_path(PROTECTED_TOKEN + "/anything")
        except Step12Error:
            return
        raise AssertionError("protected path was accepted")

    def no_recursive_discovery() -> None:
        allowlist = load_json(root_path(root, "preflight/exclusion-allowlist.json"))
        assert allowlist["verification"]["recursiveDiscoveryUsed"] is False
        assert allowlist["protectedPaths"] == [PROTECTED_PATH]

    def source_config_is_bound() -> None:
        config = root_path(root, SOURCE_CONFIG_REL)
        assert config.is_file()
        text = config.read_text(encoding="utf-8")
        assert "train_games = 30000" in text
        assert "validation_games = 2016" in text
        assert "seed = 2026091601" in text
        assert "shard_games = 10" in text
        assert "search_nodes = 10000" in text

    def phase_is_fail_closed() -> None:
        commands = load_json(root_path(root, "commands.json"))["commands"]
        by_name = {item["name"]: item for item in commands}
        assert by_name["train"]["status"] == "implemented-gated"
        assert by_name["generate-data"]["status"] == "implemented-gated"
        assert by_name["freeze-openings"]["status"] == "implemented-gated"
        assert by_name["generate-data"]["launchesRealWork"] is True
        assert by_name["host-preflight"]["launchesRealWork"] is False
        assert by_name["run-matches"]["launchesRealWork"] is True

    def source_binding_uses_repository_generator() -> None:
        bindings = load_json(root_path(root, "execution-bindings.json"))
        source = bindings["sourceGeneration"]
        assert source["status"] == "bound"
        assert source["generatorImplementation"]["path"] == SOURCE_GENERATOR
        assert source["canonicalizationImplementation"]["path"] == CANONICALIZATION_SOURCE
        command = source["generatorCommand"]
        argv = command["argv"]
        assert "generate-data" in argv
        assert command["deterministicShardMap"]["shardGames"] == 10
        if command.get("executionMode") == "remote-ssh":
            assert command["host"] == "192.168.0.12"
            assert "192.168.0.12" in argv
        else:
            assert argv[1] == "generate-data"

    def unresolved_blocks_launch() -> None:
        unresolved = load_json(root_path(root, "preflight/unresolved.json"))
        gate = load_json(root_path(root, "preflight/launch-gate.json"))
        assert unresolved["status"] == "open"
        assert unresolved["entries"]
        assert gate["launchReady"] is False

    def artifact_shape() -> None:
        item = artifact("plans/anhoku-nnue-strength-first-plan.md")
        assert item["status"] == "observed"
        assert isinstance(item["bytes"], int) and item["bytes"] > 0
        assert isinstance(item["sha256"], str) and len(item["sha256"]) == 64

    test("flags-preserve-production-boundary", assert_flags, "positive")
    test("fixed-512-game-shape", schedule_shape, "positive")
    test("checkpoint-selection-tolerance", selection_boundary, "positive")
    test("checkpoint-selection-earliest-tie", selection_earliest_tie, "positive")
    test("checkpoint-selection-rejects-nan", selection_rejects_nan, "negative")
    test("protected-path-rejection", protected_path_rejected, "negative")
    test("allowlist-does-not-recursively-discover", no_recursive_discovery, "negative")
    test("source-config-is-bound", source_config_is_bound, "positive")
    test("source-binding-uses-repository-generator", source_binding_uses_repository_generator, "positive")
    test("phase-adapters-fail-closed", phase_is_fail_closed, "negative")
    test("unresolved-gate-blocks-launch", unresolved_blocks_launch, "negative")
    test("artifact-receipt-shape", artifact_shape, "positive")

    failed = [item for item in tests if item["status"] != "passed"]
    status = "passed-preparation-fixtures" if not failed else "failed"
    fixture_receipt = {
        "schema": SCHEMAS["fixtures"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": status,
        "tests": tests,
        "requiredFullPhaseFixtures": {
            "trainerResume": False,
            "sourceFamilyOverlap": False,
            "runtimeParity": False,
            "terminalReplay": False,
            "matchValidator": False,
            "lockLedgerRecovery": False,
            "closeoutIdempotence": False,
        },
        "launchReady": False,
        "noRealWorkLaunched": True,
    }
    atomic_json(root_path(root, "preflight/fixtures.json"), fixture_receipt)
    unresolved = load_json(root_path(root, "preflight/unresolved.json"))["entries"]
    atomic_json(root_path(root, "preflight/launch-gate.json"), build_launch_gate(root, unresolved, status))
    atomic_json(root_path(root, "commands.json"), build_commands())
    status_doc = load_json(root_path(root, "status.json"))
    status_doc["updatedAt"] = now()
    status_doc["fixtures"] = root_artifact(root, "preflight/fixtures.json")
    atomic_json(root_path(root, "status.json"), status_doc)
    allowlist = load_json(root_path(root, "preflight/exclusion-allowlist.json"))
    write_manifest(root, allowlist, unresolved, status)
    print(json.dumps({
        "command": "fixtures",
        "status": status,
        "tests": len(tests),
        "failed": len(failed),
        "launchReady": False,
        "realWorkLaunched": False,
    }, sort_keys=True))
    return 0 if not failed else 1


def run_prepare_training(root: Path) -> int:
    """Create the Step 12-owned training package without importing a device stack."""
    if not root_path(root, "contract.json").is_file():
        fail("run prepare before prepare-training")
    runner = repo_path("scripts/strength_first_step12_trainer.py")
    command = [sys.executable, str(runner), "--root", str(root), "prepare"]
    completed = subprocess.run(command, cwd=REPO_ROOT, capture_output=True, text=True, check=False)
    atomic_json(root_path(root, "preflight/training-preparation-dispatch.json"), {
        "schema": "haitaka-anhoku-strength-first-step12-training-preparation-dispatch-v1",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "passed" if completed.returncode == 0 else "failed",
        "command": command,
        "returncode": completed.returncode,
        "stdout": completed.stdout[-4000:],
        "stderr": completed.stderr[-4000:],
        "launchesRealWork": False,
        "trainingOrGpuAttempted": False,
        "outcomesOpened": False,
    })
    if completed.returncode != 0:
        print(json.dumps({"command": "prepare-training", "status": "failed", "returncode": completed.returncode, "launchesRealWork": False}, sort_keys=True), file=sys.stderr)
        return 1
    unresolved = refresh_preflight(root, "training-preparation", "training-preparation-bound")
    print(json.dumps({"command": "prepare-training", "status": "passed", "unresolved": len(unresolved), "launchesRealWork": False}, sort_keys=True))
    return 0


def training_gate_receipt(root: Path, unresolved: list[dict[str, Any]]) -> dict[str, Any]:
    package_path = root_path(root, "training/package.json")
    config_path = root_path(root, "training/config.json")
    inputs_path = root_path(root, "training/inputs.json")
    cpu_path = root_path(root, "preflight/training-fixtures-cpu.json")
    gpu_path = root_path(root, "preflight/training-fixtures-cuda.json")
    qualification_path = root_path(root, "preflight/deterministic-cuda-v1/qualification.json")
    training_auth_path = root_path(root, "authorization/training.json")
    gpu_auth_path = root_path(root, "authorization/gpu-rental.json")
    classifications: list[str] = []
    reasons: list[str] = []
    if not all(path.is_file() for path in (package_path, config_path, inputs_path)):
        classifications.append("implementation-missing")
        reasons.append("the Step 12 training config/input/package is incomplete")
    package = load_json(package_path) if package_path.is_file() else {}
    if package.get("status") not in {"prepared-awaiting-training-authorization", "training-authorized", "training-complete-awaiting-qualification"}:
        if "implementation-missing" not in classifications:
            classifications.append("implementation-missing")
        reasons.append("training/package.json is not a recognized prepared package")
    cpu = load_json(cpu_path) if cpu_path.is_file() else {}
    if cpu.get("status") == "failed":
        classifications.append("cpu-fixture-failed")
        reasons.append("the MBP CPU fixture has a preserved failed summary")
    elif cpu.get("status") != "passed":
        classifications.append("device-fixtures-pending")
        reasons.append("the bounded MBP CPU fixture has not passed")
    gpu = load_json(gpu_path) if gpu_path.is_file() else {}
    training_auth = load_json(training_auth_path) if training_auth_path.is_file() else {}
    gpu_auth = load_json(gpu_auth_path) if gpu_auth_path.is_file() else {}
    qualification = load_json(qualification_path) if qualification_path.is_file() else {}
    def bound_artifact(item: Any, expected: Path) -> bool:
        if not isinstance(item, dict) or not isinstance(item.get("path"), str) or not isinstance(item.get("bytes"), int) or not isinstance(item.get("sha256"), str):
            return False
        observed = Path(item["path"])
        if not observed.is_absolute():
            observed = root_path(root, item["path"])
        return observed.resolve() == expected.resolve() and expected.is_file() and item["bytes"] == expected.stat().st_size and item["sha256"] == digest(expected)

    def artifact_path(item: Any) -> Path | None:
        if not isinstance(item, dict) or not isinstance(item.get("path"), str):
            return None
        path = Path(item["path"])
        return path if path.is_absolute() else root_path(root, item["path"])

    def bound_self(item: Any) -> Path | None:
        path = artifact_path(item)
        if path is None or not bound_artifact(item, path):
            return None
        return path

    def state_inventory_is_valid(value: Any) -> bool:
        if not isinstance(value, dict) or value.get("schema") != "haitaka-anhoku-strength-first-step12-cuda-state-inventory-v1":
            return False
        for phase in ("initial", "uninterrupted", "resumed"):
            section = value.get(phase)
            if not isinstance(section, dict):
                return False
            required = ("model",) if phase == "initial" else ("model", "optimizer")
            for name in required:
                inventory = section.get(name)
                if not isinstance(inventory, dict) or not isinstance(inventory.get("entries"), list) or not inventory["entries"]:
                    return False
                for entry in inventory["entries"]:
                    if not isinstance(entry, dict) or not isinstance(entry.get("path"), str) or entry.get("kind") not in {"tensor", "scalar"}:
                        return False
                    if entry["kind"] == "tensor":
                        if not isinstance(entry.get("dtype"), str) or not isinstance(entry.get("shape"), list) or any(not isinstance(dim, int) or dim < 0 for dim in entry["shape"]):
                            return False
                        if not isinstance(entry.get("sha256"), str) or len(entry["sha256"]) != 64:
                            return False
                    elif not isinstance(entry.get("value"), str):
                        return False
        if any(not isinstance(value.get(name), str) or len(value[name]) != 64 for name in ("uninterruptedRngDigest", "resumedRngDigest", "uninterruptedExportSha256", "resumedExportSha256")):
            return False
        if not isinstance(value.get("uninterruptedTrace"), list) or not isinstance(value.get("resumedTrace"), list):
            return False
        return True

    def load_state_inventory(worker: dict[str, Any]) -> tuple[dict[str, Any], Path] | None:
        path = bound_self(worker.get("stateInventory"))
        if path is None:
            return None
        try:
            value = load_json(path)
        except Exception:
            return None
        return (value, path) if state_inventory_is_valid(value) else None

    def qualification_evidence_is_valid() -> bool:
        if qualification.get("schema") != "haitaka-anhoku-strength-first-step12-deterministic-cuda-qualification-v1" or qualification.get("status") != "passed" or qualification.get("experimentId") != EXPERIMENT_ID:
            return False
        if not bound_artifact(qualification.get("package"), package_path) or not bound_artifact(qualification.get("config"), config_path) or not bound_artifact(qualification.get("inputs"), inputs_path) or not bound_artifact(qualification.get("authorization"), gpu_auth_path):
            return False
        if gpu_auth.get("granted") is not True or gpu_auth.get("scope") != "Vast.ai rental for Step 12 training":
            return False
        binding = gpu_auth.get("fixtureBinding", {})
        for role, path in (("config", config_path), ("inputs", inputs_path), ("package", package_path)):
            if not bound_artifact(binding.get(role), path):
                return False
        amendment = gpu_auth.get("allowanceAmendment", {})
        remaining = amendment.get("remainingCumulative", {}) if isinstance(amendment, dict) else {}
        resource = gpu_auth.get("resource", {})
        lifecycle = resource.get("lifecycle", {}) if isinstance(resource, dict) else {}
        quote = gpu_auth.get("freshQuote", {})
        try:
            grant_until = datetime.fromisoformat(str(gpu_auth.get("validUntil", "")).replace("Z", "+00:00"))
            quote_until = datetime.fromisoformat(str(quote.get("validUntil", "")).replace("Z", "+00:00"))
            quote_observed = datetime.fromisoformat(str(quote.get("observedAt", "")).replace("Z", "+00:00"))
            quote_time_valid = quote_until > quote_observed and quote_until > datetime.now(timezone.utc) and grant_until > datetime.now(timezone.utc) and grant_until <= quote_until
        except (TypeError, ValueError):
            quote_time_valid = False
        quote_bound = isinstance(quote, dict) and all(quote.get(key) for key in ("provider", "offerId", "gpuName", "image", "imageDigest", "dependencyLockSha256", "observedAt", "validUntil", "hourlyTotalUsd")) and len(str(quote.get("imageDigest", ""))) == 64 and len(str(quote.get("dependencyLockSha256", ""))) == 64
        resource_valid = isinstance(resource, dict) and isinstance(resource.get("instanceId"), int) and 0 < float(resource.get("hourlyTotalUsd", 0)) < 0.30 and 0 < float(lifecycle.get("maximumInstanceHours", 0)) <= 2.0 and 0 < float(lifecycle.get("maximumTotalUsd", 0)) <= 5.0 and float(remaining.get("hours", 0)) >= float(lifecycle.get("maximumInstanceHours", 0)) and float(remaining.get("costUsd", 0)) >= float(lifecycle.get("maximumTotalUsd", 0))
        if amendment.get("approved") is not True or amendment.get("additionalInstances") != 1 or int(remaining.get("instances", 0)) < 1 or not (0 < float(remaining.get("hours", 0)) <= 4.0) or not (0 < float(remaining.get("costUsd", 0)) <= 5.0) or not resource_valid or not quote_bound or not quote_time_valid or quote.get("provider") != resource.get("provider") or quote.get("offerId") != resource.get("offerId") or float(quote.get("hourlyTotalUsd", -1)) != float(resource.get("hourlyTotalUsd", -2)):
            return False
        gradient_entries = qualification.get("gradientTests", [])
        fixture_entries = qualification.get("fixtures", [])
        if len(gradient_entries) != 2 or len(fixture_entries) != 2:
            return False
        for entry in gradient_entries:
            if entry.get("status") != "passed" or bound_self(entry.get("artifact")) is None:
                return False
            child_path = artifact_path(entry["artifact"])
            if child_path is None:
                return False
            child = load_json(child_path)
            if child.get("status") != "passed" or not child.get("cases"):
                return False
        worker_reports: list[dict[str, Any]] = []
        for entry in fixture_entries:
            if entry.get("status") != "passed" or not isinstance(entry.get("artifact"), dict) or not isinstance(entry.get("workerReport"), dict):
                return False
            summary_path = artifact_path(entry["artifact"])
            worker_path = artifact_path(entry["workerReport"])
            if summary_path is None or worker_path is None or not bound_artifact(entry["artifact"], summary_path) or not bound_artifact(entry["workerReport"], worker_path):
                return False
            summary = load_json(summary_path)
            worker = load_json(worker_path)
            if summary.get("status") != "passed" or worker.get("status") != "passed" or worker.get("resumeEquivalence", {}).get("weightsExact") is not True or worker.get("resumeEquivalence", {}).get("optimizerExact") is not True:
                return False
            worker_reports.append(worker)
        inventories = [load_state_inventory(worker) for worker in worker_reports]
        if any(item is None for item in inventories):
            return False
        first_inventory = inventories[0][0]
        second_inventory = inventories[1][0]
        cross_attempt_fields = ("initial", "uninterrupted", "resumed", "uninterruptedRngDigest", "resumedRngDigest", "uninterruptedTrace", "resumedTrace", "uninterruptedExportSha256", "resumedExportSha256")
        if any(first_inventory.get(field) != second_inventory.get(field) for field in cross_attempt_fields):
            return False
        probes = [worker.get("cudaRngProbe") for worker in worker_reports]
        for probe in probes:
            if not isinstance(probe, dict) or probe.get("profile") != "seeded-cuda-save-restore-v2" or not isinstance(probe.get("seed"), int) or probe.get("exact") is not True or probe.get("stateRestoredExact") is not True or not isinstance(probe.get("stateBefore"), list) or not probe.get("stateBefore") or probe.get("stateAfterRestore") != probe.get("stateBefore") or not isinstance(probe.get("drawSha256"), str) or len(probe.get("drawSha256")) != 64 or any(not isinstance(item, str) or len(item) != 64 for item in probe.get("stateBefore", [])):
                return False
        if probes[0] != probes[1] or worker_reports[0].get("nextRandomDraw") != worker_reports[1].get("nextRandomDraw"):
            return False
        adapter_hash = package.get("implementation", {}).get("cudaAdapter", {}).get("sha256")
        kernel_hash = package.get("implementation", {}).get("cudaKernel", {}).get("sha256")
        for worker in worker_reports:
            runtime = worker.get("actualComponents", {}).get("deterministicCudaRuntime", {})
            requested = runtime.get("adapter", {}).get("binding", {}).get("requestedBinding", {})
            if runtime.get("featureTransformerModule") != "feature_transformer" or runtime.get("adapter", {}).get("profile") != "ordered-segment-v1" or requested.get("adapter", {}).get("sha256") != adapter_hash or requested.get("kernel", {}).get("sha256") != kernel_hash:
                return False
        ledger = qualification.get("budgetLedger")
        expected_stages = ["gradient-process-1", "gradient-process-2", "fixture-1", "fixture-2"]
        if not isinstance(ledger, list) or [item.get("stage") for item in ledger] != expected_stages or any(item.get("status") != "passed" or float(item.get("estimatedCostUsd", 0)) < 0 for item in ledger):
            return False
        return qualification.get("productionTrainingAttempted") is False and qualification.get("gpuAttempted") is True and qualification.get("crossAttemptExact") is True and qualification.get("sharedProductionAdapterBinding") is True

    cuda_qualified = qualification_evidence_is_valid()
    if cuda_qualified:
        pass
    elif gpu.get("status") == "failed":
        classifications.append("cuda-fixture-failed-exact-resume-determinism")
        reasons.append("the authorized CUDA fixture failed exact full-precision interrupted/resumed model and optimizer equivalence")
    else:
        classifications.append("cuda-qualification-pending-or-invalid")
        reasons.append("the deterministic CUDA amendment lacks a hash-verified qualification receipt with two gradient processes and two exact fresh-process fixtures")
    if training_auth.get("granted") is not True:
        classifications.append("training-authorization-missing")
        reasons.append("authorization/training.json is not granted")
    if gpu_auth.get("granted") is not True:
        classifications.append("gpu-authorization-missing")
        reasons.append("authorization/gpu-rental.json is not granted")
    training_scoped = [entry for entry in unresolved if entry.get("phase") in PHASE_DEPENDENCIES["training"]]
    upstream = [entry for entry in training_scoped if entry.get("id") != "training-authorization"]
    if upstream:
        classifications.append("upstream-dependencies-pending")
        reasons.extend(entry["reason"] for entry in upstream)
    classifications = list(dict.fromkeys(classifications))
    ready = not classifications and not reasons
    unresolved_ids = [entry["id"] for entry in training_scoped if not (cuda_qualified and entry["id"] == "cuda-determinism")]
    if "cuda-fixture-failed-exact-resume-determinism" in classifications and "cuda-determinism" not in unresolved_ids:
        unresolved_ids.insert(0, "cuda-determinism")
    return {
        "schema": SCHEMAS["trainingGate"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "ready" if ready else "blocked",
        "classification": [] if ready else classifications,
        "reasons": reasons,
        "unresolvedIds": unresolved_ids,
        "package": root_artifact(root, "training/package.json"),
        "config": root_artifact(root, "training/config.json"),
        "inputs": root_artifact(root, "training/inputs.json"),
        "cpuFixture": root_artifact(root, "preflight/training-fixtures-cpu.json"),
        "cudaFixture": root_artifact(root, "preflight/training-fixtures-cuda.json"),
        **({"cudaQualification": root_artifact(root, "preflight/deterministic-cuda-v1/qualification.json")} if qualification_path.is_file() else {}),
        **({"cudaDeterminismBlocker": root_artifact(root, "preflight/training-cuda-determinism-blocker.json")} if root_path(root, "preflight/training-cuda-determinism-blocker.json").is_file() else {}),
        "hostBindingValidation": root_artifact(root, "preflight/training-host-binding-validation.json"),
        "fixtureCoverage": root_artifact(root, "preflight/training-fixture-coverage.json"),
        "resourceForecast": root_artifact(root, "preflight/training-resource-forecast.json"),
        "resourceProposal": root_artifact(root, "training/resource-proposal.json"),
        "trainingAuthorization": root_artifact(root, "authorization/training.json"),
        "gpuAuthorization": root_artifact(root, "authorization/gpu-rental.json"),
        "launchesRealWork": False,
        "productionTrainingAttempted": False,
        "gpuAttempted": bool(gpu.get("gpuAttempted") or gpu_auth.get("trainingOrGpuAttempted")),
        "outcomesOpened": False,
        "flags": {
            "confirmationAuthorized": False,
            "deploymentAuthorized": False,
            "handcraftedRemainsProduction": True,
            "trainingOrGpuAttempted": bool(gpu.get("gpuAttempted") or gpu_auth.get("trainingOrGpuAttempted")),
        },
    }


def run_training_gate(root: Path) -> int:
    if not root_path(root, "contract.json").is_file():
        fail("run prepare before training-gate")
    status_doc = load_json(root_path(root, "status.json"))
    unresolved = refresh_preflight(root, "training-gate", "training-gate-pending")
    receipt = training_gate_receipt(root, unresolved)
    atomic_json(root_path(root, "preflight/training-gate.json"), receipt)
    status_doc = load_json(root_path(root, "status.json"))
    status_doc.update({"trainingGate": root_artifact(root, "preflight/training-gate.json"), "nextGate": "training" if receipt["status"] == "ready" else "training-gate", "updatedAt": now()})
    atomic_json(root_path(root, "status.json"), status_doc)
    print(json.dumps({"command": "training-gate", "status": receipt["status"], "classification": receipt["classification"], "launchesRealWork": False}, sort_keys=True), file=sys.stdout if receipt["status"] == "ready" else sys.stderr)
    return 0 if receipt["status"] == "ready" else 2


def run_train(root: Path, resume: bool) -> int:
    gate_path = root_path(root, "preflight/training-gate.json")
    if not gate_path.is_file():
        run_training_gate(root)
        return 2
    gate = load_json(gate_path)
    if gate.get("status") != "ready":
        print(json.dumps({"command": "train", "status": "blocked-preparation", "reason": "training-gate is not ready", "classification": gate.get("classification", []), "launchesRealWork": False}, sort_keys=True), file=sys.stderr)
        return 2
    package = load_json(root_path(root, "training/package.json"))
    command = list(package.get("hostBinding", {}).get("productionCommand", []))
    if resume:
        command.append("--resume")
    if not command:
        fail("training package has no bound production command")
    remote_command = remote_ssh_command(command)
    started = now()
    completed = subprocess.run(remote_command, cwd=REPO_ROOT, capture_output=True, text=True, check=False)
    atomic_text(root_path(root, "preflight/training-dispatch.stdout"), completed.stdout)
    atomic_text(root_path(root, "preflight/training-dispatch.stderr"), completed.stderr)
    receipt = {
        "schema": "haitaka-anhoku-strength-first-step12-training-dispatch-v1",
        "createdAt": started,
        "experimentId": EXPERIMENT_ID,
        "status": "completed" if completed.returncode == 0 else "failed",
        "command": remote_command,
        "returncode": completed.returncode,
        "stdout": root_artifact(root, "preflight/training-dispatch.stdout"),
        "stderr": root_artifact(root, "preflight/training-dispatch.stderr"),
        "launchesRealWork": True,
        "trainingOrGpuAttempted": True,
        "gpuAttempted": True,
        "outcomesOpened": False,
    }
    atomic_json(root_path(root, "preflight/training-dispatch.json"), receipt)
    return 0 if completed.returncode == 0 else 1


def run_gate(root: Path, command: str) -> int:
    if command == "training-gate":
        return run_training_gate(root)
    all_unresolved = load_json(root_path(root, "preflight/unresolved.json"))["entries"]
    phase = "matches"
    unresolved = phase_unresolved(all_unresolved, phase)
    auth_name = "cpu-match.json"
    auth = load_json(root_path(root, f"authorization/{auth_name}"))
    reasons = [entry["reason"] for entry in unresolved]
    resolution = None
    if not any(entry.get("id") == "training-authorization" for entry in unresolved):
        try:
            resolution = training_authorization_resolution(root)
        except Step12Error as exc:
            reasons.append(str(exc))
        if resolution is None:
            reasons.append("training-authorization was removed without a validated reconciliation receipt")
    if auth.get("granted") is not True:
        reasons.append(f"{auth_name} is not granted")
    if command == "match-gate" and not root_path(root, "preflight/qualification-gate.json").is_file():
        reasons.append("preflight/qualification-gate.json is missing")
    if command == "match-gate" and root_path(root, "preflight/qualification-gate.json").is_file():
        qualification = load_json(root_path(root, "preflight/qualification-gate.json"))
        if qualification.get("status") != "passed":
            reasons.append("qualification gate is not passed")
    status = "blocked"
    receipt = {
        "schema": f"haitaka-anhoku-strength-first-step12-{command}-v1",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": status,
        "reasons": reasons,
        "launchesRealWork": False,
        "flags": {
            "confirmationAuthorized": False,
            "deploymentAuthorized": False,
            "handcraftedRemainsProduction": True,
        },
    }
    if not reasons and command == "match-gate":
        helper = repo_path("scripts/strength_first_step12_matches.py")
        completed = subprocess.run(
            [sys.executable, str(helper), "gate", "--root", str(root)],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        atomic_text(root_path(root, "preflight/match-gate.stdout"), completed.stdout)
        atomic_text(root_path(root, "preflight/match-gate.stderr"), completed.stderr)
        if completed.returncode == 0:
            return 0
        return 2
    atomic_json(root_path(root, f"preflight/{command}.json"), receipt)
    print(json.dumps({"command": command, "status": status, "reasons": len(reasons), "launchesRealWork": False}, sort_keys=True))
    return 2


def run_select_and_qualify(root: Path) -> int:
    """Bind validation-only selection; parity worker execution is host-bound."""

    all_unresolved = load_json(root_path(root, "preflight/unresolved.json"))["entries"]
    unresolved = phase_unresolved(all_unresolved, "qualification")
    if unresolved:
        print(json.dumps({"command": "select-and-qualify", "status": "blocked-preparation", "unresolvedIds": [item["id"] for item in unresolved], "launchesRealWork": False}, sort_keys=True), file=sys.stderr)
        return 2
    helper = repo_path("scripts/strength_first_step12_qualification.py")
    completed = subprocess.run(
        [sys.executable, str(helper), "select", "--root", str(root)],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    atomic_text(root_path(root, "preflight/selection-worker.stdout"), completed.stdout)
    atomic_text(root_path(root, "preflight/selection-worker.stderr"), completed.stderr)
    receipt = {
        "schema": "haitaka-anhoku-strength-first-step12-selection-worker-dispatch-v1",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "selected-awaiting-mbp-parity" if completed.returncode == 0 else "failed",
        "command": [sys.executable, str(helper), "select", "--root", str(root)],
        "returncode": completed.returncode,
        "stdout": root_artifact(root, "preflight/selection-worker.stdout"),
        "stderr": root_artifact(root, "preflight/selection-worker.stderr"),
        "hostBinding": "MBP 192.168.0.12 must execute worker-parity before qualification gate",
        "gamesLaunched": 0,
        "outcomesOpened": False,
    }
    atomic_json(root_path(root, "preflight/selection-worker.json"), receipt)
    print(json.dumps({"command": "select-and-qualify", "status": receipt["status"], "launchesRealWork": False, "workerHost": "192.168.0.12"}, sort_keys=True), file=sys.stdout if completed.returncode == 0 else sys.stderr)
    return 0 if completed.returncode == 0 else 1


def run_matches(root: Path, resume: bool) -> int:
    helper = repo_path("scripts/strength_first_step12_matches.py")
    completed = subprocess.run(
        [sys.executable, str(helper), "run", "--root", str(root), *(["--resume"] if resume else [])],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    atomic_text(root_path(root, "matches/worker.stdout"), completed.stdout)
    atomic_text(root_path(root, "matches/worker.stderr"), completed.stderr)
    print(completed.stdout if completed.returncode == 0 else completed.stderr, end="", file=sys.stdout if completed.returncode == 0 else sys.stderr)
    return completed.returncode


def run_closeout(root: Path) -> int:
    helper = repo_path("scripts/strength_first_step12_closeout.py")
    completed = subprocess.run(
        [sys.executable, str(helper), "run", "--root", str(root)],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    atomic_text(root_path(root, "closeout.stdout"), completed.stdout)
    atomic_text(root_path(root, "closeout.stderr"), completed.stderr)
    print(completed.stdout if completed.returncode == 0 else completed.stderr, end="", file=sys.stdout if completed.returncode == 0 else sys.stderr)
    return completed.returncode


def run_generate_data(root: Path) -> int:
    if not root_path(root, "contract.json").is_file():
        fail("run prepare before generate-data")
    all_unresolved = load_json(root_path(root, "preflight/unresolved.json"))["entries"]
    unresolved = phase_unresolved(all_unresolved, "source-generation")
    binding = load_json(root_path(root, "execution-bindings.json")).get("sourceGeneration", {})
    reasons = [entry["reason"] for entry in unresolved]
    if binding.get("status") != "bound":
        reasons.append("source-generation binding is not status=bound")
    auth = load_json(root_path(root, "authorization/cpu-generation.json"))
    if auth.get("granted") is not True:
        reasons.append("authorization/cpu-generation.json is not granted")
    if reasons:
        receipt = {
            "schema": "haitaka-anhoku-strength-first-step12-source-generation-gate-v1",
            "createdAt": now(),
            "experimentId": EXPERIMENT_ID,
            "status": "blocked",
            "phase": "source-generation",
            "reasons": reasons,
            "unresolvedIds": [entry["id"] for entry in unresolved],
            "binding": binding,
            "launchesRealWork": False,
            "outcomesOpened": False,
            "trainingOrGpuAttempted": False,
        }
        atomic_json(root_path(root, "preflight/source-generation-gate.json"), receipt)
        print(json.dumps({
            "command": "generate-data",
            "status": "blocked-preparation",
            "reasons": len(reasons),
            "unresolved": len(unresolved),
            "launchesRealWork": False,
        }, sort_keys=True), file=sys.stderr)
        return 2

    command = binding.get("generatorCommand", {})
    argv = command.get("argv")
    if not isinstance(argv, list) or not argv:
        fail("source-generation binding has no exact generator argv")
    # This branch is intentionally reached only after the host and forecast
    # gates are resolved.  The bound command is executed without a shell and
    # its stdout/stderr are retained under the source root for later hashing.
    output_dir = root_path(root, SOURCE_OUTPUT_REL)
    output_dir.mkdir(parents=True, exist_ok=True)
    stdout_path = root_path(root, "source/generation.stdout")
    stderr_path = root_path(root, "source/generation.stderr")
    try:
        completed = subprocess.run(
            [str(value) for value in argv],
            cwd=command.get("cwd", str(REPO_ROOT)),
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as exc:
        fail(f"bound source generator could not start: {exc}")
    atomic_text(stdout_path, completed.stdout)
    atomic_text(stderr_path, completed.stderr)
    if completed.returncode != 0:
        receipt = {
            "schema": "haitaka-anhoku-strength-first-step12-source-generation-v1",
            "createdAt": now(),
            "experimentId": EXPERIMENT_ID,
            "status": "failed",
            "returncode": completed.returncode,
            "command": command,
            "stdout": root_artifact(root, "source/generation.stdout"),
            "stderr": root_artifact(root, "source/generation.stderr"),
            "launchesRealWork": True,
            "outcomesOpened": False,
            "trainingOrGpuAttempted": False,
        }
        atomic_json(root_path(root, "source/generation.json"), receipt)
        print(json.dumps({"command": "generate-data", "status": "failed", "returncode": completed.returncode, "launchesRealWork": True}, sort_keys=True), file=sys.stderr)
        return 1
    receipt = {
        "schema": "haitaka-anhoku-strength-first-step12-source-generation-v1",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "completed-awaiting-independent-freeze",
        "command": command,
        "stdout": root_artifact(root, "source/generation.stdout"),
        "stderr": root_artifact(root, "source/generation.stderr"),
        "outputDirectory": str(output_dir),
        "launchesRealWork": True,
        "outcomesOpened": False,
        "trainingOrGpuAttempted": False,
    }
    atomic_json(root_path(root, "source/generation.json"), receipt)
    print(json.dumps({"command": "generate-data", "status": receipt["status"], "launchesRealWork": True}, sort_keys=True))
    return 0


def remote_ssh_command(arguments: list[str]) -> list[str]:
    return [
        "ssh", "-F", "/dev/null", "-o", "ConnectTimeout=10", "-o", "BatchMode=yes",
        "-o", "ConnectionAttempts=1", REMOTE_HOST, *arguments,
    ]


def remote_rsync_to(local_path: Path, remote_path: str) -> list[str]:
    return [
        "rsync", "-a", "-e", "ssh -F /dev/null -o ConnectTimeout=10 -o BatchMode=yes -o ConnectionAttempts=1",
        str(local_path), f"{REMOTE_HOST}:{remote_path}",
    ]


def remote_rsync_from(remote_path: str, local_path: Path) -> list[str]:
    return [
        "rsync", "-a", "-e", "ssh -F /dev/null -o ConnectTimeout=10 -o BatchMode=yes -o ConnectionAttempts=1",
        f"{REMOTE_HOST}:{remote_path}", str(local_path),
    ]


def parse_mbp_time(text: str) -> dict[str, Any]:
    patterns = {
        "wallSeconds": r"^real ([0-9.]+)$",
        "userSeconds": r"^user ([0-9.]+)$",
        "systemSeconds": r"^sys ([0-9.]+)$",
        "peakRssBytes": r"^\s*([0-9]+)\s+maximum resident set size$",
    }
    result: dict[str, Any] = {}
    for line in text.splitlines():
        for key, pattern in patterns.items():
            match = __import__("re").match(pattern, line)
            if match:
                result[key] = float(match.group(1)) if key != "peakRssBytes" else int(match.group(1))
    return result


def remote_file_identity(path: str) -> dict[str, Any]:
    result = subprocess.run(
        remote_ssh_command(["/usr/bin/shasum", "-a", "256", path]),
        cwd=REPO_ROOT,
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
    )
    if result.returncode != 0:
        fail(f"cannot hash MBP file {path}: {result.stderr[-1000:]}")
    fields = result.stdout.strip().split()
    if not fields:
        fail(f"MBP file hash output was empty for {path}")
    size = subprocess.run(
        remote_ssh_command(["/usr/bin/stat", "-f", "%z", path]),
        cwd=REPO_ROOT,
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
    )
    if size.returncode != 0:
        fail(f"cannot stat MBP file {path}: {size.stderr[-1000:]}")
    try:
        bytes_count = int(size.stdout.strip())
    except ValueError as exc:
        fail(f"invalid MBP size for {path}: {exc}")
    return {"path": path, "bytes": bytes_count, "sha256": fields[0]}


def freeze_input_refs(root: Path) -> dict[str, Any]:
    source = root_path(root, SOURCE_FULL_OUTPUT_REL) / "datasets"
    return {
        "sourceGenerationCompletion": root_artifact(root, "preflight/source-generation-completion.json"),
        "recoverySupervisor": root_artifact(root, "preflight/recovery-supervisor.json"),
        "trainAudit": root_artifact(root, "preflight/train-audit.json"),
        "validationAudit": root_artifact(root, "preflight/validation-audit.json"),
        "exclusionFreeze": root_artifact(root, "preflight/exclusion-freeze.json"),
        "exclusionAllowlist": root_artifact(root, "preflight/exclusion-allowlist.json"),
        "sourceFullGeneration": root_artifact(root, "source/full-generation.json"),
        "sourceCanonicalConfig": root_artifact(root, SOURCE_CONFIG_REL),
        "sourceTrainAggregate": root_artifact(root, f"{SOURCE_FULL_OUTPUT_REL}/datasets/train.bin"),
        "sourceTrainManifest": root_artifact(root, f"{SOURCE_FULL_OUTPUT_REL}/datasets/train.json"),
        "sourceValidationAggregate": root_artifact(root, f"{SOURCE_FULL_OUTPUT_REL}/datasets/validation.bin"),
        "sourceValidationManifest": root_artifact(root, f"{SOURCE_FULL_OUTPUT_REL}/datasets/validation.json"),
        "sourceTrainShardDirectory": str(source / "shards/train"),
        "sourceValidationShardDirectory": str(source / "shards/validation"),
    }


def write_data_freeze_blocked(root: Path, reason: str, details: dict[str, Any] | None = None, *, real_work: bool = False) -> int:
    receipt: dict[str, Any] = {
        "schema": SCHEMAS["dataFreeze"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "blocked",
        "phase": "data-freeze",
        "reason": reason,
        "launchesRealWork": real_work,
        "outcomesOpened": False,
        "trainingOrGpuAttempted": False,
    }
    if details:
        receipt.update(details)
    atomic_json(root_path(root, "preflight/data-freeze-gate.json"), receipt)
    print(json.dumps({
        "command": "freeze-data",
        "status": "blocked",
        "reason": reason,
        "realWorkLaunched": real_work,
    }, sort_keys=True), file=sys.stderr)
    return 2


def valid_published_freeze(root: Path) -> bool:
    freeze_path = root_path(root, "data/freeze.json")
    if not freeze_path.is_file():
        return False
    try:
        freeze = load_json(freeze_path)
    except Step12Error:
        return False
    if freeze.get("schema") != CORPUS_FREEZE_SCHEMA or freeze.get("status") != "frozen":
        return False
    validation = root_path(root, "data/independent-validation.json")
    selection = root_path(root, "data/selection.json")
    if not validation.is_file() or not selection.is_file():
        return False
    try:
        if load_json(validation).get("status") != "passed":
            return False
        if load_json(selection).get("status") != "passed":
            return False
    except Step12Error:
        return False
    artifacts = freeze.get("publishedArtifacts", {})
    if not isinstance(artifacts, dict):
        return False
    for relative, expected in artifacts.items():
        if not isinstance(expected, dict) or expected.get("status") != "observed":
            return False
        observed = root_artifact(root, relative)
        if observed.get("status") != "observed" or observed.get("sha256") != expected.get("sha256") or observed.get("bytes") != expected.get("bytes"):
            return False
    return True


def valid_published_openings(root: Path) -> bool:
    freeze_path = root_path(root, "openings/freeze.json")
    validation_path = root_path(root, "openings/independent-validation.json")
    if not freeze_path.is_file() or not validation_path.is_file():
        return False
    try:
        freeze = load_json(freeze_path)
        validation = load_json(validation_path)
    except Step12Error:
        return False
    if freeze.get("schema") != SCHEMAS["openingFreeze"] or freeze.get("status") != "frozen-before-training":
        return False
    if validation.get("schema") != SCHEMAS["openingValidation"] or validation.get("status") != "passed":
        return False
    if freeze.get("modelsBound") is not False or freeze.get("outcomesOpened") is not False:
        return False
    if freeze.get("pairingFixture", {}).get("status") != "passed":
        return False
    artifacts = freeze.get("publishedArtifacts")
    if not isinstance(artifacts, dict):
        return False
    for relative, expected in artifacts.items():
        if not isinstance(expected, dict):
            return False
        observed = root_artifact(root, relative)
        if observed.get("status") != "observed" or observed.get("bytes") != expected.get("bytes") or observed.get("sha256") != expected.get("sha256"):
            return False
    return True


def remote_artifact(local_path: Path, remote_path: str, *, coordinator_path: str | None = None) -> dict[str, Any]:
    if not local_path.is_file():
        fail(f"missing artifact for remote binding: {local_path}")
    value = {"path": remote_path, "bytes": local_path.stat().st_size, "sha256": digest(local_path)}
    if coordinator_path is not None:
        value["coordinatorPath"] = coordinator_path
    return value


def opening_spec(root: Path, allowlist: dict[str, Any], remote_attempt_root: str, build: dict[str, Any]) -> dict[str, Any]:
    inventory_refs = []
    for entry in allowlist.get("entries", []):
        family = entry.get("familyInventory", {})
        relative = str(family.get("path", ""))
        local = root_path(root, relative)
        if family.get("status") != "observed" or not local.is_file():
            fail(f"opening freeze family inventory is not observed: {relative}")
        inventory_refs.append({
            "id": entry["id"],
            **remote_artifact(local, f"{remote_attempt_root}/inputs/exclusion-families/{Path(relative).name}", coordinator_path=relative),
        })
    training_relative = "data/frozen-corpus-v1/training.bin"
    validation_relative = "data/frozen-corpus-v1/validation.bin"
    training = root_path(root, training_relative)
    validation = root_path(root, validation_relative)
    if not valid_published_freeze(root):
        fail("the published Step 12 corpus freeze is not valid")
    corpus_refs = [
        {
            "id": "step12-training",
            "role": "training",
            "records": 880000,
            **remote_artifact(training, f"{remote_attempt_root}/inputs/training.bin", coordinator_path=training_relative),
        },
        {
            "id": "step12-validation",
            "role": "validation",
            "records": 60000,
            **remote_artifact(validation, f"{remote_attempt_root}/inputs/validation.bin", coordinator_path=validation_relative),
        },
    ]
    canonical = repo_path(CANONICALIZATION_SOURCE)
    pairing = repo_path(OPENING_PAIRING_IMPLEMENTATION)
    documentation = repo_path(OPENING_PAIRING_DOCUMENTATION)
    return {
        "schema": SCHEMAS["openingSpec"],
        "status": "frozen-before-generation",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "host": REMOTE_HOST,
        "seed": SEEDS["developmentOpenings"],
        "matchSeed": SEEDS["match"],
        "acceptedPositions": 128,
        "randomPlies": 16,
        "maximumCandidateAttempts": 100000,
        "candidateIdPrefix": "step12-development-source-",
        "openingIdPrefix": "step12-development-",
        "rng": "rand-std-rng-0.9-lockfile-v1",
        "legalMoveOrder": "Board::generate_moves callback order; no sorting",
        "rejectionPrecedence": [
            "incomplete-trajectory",
            "invalid-terminal-endpoint",
            "historical-excluded-family",
            "new-training-family",
            "new-validation-family",
            "accepted-family-duplicate",
        ],
        "inputs": {
            "historicalFamilyInventories": inventory_refs,
            "newCorpora": corpus_refs,
            "canonicalizationSource": remote_artifact(canonical, f"{REMOTE_REPO}/{CANONICALIZATION_SOURCE}", coordinator_path=CANONICALIZATION_SOURCE),
        },
        "build": {
            "binary": build["binary"],
            "lockfile": build["lockfile"],
            "sources": build["sources"],
        },
        "pairing": {
            "transformation": "anhoku-rotate-180-and-swap-colors-v1",
            "implementation": remote_artifact(pairing, f"{REMOTE_REPO}/{OPENING_PAIRING_IMPLEMENTATION}", coordinator_path=OPENING_PAIRING_IMPLEMENTATION),
            "documentation": remote_artifact(documentation, f"{REMOTE_REPO}/{OPENING_PAIRING_DOCUMENTATION}", coordinator_path=OPENING_PAIRING_DOCUMENTATION),
            "fixture": {
                "baseSfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
                "aColor": {"base": "black", "transformed": "white"},
                "bColor": {"base": "white", "transformed": "black"},
            },
        },
        "modelsBound": False,
        "outcomesOpened": False,
        "protectedPaths": [PROTECTED_PATH],
        "recursiveDiscoveryUsed": False,
    }


def run_freeze_data(root: Path) -> int:
    if not root_path(root, "contract.json").is_file():
        fail("run prepare before freeze-data")
    allowlist = load_json(root_path(root, "preflight/exclusion-allowlist.json"))
    if allowlist.get("status") != "frozen":
        return write_data_freeze_blocked(root, "the exclusion allowlist is not frozen")
    completion = load_json(root_path(root, "preflight/source-generation-completion.json")) if root_path(root, "preflight/source-generation-completion.json").is_file() else {}
    if completion.get("status") != "completed":
        return write_data_freeze_blocked(root, "completed source-generation evidence is missing")
    host = load_json(root_path(root, HOST_CONNECTIVITY_REL))
    forecast = load_json(root_path(root, "preflight/parallel-forecast.json")) if root_path(root, "preflight/parallel-forecast.json").is_file() else {}
    if host.get("status") != "inventory-complete" or forecast.get("status") != "passed":
        return write_data_freeze_blocked(root, "the MBP inventory or measured source forecast is unresolved")
    auth = load_json(root_path(root, "authorization/cpu-generation.json"))
    if auth.get("granted") is not True:
        return write_data_freeze_blocked(root, "authorization/cpu-generation.json is not granted")
    if valid_published_freeze(root):
        freeze = load_json(root_path(root, "data/freeze.json"))
        atomic_json(root_path(root, "preflight/data-freeze-gate.json"), {
            "schema": SCHEMAS["dataFreeze"],
            "status": "passed",
            "phase": "data-freeze",
            "experimentId": EXPERIMENT_ID,
            "producer": freeze.get("producer"),
            "validator": freeze.get("validator"),
            "publishedCorpus": root_artifact(root, "data/freeze.json"),
            "launchesRealWork": False,
            "outcomesOpened": False,
            "trainingOrGpuAttempted": False,
        })
        unresolved = refresh_preflight(root, "data-freeze", "data-freeze-completed")
        status_doc = load_json(root_path(root, "status.json"))
        status_doc.update({
            "dataFreeze": root_artifact(root, "data/freeze.json"),
            "lastCompletedGate": "data-freeze-completed",
            "nextGate": "freeze-openings",
            "updatedAt": now(),
        })
        atomic_json(root_path(root, "status.json"), status_doc)
        write_manifest(root, allowlist, unresolved, str(load_json(root_path(root, "preflight/fixtures.json")).get("status", "not-run")))
        print(json.dumps({"command": "freeze-data", "status": "already-frozen", "unresolved": len(unresolved), "realWorkLaunched": False}, sort_keys=True))
        return 0
    if root_path(root, "data/freeze.json").exists() or root_path(root, "data/frozen-corpus-v1").exists():
        return write_data_freeze_blocked(root, "an existing Step 12 corpus publication conflicts with the required freeze receipt")

    # Refresh the phase binding before remote work so the exact inventory and
    # source paths are visible even if the remote build or producer fails.
    refresh_preflight(root, "data-freeze", "data-freeze-running")
    inventory_local = root_path(root, "preflight/exclusion-families")
    inventory_names = [Path(entry["familyInventory"]["path"]).name for entry in allowlist["entries"]]
    if sorted(p.name for p in inventory_local.glob("*.jsonl")) != sorted(inventory_names):
        return write_data_freeze_blocked(root, "the explicit exclusion inventory set is incomplete")

    sync_commands = []
    try:
        mkdir_result = subprocess.run(
            remote_ssh_command(["/bin/mkdir", "-p", REMOTE_INVENTORIES]),
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
            text=True,
            timeout=30,
        )
        sync_commands.append({"command": remote_ssh_command(["/bin/mkdir", "-p", REMOTE_INVENTORIES]), "returncode": mkdir_result.returncode})
        if mkdir_result.returncode != 0:
            return write_data_freeze_blocked(root, "could not prepare the MBP exclusion-inventory directory", {"remote": mkdir_result.stderr[-2000:]})
        for relative in ("haitaka_learn/src/main.rs", CORPUS_FREEZE_IMPLEMENTATION):
            command = remote_rsync_to(REPO_ROOT / relative, f"{REMOTE_REPO}/{relative}")
            result = subprocess.run(command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=120)
            sync_commands.append({"command": command, "returncode": result.returncode, "stderr": result.stderr[-1000:]})
            if result.returncode != 0:
                return write_data_freeze_blocked(root, f"could not sync {relative} to the MBP", {"sync": sync_commands})
        inventory_command = remote_rsync_to(inventory_local, f"{REMOTE_ROOT}/preflight/")
        inventory_result = subprocess.run(inventory_command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=120)
        sync_commands.append({"command": inventory_command, "returncode": inventory_result.returncode, "stderr": inventory_result.stderr[-1000:]})
        if inventory_result.returncode != 0:
            return write_data_freeze_blocked(root, "could not sync the explicit exclusion inventories to the MBP", {"sync": sync_commands})
        build_command = remote_ssh_command([f"cd {REMOTE_REPO} && {REMOTE_CARGO} build --release --features anhoku --bin haitaka_learn"])
        build_result = subprocess.run(build_command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=3600)
    except (OSError, subprocess.TimeoutExpired) as exc:
        return write_data_freeze_blocked(root, f"MBP build or transfer failed: {exc}", {"sync": sync_commands})
    build_receipt: dict[str, Any] = {
        "schema": "haitaka-anhoku-strength-first-step12-data-freeze-build-v1",
        "createdAt": now(),
        "status": "passed" if build_result.returncode == 0 else "failed",
        "host": REMOTE_HOST,
        "command": build_command,
        "returncode": build_result.returncode,
        "stdout": build_result.stdout[-20000:],
        "stderr": build_result.stderr[-20000:],
        "synced": sync_commands,
        "sourceImplementation": artifact(CORPUS_FREEZE_IMPLEMENTATION),
    }
    if build_result.returncode == 0:
        try:
            build_receipt["binary"] = remote_file_identity(REMOTE_BINARY)
            probe = subprocess.run(remote_ssh_command([REMOTE_BINARY, "--help"]), cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=30)
            build_receipt["binaryProbe"] = {"returncode": probe.returncode, "supportsProducer": "strength-first-step12-freeze-corpus" in probe.stdout, "supportsValidator": "strength-first-step12-validate-corpus" in probe.stdout}
            if probe.returncode != 0 or not build_receipt["binaryProbe"]["supportsProducer"] or not build_receipt["binaryProbe"]["supportsValidator"]:
                build_receipt["status"] = "failed"
                build_receipt["error"] = "rebuilt MBP binary does not expose both Step 12 corpus commands"
        except (OSError, subprocess.TimeoutExpired, Step12Error) as exc:
            build_receipt["status"] = "failed"
            build_receipt["error"] = str(exc)
    atomic_json(root_path(root, "preflight/data-freeze-build.json"), build_receipt)
    if build_receipt["status"] != "passed":
        return write_data_freeze_blocked(root, "the MBP native Step 12 freezer build did not pass", {"build": build_receipt})

    attempt_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    stage_rel = f"data/.staging-corpus-freeze-v1-{attempt_id}"
    stage_local = root_path(root, stage_rel)
    stage_remote = f"{REMOTE_ROOT}/{stage_rel}"
    producer_command = [
        "ssh", "-F", "/dev/null", "-o", "ConnectTimeout=10", "-o", "BatchMode=yes", "-o", "ConnectionAttempts=1",
        REMOTE_HOST, "caffeinate", "-i", "/usr/bin/time", "-l", REMOTE_BINARY, "strength-first-step12-freeze-corpus",
    ]
    for name in inventory_names:
        producer_command.extend(["--exclusion-inventory", f"{REMOTE_INVENTORIES}/{name}"])
    producer_command.extend([
        "--source-train", f"{REMOTE_SOURCE_OUTPUT}/datasets/train.bin",
        "--source-train-shards", f"{REMOTE_SOURCE_OUTPUT}/datasets/shards/train",
        "--source-validation", f"{REMOTE_SOURCE_OUTPUT}/datasets/validation.bin",
        "--source-validation-shards", f"{REMOTE_SOURCE_OUTPUT}/datasets/shards/validation",
        "--output-dir", stage_remote, "--train-records", "880000", "--validation-records", "60000",
        "--selection-seed", str(SEEDS["corpusSelection"]),
    ])
    producer_result = subprocess.run(producer_command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=7200)
    atomic_text(root_path(root, "preflight/data-freeze-producer.stdout"), producer_result.stdout)
    atomic_text(root_path(root, "preflight/data-freeze-producer.stderr"), producer_result.stderr)
    if producer_result.returncode != 0:
        return write_data_freeze_blocked(root, "the MBP native Step 12 corpus producer failed", {"producerCommand": producer_command, "returncode": producer_result.returncode, "resource": parse_mbp_time(producer_result.stderr)}, real_work=True)

    validator_command = [
        "ssh", "-F", "/dev/null", "-o", "ConnectTimeout=10", "-o", "BatchMode=yes", "-o", "ConnectionAttempts=1",
        REMOTE_HOST, "caffeinate", "-i", "/usr/bin/time", "-l", REMOTE_BINARY, "strength-first-step12-validate-corpus",
    ]
    for name in inventory_names:
        validator_command.extend(["--exclusion-inventory", f"{REMOTE_INVENTORIES}/{name}"])
    validator_command.extend([
        "--source-train", f"{REMOTE_SOURCE_OUTPUT}/datasets/train.bin",
        "--source-train-shards", f"{REMOTE_SOURCE_OUTPUT}/datasets/shards/train",
        "--source-validation", f"{REMOTE_SOURCE_OUTPUT}/datasets/validation.bin",
        "--source-validation-shards", f"{REMOTE_SOURCE_OUTPUT}/datasets/shards/validation",
        "--output-dir", stage_remote, "--report", f"{stage_remote}/independent-validation.json",
        "--train-records", "880000", "--validation-records", "60000", "--selection-seed", str(SEEDS["corpusSelection"]),
    ])
    validator_result = subprocess.run(validator_command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=7200)
    atomic_text(root_path(root, "preflight/data-freeze-validator.stdout"), validator_result.stdout)
    atomic_text(root_path(root, "preflight/data-freeze-validator.stderr"), validator_result.stderr)
    if validator_result.returncode != 0:
        return write_data_freeze_blocked(root, "the independent MBP Step 12 corpus validator failed", {"validatorCommand": validator_command, "returncode": validator_result.returncode, "resource": parse_mbp_time(validator_result.stderr)}, real_work=True)

    stage_local.parent.mkdir(parents=True, exist_ok=True)
    pull_command = remote_rsync_from(f"{stage_remote}/", stage_local)
    pull_result = subprocess.run(pull_command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=600)
    if pull_result.returncode != 0:
        return write_data_freeze_blocked(root, "the validated MBP corpus could not be transferred back", {"pullCommand": pull_command, "stderr": pull_result.stderr[-2000:]}, real_work=True)
    native_selection_path = stage_local / "selection.json"
    native_validation_path = stage_local / "independent-validation.json"
    if not native_selection_path.is_file() or not native_validation_path.is_file():
        return write_data_freeze_blocked(root, "the validated MBP stage is missing native selection or validation receipts", real_work=True)
    native_selection = load_json(native_selection_path)
    native_validation = load_json(native_validation_path)
    if native_selection.get("status") != "passed" or native_validation.get("status") != "passed":
        return write_data_freeze_blocked(root, "the MBP stage receipts are not passed", real_work=True)
    final_dir = root_path(root, "data/frozen-corpus-v1")
    if final_dir.exists():
        return write_data_freeze_blocked(root, "the final corpus directory appeared during publication", real_work=True)
    os.replace(stage_local, final_dir)

    published_selection = dict(native_selection)
    published_selection["publishedCorpusDirectory"] = "data/frozen-corpus-v1"
    published_selection["nativeReceipt"] = root_artifact(root, "data/frozen-corpus-v1/selection.json")
    atomic_json(root_path(root, "data/selection.json"), published_selection)
    published_validation = dict(native_validation)
    published_validation["publishedCorpusDirectory"] = "data/frozen-corpus-v1"
    published_validation["nativeReceipt"] = root_artifact(root, "data/frozen-corpus-v1/independent-validation.json")
    atomic_json(root_path(root, "data/independent-validation.json"), published_validation)

    published_relatives = [
        "data/selection.json", "data/independent-validation.json",
        "data/frozen-corpus-v1/selection.json", "data/frozen-corpus-v1/independent-validation.json",
        "data/frozen-corpus-v1/source-record-map.json", "data/frozen-corpus-v1/source-record-map-training.bin",
        "data/frozen-corpus-v1/source-record-map-validation.bin", "data/frozen-corpus-v1/training.bin",
        "data/frozen-corpus-v1/training.jsonl", "data/frozen-corpus-v1/training.json",
        "data/frozen-corpus-v1/validation.bin", "data/frozen-corpus-v1/validation.jsonl",
        "data/frozen-corpus-v1/validation.json",
    ]
    published_artifacts = {relative: root_artifact(root, relative) for relative in published_relatives}
    freeze = {
        "schema": CORPUS_FREEZE_SCHEMA,
        "status": "frozen",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "publishedCorpusDirectory": "data/frozen-corpus-v1",
        "publishedArtifacts": published_artifacts,
        "selection": root_artifact(root, "data/selection.json"),
        "independentValidation": root_artifact(root, "data/independent-validation.json"),
        "sourceRecordMap": root_artifact(root, "data/frozen-corpus-v1/source-record-map.json"),
        "upstream": freeze_input_refs(root),
        "exclusions": {
            "allowlist": root_artifact(root, "preflight/exclusion-allowlist.json"),
            "familyInventoryCount": len(allowlist["entries"]),
            "protectedPaths": [PROTECTED_PATH],
            "recursiveDiscoveryUsed": False,
        },
        "producer": {
            "host": REMOTE_HOST,
            "command": producer_command,
            "returncode": producer_result.returncode,
            "binary": build_receipt["binary"],
            "implementation": artifact(CORPUS_FREEZE_IMPLEMENTATION),
            "resource": parse_mbp_time(producer_result.stderr),
            "stdout": root_artifact(root, "preflight/data-freeze-producer.stdout"),
            "stderr": root_artifact(root, "preflight/data-freeze-producer.stderr"),
        },
        "validator": {
            "host": REMOTE_HOST,
            "command": validator_command,
            "returncode": validator_result.returncode,
            "binary": build_receipt["binary"],
            "implementation": artifact(CORPUS_FREEZE_IMPLEMENTATION),
            "resource": parse_mbp_time(validator_result.stderr),
            "stdout": root_artifact(root, "preflight/data-freeze-validator.stdout"),
            "stderr": root_artifact(root, "preflight/data-freeze-validator.stderr"),
        },
        "provenance": {
            "sourceShardAndRecordOffset": "verified-by-exact-assembly-permutation",
            "sourceGameId": None,
            "sourceGameAttribution": "per-game-boundaries-not-retained",
            "sourceGameSplitIsolation": "complete-disjoint-shard-game-and-opening-group-inventory",
        },
        "flags": {
            "confirmationAuthorized": False,
            "deploymentAuthorized": False,
            "outcomesOpened": False,
            "trainingOrGpuAttempted": False,
            "handcraftedRemainsProduction": True,
        },
    }
    atomic_json(root_path(root, "data/freeze.json"), freeze)
    atomic_json(root_path(root, "preflight/data-freeze-gate.json"), {
        "schema": SCHEMAS["dataFreeze"],
        "status": "passed",
        "phase": "data-freeze",
        "experimentId": EXPERIMENT_ID,
        "producer": freeze["producer"],
        "validator": freeze["validator"],
        "publishedCorpus": root_artifact(root, "data/freeze.json"),
        "launchesRealWork": True,
        "outcomesOpened": False,
        "trainingOrGpuAttempted": False,
    })
    unresolved = refresh_preflight(root, "data-freeze", "data-freeze-completed")
    status_doc = load_json(root_path(root, "status.json"))
    status_doc.update({
        "dataFreeze": root_artifact(root, "data/freeze.json"),
        "lastCompletedGate": "data-freeze-completed",
        "nextGate": "freeze-openings",
        "updatedAt": now(),
    })
    atomic_json(root_path(root, "status.json"), status_doc)
    write_manifest(root, allowlist, unresolved, str(load_json(root_path(root, "preflight/fixtures.json")).get("status", "not-run")))
    print(json.dumps({"command": "freeze-data", "status": "frozen", "unresolved": len(unresolved), "trainRecords": 880000, "validationRecords": 60000, "realWorkLaunched": True, "nextGate": "freeze-openings"}, sort_keys=True))
    return 0


def write_opening_blocked(
    root: Path,
    reason: str,
    details: dict[str, Any] | None = None,
    *,
    real_work: bool = False,
) -> int:
    receipt: dict[str, Any] = {
        "schema": SCHEMAS["openingGate"],
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "phase": "opening-generation",
        "status": "blocked",
        "reason": reason,
        "launchesRealWork": real_work,
        "outcomesOpened": False,
        "trainingOrGpuAttempted": False,
    }
    if details:
        receipt.update(details)
    atomic_json(root_path(root, "preflight/opening-freeze-gate.json"), receipt)
    print(json.dumps({
        "command": "freeze-openings",
        "status": "blocked",
        "reason": reason,
        "realWorkLaunched": real_work,
    }, sort_keys=True), file=sys.stderr)
    return 2


def opening_remote_ref(local: Path, remote: str, relative: str) -> dict[str, Any]:
    if not local.is_file():
        fail(f"missing opening input {local}")
    return {
        "path": remote,
        "bytes": local.stat().st_size,
        "sha256": digest(local),
        "coordinatorPath": relative,
    }


def remote_source_ref(relative: str) -> dict[str, Any]:
    local = repo_path(relative)
    remote = remote_file_identity(f"{REMOTE_REPO}/{relative}")
    expected = {"bytes": local.stat().st_size, "sha256": digest(local)}
    if remote.get("bytes") != expected["bytes"] or remote.get("sha256") != expected["sha256"]:
        fail(f"MBP source identity differs for {relative}")
    return {
        "path": f"{REMOTE_REPO}/{relative}",
        "bytes": remote["bytes"],
        "sha256": remote["sha256"],
        "coordinatorPath": relative,
    }


def verify_opening_stage(stage: Path) -> dict[str, Any]:
    required = [
        "candidates.jsonl",
        "development.tsv",
        "development.sfen",
        "generation.json",
        "exclusions.json",
        "schedule.json",
        "independent-validation.json",
        "freeze.json",
    ]
    for relative in required:
        if not root_path(stage, relative).is_file():
            fail(f"validated MBP opening stage is missing {relative}")
    for block in range(4):
        if not root_path(stage, f"blocks/block-{block:02}.sfen").is_file():
            fail(f"validated MBP opening stage is missing block-{block:02}.sfen")
    generation = load_json(root_path(stage, "generation.json"))
    validation = load_json(root_path(stage, "independent-validation.json"))
    freeze = load_json(root_path(stage, "freeze.json"))
    if generation.get("schema") != SCHEMAS["openingGeneration"] or generation.get("status") != "passed":
        fail("MBP opening generation receipt is not a pass")
    if validation.get("schema") != SCHEMAS["openingValidation"] or validation.get("status") != "passed":
        fail("MBP opening validation receipt is not a pass")
    if freeze.get("schema") != SCHEMAS["openingFreeze"] or freeze.get("validationCompleted") is not True:
        fail("MBP opening freeze stage marker is not validated")
    if freeze.get("pairingFixture", {}).get("status") != "passed":
        fail("MBP opening pairing fixture did not pass")
    if validation.get("acceptedPositions") != 128 or generation.get("acceptedPositions") != 128:
        fail("MBP opening stage has the wrong accepted quota")
    return {"generation": generation, "validation": validation, "freeze": freeze}


def run_freeze_openings(root: Path) -> int:
    if not root_path(root, "contract.json").is_file():
        fail("run prepare before freeze-openings")
    if valid_published_openings(root):
        unresolved = refresh_preflight(root, "opening-generation", "openings-frozen")
        status_doc = load_json(root_path(root, "status.json"))
        status_doc.update({
            "openingsFreeze": root_artifact(root, "openings/freeze.json"),
            "lastCompletedGate": "opening-freeze-completed",
            "nextGate": "training-gate",
            "updatedAt": now(),
        })
        atomic_json(root_path(root, "status.json"), status_doc)
        write_manifest(root, load_json(root_path(root, "preflight/exclusion-allowlist.json")), unresolved, str(load_json(root_path(root, "preflight/fixtures.json")).get("status", "not-run")))
        print(json.dumps({
            "command": "freeze-openings",
            "status": "already-frozen",
            "unresolved": len(unresolved),
            "realWorkLaunched": False,
        }, sort_keys=True))
        return 0
    if not valid_published_freeze(root):
        return write_opening_blocked(root, "the published Step 12 corpus freeze is missing or invalid")
    allowlist = load_json(root_path(root, "preflight/exclusion-allowlist.json"))
    if allowlist.get("status") != "frozen":
        return write_opening_blocked(root, "the exclusion allowlist is not frozen")
    completion = load_json(root_path(root, "preflight/source-generation-completion.json"))
    if completion.get("status") != "completed":
        return write_opening_blocked(root, "completed source-generation evidence is missing")
    host = load_json(root_path(root, HOST_CONNECTIVITY_REL))
    if host.get("status") != "inventory-complete":
        return write_opening_blocked(root, "the MBP host inventory is unresolved")
    fixtures = load_json(root_path(root, "preflight/fixtures.json"))
    if fixtures.get("status") != "passed-preparation-fixtures":
        return write_opening_blocked(root, "required preparation fixtures have not passed")
    auth = load_json(root_path(root, "authorization/cpu-generation.json"))
    if auth.get("granted") is not True:
        return write_opening_blocked(root, "authorization/cpu-generation.json is not granted")
    final_dir = root_path(root, "openings")
    final_dir.mkdir(parents=True, exist_ok=True)
    if any(path.name != ".gitkeep" for path in final_dir.iterdir()):
        return write_opening_blocked(root, "the openings publication directory already contains an unaccepted attempt or artifact")
    prior_attempt_archive = archive_prior_opening_attempt(root)

    attempt_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    remote_attempt_root = f"{REMOTE_OPENING_ROOT}/attempt-{attempt_id}"
    stage_remote = f"{remote_attempt_root}/stage"
    stage_rel = f"openings/.staging-{attempt_id}"
    stage_local = root_path(root, stage_rel)
    spec_rel = f"preflight/opening-spec-{attempt_id}.json"
    spec_local = root_path(root, spec_rel)
    if spec_local.exists():
        return write_opening_blocked(root, f"the unique opening spec path already exists: {spec_rel}")
    sync_commands: list[dict[str, Any]] = []
    try:
        mkdir_command = remote_ssh_command([
            "/bin/mkdir", "-p",
            f"{remote_attempt_root}/inputs/exclusion-families",
            f"{remote_attempt_root}/inputs",
        ])
        mkdir_result = subprocess.run(mkdir_command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=30)
        sync_commands.append({"command": mkdir_command, "returncode": mkdir_result.returncode, "stderr": mkdir_result.stderr[-1000:]})
        if mkdir_result.returncode != 0:
            return write_opening_blocked(root, "could not prepare the MBP opening-freeze attempt directory", {"sync": sync_commands})

        source_files = [
            "haitaka_learn/src/main.rs",
            OPENING_IMPLEMENTATION,
            OPENING_VALIDATION_IMPLEMENTATION,
            "haitaka_learn/src/step12_corpus_freeze.rs",
            CANONICALIZATION_SOURCE,
            OPENING_PAIRING_IMPLEMENTATION,
            "Cargo.lock",
            OPENING_PAIRING_DOCUMENTATION,
        ]
        for relative in source_files:
            command = remote_rsync_to(REPO_ROOT / relative, f"{REMOTE_REPO}/{relative}")
            result = subprocess.run(command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=180)
            sync_commands.append({"command": command, "returncode": result.returncode, "stderr": result.stderr[-1000:]})
            if result.returncode != 0:
                return write_opening_blocked(root, f"could not sync {relative} to the MBP", {"sync": sync_commands})

        inventory_local = root_path(root, "preflight/exclusion-families")
        inventory_command = remote_rsync_to(inventory_local, f"{remote_attempt_root}/inputs/")
        inventory_result = subprocess.run(inventory_command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=180)
        sync_commands.append({"command": inventory_command, "returncode": inventory_result.returncode, "stderr": inventory_result.stderr[-1000:]})
        if inventory_result.returncode != 0:
            return write_opening_blocked(root, "could not sync the frozen family inventories to the MBP", {"sync": sync_commands})

        for relative in ("data/frozen-corpus-v1/training.bin", "data/frozen-corpus-v1/validation.bin"):
            command = remote_rsync_to(root_path(root, relative), f"{remote_attempt_root}/inputs/")
            result = subprocess.run(command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=300)
            sync_commands.append({"command": command, "returncode": result.returncode, "stderr": result.stderr[-1000:]})
            if result.returncode != 0:
                return write_opening_blocked(root, f"could not sync {relative} to the MBP", {"sync": sync_commands})

        build_command = remote_ssh_command([
            f"cd {shlex.quote(REMOTE_REPO)} && {shlex.quote(REMOTE_CARGO)} build --locked --release --features anhoku --bin haitaka_learn"
        ])
        build_result = subprocess.run(build_command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=3600)
    except (OSError, subprocess.TimeoutExpired) as exc:
        return write_opening_blocked(root, f"MBP opening build or transfer failed: {exc}", {"sync": sync_commands})

    build_receipt: dict[str, Any] = {
        "schema": "haitaka-anhoku-strength-first-step12-opening-build-v1",
        "createdAt": now(),
        "status": "passed" if build_result.returncode == 0 else "failed",
        "host": REMOTE_HOST,
        "command": build_command,
        "returncode": build_result.returncode,
        "stdout": build_result.stdout[-20000:],
        "stderr": build_result.stderr[-20000:],
        "synced": sync_commands,
    }
    if build_result.returncode == 0:
        try:
            binary = remote_file_identity(REMOTE_BINARY)
            probe = subprocess.run(
                remote_ssh_command([shlex.join([REMOTE_BINARY, "--help"])]),
                cwd=REPO_ROOT,
                check=False,
                capture_output=True,
                text=True,
                timeout=30,
            )
            build_receipt["binary"] = binary
            build_receipt["binaryProbe"] = {
                "returncode": probe.returncode,
                "supportsProducer": "strength-first-step12-generate-openings" in probe.stdout,
                "supportsValidator": "strength-first-step12-validate-openings" in probe.stdout,
            }
            if probe.returncode != 0 or not build_receipt["binaryProbe"]["supportsProducer"] or not build_receipt["binaryProbe"]["supportsValidator"]:
                build_receipt["status"] = "failed"
                build_receipt["error"] = "MBP binary does not expose both Step 12 opening commands"
        except (OSError, subprocess.TimeoutExpired, Step12Error) as exc:
            build_receipt["status"] = "failed"
            build_receipt["error"] = str(exc)
    atomic_json(root_path(root, "preflight/opening-build.json"), build_receipt)
    if build_receipt["status"] != "passed":
        return write_opening_blocked(root, "the MBP Step 12 opening build did not pass", {"build": build_receipt})

    # Confirm every explicitly synced source and the lockfile before writing
    # the spec.  The remote spec then verifies the same identities again.
    source_refs = []
    for relative in source_files:
        source_refs.append(remote_source_ref(relative))
    input_root = f"{remote_attempt_root}/inputs"
    inventory_refs = []
    for entry in allowlist.get("entries", []):
        relative = str(entry["familyInventory"]["path"])
        local = root_path(root, relative)
        inventory_refs.append({
            "id": entry["id"],
            **opening_remote_ref(local, f"{input_root}/exclusion-families/{Path(relative).name}", relative),
        })
    spec = opening_spec(root, allowlist, remote_attempt_root, {
        "binary": build_receipt["binary"],
        "lockfile": next(item for item in source_refs if item["coordinatorPath"] == "Cargo.lock"),
        "sources": source_refs,
    })
    # opening_spec constructs the same inventory paths from the allowlist; keep
    # this explicit check in the coordinator receipt as well.
    spec["inputs"]["historicalFamilyInventories"] = inventory_refs
    spec["inputs"]["newCorpora"][0]["path"] = f"{input_root}/training.bin"
    spec["inputs"]["newCorpora"][1]["path"] = f"{input_root}/validation.bin"
    spec["build"]["binary"] = build_receipt["binary"]
    atomic_json(spec_local, spec)
    spec_remote = f"{remote_attempt_root}/opening-spec.json"
    spec_command = remote_rsync_to(spec_local, spec_remote)
    spec_result = subprocess.run(spec_command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=120)
    sync_commands.append({"command": spec_command, "returncode": spec_result.returncode, "stderr": spec_result.stderr[-1000:]})
    if spec_result.returncode != 0:
        return write_opening_blocked(root, "could not sync the frozen opening spec to the MBP", {"sync": sync_commands})

    producer_args = [
        "/usr/bin/caffeinate", "-i", "/usr/bin/time", "-l", REMOTE_BINARY,
        "strength-first-step12-generate-openings", "--spec", spec_remote,
        "--output-dir", stage_remote,
    ]
    producer_command = remote_ssh_command([shlex.join(producer_args)])
    producer_result = subprocess.run(producer_command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=3600)
    atomic_text(root_path(root, "preflight/opening-producer.stdout"), producer_result.stdout)
    atomic_text(root_path(root, "preflight/opening-producer.stderr"), producer_result.stderr)
    if producer_result.returncode != 0:
        return write_opening_blocked(root, "the MBP Step 12 opening producer failed", {
            "producerCommand": producer_command,
            "remoteAttemptRoot": remote_attempt_root,
            "returncode": producer_result.returncode,
            "resource": parse_mbp_time(producer_result.stderr),
        }, real_work=True)

    validator_args = [
        "/usr/bin/caffeinate", "-i", "/usr/bin/time", "-l", REMOTE_BINARY,
        "strength-first-step12-validate-openings", "--spec", spec_remote,
        "--output-dir", stage_remote, "--report", f"{stage_remote}/independent-validation.json",
    ]
    validator_command = remote_ssh_command([shlex.join(validator_args)])
    validator_result = subprocess.run(validator_command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=3600)
    atomic_text(root_path(root, "preflight/opening-validator.stdout"), validator_result.stdout)
    atomic_text(root_path(root, "preflight/opening-validator.stderr"), validator_result.stderr)
    if validator_result.returncode != 0:
        return write_opening_blocked(root, "the independent MBP Step 12 opening validator failed", {
            "validatorCommand": validator_command,
            "remoteAttemptRoot": remote_attempt_root,
            "returncode": validator_result.returncode,
            "resource": parse_mbp_time(validator_result.stderr),
        }, real_work=True)

    stage_local.parent.mkdir(parents=True, exist_ok=True)
    pull_command = remote_rsync_from(f"{stage_remote}/", stage_local)
    pull_result = subprocess.run(pull_command, cwd=REPO_ROOT, check=False, capture_output=True, text=True, timeout=600)
    if pull_result.returncode != 0:
        return write_opening_blocked(root, "the validated MBP opening stage could not be transferred back", {
            "pullCommand": pull_command,
            "remoteAttemptRoot": remote_attempt_root,
            "stderr": pull_result.stderr[-2000:],
        }, real_work=True)
    try:
        stage_receipts = verify_opening_stage(stage_local)
    except (Step12Error, OSError, UnicodeError, json.JSONDecodeError) as exc:
        return write_opening_blocked(root, f"the pulled MBP opening stage failed local inventory verification: {exc}", {
            "remoteAttemptRoot": remote_attempt_root,
        }, real_work=True)

    # The native stage freeze is preserved separately.  The coordinator writes
    # the publication freeze marker only after every other file is installed.
    producer_freeze = root_path(stage_local, "freeze.json")
    producer_freeze_copy = root_path(stage_local, "producer-freeze.json")
    os.replace(producer_freeze, producer_freeze_copy)
    for child in list(stage_local.iterdir()):
        destination = final_dir / child.name
        if destination.exists():
            return write_opening_blocked(root, f"opening publication conflict at {destination}", {"remoteAttemptRoot": remote_attempt_root}, real_work=True)
        os.replace(child, destination)
    os.rmdir(stage_local)

    published_relatives = [
        "openings/candidates.jsonl",
        "openings/development.tsv",
        "openings/development.sfen",
        "openings/generation.json",
        "openings/exclusions.json",
        "openings/schedule.json",
        "openings/independent-validation.json",
        "openings/producer-freeze.json",
    ]
    for block in range(4):
        published_relatives.append(f"openings/blocks/block-{block:02}.sfen")
    published_artifacts = {relative: root_artifact(root, relative) for relative in published_relatives}
    amendment = {
        "schema": "haitaka-anhoku-strength-first-step12-opening-implementation-amendment-v1",
        "status": "passed",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "workerRunbook": artifact("plans/anhoku-nnue-strength-first-step12-opening-freeze-worker-runbook.md"),
        "implementation": artifact(OPENING_IMPLEMENTATION),
        "independentValidator": artifact(OPENING_VALIDATION_IMPLEMENTATION),
        "pairingImplementation": artifact(OPENING_PAIRING_IMPLEMENTATION),
        "pairingDocumentation": artifact(OPENING_PAIRING_DOCUMENTATION),
        "build": build_receipt,
        "remoteAttemptRoot": remote_attempt_root,
        "pairingFinding": "historical native CLI generic suite command reused the same row; the opening freeze binds explicit transformed second-leg SFENs and colors under the required contract",
        "pairingResolution": stage_receipts["freeze"]["pairingFixture"],
        "resource": {
            "producer": parse_mbp_time(producer_result.stderr),
            "validator": parse_mbp_time(validator_result.stderr),
            "host": REMOTE_HOST,
        },
        "trainingOrGpuAttempted": False,
        "outcomesOpened": False,
    }
    atomic_json(root_path(root, "preflight/opening-implementation-amendment.json"), amendment)
    freeze = {
        "schema": SCHEMAS["openingFreeze"],
        "status": "frozen-before-training",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "spec": root_artifact(root, spec_rel),
        "build": root_artifact(root, "preflight/opening-build.json"),
        "generation": root_artifact(root, "openings/generation.json"),
        "exclusions": root_artifact(root, "openings/exclusions.json"),
        "independentValidation": root_artifact(root, "openings/independent-validation.json"),
        "schedule": root_artifact(root, "openings/schedule.json"),
        "producerFreeze": root_artifact(root, "openings/producer-freeze.json"),
        "publishedArtifacts": published_artifacts,
        "pairingFixture": stage_receipts["freeze"]["pairingFixture"],
        "modelsBound": False,
        "outcomesOpened": False,
        "trainingOrGpuAttempted": False,
        "remoteAttemptRoot": remote_attempt_root,
        "flags": {
            "confirmationAuthorized": False,
            "deploymentAuthorized": False,
            "handcraftedRemainsProduction": True,
        },
        "writtenLast": True,
    }
    atomic_json(root_path(root, "openings/freeze.json"), freeze)
    atomic_json(root_path(root, "preflight/opening-freeze-gate.json"), {
        "schema": SCHEMAS["openingGate"],
        "status": "passed",
        "phase": "opening-generation",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "producer": {
            "command": producer_command,
            "returncode": producer_result.returncode,
            "resource": parse_mbp_time(producer_result.stderr),
            "stdout": root_artifact(root, "preflight/opening-producer.stdout"),
            "stderr": root_artifact(root, "preflight/opening-producer.stderr"),
        },
        "validator": {
            "command": validator_command,
            "returncode": validator_result.returncode,
            "resource": parse_mbp_time(validator_result.stderr),
            "stdout": root_artifact(root, "preflight/opening-validator.stdout"),
            "stderr": root_artifact(root, "preflight/opening-validator.stderr"),
        },
        "freeze": root_artifact(root, "openings/freeze.json"),
        "remoteAttemptRoot": remote_attempt_root,
        "launchesRealWork": True,
        "outcomesOpened": False,
        "trainingOrGpuAttempted": False,
    })
    unresolved = refresh_preflight(root, "opening-generation", "openings-frozen")
    status_doc = load_json(root_path(root, "status.json"))
    status_doc.update({
        "openingsFreeze": root_artifact(root, "openings/freeze.json"),
        "lastCompletedGate": "opening-freeze-completed",
        "nextGate": "training-gate",
        "updatedAt": now(),
    })
    atomic_json(root_path(root, "status.json"), status_doc)
    write_manifest(root, allowlist, unresolved, str(fixtures.get("status", "not-run")))
    print(json.dumps({
        "command": "freeze-openings",
        "status": "frozen",
        "unresolved": len(unresolved),
        "acceptedPositions": 128,
        "attemptedCandidates": stage_receipts["generation"].get("attemptedCandidates"),
        "realWorkLaunched": True,
        "nextGate": "training-gate",
    }, sort_keys=True))
    return 0


def blocked_phase(root: Path, command: str) -> int:
    if not root_path(root, "contract.json").is_file():
        fail("run prepare before invoking a phase command")
    all_unresolved = load_json(root_path(root, "preflight/unresolved.json"))["entries"]
    phase_for_command = {
        "freeze-data": "data-freeze",
        "freeze-openings": "opening-generation",
        "train": "training",
        "select-and-qualify": "qualification",
        "run-matches": "matches",
        "closeout": "closeout",
    }
    phase = phase_for_command.get(command)
    unresolved = phase_unresolved(all_unresolved, phase) if phase else all_unresolved
    payload = {
        "command": command,
        "status": "blocked-preparation",
        "reason": "Step 12 adapter phase is not implemented or its launch gate is unresolved",
        "unresolved": len(unresolved),
        "unresolvedIds": [entry["id"] for entry in unresolved],
        "launchesRealWork": False,
        "trainingOrGpuAttempted": False,
        "outcomesOpened": False,
    }
    print(json.dumps(payload, sort_keys=True), file=sys.stderr)
    return 2


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=[
        "prepare",
        "fixtures",
        "freeze-exclusions",
        "host-preflight",
        "generate-data",
        "freeze-data",
        "freeze-openings",
        "prepare-training",
        "training-gate",
        "train",
        "select-and-qualify",
        "match-gate",
        "run-matches",
        "closeout",
    ])
    parser.add_argument("--root", type=Path, default=REPO_ROOT / ROOT_NAME)
    parser.add_argument("--resume", action="store_true", help="allow deterministic preparation receipts to be rebuilt")
    args = parser.parse_args()
    root = args.root.expanduser().resolve()
    try:
        if args.command == "prepare":
            return run_prepare(root, args.resume)
        if args.command == "fixtures":
            return run_fixtures(root)
        if args.command == "freeze-exclusions":
            return run_freeze_exclusions(root)
        if args.command == "host-preflight":
            return run_host_preflight(root)
        if args.command in {"training-gate", "match-gate"}:
            return run_gate(root, args.command)
        if args.command == "train":
            return run_train(root, args.resume)
        if args.command == "generate-data":
            return run_generate_data(root)
        if args.command == "freeze-data":
            return run_freeze_data(root)
        if args.command == "freeze-openings":
            return run_freeze_openings(root)
        if args.command == "prepare-training":
            return run_prepare_training(root)
        if args.command == "select-and-qualify":
            return run_select_and_qualify(root)
        if args.command == "run-matches":
            return run_matches(root, args.resume)
        if args.command == "closeout":
            return run_closeout(root)
        return blocked_phase(root, args.command)
    except (Step12Error, OSError, UnicodeError, json.JSONDecodeError, KeyError, TypeError) as exc:
        print(f"Step 12 blocked: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
