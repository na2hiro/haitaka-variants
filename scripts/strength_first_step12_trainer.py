#!/usr/bin/env python3
"""Step 12 training contract, fixture, authorization and selection adapter.

The module is deliberately standard-library-only at import time.  In
particular, importing it cannot initialize PyTorch, CUDA, a trainer context,
or a model.  The CPU fixture is isolated in
``strength_first_step12_trainer_fixture.py`` and the production backend is
loaded only after both explicit execution ledgers have passed.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[1]
EXPERIMENT_ID = "strength-first-step12-independent-replication-20260916-v1"
TRAINER_REVISION = "61666d9e3653e4df9881b14c23f8fdcc4bf7779b"
PROTECTED_TOKEN = "step9-independent-confirmation-20260913-abandoned-1000ms-partial-unopened"
PROTECTED_PATH = f"out/anhoku-strength-first/{PROTECTED_TOKEN}"

CONFIG_SCHEMA = "haitaka-anhoku-strength-first-step12-training-config-v1"
INPUTS_SCHEMA = "haitaka-anhoku-strength-first-step12-training-inputs-v1"
PACKAGE_SCHEMA = "haitaka-anhoku-strength-first-step12-training-package-v1"
HOST_BINDING_SCHEMA = "haitaka-anhoku-strength-first-step12-training-host-binding-v1"
INPUT_VALIDATION_SCHEMA = "haitaka-anhoku-strength-first-step12-training-input-validation-v1"
CPU_FIXTURE_SUMMARY_SCHEMA = "haitaka-anhoku-strength-first-step12-training-fixtures-cpu-v1"
CUDA_FIXTURE_SUMMARY_SCHEMA = "haitaka-anhoku-strength-first-step12-training-fixtures-cuda-v1"
TRAINING_GATE_SCHEMA = "haitaka-anhoku-strength-first-step12-training-gate-v1"
SELECTION_INPUT_SCHEMA = "haitaka-anhoku-strength-first-step12-selection-input-v1"
SELECTION_SCHEMA = "haitaka-anhoku-strength-first-step12-selection-v1"
ADAPTER_UNBLOCK_SCHEMA = "haitaka-anhoku-strength-first-step12-training-adapter-unblock-v1"
STATIC_FIXTURE_SCHEMA = "haitaka-anhoku-strength-first-step12-training-static-fixtures-v1"

CANDIDATE_UPDATES = [128, 256, 384, 512, 640, 768, 896, 1024]
CONTROL_UPDATE = 2688
SCHEDULE_HORIZON = 4096
SELECTION_TOLERANCE = 1e-12

MBP_HOST = "192.168.0.12"
MBP_REMOTE_ROOT = "/Users/na2hiro/anhoku-strength-first-step12-20260916-v1"
MBP_REMOTE_REPO = f"{MBP_REMOTE_ROOT}/repo"

ANCHORS = {
    "contract.json": (2531, "79e06af4a965f7591ebbd49aec2a17e5bff4dbba952f142675579bc31ac4f486"),
    "data/freeze.json": (22447, "23e5ebb3b823c0fe448d42115fb47c2385d6ecb61678c112af30327790454f64"),
    "data/independent-validation.json": (12234, "ee4c5b2ca209710a8f6738383b45c7981a6d080af4a0658edf64e0772f815fba"),
    "data/frozen-corpus-v1/training.bin": (63360000, "de716d7e4ce11a2cacd0b9e7cde412f246a7ea9c07cab704a2ab28ee2edd1aa6"),
    "data/frozen-corpus-v1/validation.bin": (4320000, "313aa823e3c4802f3672fff326c5bc5333937e463eb04be36d50f777d0af51b2"),
    "data/frozen-corpus-v1/training.jsonl": (306666545, "760ad99d53ceeb7e7a2e5ae3ffee03d6cdc6aaa1adfc48ae839153a106801072"),
    "data/frozen-corpus-v1/validation.jsonl": (20691906, "b4148a5e66e518e15e026e90d856b8084743aee34981e1a299e6ff0400d42001"),
    "openings/freeze.json": (5744, "8fddb2c6b860730461ccd1f3ed5bbdab5111b7e3922505fcdb4a69b5eef6a8aa"),
    "openings/development.tsv": (12895, "83e1f6e744228e87e0a95dd48266940096fc00b6a8454f8030e369d2036875ad"),
    "openings/development.sfen": (9813, "434cd1aff90ff8456227cc1141138e8a796228ce8f022e31cad7d3f6de3c565c"),
    "openings/schedule.json": (497903, "07fa07ad39c2d5ec0218d9c965e8abdcf83b58e7c5106923b8c0284cce8e7ad1"),
    "openings/independent-validation.json": (50323, "cb29a4ff3eee7bde4985458914166c3a125bdde4f85823c653dbd33fb41f7b45"),
    "preflight/exclusion-allowlist.json": (95978, "8a9930bcd5c3bcf2df8283fa53084e0c7d4d9fc64f5ae85c2ed2d68e3f393454"),
}


class Step12TrainingError(RuntimeError):
    """Expected, sanitized contract or gate failure."""


def fail(message: str) -> None:
    raise Step12TrainingError(message)


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def canonical_json(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
    except OSError as exc:
        fail(f"cannot hash {path}: {exc}")
    return digest.hexdigest()


def atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_bytes(json.dumps(value, indent=2, sort_keys=True).encode("utf-8") + b"\n")
    os.replace(temporary, path)


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        fail(f"cannot load JSON {path}: {exc}")
    if not isinstance(value, dict):
        fail(f"JSON root is not an object: {path}")
    return value


def protected(value: str) -> None:
    if PROTECTED_TOKEN in value:
        fail(f"protected historical path is forbidden: {PROTECTED_TOKEN}")


def root_from_config(config_path: Path) -> Path:
    config = config_path.resolve()
    if config.name != "config.json" or config.parent.name != "training":
        fail("config must be ROOT/training/config.json")
    root = config.parent.parent
    protected(str(root))
    return root


def root_file(root: Path, relative: str) -> Path:
    protected(str(root))
    protected(relative)
    path = (root / relative).resolve()
    try:
        path.relative_to(root.resolve())
    except ValueError:
        fail(f"path escapes training root: {relative}")
    protected(str(path))
    return path


def repo_file(relative: str) -> Path:
    protected(relative)
    path = (REPO_ROOT / relative).resolve()
    try:
        path.relative_to(REPO_ROOT.resolve())
    except ValueError:
        fail(f"repository path escapes repository root: {relative}")
    protected(str(path))
    return path


def observed_artifact(path: Path, *, relative: str | None = None) -> dict[str, Any]:
    if path.is_symlink():
        fail(f"unexpected symlink at explicitly bound artifact: {path}")
    if not path.is_file():
        fail(f"missing explicitly bound artifact: {path}")
    return {
        "path": relative if relative is not None else str(path),
        "bytes": path.stat().st_size,
        "sha256": sha256(path),
    }


def root_artifact(root: Path, relative: str) -> dict[str, Any]:
    return observed_artifact(root_file(root, relative), relative=relative)


def repo_artifact(relative: str) -> dict[str, Any]:
    return observed_artifact(repo_file(relative))


def mbp_remote_root(relative: str) -> str:
    protected(relative)
    return f"{MBP_REMOTE_ROOT}/{relative}"


def mbp_remote_repo(relative: str) -> str:
    protected(relative)
    return f"{MBP_REMOTE_REPO}/{relative}"


def verify_artifact(item: dict[str, Any], root: Path, where: str, *, allow_relative: bool = True) -> Path:
    if not isinstance(item, dict) or set(item) != {"path", "bytes", "sha256"}:
        fail(f"{where} must be an exact path/bytes/sha256 artifact")
    value = item["path"]
    if not isinstance(value, str) or not value:
        fail(f"{where}.path must be a nonempty string")
    path = Path(value)
    if not path.is_absolute():
        if not allow_relative:
            fail(f"{where}.path must be absolute")
        path = root_file(root, value)
    else:
        protected(str(path))
        path = path.resolve()
    if path.is_symlink() or not path.is_file():
        fail(f"{where} is missing or is an unexpected symlink: {path}")
    if isinstance(item["bytes"], bool) or not isinstance(item["bytes"], int) or item["bytes"] < 0:
        fail(f"{where}.bytes is invalid")
    if not isinstance(item["sha256"], str) or len(item["sha256"]) != 64:
        fail(f"{where}.sha256 is invalid")
    if path.stat().st_size != item["bytes"]:
        fail(f"{where}.bytes differs from the observed file")
    if sha256(path) != item["sha256"]:
        fail(f"{where}.sha256 differs from the observed file")
    return path


def exact(value: Any, expected: Any, where: str) -> None:
    if value != expected:
        fail(f"{where} is {value!r}; required {expected!r}")


def exact_keys(value: Any, expected: set[str], where: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{where} must be an object")
    actual = set(value)
    if actual != expected:
        fail(f"{where} keys differ: missing={sorted(expected - actual)}, extra={sorted(actual - expected)}")
    return value


def finite(value: Any, where: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(float(value)):
        fail(f"{where} must be finite")
    return float(value)


def flags() -> dict[str, bool]:
    return {
        "confirmationAuthorized": False,
        "deploymentAuthorized": False,
        "outcomesOpened": False,
        "handcraftedRemainsProduction": True,
    }


def source_record(path: Path, relative: str) -> dict[str, Any]:
    return root_artifact(path.parent.parent.parent.parent, relative)


def published_artifacts(root: Path, freeze_relative: str, where: str) -> dict[str, dict[str, Any]]:
    receipt = load_json(root_file(root, freeze_relative))
    if receipt.get("experimentId") != EXPERIMENT_ID:
        fail(f"{where} experiment identity differs")
    if receipt.get("status") not in {"frozen", "frozen-before-training"}:
        fail(f"{where} is not frozen")
    mapping = receipt.get("publishedArtifacts")
    if not isinstance(mapping, dict) or not mapping:
        fail(f"{where}.publishedArtifacts is empty or absent")
    verified: dict[str, dict[str, Any]] = {}
    for key, item in sorted(mapping.items()):
        if key != item.get("path"):
            fail(f"{where}.publishedArtifacts key/path mismatch: {key}")
        if not isinstance(key, str) or Path(key).is_absolute() or ".." in Path(key).parts:
            fail(f"{where}.publishedArtifacts contains an escaped path: {key}")
        observed = root_artifact(root, key)
        if observed != {"path": key, "bytes": item.get("bytes"), "sha256": item.get("sha256")}:
            fail(f"{where}.publishedArtifacts entry differs from the observed file: {key}")
        verified[key] = observed
    return verified


def verify_frozen_inputs(root: Path) -> dict[str, Any]:
    """Verify the explicit Step 12 input graph without filesystem discovery."""

    checks: list[dict[str, Any]] = []

    def check(name: str, fn: Any) -> Any:
        try:
            value = fn()
        except Step12TrainingError as exc:
            checks.append({"name": name, "status": "failed", "message": str(exc)})
            raise
        checks.append({"name": name, "status": "passed"})
        return value

    anchor_refs: dict[str, dict[str, Any]] = {}
    for relative, (size, expected_hash) in ANCHORS.items():
        item = check(f"anchor:{relative}", lambda relative=relative: root_artifact(root, relative))
        exact(item["bytes"], size, f"anchor:{relative}.bytes")
        exact(item["sha256"], expected_hash, f"anchor:{relative}.sha256")
        anchor_refs[relative] = item

    contract = load_json(root_file(root, "contract.json"))
    exact(contract.get("experimentId"), EXPERIMENT_ID, "contract.experimentId")
    exact(contract.get("flags"), {
        "confirmationAuthorized": False,
        "deploymentAuthorized": False,
        "handcraftedRemainsProduction": True,
    }, "contract.flags")

    data_freeze = load_json(root_file(root, "data/freeze.json"))
    opening_freeze = load_json(root_file(root, "openings/freeze.json"))
    exact(data_freeze.get("schema"), "haitaka-anhoku-strength-first-step12-corpus-freeze-v1", "data/freeze.schema")
    exact(opening_freeze.get("schema"), "haitaka-anhoku-strength-first-step12-opening-freeze-v1", "openings/freeze.schema")
    exact(data_freeze.get("experimentId"), EXPERIMENT_ID, "data/freeze.experimentId")
    exact(opening_freeze.get("experimentId"), EXPERIMENT_ID, "openings/freeze.experimentId")
    data_published = check("data-freeze:publishedArtifacts", lambda: published_artifacts(root, "data/freeze.json", "data/freeze"))
    opening_published = check("opening-freeze:publishedArtifacts", lambda: published_artifacts(root, "openings/freeze.json", "openings/freeze"))

    data_validation = load_json(root_file(root, "data/independent-validation.json"))
    opening_validation = load_json(root_file(root, "openings/independent-validation.json"))
    exact(data_validation.get("status"), "passed", "data/independent-validation.status")
    exact(opening_validation.get("status"), "passed", "openings/independent-validation.status")
    exact(opening_validation.get("outcomesOpened"), False, "openings/independent-validation.outcomesOpened")
    exact(opening_validation.get("trainingOrGpuAttempted"), False, "openings/independent-validation.trainingOrGpuAttempted")

    data_root = root_file(root, "data/frozen-corpus-v1")
    train_manifest = load_json(data_root / "training.json")
    validation_manifest = load_json(data_root / "validation.json")
    for name, manifest, records, split in (
        ("training", train_manifest, 880000, "training"),
        ("validation", validation_manifest, 60000, "validation"),
    ):
        exact(manifest.get("dataset"), split, f"{name}.manifest.dataset")
        exact(manifest.get("records"), records, f"{name}.manifest.records")
        exact(manifest.get("entryBytes"), 72, f"{name}.manifest.entryBytes")
        exact(manifest.get("sourceGameId"), None, f"{name}.manifest.sourceGameId")
        exact(manifest.get("sourceGameAttribution"), "per-game-boundaries-not-retained", f"{name}.manifest.sourceGameAttribution")
        binary = data_root / f"{name}.bin"
        sidecar = data_root / f"{name}.jsonl"
        exact(binary.stat().st_size, records * 72, f"{name}.binary.bytes")
        if not sidecar.is_file() or sidecar.is_symlink():
            fail(f"{name}.sidecar is missing or symlinked")

    roles = {
        "contract": anchor_refs["contract.json"],
        "dataFreeze": anchor_refs["data/freeze.json"],
        "dataIndependentValidation": anchor_refs["data/independent-validation.json"],
        "trainingManifest": root_artifact(root, "data/frozen-corpus-v1/training.json"),
        "trainingBinary": root_artifact(root, "data/frozen-corpus-v1/training.bin"),
        "trainingSidecar": root_artifact(root, "data/frozen-corpus-v1/training.jsonl"),
        "validationManifest": root_artifact(root, "data/frozen-corpus-v1/validation.json"),
        "validationBinary": root_artifact(root, "data/frozen-corpus-v1/validation.bin"),
        "validationSidecar": root_artifact(root, "data/frozen-corpus-v1/validation.jsonl"),
        "openingFreeze": anchor_refs["openings/freeze.json"],
        "openingValidation": anchor_refs["openings/independent-validation.json"],
        "openingSchedule": anchor_refs["openings/schedule.json"],
        "exclusionAllowlist": anchor_refs["preflight/exclusion-allowlist.json"],
    }
    return {
        "schema": INPUTS_SCHEMA,
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "root": str(root),
        "roles": roles,
        "dataFreezePublishedArtifacts": data_published,
        "openingFreezePublishedArtifacts": opening_published,
        "directAnchors": anchor_refs,
        "validation": {
            "manifestDirected": True,
            "recursiveFilesystemDiscoveryUsed": False,
            "unexpectedSymlinksRejected": True,
            "protectedPathTraversed": False,
            "trainingRecords": 880000,
            "validationRecords": 60000,
            "entryBytes": 72,
            "sourceGameAttributionPreservedAsNull": True,
        },
        "excludedFromTrainingAndSelection": [
            "openings/development.tsv",
            "openings/development.sfen",
            "openings/schedule.json",
            "openings/independent-validation.json",
            "openings/freeze.json",
        ],
        "flags": flags(),
    }


def input_validation_receipt(root: Path, inventory: dict[str, Any], *, status: str, failures: list[str] | None = None) -> dict[str, Any]:
    return {
        "schema": INPUT_VALIDATION_SCHEMA,
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": status,
        "inventory": {"path": "training/inputs.json", "bytes": None, "sha256": None},
        "checks": {
            "anchorCount": len(ANCHORS),
            "dataFreezePublishedArtifactCount": len(inventory.get("dataFreezePublishedArtifacts", {})),
            "openingFreezePublishedArtifactCount": len(inventory.get("openingFreezePublishedArtifacts", {})),
            "manifestDirected": True,
            "recursiveFilesystemDiscoveryUsed": False,
            "protectedPathTraversed": False,
        },
        "failures": failures or [],
        "flags": flags(),
    }


def training_data_artifact(root: Path, name: str) -> dict[str, Any]:
    return observed_artifact(root_file(root, f"data/frozen-corpus-v1/{name}"))


def build_config(root: Path) -> dict[str, Any]:
    environment_path = root_file(root, "preflight/gpu-training-environment.json")
    if not environment_path.is_file():
        environment_path = root_file(root, "preflight/mbp-training-environment.json")
    environment = load_json(environment_path) if environment_path.is_file() else {
        "schema": "haitaka-anhoku-strength-first-step12-mbp-training-environment-v1",
        "status": "pending-mbp-training-environment",
        "host": "192.168.0.12",
        "architecture": "arm64",
        "interpreter": "/Users/na2hiro/engine/variant-nnue-pytorch/.venv-step12/bin/python",
        "torch": None,
        "numpy": None,
        "cudaAvailable": None,
        "note": "Install/probe the bound MBP environment before any production grant; CPU preparation remains allowed.",
    }
    external_candidates = []
    if isinstance(environment.get("externalCheckout"), str):
        external_candidates.append(Path(environment["externalCheckout"]))
    external_candidates.append((REPO_ROOT / "../engine/variant-nnue-pytorch").resolve())
    external = next((candidate.resolve() for candidate in external_candidates if candidate.is_dir()), None)
    if external is None:
        fail(f"pinned trainer checkout is missing; checked {external_candidates}")
    protected(str(external))
    try:
        revision = subprocess.check_output(["git", "-C", str(external), "rev-parse", "HEAD"], text=True).strip()
    except (OSError, subprocess.CalledProcessError) as exc:
        fail(f"cannot inspect pinned trainer checkout: {exc}")
    exact(revision, TRAINER_REVISION, "pinned trainer revision")

    return {
        "schema": CONFIG_SCHEMA,
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "prepared-awaiting-training-authorization",
        "authorization": {
            "trainingLedger": {"path": str(root_file(root, "authorization/training.json")), "grantedAtPreparation": False},
            "gpuRentalLedger": {"path": str(root_file(root, "authorization/gpu-rental.json")), "grantedAtPreparation": False},
            "separateGrantsRequired": True,
            "productionTrainingAttempted": False,
            "gpuAttempted": False,
        },
        "upstream": {
            "contract": root_artifact(root, "contract.json"),
            "dataFreeze": root_artifact(root, "data/freeze.json"),
            "dataIndependentValidation": root_artifact(root, "data/independent-validation.json"),
            "openingFreeze": root_artifact(root, "openings/freeze.json"),
            "openingIndependentValidation": root_artifact(root, "openings/independent-validation.json"),
        },
        "trainer": {
            "runner": repo_artifact("scripts/strength_first_step12_trainer.py"),
            "fixture": repo_artifact("scripts/strength_first_step12_trainer_fixture.py"),
            "backend": repo_artifact("scripts/strength_first_step12_trainer_backend.py"),
            "contextBuilder": repo_artifact("scripts/strength_first_step12_context.py"),
            "cudaAdapter": repo_artifact("scripts/strength_first_step12_cuda_backward.py"),
            "cudaKernel": repo_artifact("scripts/strength_first_step12_cuda_backward.cu"),
            "cudaReference": repo_artifact("scripts/strength_first_step12_cuda_backward_reference.py"),
            "cudaTests": repo_artifact("scripts/strength_first_step12_cuda_backward_tests.py"),
            "cudaRuntime": repo_artifact("scripts/strength_first_step12_cuda_runtime.py"),
            "historicalHelpers": [repo_artifact("scripts/strength_first_trainer.py"), repo_artifact("scripts/r5_trainer.py")],
            "pinnedCheckout": {"path": str(external), "revision": TRAINER_REVISION},
            "patches": [repo_artifact("trainer-patches/variant-nnue-pytorch-phase7.1.patch"), repo_artifact("trainer-patches/variant-nnue-pytorch-r5.patch")],
            "evaluateOverlay": repo_artifact("trainer-patches/evaluate.py"),
            "overlays": [
                repo_artifact("haitaka_learn/trainer_overlay/feature_set.py"),
                repo_artifact("haitaka_learn/trainer_overlay/features.py"),
                repo_artifact("haitaka_learn/trainer_overlay/donor_features.py"),
                repo_artifact("haitaka_learn/trainer_overlay/training_data_loader.cpp"),
            ],
            "patchExecutable": environment.get("patchExecutable"),
            "historicalEntrypointsForbidden": [
                "strength_first_trainer.validate_config",
                "strength_first_trainer.run_training",
                "strength_first_trainer.run_real_cpu_fixture",
                "strength_first_step10_trainer_backend.legacy_view",
                "strength_first_step10_trainer_backend.patch_base_for_step10",
            ],
        },
        "recipe": {
            "seed": 2026091605,
            "initialization": "fresh-scratch-no-inherited-state",
            "independence": {"historicalA": 2026091002, "historicalB": 2026091108, "historicalStep10": 2026091414, "fixture": 20260906},
            "featureFamily": "HalfKAv2^+DonorSingleEff",
            "width": {"l1": 256, "l2": 16, "l3": 32},
            "outputScale": 1,
            "data": {
                "training": {"records": 880000, "entryBytes": 72, "manifest": training_data_artifact(root, "training.json"), "binary": training_data_artifact(root, "training.bin"), "sidecar": training_data_artifact(root, "training.jsonl")},
                "validation": {"records": 60000, "entryBytes": 72, "manifest": training_data_artifact(root, "validation.json"), "binary": training_data_artifact(root, "validation.bin"), "sidecar": training_data_artifact(root, "validation.jsonl")},
            },
            "loader": {"batchSize": 16384, "workers": 1, "cyclic": False, "filtered": False, "randomFenSkipping": 0, "finiteFinalPartialBatch": True, "trainingOrder": "pcg64dxsm-seed-epoch-record-permutation-v1", "validationOrder": "fixed-source-order-once-per-evaluation-v1"},
            "objective": {"kind": "robust-clipped-huber-v1", "teacherOnly": True, "resultLossEnabled": False, "lambda": 1.0, "targetScoreClip": 12000.0, "normalizedScoreScale": 12000.0, "huberDeltaNormalized": 0.1, "modelOutputToRuntimeScore": 600.0},
            "optimizer": {"kind": "Ranger", "betas": [0.9, 0.999], "epsilon": 1e-7, "weightDecay": 0.0, "lookaheadAlpha": 0.5, "lookaheadK": 6, "nSmaThreshold": 5, "gradientCentralizationLocation": False, "gradientCentralizationEnabled": False, "gradientCentralizationConvOnly": False, "parameterGroupNames": ["feature-transformer", "l1-factor-weight", "l1-weight", "l1-bias", "l2-weight", "l2-bias", "output-weight", "output-bias"], "parameterGroupLearningRateMultipliers": [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.1, 0.1], "weightProjection": "pinned-model-clip-before-and-after-update-v1"},
            "schedule": {"kind": "linear-warmup-then-cosine-by-optimizer-update-v1", "baseLearningRate": 0.0015, "minimumLearningRate": 0.00005, "warmupUpdates": 64, "horizonUpdates": SCHEDULE_HORIZON, "executionStopUpdate": CONTROL_UPDATE},
            "evaluation": {"everyUpdates": 128, "initialValidation": True, "validationRecordsPerEvaluation": 60000, "batchSize": 16384, "repeatWithinEvaluation": False, "expectedEvaluations": 22},
            "checkpoints": {"candidateUpdates": CANDIDATE_UPDATES, "controlUpdate": CONTROL_UPDATE, "retainAllCandidateAndControl": True, "atomicLastState": True, "portableFullPrecision": True},
            "diagnostics": {"everyUpdates": 128, "finiteChecks": True, "gradientMagnitude": True, "updateMagnitude": True, "scoreRange": True, "phaseAndDonor": True, "featureCoverage": True, "timing": True},
            "realCpuFixture": {"device": "cpu", "dataOrigin": "frozen-training-prefix", "sourceOffset": 0, "records": 9, "entryBytes": 72, "batchSize": 5, "finiteBatchSizesPerEpoch": [5, 4], "width": {"l1": 16, "l2": 8, "l3": 8}, "seed": 20260906, "maximumUpdates": 12, "interruptAfterUpdates": 1, "schedule": {"kind": "linear-warmup-then-cosine-by-optimizer-update-v1", "baseLearningRate": 0.0015, "minimumLearningRate": 0.00005, "warmupUpdates": 2, "maximumUpdates": 12}, "productionTrainingAttempted": False, "gpuAttempted": False},
            "cudaDeterminism": {"profile": "ordered-segment-v1", "adapterSchema": "haitaka-anhoku-strength-first-step12-deterministic-cuda-backward-v1", "sourceReduction": "pinned-forward-with-step12-owned-ordered-binary32-backward", "compileOptions": ["--std=c++14", "--fmad=false", "--ftz=false"], "threadsPerBlock": 128, "sort": "cupy.ndarray.argsort-unique-int64-feature-then-ordinal", "exactResumeRequired": True},
        },
        "retention": {"candidateUpdates": CANDIDATE_UPDATES, "controlUpdate": CONTROL_UPDATE, "allValidationEvaluations": True, "lastStateIncludes": ["model", "optimizer", "lookahead", "schedule", "pythonRng", "numpyRng", "torchRng", "permutation", "cursor", "examplesSeen", "validationHistory", "retainedArtifacts", "resourceLedger", "configHash", "inputHash", "implementationHash"]},
        "environment": environment,
        "output": {"trainingDirectory": str(root_file(root, "training/run")), "checkpointDirectory": str(root_file(root, "checkpoints")), "exportDirectory": str(root_file(root, "exports")), "selectionInput": str(root_file(root, "training/selection-input.json")), "selectionOutput": str(root_file(root, "training/selection.json"))},
        "flags": flags(),
    }


def validate_config(config: dict[str, Any], root: Path, *, verify_inputs: bool) -> None:
    exact(config.get("schema"), CONFIG_SCHEMA, "config.schema")
    exact(config.get("experimentId"), EXPERIMENT_ID, "config.experimentId")
    if config.get("status") not in {"prepared-awaiting-training-authorization", "training-authorized", "training-complete-awaiting-qualification"}:
        fail("config.status is not a recognized Step 12 training state")
    recipe = config.get("recipe")
    if not isinstance(recipe, dict):
        fail("config.recipe is missing")
    exact(recipe.get("seed"), 2026091605, "recipe.seed")
    exact(recipe.get("featureFamily"), "HalfKAv2^+DonorSingleEff", "recipe.featureFamily")
    exact(recipe.get("width"), {"l1": 256, "l2": 16, "l3": 32}, "recipe.width")
    exact(recipe.get("outputScale"), 1, "recipe.outputScale")
    exact(recipe.get("data", {}).get("training", {}).get("records"), 880000, "recipe.data.training.records")
    exact(recipe.get("data", {}).get("validation", {}).get("records"), 60000, "recipe.data.validation.records")
    for split in ("training", "validation"):
        data = recipe["data"][split]
        exact(data.get("entryBytes"), 72, f"recipe.data.{split}.entryBytes")
        for name in ("manifest", "binary", "sidecar"):
            verify_artifact(data[name], root, f"recipe.data.{split}.{name}", allow_relative=False)
    exact(recipe.get("loader"), {"batchSize": 16384, "workers": 1, "cyclic": False, "filtered": False, "randomFenSkipping": 0, "finiteFinalPartialBatch": True, "trainingOrder": "pcg64dxsm-seed-epoch-record-permutation-v1", "validationOrder": "fixed-source-order-once-per-evaluation-v1"}, "recipe.loader")
    exact(recipe.get("objective"), {"kind": "robust-clipped-huber-v1", "teacherOnly": True, "resultLossEnabled": False, "lambda": 1.0, "targetScoreClip": 12000.0, "normalizedScoreScale": 12000.0, "huberDeltaNormalized": 0.1, "modelOutputToRuntimeScore": 600.0}, "recipe.objective")
    exact(recipe.get("schedule"), {"kind": "linear-warmup-then-cosine-by-optimizer-update-v1", "baseLearningRate": 0.0015, "minimumLearningRate": 0.00005, "warmupUpdates": 64, "horizonUpdates": SCHEDULE_HORIZON, "executionStopUpdate": CONTROL_UPDATE}, "recipe.schedule")
    exact(recipe.get("evaluation"), {"everyUpdates": 128, "initialValidation": True, "validationRecordsPerEvaluation": 60000, "batchSize": 16384, "repeatWithinEvaluation": False, "expectedEvaluations": 22}, "recipe.evaluation")
    exact(recipe.get("checkpoints"), {"candidateUpdates": CANDIDATE_UPDATES, "controlUpdate": CONTROL_UPDATE, "retainAllCandidateAndControl": True, "atomicLastState": True, "portableFullPrecision": True}, "recipe.checkpoints")
    fixture = recipe.get("realCpuFixture", {})
    exact(fixture.get("device"), "cpu", "recipe.realCpuFixture.device")
    exact(fixture.get("dataOrigin"), "frozen-training-prefix", "recipe.realCpuFixture.dataOrigin")
    exact(fixture.get("sourceOffset"), 0, "recipe.realCpuFixture.sourceOffset")
    exact(fixture.get("records"), 9, "recipe.realCpuFixture.records")
    exact(fixture.get("entryBytes"), 72, "recipe.realCpuFixture.entryBytes")
    exact(fixture.get("batchSize"), 5, "recipe.realCpuFixture.batchSize")
    exact(fixture.get("finiteBatchSizesPerEpoch"), [5, 4], "recipe.realCpuFixture.finiteBatchSizesPerEpoch")
    exact(fixture.get("width"), {"l1": 16, "l2": 8, "l3": 8}, "recipe.realCpuFixture.width")
    exact(fixture.get("seed"), 20260906, "recipe.realCpuFixture.seed")
    exact(fixture.get("maximumUpdates"), 12, "recipe.realCpuFixture.maximumUpdates")
    exact(fixture.get("interruptAfterUpdates"), 1, "recipe.realCpuFixture.interruptAfterUpdates")
    exact(fixture.get("productionTrainingAttempted"), False, "recipe.realCpuFixture.productionTrainingAttempted")
    exact(fixture.get("gpuAttempted"), False, "recipe.realCpuFixture.gpuAttempted")
    exact(recipe.get("cudaDeterminism"), {"profile": "ordered-segment-v1", "adapterSchema": "haitaka-anhoku-strength-first-step12-deterministic-cuda-backward-v1", "sourceReduction": "pinned-forward-with-step12-owned-ordered-binary32-backward", "compileOptions": ["--std=c++14", "--fmad=false", "--ftz=false"], "threadsPerBlock": 128, "sort": "cupy.ndarray.argsort-unique-int64-feature-then-ordinal", "exactResumeRequired": True}, "recipe.cudaDeterminism")
    exact(config.get("flags"), flags(), "config.flags")
    trainer = config.get("trainer", {})
    for name in ("runner", "fixture", "backend", "contextBuilder", "cudaAdapter", "cudaKernel", "cudaReference", "cudaTests", "cudaRuntime", "evaluateOverlay"):
        verify_artifact(trainer.get(name), root, f"trainer.{name}", allow_relative=False)
    for name, items in (("historicalHelpers", trainer.get("historicalHelpers")), ("patches", trainer.get("patches")), ("overlays", trainer.get("overlays"))):
        if not isinstance(items, list) or not items:
            fail(f"trainer.{name} is empty")
        for index, item in enumerate(items):
            verify_artifact(item, root, f"trainer.{name}[{index}]", allow_relative=False)
    exact(trainer.get("pinnedCheckout", {}).get("revision"), TRAINER_REVISION, "trainer.pinnedCheckout.revision")
    patch_binding = trainer.get("patchExecutable")
    if not isinstance(patch_binding, dict) or not isinstance(patch_binding.get("path"), str) or not Path(patch_binding["path"]).is_absolute() or not isinstance(patch_binding.get("bytes"), int) or not isinstance(patch_binding.get("sha256"), str) or len(patch_binding["sha256"]) != 64:
        fail("trainer.patchExecutable is not an absolute hash-bound executable binding")
    if verify_inputs:
        verify_frozen_inputs(root)
        inputs_path = root_file(root, "training/inputs.json")
        if not inputs_path.is_file():
            fail("training/inputs.json has not been prepared")
        inputs = load_json(inputs_path)
        exact(inputs.get("schema"), INPUTS_SCHEMA, "training/inputs.schema")
        exact(inputs.get("experimentId"), EXPERIMENT_ID, "training/inputs.experimentId")
        for name, item in inputs.get("roles", {}).items():
            verify_artifact(item, root, f"training/inputs.roles.{name}", allow_relative=True)


def prepare_training(root: Path) -> dict[str, Any]:
    protected(str(root))
    if not root_file(root, "openings/freeze.json").is_file():
        fail("the opening freeze must be complete before training preparation")
    root_file(root, "training").mkdir(parents=True, exist_ok=True)
    inventory: dict[str, Any]
    failures: list[str] = []
    try:
        inventory = verify_frozen_inputs(root)
        atomic_json(root_file(root, "training/inputs.json"), inventory)
        validation = input_validation_receipt(root, inventory, status="passed")
    except Step12TrainingError as exc:
        failures.append(str(exc))
        inventory = {"schema": INPUTS_SCHEMA, "createdAt": now(), "experimentId": EXPERIMENT_ID, "root": str(root), "roles": {}, "flags": flags()}
        atomic_json(root_file(root, "training/inputs.json"), inventory)
        validation = input_validation_receipt(root, inventory, status="failed", failures=failures)
    inputs_path = root_file(root, "training/inputs.json")
    validation["inventory"] = {"path": "training/inputs.json", "bytes": inputs_path.stat().st_size, "sha256": sha256(inputs_path)}
    atomic_json(root_file(root, "preflight/training-input-validation.json"), validation)
    if failures:
        fail(f"training input validation failed: {failures[0]}")

    config_path = root_file(root, "training/config.json")
    if config_path.is_file():
        config = load_json(config_path)
        environment_receipt = root_file(root, "preflight/mbp-training-environment.json")
        observed_environment = load_json(environment_receipt) if environment_receipt.is_file() else {}
        expected_checkout = observed_environment.get("externalCheckout")
        expected_interpreter = observed_environment.get("interpreter")
        config_environment = config.get("environment", {})
        config_trainer = config.get("trainer", {})
        config_checkout = config_trainer.get("pinnedCheckout", {}).get("path")
        needs_environment_rebind = (
            observed_environment.get("status") == "observed"
            and (
                config_environment.get("status") != "observed"
                or (isinstance(expected_checkout, str) and config_checkout != expected_checkout)
                or (isinstance(expected_interpreter, str) and config_environment.get("interpreter") != expected_interpreter)
            )
        )
        if needs_environment_rebind:
            # The initial package was deliberately marked pending until the
            # MBP probe completed.  Bind the observed interpreter/toolchain
            # now.  A previous probe may have recorded a recovery-volume
            # alias that is not the checkout actually bound to this worker.
            config = build_config(root)
            atomic_json(config_path, config)
        try:
            validate_config(config, root, verify_inputs=True)
        except Step12TrainingError:
            # A previous interrupted preparation may have written the
            # config before the final absolute-path validation.  Rebuild only
            # that recognizably local, schema-bound preparation artifact; an
            # otherwise valid existing config remains immutable.
            if config.get("schema") != CONFIG_SCHEMA or config.get("experimentId") != EXPERIMENT_ID:
                raise
            config = build_config(root)
            atomic_json(config_path, config)
            validate_config(config, root, verify_inputs=True)
        else:
            # Preparation artifacts are mutable only before a grant.  Refresh
            # the host-bound implementation section when the worker, shared
            # context builder, patch executable or overlay inventory changed;
            # an already authorized config remains immutable.
            expected = build_config(root)
            if config.get("trainer") != expected.get("trainer") or config.get("environment") != expected.get("environment"):
                if config.get("status") != "prepared-awaiting-training-authorization":
                    fail("existing training/config.json is bound to a non-preparation state")
                config = expected
                atomic_json(config_path, config)
                validate_config(config, root, verify_inputs=True)
    else:
        config = build_config(root)
        atomic_json(config_path, config)
        validate_config(config, root, verify_inputs=True)

    package_path = root_file(root, "training/package.json")
    running_on_mbp_worker = str(root.resolve()) == MBP_REMOTE_ROOT and str(REPO_ROOT.resolve()) == MBP_REMOTE_REPO
    coordinator_binding = {
        "host": "worker-generation" if running_on_mbp_worker else "coordinator",
        "root": str(root.resolve()),
        "repo": str(REPO_ROOT),
        "cwd": str(REPO_ROOT),
        "interpreter": sys.executable,
        "config": str(config_path),
        "inputs": str(inputs_path),
        "trainingAuthorization": str(root_file(root, "authorization/training.json")),
        "gpuRentalAuthorization": str(root_file(root, "authorization/gpu-rental.json")),
    }
    coordinator_binding["cpuFixtureCommand"] = [
        sys.executable,
        str(repo_file("scripts/strength_first_step12_trainer.py")),
        "--config",
        str(config_path),
        "fixtures",
        "--device",
        "cpu",
        "--output",
        str(root_file(root, "preflight/training-fixtures-cpu.json")),
    ]
    coordinator_binding["productionCommand"] = [
        sys.executable,
        str(repo_file("scripts/strength_first_step12_trainer.py")),
        "--config",
        str(config_path),
        "train",
        "--authorization-ledger",
        str(root_file(root, "authorization/training.json")),
    ]
    host_binding = {
        "host": MBP_HOST,
        "architecture": "arm64",
        "root": MBP_REMOTE_ROOT,
        "repo": MBP_REMOTE_REPO,
        "cwd": MBP_REMOTE_REPO,
        "interpreter": config["environment"].get("interpreter"),
        "config": mbp_remote_root("training/config.json"),
        "inputs": mbp_remote_root("training/inputs.json"),
        "trainingAuthorization": mbp_remote_root("authorization/training.json"),
        "gpuRentalAuthorization": mbp_remote_root("authorization/gpu-rental.json"),
        "environmentAllowlist": ["CUDA_VISIBLE_DEVICES", "PYTHONHASHSEED", "CUBLAS_WORKSPACE_CONFIG"],
    }
    host_binding["cpuFixtureCommand"] = [
        config["environment"].get("interpreter"),
        mbp_remote_repo("scripts/strength_first_step12_trainer.py"),
        "--config",
        mbp_remote_root("training/config.json"),
        "fixtures",
        "--device",
        "cpu",
        "--output",
        mbp_remote_root("preflight/training-fixtures-cpu.json"),
    ]
    host_binding["productionCommand"] = [
        config["environment"].get("interpreter"),
        mbp_remote_repo("scripts/strength_first_step12_trainer.py"),
        "--config",
        mbp_remote_root("training/config.json"),
        "train",
        "--authorization-ledger",
        mbp_remote_root("authorization/training.json"),
    ]
    package = {
        "schema": PACKAGE_SCHEMA,
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "prepared-awaiting-mbp-environment" if config["environment"].get("status") != "observed" else "prepared-awaiting-training-authorization",
        "config": root_artifact(root, "training/config.json"),
        "inputs": root_artifact(root, "training/inputs.json"),
        "implementation": {
            "runner": config["trainer"]["runner"],
            "fixture": config["trainer"]["fixture"],
            "backend": config["trainer"]["backend"],
            "contextBuilder": config["trainer"]["contextBuilder"],
            "cudaAdapter": config["trainer"]["cudaAdapter"],
            "cudaKernel": config["trainer"]["cudaKernel"],
            "cudaReference": config["trainer"]["cudaReference"],
            "cudaTests": config["trainer"]["cudaTests"],
            "cudaRuntime": config["trainer"]["cudaRuntime"],
            "historicalHelpers": config["trainer"]["historicalHelpers"],
            "pinnedCheckout": config["trainer"]["pinnedCheckout"],
            "patches": config["trainer"]["patches"],
            "evaluateOverlay": config["trainer"]["evaluateOverlay"],
            "overlays": config["trainer"]["overlays"],
            "patchExecutable": config["trainer"]["patchExecutable"],
        },
        "helperCallAllowlist": [
            "strength_first_trainer.epoch_permutation",
            "strength_first_trainer.materialize_epoch",
            "strength_first_trainer.finite_batches",
            "strength_first_trainer.fixture_recipe",
            "strength_first_trainer.import_cpu_fixture_trainer",
            "strength_first_trainer.install_cpu_fixture_cupy_module",
            "strength_first_trainer.install_cpu_feature_transformer",
            "strength_first_trainer.install_cpu_batch_tensors",
            "strength_first_trainer.fixture_evaluate",
            "strength_first_trainer.fixture_train",
            "strength_first_trainer.nested_equal",
            "strength_first_trainer.loss_tensors",
            "strength_first_trainer.configured_optimizer",
            "strength_first_trainer.save_portable_model",
            "strength_first_trainer.export_checkpoint",
            "r5_trainer.write_variant",
            "r5_trainer.import_trainer",
            "strength_first_step12_context.prepare_context",
            "strength_first_step12_cuda_runtime.install",
            "strength_first_step12_cuda_backward.install",
        ],
        "historicalEntrypointsForbidden": config["trainer"]["historicalEntrypointsForbidden"],
        "coordinatorBinding": coordinator_binding,
        "hostBinding": host_binding,
        "bindingPolicy": {
            "schema": HOST_BINDING_SCHEMA,
            "coordinatorPathsAreNotWorkerPaths": not running_on_mbp_worker,
            "generationContext": "worker" if running_on_mbp_worker else "coordinator",
            "workerRoot": MBP_REMOTE_ROOT,
            "workerRepo": MBP_REMOTE_REPO,
            "remoteReverificationRequiredBeforeGrant": True,
        },
        "environment": config["environment"],
        "determinism": {"torchDeterministicAlgorithms": True, "torchThreads": 1, "trainingOrder": config["recipe"]["loader"]["trainingOrder"], "validationOrder": config["recipe"]["loader"]["validationOrder"]},
        "resourceLimits": {"productionRuns": 1, "exactStateRecoveries": 1, "gpuInstanceHours": 4.0, "gpuUsd": 5.0, "newStorageGiB": 64, "freeReserveGiB": 16},
        "flags": flags(),
    }
    if package_path.is_file():
        prior = load_json(package_path)
        if (
            prior.get("config", {}).get("sha256") != package["config"]["sha256"]
            or prior.get("inputs", {}).get("sha256") != package["inputs"]["sha256"]
            or prior.get("hostBinding") != package["hostBinding"]
            or prior.get("coordinatorBinding") != package["coordinatorBinding"]
            or prior.get("bindingPolicy") != package["bindingPolicy"]
        ):
            if prior.get("status") not in {"prepared-awaiting-mbp-environment", "prepared-awaiting-training-authorization"}:
                fail("existing training/package.json is bound to different config or inputs")
            atomic_json(package_path, package)
        else:
            package = prior
    else:
        atomic_json(package_path, package)

    host_binding_checks = [
        {
            "name": "worker-root",
            "status": "passed" if package["hostBinding"].get("root") == MBP_REMOTE_ROOT else "failed",
            "expected": MBP_REMOTE_ROOT,
            "observed": package["hostBinding"].get("root"),
        },
        {
            "name": "worker-repository",
            "status": "passed" if package["hostBinding"].get("repo") == MBP_REMOTE_REPO else "failed",
            "expected": MBP_REMOTE_REPO,
            "observed": package["hostBinding"].get("repo"),
        },
        {
            "name": "worker-cwd",
            "status": "passed" if package["hostBinding"].get("cwd") == MBP_REMOTE_REPO else "failed",
            "expected": MBP_REMOTE_REPO,
            "observed": package["hostBinding"].get("cwd"),
        },
        {
            "name": "worker-command-paths",
            "status": "passed"
            if all(
                isinstance(value, str)
                and (
                    "/" not in value
                    or value.startswith(MBP_REMOTE_ROOT)
                    or value.startswith(MBP_REMOTE_REPO)
                    or value == config["environment"].get("interpreter")
                )
                for command_name in ("cpuFixtureCommand", "productionCommand")
                for value in package["hostBinding"].get(command_name, [])
            )
            else "failed",
            "expected": "all worker command paths resolve under the MBP root/repo or bound interpreter",
            "observed": package["hostBinding"].get("productionCommand"),
        },
        {
            "name": "coordinator-worker-path-separation",
            "status": "passed"
            if (
                (running_on_mbp_worker and package.get("bindingPolicy", {}).get("generationContext") == "worker")
                or (
                    not running_on_mbp_worker
                    and package.get("coordinatorBinding", {}).get("root") != package["hostBinding"].get("root")
                    and package.get("coordinatorBinding", {}).get("repo") != package["hostBinding"].get("repo")
                )
            )
            else "failed",
            "expected": "distinct coordinator/worker roots, or explicit worker-local generation context",
            "observed": {
                "coordinatorRoot": package.get("coordinatorBinding", {}).get("root"),
                "workerRoot": package["hostBinding"].get("root"),
            },
        },
    ]
    binding_status = "passed" if all(item["status"] == "passed" for item in host_binding_checks) else "failed"
    atomic_json(
        root_file(root, "preflight/training-host-binding-validation.json"),
        {
            "schema": HOST_BINDING_SCHEMA,
            "createdAt": now(),
            "experimentId": EXPERIMENT_ID,
            "status": binding_status,
            "host": package["hostBinding"].get("host"),
            "coordinatorBinding": package.get("coordinatorBinding"),
            "workerBinding": package.get("hostBinding"),
            "checks": host_binding_checks,
            "remoteVerification": {
                "status": "pending",
                "requiredBeforeGrant": True,
                "reason": "verify the staged MBP paths and hashes after the package is transferred",
            },
            "flags": flags(),
        },
    )

    for filename, scope in (("training.json", "one scratch training run through update 2688"), ("gpu-rental.json", "Vast.ai rental for Step 12 training")):
        auth_path = root_file(root, f"authorization/{filename}")
        auth = load_json(auth_path)
        auth["boundPreparation"] = {
            "package": {"path": "training/package.json", "bytes": package_path.stat().st_size, "sha256": sha256(package_path)},
            "config": {"path": "training/config.json", "bytes": config_path.stat().st_size, "sha256": sha256(config_path)},
            "inputs": {"path": "training/inputs.json", "bytes": inputs_path.stat().st_size, "sha256": sha256(inputs_path)},
        }
        auth["scope"] = scope
        auth["trainingOrGpuAttempted"] = False
        auth["outcomesOpened"] = False
        atomic_json(auth_path, auth)

    unblock = {
        "schema": ADAPTER_UNBLOCK_SCHEMA,
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "passed-preparation-package-awaiting-mbp-environment" if config["environment"].get("status") != "observed" else "passed-preparation-package",
        "worker": {"host": "192.168.0.12", "architecture": "arm64"},
        "beforeHistoricalSources": {"strengthFirstTrainer": config["trainer"]["historicalHelpers"][0], "r5Trainer": config["trainer"]["historicalHelpers"][1]},
        "step12Owned": {"runner": config["trainer"]["runner"], "fixture": config["trainer"]["fixture"], "backend": config["trainer"]["backend"]},
        "helperCallAllowlist": package["helperCallAllowlist"],
        "qualificationLimits": ["production training loop", "CUDA fixture/RNG", "nine-checkpoint retention", "runtime parity", "matches"],
        "remainingGrants": ["authorization/training.json", "authorization/gpu-rental.json"],
        "flags": flags(),
    }
    atomic_json(root_file(root, "preflight/training-adapter-unblock.json"), unblock)
    return {"config": config_path, "inputs": inputs_path, "package": package_path, "environmentStatus": config["environment"].get("status"), "status": unblock["status"]}


def auth_granted(path: Path, expected_scope: str) -> dict[str, Any]:
    auth = load_json(path)
    exact(auth.get("schema"), "haitaka-anhoku-strength-first-step12-authorization-v1", f"{path}.schema")
    exact(auth.get("experimentId"), EXPERIMENT_ID, f"{path}.experimentId")
    if auth.get("scope") != expected_scope:
        fail(f"authorization scope does not match the Step 12 training package: {path}")
    if auth.get("granted") is not True:
        fail(f"training-authorization-missing: {path} is not granted")
    return auth


def _grant_timestamp(value: Any, label: str) -> datetime:
    if not isinstance(value, str):
        fail(f"CUDA grant {label} is missing")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as exc:
        fail(f"CUDA grant {label} is not an ISO timestamp: {exc}")
    if parsed.tzinfo is None:
        fail(f"CUDA grant {label} has no timezone")
    return parsed.astimezone(timezone.utc)


def validate_cuda_grant_document(grant: dict[str, Any], *, observed_instance: str | None = None, now_value: datetime | None = None) -> dict[str, Any]:
    """Validate the complete second-instance envelope before CUDA imports."""
    if grant.get("schema") != "haitaka-anhoku-strength-first-step12-authorization-v1" or grant.get("experimentId") != EXPERIMENT_ID:
        fail("CUDA grant schema or experiment identity is invalid")
    if grant.get("scope") != "Vast.ai rental for Step 12 training" or grant.get("granted") is not True:
        fail("CUDA work requires a granted Step 12 GPU rental ledger")
    resource = grant.get("resource")
    if not isinstance(resource, dict) or not isinstance(resource.get("instanceId"), int):
        fail("CUDA grant has no bound instance identity")
    if observed_instance is None:
        observed_instance = os.environ.get("STEP12_VAST_INSTANCE_ID")
    if observed_instance is None or observed_instance != str(resource["instanceId"]):
        fail("CUDA work requires the observed instance identity to match the grant")
    lifecycle = resource.get("lifecycle")
    if not isinstance(lifecycle, dict):
        fail("CUDA grant has no lifecycle envelope")
    maximum_hours = float(lifecycle.get("maximumInstanceHours", 0))
    maximum_cost = float(lifecycle.get("maximumTotalUsd", 0))
    hourly_total = float(resource.get("hourlyTotalUsd", 0))
    if not (0 < maximum_hours <= 2.0 and 0 < maximum_cost <= 5.0):
        fail("CUDA grant exceeds the amended qualification lifecycle limits")
    if not (0 < hourly_total < 0.30):
        fail("CUDA grant exceeds the USD 0.30/hour ceiling")
    amendment = grant.get("allowanceAmendment")
    remaining = amendment.get("remainingCumulative", {}) if isinstance(amendment, dict) else {}
    if not isinstance(amendment, dict) or amendment.get("approved") is not True or amendment.get("additionalInstances") != 1:
        fail("CUDA grant lacks the explicit one-additional-instance allowance amendment")
    remaining_hours = float(remaining.get("hours", 0))
    remaining_cost = float(remaining.get("costUsd", 0))
    remaining_instances = int(remaining.get("instances", 0)) if str(remaining.get("instances", "")).isdigit() else 0
    if not (0 < remaining_hours <= 4.0 and 0 < remaining_cost <= 5.0 and remaining_instances >= 1):
        fail("CUDA grant has no positive remaining cumulative allowance")
    if remaining_hours < maximum_hours or remaining_cost < maximum_cost:
        fail("CUDA grant lifecycle envelope exceeds the remaining cumulative allowance")
    reserve_minutes = float(amendment.get("archiveTerminateReserveMinutes", 20))
    if reserve_minutes < 20:
        fail("CUDA grant archive/termination reserve is below 20 minutes")
    grant_valid_until = _grant_timestamp(grant.get("validUntil"), "validUntil")
    quote = grant.get("freshQuote")
    if not isinstance(quote, dict):
        fail("CUDA grant has no fresh quote binding")
    required_quote = ("provider", "offerId", "gpuName", "image", "imageDigest", "dependencyLockSha256", "observedAt", "validUntil", "hourlyTotalUsd")
    if any(not quote.get(key) for key in required_quote):
        fail("CUDA grant fresh quote is missing provider, offer, image, dependency, price or validity binding")
    if quote.get("provider") != resource.get("provider") or quote.get("offerId") != resource.get("offerId"):
        fail("CUDA grant quote and resource offer identities differ")
    if float(quote.get("hourlyTotalUsd", -1)) != hourly_total:
        fail("CUDA grant quote hourly price differs from the bound resource price")
    if len(str(quote.get("dependencyLockSha256"))) != 64 or len(str(quote.get("imageDigest"))) != 64:
        fail("CUDA grant quote image/dependency bindings are not SHA-256 values")
    quote_observed_at = _grant_timestamp(quote.get("observedAt"), "freshQuote.observedAt")
    quote_valid_until = _grant_timestamp(quote.get("validUntil"), "freshQuote.validUntil")
    current = now_value or datetime.now(timezone.utc)
    if quote_valid_until <= quote_observed_at or quote_valid_until <= current or grant_valid_until <= current:
        fail("CUDA grant or fresh quote has expired")
    if grant_valid_until > quote_valid_until:
        fail("CUDA grant validity extends beyond the fresh quote validity")
    return {"maximumInstanceHours": maximum_hours, "maximumTotalUsd": maximum_cost, "hourlyTotalUsd": hourly_total, "remainingHours": remaining_hours, "remainingCostUsd": remaining_cost, "remainingInstances": remaining_instances, "archiveTerminateReserveMinutes": reserve_minutes, "validUntil": grant_valid_until.isoformat(), "quoteValidUntil": quote_valid_until.isoformat(), "instanceId": resource["instanceId"]}


def require_production_authorization(root: Path, ledger_path: Path, config: dict[str, Any], config_path: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    """Check both grants before importing any backend or device library."""

    training = auth_granted(ledger_path, "one scratch training run through update 2688")
    gpu = auth_granted(root_file(root, "authorization/gpu-rental.json"), "Vast.ai rental for Step 12 training")
    for name, auth in (("training", training), ("gpu", gpu)):
        bound = auth.get("boundPreparation", {})
        for role, relative in (("config", "training/config.json"), ("inputs", "training/inputs.json"), ("package", "training/package.json")):
            item = bound.get(role)
            if not isinstance(item, dict) or item.get("sha256") != sha256(root_file(root, relative)):
                fail(f"{name} authorization is not bound to the current {role}")
    if config.get("environment", {}).get("status") != "observed":
        fail("device-environment-pending: the authorized MBP environment receipt is not complete")
    return training, gpu


def run_fixture(config_path: Path, root: Path, device: str, output_path: Path, timeout_seconds: float | None = None) -> int:
    config = load_json(config_path)
    validate_config(config, root, verify_inputs=True)
    if device not in {"cpu", "cuda"}:
        fail("fixture device must be cpu or cuda")
    gpu_auth = None
    if device == "cuda":
        gpu_auth = auth_granted(root_file(root, "authorization/gpu-rental.json"), "Vast.ai rental for Step 12 training")
        validate_cuda_grant_document(gpu_auth, observed_instance=os.environ.get("STEP12_VAST_INSTANCE_ID"))
        binding = gpu_auth.get("fixtureBinding")
        if not isinstance(binding, dict):
            fail("CUDA fixture grant has no host-specific package binding")
        for role, relative in (("config", "training/config.json"), ("inputs", "training/inputs.json"), ("package", "training/package.json")):
            expected = binding.get(role)
            target = root_file(root, relative)
            if not isinstance(expected, dict) or expected.get("bytes") != target.stat().st_size or expected.get("sha256") != sha256(target):
                fail(f"CUDA fixture grant is not bound to current {relative}")
    if output_path.exists():
        prior = load_json(output_path)
        if prior.get("status") == "passed":
            fail(f"a passed fixture summary is immutable: {output_path}")
        archive_name = f"{output_path.stem}-failed-{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')}{output_path.suffix}"
        archived = output_path.with_name(archive_name)
        output_path.replace(archived)
    attempt = output_path.parent / f"{output_path.stem}-attempt-{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')}-{os.getpid()}"
    fixture_script = repo_file("scripts/strength_first_step12_trainer_fixture.py")
    environment = os.environ.copy()
    environment["PYTHONHASHSEED"] = "0"
    if device == "cpu":
        environment["CUDA_VISIBLE_DEVICES"] = ""
    else:
        environment["CUBLAS_WORKSPACE_CONFIG"] = ":4096:8"
        environment["STEP12_VAST_INSTANCE_ID"] = str(gpu_auth["resource"]["instanceId"])
    command = [sys.executable, str(fixture_script), "--config", str(config_path), "--output-dir", str(attempt), "--device", device, "--worker"]
    started = time.monotonic()
    try:
        completed = subprocess.run(command, cwd=REPO_ROOT, env=environment, capture_output=True, text=True, check=False, timeout=timeout_seconds)
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout.decode("utf-8", "replace") if isinstance(exc.stdout, bytes) else (exc.stdout or "")
        stderr = exc.stderr.decode("utf-8", "replace") if isinstance(exc.stderr, bytes) else (exc.stderr or "")
        completed = subprocess.CompletedProcess(command, 124, stdout, stderr + "\nqualification child exceeded its bounded timeout\n")
    if attempt.is_dir():
        (attempt / "wrapper.stdout").write_text(completed.stdout, encoding="utf-8")
        (attempt / "wrapper.stderr").write_text(completed.stderr, encoding="utf-8")
    worker_report_path = attempt / "report.json"
    worker_report = load_json(worker_report_path) if worker_report_path.is_file() else None
    expected_worker_schema = "haitaka-anhoku-strength-first-step12-real-cuda-fixture-v1" if device == "cuda" else "haitaka-anhoku-strength-first-step12-real-cpu-fixture-v1"
    worker_passed = bool(completed.returncode == 0 and isinstance(worker_report, dict) and worker_report.get("status") == "passed" and worker_report.get("schema") == expected_worker_schema)
    summary = {
        "schema": CUDA_FIXTURE_SUMMARY_SCHEMA if device == "cuda" else CPU_FIXTURE_SUMMARY_SCHEMA,
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "device": device,
        "status": "passed" if worker_passed else "failed",
        "attemptDirectory": str(attempt),
        "command": command,
        "returncode": completed.returncode,
        "elapsedSeconds": time.monotonic() - started,
        "workerReport": {"path": str(worker_report_path), "bytes": worker_report_path.stat().st_size, "sha256": sha256(worker_report_path), "schema": worker_report.get("schema"), "status": worker_report.get("status")} if worker_report_path.is_file() else None,
        "stdout": completed.stdout[-4000:],
        "stderr": completed.stderr[-4000:],
        "fixtureComputeAttempted": attempt.is_dir(),
        "productionTrainingAttempted": False,
        "gpuAttempted": device == "cuda",
        "flags": flags(),
    }
    atomic_json(output_path, summary)
    print(json.dumps({"status": summary["status"], "output": str(output_path), "attempt": str(attempt)}, sort_keys=True))
    return 0 if worker_passed else 1


def run_cuda_qualification(config_path: Path, root: Path, output_dir: Path) -> int:
    """Run the reviewed gradient suite and two fresh exact CUDA fixtures."""
    config = load_json(config_path)
    validate_config(config, root, verify_inputs=True)
    gpu_auth_path = root_file(root, "authorization/gpu-rental.json")
    gpu_auth = auth_granted(gpu_auth_path, "Vast.ai rental for Step 12 training")
    resource = gpu_auth.get("resource", {})
    if not isinstance(resource, dict) or not isinstance(resource.get("instanceId"), int):
        fail("CUDA qualification grant has no instance identity")
    output_dir = output_dir.expanduser().resolve()
    if output_dir.exists():
        fail(f"CUDA qualification refuses an existing output directory: {output_dir}")
    output_dir.mkdir(parents=True)
    def local_artifact(path: Path) -> dict[str, Any]:
        try:
            display = str(path.relative_to(root))
        except ValueError:
            display = str(path)
        return {"path": display, "bytes": path.stat().st_size, "sha256": sha256(path)}
    started = time.monotonic()
    commands: list[dict[str, Any]] = []
    budget_ledger: list[dict[str, Any]] = []
    gradient_reports: list[dict[str, Any]] = []
    fixture_reports: list[dict[str, Any]] = []
    env = os.environ.copy()
    env["PYTHONHASHSEED"] = "0"
    env["CUBLAS_WORKSPACE_CONFIG"] = ":4096:8"
    env["STEP12_VAST_INSTANCE_ID"] = str(resource["instanceId"])
    test_script = repo_file("scripts/strength_first_step12_cuda_backward_tests.py")

    def budget_before(stage: str) -> float:
        limits = validate_cuda_grant_document(gpu_auth, observed_instance=env["STEP12_VAST_INSTANCE_ID"])
        elapsed = time.monotonic() - started
        maximum_seconds = min(limits["maximumInstanceHours"] * 3600.0, limits["maximumTotalUsd"] / limits["hourlyTotalUsd"] * 3600.0, limits["remainingHours"] * 3600.0, limits["remainingCostUsd"] / limits["hourlyTotalUsd"] * 3600.0)
        usable_seconds = maximum_seconds - limits["archiveTerminateReserveMinutes"] * 60.0
        remaining_seconds = usable_seconds - elapsed
        if remaining_seconds <= 1.0:
            fail(f"CUDA qualification cannot start {stage}: lifecycle budget is at the archive reserve")
        entry = {"stage": stage, "status": "started", "startedAt": now(), "elapsedBeforeSeconds": elapsed, "remainingUsableSeconds": remaining_seconds, "remainingAllowanceHours": limits["remainingHours"], "remainingAllowanceCostUsd": limits["remainingCostUsd"], "hourlyTotalUsd": limits["hourlyTotalUsd"], "timeoutSeconds": remaining_seconds, "_startedMonotonic": time.monotonic()}
        budget_ledger.append(entry)
        return remaining_seconds

    def budget_after(stage: str, status: str) -> None:
        for entry in reversed(budget_ledger):
            if entry["stage"] == stage and entry["status"] == "started":
                entry["status"] = status
                entry["finishedAt"] = now()
                entry["elapsedAfterSeconds"] = time.monotonic() - started
                entry["elapsedChildSeconds"] = time.monotonic() - entry.pop("_startedMonotonic")
                entry["estimatedCostUsd"] = entry["elapsedChildSeconds"] / 3600.0 * entry["hourlyTotalUsd"]
                return

    for index in (1, 2):
        report_path = output_dir / f"gradient-process-{index}.json"
        command = [sys.executable, str(test_script), "cuda", "--config", str(config_path), "--authorization-ledger", str(gpu_auth_path), "--output", str(report_path)]
        stage = f"gradient-process-{index}"
        timeout_seconds = budget_before(stage)
        try:
            completed = subprocess.run(command, cwd=REPO_ROOT, env=env, capture_output=True, text=True, check=False, timeout=timeout_seconds)
        except subprocess.TimeoutExpired as exc:
            stdout = exc.stdout.decode("utf-8", "replace") if isinstance(exc.stdout, bytes) else (exc.stdout or "")
            stderr = exc.stderr.decode("utf-8", "replace") if isinstance(exc.stderr, bytes) else (exc.stderr or "")
            completed = subprocess.CompletedProcess(command, 124, stdout, stderr + "\nqualification child exceeded its bounded timeout\n")
        budget_after(stage, "passed" if completed.returncode == 0 else "failed")
        record = {"argv": command, "cwd": str(REPO_ROOT), "returncode": completed.returncode, "stdout": completed.stdout[-4000:], "stderr": completed.stderr[-4000:], "output": str(report_path)}
        commands.append(record)
        if not report_path.is_file():
            break
        report = load_json(report_path)
        gradient_reports.append(report)
        if completed.returncode != 0 or report.get("status") != "passed":
            break
    if len(gradient_reports) == 2 and all(item.get("status") == "passed" for item in gradient_reports):
        for index in (1, 2):
            summary_path = output_dir / f"fixture-{index}-summary.json"
            stage = f"fixture-{index}"
            timeout_seconds = budget_before(stage)
            result = run_fixture(config_path, root, "cuda", summary_path, timeout_seconds=timeout_seconds)
            budget_after(stage, "passed" if result == 0 else "failed")
            summary = load_json(summary_path) if summary_path.is_file() else {}
            fixture_reports.append(summary)
            if result != 0 or summary.get("status") != "passed":
                break
    cross_attempt = False
    shared_production_adapter_binding = False
    if len(fixture_reports) == 2 and all(item.get("status") == "passed" for item in fixture_reports):
        first_report = load_json(Path(fixture_reports[0]["workerReport"]["path"]))
        second_report = load_json(Path(fixture_reports[1]["workerReport"]["path"]))
        first_inventory = first_report.get("stateInventory", {})
        second_inventory = second_report.get("stateInventory", {})
        def adapter_binding(report: dict[str, Any]) -> bool:
            runtime = report.get("actualComponents", {}).get("deterministicCudaRuntime", {})
            adapter = runtime.get("adapter", {})
            requested = adapter.get("binding", {}).get("requestedBinding", {})
            return bool(runtime.get("featureTransformerModule") == "feature_transformer" and adapter.get("profile") == "ordered-segment-v1" and requested.get("adapter", {}).get("sha256") == config.get("trainer", {}).get("cudaAdapter", {}).get("sha256") and requested.get("kernel", {}).get("sha256") == config.get("trainer", {}).get("cudaKernel", {}).get("sha256"))
        shared_production_adapter_binding = bool(adapter_binding(first_report) and adapter_binding(second_report) and config.get("trainer", {}).get("backend", {}).get("sha256") and config.get("trainer", {}).get("cudaRuntime", {}).get("sha256"))
        first_probe = first_report.get("cudaRngProbe", {})
        second_probe = second_report.get("cudaRngProbe", {})
        probe_valid = all(
            isinstance(probe, dict)
            and probe.get("profile") == "seeded-cuda-save-restore-v2"
            and isinstance(probe.get("seed"), int)
            and probe.get("exact") is True
            and probe.get("stateRestoredExact") is True
            and isinstance(probe.get("stateBefore"), list)
            and probe.get("stateBefore")
            and probe.get("stateAfterRestore") == probe.get("stateBefore")
            and isinstance(probe.get("drawSha256"), str)
            and len(probe.get("drawSha256")) == 64
            and all(isinstance(item, str) and len(item) == 64 for item in probe.get("stateBefore", []))
            for probe in (first_probe, second_probe)
        )
        cross_attempt = bool(first_inventory and second_inventory and first_inventory.get("sha256") == second_inventory.get("sha256") and probe_valid and first_probe == second_probe and first_report.get("nextRandomDraw") == second_report.get("nextRandomDraw") and adapter_binding(first_report) and adapter_binding(second_report))
    passed = len(gradient_reports) == 2 and all(item.get("status") == "passed" for item in gradient_reports) and len(fixture_reports) == 2 and all(item.get("status") == "passed" for item in fixture_reports) and cross_attempt and shared_production_adapter_binding
    gradient_entries = []
    for index in (1, 2):
        path = output_dir / f"gradient-process-{index}.json"
        gradient_entries.append({"artifact": local_artifact(path) if path.is_file() else None, "status": gradient_reports[index - 1].get("status") if len(gradient_reports) >= index else "missing"})
    fixture_entries = []
    for index in (1, 2):
        path = output_dir / f"fixture-{index}-summary.json"
        summary = fixture_reports[index - 1] if len(fixture_reports) >= index else {}
        worker_path = Path(summary.get("workerReport", {}).get("path", "")) if isinstance(summary.get("workerReport"), dict) else Path("")
        fixture_entries.append({"artifact": local_artifact(path) if path.is_file() else None, "status": summary.get("status", "missing"), "workerReport": local_artifact(worker_path) if worker_path.is_file() else None})
    receipt = {"schema": "haitaka-anhoku-strength-first-step12-deterministic-cuda-qualification-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "status": "passed" if passed else "failed", "package": root_artifact(root, "training/package.json"), "config": root_artifact(root, "training/config.json"), "inputs": root_artifact(root, "training/inputs.json"), "authorization": root_artifact(root, "authorization/gpu-rental.json"), "gradientTests": gradient_entries, "fixtures": fixture_entries, "commands": commands, "budgetLedger": budget_ledger, "crossAttemptExact": cross_attempt, "sharedProductionAdapterBinding": shared_production_adapter_binding, "productionTrainingAttempted": False, "gpuAttempted": True, "elapsedSeconds": time.monotonic() - started, "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "outcomesOpened": False, "handcraftedRemainsProduction": True}}
    atomic_json(output_dir / "qualification.json", receipt)
    print(json.dumps({"status": receipt["status"], "output": str(output_dir / "qualification.json")}, sort_keys=True))
    return 0 if passed else 1


def artifact_from_selection(value: Any, root: Path, where: str) -> dict[str, Any]:
    item = exact_keys(value, {"path", "bytes", "sha256"}, where)
    verify_artifact(item, root, where, allow_relative=True)
    return item


def select_checkpoints(payload: dict[str, Any], root: Path, verify_artifacts: bool) -> dict[str, Any]:
    exact(payload.get("schema"), SELECTION_INPUT_SCHEMA, "selection input.schema")
    exact(payload.get("experimentId"), EXPERIMENT_ID, "selection input.experimentId")
    exact(payload.get("matchEvidenceUsed"), False, "selection input.matchEvidenceUsed")
    candidates = payload.get("candidates")
    if not isinstance(candidates, list) or len(candidates) != len(CANDIDATE_UPDATES):
        fail("selection requires exactly eight candidate updates")
    control = payload.get("control")
    if not isinstance(control, dict) or control.get("update") != CONTROL_UPDATE:
        fail("selection requires a distinct u2688 control")
    records: list[dict[str, Any]] = []
    for index, (item, expected_update) in enumerate(zip(candidates, CANDIDATE_UPDATES)):
        exact(item.get("update"), expected_update, f"candidates[{index}].update")
        validation = item.get("validation", {})
        exact(validation.get("records"), 60000, f"candidates[{index}].validation.records")
        exact(validation.get("order"), "fixed-source-order-once-per-evaluation-v1", f"candidates[{index}].validation.order")
        exact(validation.get("finite"), True, f"candidates[{index}].validation.finite")
        loss = finite(validation.get("teacherOnlyLoss"), f"candidates[{index}].validation.teacherOnlyLoss")
        if verify_artifacts:
            artifact_from_selection(item.get("checkpoint"), root, f"candidates[{index}].checkpoint")
            artifact_from_selection(item.get("export"), root, f"candidates[{index}].export")
            artifact_from_selection(validation.get("receipt"), root, f"candidates[{index}].validation.receipt")
        records.append({"update": expected_update, "loss": loss, "input": item})

    exact(control.get("validation", {}).get("records"), 60000, "control.validation.records")
    exact(control.get("validation", {}).get("order"), "fixed-source-order-once-per-evaluation-v1", "control.validation.order")
    exact(control.get("validation", {}).get("finite"), True, "control.validation.finite")
    control_loss = finite(control["validation"].get("teacherOnlyLoss"), "control.validation.teacherOnlyLoss")
    if verify_artifacts:
        artifact_from_selection(control.get("checkpoint"), root, "control.checkpoint")
        artifact_from_selection(control.get("export"), root, "control.export")
        artifact_from_selection(control["validation"].get("receipt"), root, "control.validation.receipt")
    minimum = min(item["loss"] for item in records)
    tied = [item for item in records if item["loss"] <= minimum + SELECTION_TOLERANCE]
    selected = tied[0]
    return {
        "schema": SELECTION_SCHEMA,
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "rule": "global-minimum-within-1e-12-then-earliest-update-v1",
        "candidateUpdates": CANDIDATE_UPDATES,
        "globalMinimum": minimum,
        "tolerance": SELECTION_TOLERANCE,
        "tieUpdates": [item["update"] for item in tied],
        "selectedUpdate": selected["update"],
        "selectedCheckpoint": selected["input"]["checkpoint"],
        "candidates": candidates,
        "control": control,
        "controlLoss": control_loss,
        "matchEvidenceUsed": False,
        "status": "selected-validation-only",
        "flags": flags(),
    }


def run_static_fixtures(root: Path) -> dict[str, Any]:
    """Exercise the pure selection boundary without importing a device stack."""

    def candidate(update: int, loss: float) -> dict[str, Any]:
        dummy = {"path": f"fixture-{update}", "bytes": 0, "sha256": "0" * 64}
        return {
            "update": update,
            "validation": {"records": 60000, "order": "fixed-source-order-once-per-evaluation-v1", "finite": True, "teacherOnlyLoss": loss},
            "checkpoint": dummy,
            "export": dummy,
        }

    tests: list[dict[str, Any]] = []

    def check(name: str, fn: Any) -> None:
        try:
            fn()
        except Exception as exc:
            tests.append({"name": name, "status": "failed", "error": str(exc)})
        else:
            tests.append({"name": name, "status": "passed"})

    def tolerance_boundary() -> None:
        losses = {update: 0.7 + update / 100000.0 for update in CANDIDATE_UPDATES}
        losses[768] = 0.1
        losses[1024] = 0.1 + 0.5e-12
        payload = {"schema": SELECTION_INPUT_SCHEMA, "experimentId": EXPERIMENT_ID, "matchEvidenceUsed": False, "candidates": [candidate(update, losses[update]) for update in CANDIDATE_UPDATES], "control": candidate(CONTROL_UPDATE, 0.2)}
        if select_checkpoints(payload, root, False)["selectedUpdate"] != 768:
            fail("1e-12 selection boundary chose the wrong earliest candidate")

    def global_minimum_nonmonotonic() -> None:
        losses = {update: 0.6 for update in CANDIDATE_UPDATES}
        losses[256] = 0.4
        losses[768] = 0.2
        losses[1024] = 0.2 + 0.8e-12
        payload = {"schema": SELECTION_INPUT_SCHEMA, "experimentId": EXPERIMENT_ID, "matchEvidenceUsed": False, "candidates": [candidate(update, losses[update]) for update in CANDIDATE_UPDATES], "control": candidate(CONTROL_UPDATE, 0.1)}
        if select_checkpoints(payload, root, False)["selectedUpdate"] != 768:
            fail("selection did not compute the global minimum before applying tolerance")

    def earliest_tie() -> None:
        payload = {"schema": SELECTION_INPUT_SCHEMA, "experimentId": EXPERIMENT_ID, "matchEvidenceUsed": False, "candidates": [candidate(update, 0.2) for update in CANDIDATE_UPDATES], "control": candidate(CONTROL_UPDATE, 0.1)}
        if select_checkpoints(payload, root, False)["selectedUpdate"] != 128:
            fail("exact candidate tie did not choose the earliest update")

    def rejects_nan() -> None:
        payload = {"schema": SELECTION_INPUT_SCHEMA, "experimentId": EXPERIMENT_ID, "matchEvidenceUsed": False, "candidates": [candidate(update, float("nan") if update == 768 else 0.2) for update in CANDIDATE_UPDATES], "control": candidate(CONTROL_UPDATE, 0.1)}
        try:
            select_checkpoints(payload, root, False)
        except Step12TrainingError:
            return
        fail("NaN candidate was accepted")

    def rejects_match_evidence() -> None:
        payload = {"schema": SELECTION_INPUT_SCHEMA, "experimentId": EXPERIMENT_ID, "matchEvidenceUsed": True, "candidates": [candidate(update, 0.2) for update in CANDIDATE_UPDATES], "control": candidate(CONTROL_UPDATE, 0.1)}
        try:
            select_checkpoints(payload, root, False)
        except Step12TrainingError:
            return
        fail("match evidence was accepted by validation-only selector")

    for name, fn in (("global-minimum-tolerance", tolerance_boundary), ("nonmonotonic-global-minimum", global_minimum_nonmonotonic), ("earliest-exact-tie", earliest_tie), ("rejects-nan", rejects_nan), ("rejects-match-evidence", rejects_match_evidence)):
        check(name, fn)
    failed = [item for item in tests if item["status"] != "passed"]
    receipt = {"schema": STATIC_FIXTURE_SCHEMA, "createdAt": now(), "experimentId": EXPERIMENT_ID, "status": "passed" if not failed else "failed", "tests": tests, "deviceInitialized": False, "trainingOrGpuAttempted": False, "outcomesOpened": False, "flags": flags()}
    atomic_json(root_file(root, "preflight/training-static-fixtures.json"), receipt)
    if failed:
        fail(f"static training fixtures failed: {failed[0]['name']}")
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path)
    parser.add_argument("--root", type=Path)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("prepare")
    sub.add_parser("self-test")
    validate_parser = sub.add_parser("validate-contract")
    validate_parser.add_argument("--verify-inputs", action="store_true")
    fixture_parser = sub.add_parser("fixtures")
    fixture_parser.add_argument("--device", choices=("cpu", "cuda"), required=True)
    fixture_parser.add_argument("--output", type=Path, required=True)
    qualification_parser = sub.add_parser("cuda-qualification")
    qualification_parser.add_argument("--output-dir", type=Path, required=True)
    train_parser = sub.add_parser("train")
    train_parser.add_argument("--authorization-ledger", type=Path, required=True)
    train_parser.add_argument("--resume", action="store_true")
    select_parser = sub.add_parser("select")
    select_parser.add_argument("--input", type=Path, required=True)
    select_parser.add_argument("--output", type=Path, required=True)
    select_parser.add_argument("--verify-artifacts", action="store_true")
    args = parser.parse_args()
    try:
        if args.command == "prepare":
            if args.root is not None:
                root = args.root.expanduser().resolve()
            elif args.config is not None:
                root = root_from_config(args.config)
            else:
                fail("prepare requires --root or --config ROOT/training/config.json")
            result = prepare_training(root)
            print(json.dumps({"status": result["status"], "config": str(result["config"]), "inputs": str(result["inputs"]), "package": str(result["package"])}, sort_keys=True))
            return 0
        if args.config is None:
            fail(f"{args.command} requires --config ROOT/training/config.json")
        config_path = args.config.expanduser().resolve()
        root = root_from_config(config_path)
        config = load_json(config_path)
        validate_config(config, root, verify_inputs=getattr(args, "verify_inputs", False) or args.command in {"fixtures", "train"})
        if args.command == "self-test":
            receipt = run_static_fixtures(root)
            print(json.dumps({"status": receipt["status"], "tests": len(receipt["tests"])}, sort_keys=True))
            return 0
        if args.command == "validate-contract":
            print(json.dumps({"status": "passed", "configSha256": sha256(config_path)}, sort_keys=True))
            return 0
        if args.command == "fixtures":
            return run_fixture(config_path, root, args.device, args.output.expanduser().resolve())
        if args.command == "cuda-qualification":
            return run_cuda_qualification(config_path, root, args.output_dir)
        if args.command == "select":
            result = select_checkpoints(load_json(args.input.expanduser().resolve()), root, args.verify_artifacts)
            atomic_json(args.output.expanduser().resolve(), result)
            print(json.dumps({"status": result["status"], "selectedUpdate": result["selectedUpdate"], "outputSha256": sha256(args.output.expanduser().resolve())}, sort_keys=True))
            return 0
        if args.command == "train":
            # This call is intentionally the last operation before importing
            # the backend.  Missing grants therefore cannot initialize torch,
            # CUDA, a model, a build context, or an output directory.
            training, gpu = require_production_authorization(root, args.authorization_ledger.expanduser().resolve(), config, config_path)
            from strength_first_step12_trainer_backend import run_training

            result = run_training(config_path=config_path, root=root, training_authorization=training, gpu_authorization=gpu, resume=args.resume)
            print(json.dumps({"status": result["status"], "optimizerUpdates": result["optimizerUpdates"]}, sort_keys=True))
            return 0
        fail(f"unhandled command: {args.command}")
    except (Step12TrainingError, OSError, UnicodeError, json.JSONDecodeError, KeyError, TypeError) as exc:
        print(f"Step 12 trainer blocked: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
