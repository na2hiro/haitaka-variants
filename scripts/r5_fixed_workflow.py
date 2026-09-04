#!/usr/bin/env python3
"""Fail-closed preparation for the fixed-data R5 pilot.

This command is deliberately a preflight, not a training launcher.  R5 uses a
new immutable dataset schema and cannot be routed through the legacy ``train``
command.  The preflight binds every input to the passing replacement-R4 gate,
checks the complete finite payload, and writes a small handoff manifest.  It
does not copy or rewrite the R4 data and it never starts Python ML training.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import subprocess
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
EXPECTED_R4_GATES = {
    "accepted-finite-cohort-complete",
    "attempt-ledger-complete",
    "authorization-chain-valid",
    "color-swap-split-isolation",
    "cost-forecast-and-ceiling",
    "declared-mixtures-preserved",
    "deterministic-selection-and-replenishment",
    "finite-deep-core-complete",
    "grouped-split-isolation",
    "imported-label-provenance-valid",
    "incomplete-targets-null",
    "label-executable-equivalence",
    "label-policy-identity-valid",
    "maximum-incomplete-attempt-rate",
    "no-downstream-work",
    "node-accounting-exact",
    "position-and-label-regeneration",
    "replacement-contract-frozen-before-work",
    "retained-attempted-bias-bounded",
    "retained-feature-coverage",
    "sole-r5-dataset-identified",
    "source-pool-bound",
    "split-counts-exact",
}
EXPECTED_SPLITS = ("training", "validation", "development")
SHA256_LENGTH = 64


class PreflightError(RuntimeError):
    """A contract or immutable input failed closed validation."""


def fail(message: str) -> None:
    raise PreflightError(message)


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot read JSON {path}: {exc}")
    if not isinstance(value, dict):
        fail(f"JSON root is not an object: {path}")
    return value


def relpath(value: str) -> Path:
    path = Path(value)
    if path.is_absolute():
        fail(f"contract path must be workspace-relative: {value}")
    return ROOT / path


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
    except OSError as exc:
        fail(f"cannot hash {path}: {exc}")
    return digest.hexdigest()


def validate_hash(value: Any, label: str) -> str:
    if not isinstance(value, str) or len(value) != SHA256_LENGTH:
        fail(f"{label} is not a 64-character SHA-256")
    if value != value.lower() or any(char not in "0123456789abcdef" for char in value):
        fail(f"{label} is not lowercase hexadecimal")
    return value


def artifact(value: dict[str, Any], label: str, *, hash_content: bool = True) -> dict[str, Any]:
    path_value = value.get("path")
    if not isinstance(path_value, str):
        fail(f"{label}.path is missing")
    path = relpath(path_value)
    expected_bytes = value.get("bytes")
    if not isinstance(expected_bytes, int) or expected_bytes < 0:
        fail(f"{label}.bytes is invalid")
    expected_hash = validate_hash(value.get("sha256"), f"{label}.sha256")
    try:
        actual_bytes = path.stat().st_size
    except OSError as exc:
        fail(f"{label} is missing: {path} ({exc})")
    if actual_bytes != expected_bytes:
        fail(f"{label} has {actual_bytes} bytes, expected {expected_bytes}")
    actual_hash = sha256_file(path) if hash_content else None
    if hash_content and actual_hash != expected_hash:
        fail(f"{label} has sha256 {actual_hash}, expected {expected_hash}")
    return {
        "path": path_value,
        "bytes": expected_bytes,
        "sha256": expected_hash,
    }


def expect_equal(actual: Any, expected: Any, label: str) -> None:
    if actual != expected:
        fail(f"{label} is {actual!r}, expected {expected!r}")


def validate_r4_authorization(contract: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
    authorization = contract.get("authorization")
    if not isinstance(authorization, dict):
        fail("authorization object is missing")

    replacement_contract_ref = authorization.get("replacementR4Contract")
    replacement_gate_ref = authorization.get("replacementR4Gate")
    pilot_ref = authorization.get("pilotManifest")
    for label, value in (
        ("authorization.replacementR4Contract", replacement_contract_ref),
        ("authorization.replacementR4Gate", replacement_gate_ref),
        ("authorization.pilotManifest", pilot_ref),
    ):
        if not isinstance(value, dict):
            fail(f"{label} is missing")
        validate_hash(value.get("sha256"), f"{label}.sha256")

    replacement_contract = artifact(replacement_contract_ref, "replacement R4 contract")
    replacement_gate = artifact(replacement_gate_ref, "replacement R4 gate report")
    pilot_manifest_artifact = artifact(pilot_ref, "eligible pilot manifest")

    gate = load_json(relpath(replacement_gate["path"]))
    expect_equal(gate.get("schema"), "haitaka-anhoku-r4-replacement-gate-v8", "R4 gate schema")
    expect_equal(gate.get("passed"), True, "R4 gate passed")
    expect_equal(gate.get("r5Authorized"), True, "R4 r5Authorized")
    expect_equal(gate.get("trainingStarted"), False, "R4 trainingStarted")
    expect_equal(gate.get("broadGenerationStarted"), False, "R4 broadGenerationStarted")
    expect_equal(gate.get("strengthGamesPlayed"), False, "R4 strengthGamesPlayed")
    gates = gate.get("gates")
    if not isinstance(gates, dict) or set(gates) != EXPECTED_R4_GATES:
        fail("R4 gate does not contain exactly the frozen 23-gate set")
    if not all(value is True for value in gates.values()):
        fail("R4 gate contains a false gate")
    expect_equal(gate.get("contractSha256"), replacement_contract["sha256"], "R4 contract hash")
    sole = gate.get("soleEligible1mDataset")
    if not isinstance(sole, dict):
        fail("R4 gate does not identify a sole eligible 1M pilot")
    expect_equal(sole.get("path"), pilot_manifest_artifact["path"], "sole pilot path")
    expect_equal(sole.get("bytes"), pilot_manifest_artifact["bytes"], "sole pilot bytes")
    expect_equal(sole.get("sha256"), pilot_manifest_artifact["sha256"], "sole pilot hash")

    pilot = load_json(relpath(pilot_manifest_artifact["path"]))
    expect_equal(pilot.get("schema"), "haitaka-r4-replacement-eligible-pilot-v1", "pilot schema")
    expect_equal(pilot.get("eligibleForR5"), True, "pilot eligibility")
    expect_equal(pilot.get("records"), contract["pilot"]["records"], "pilot record count")
    expect_equal(pilot.get("contractSha256"), replacement_contract["sha256"], "pilot contract hash")
    return gate, pilot


def validate_split(
    contract: dict[str, Any],
    split_name: str,
    split: dict[str, Any],
    pilot: dict[str, Any],
) -> dict[str, Any]:
    if split_name not in EXPECTED_SPLITS:
        fail(f"unexpected pilot split {split_name}")
    expected_records = split.get("records")
    if not isinstance(expected_records, int) or expected_records <= 0:
        fail(f"{split_name} records are invalid")
    pilot_split = pilot.get("splits", {}).get(split_name)
    if not isinstance(pilot_split, dict):
        fail(f"pilot manifest lacks {split_name} split")
    expect_equal(pilot_split.get("records"), expected_records, f"pilot {split_name} records")

    manifest_ref = split.get("manifest")
    binary_ref = split.get("binary")
    sidecar_ref = split.get("sidecar")
    if not all(isinstance(item, dict) for item in (manifest_ref, binary_ref, sidecar_ref)):
        fail(f"{split_name} has an incomplete artifact declaration")
    manifest = artifact(manifest_ref, f"{split_name} manifest")
    binary = artifact(binary_ref, f"{split_name} binary")
    sidecar = artifact(sidecar_ref, f"{split_name} sidecar")
    expect_equal(pilot_split.get("manifestSha256"), manifest["sha256"], f"pilot {split_name} manifest hash")

    manifest_json = load_json(relpath(manifest["path"]))
    expect_equal(manifest_json.get("schema"), "haitaka-r4-replacement-dataset-manifest-v1", f"{split_name} manifest schema")
    expect_equal(manifest_json.get("dataset"), split_name, f"{split_name} manifest dataset")
    expect_equal(manifest_json.get("records"), expected_records, f"{split_name} manifest records")
    expect_equal(manifest_json.get("entryBytes"), contract["pilot"]["entryBytes"], f"{split_name} entry bytes")
    expect_equal(manifest_json.get("contractSha256"), contract["pilot"]["labelContractSha256"], f"{split_name} label contract")
    expect_equal(manifest_json.get("labelEvaluator"), contract["pilot"]["labelEvaluator"], f"{split_name} label evaluator")
    expect_equal(manifest_json.get("labelBudget"), contract["pilot"]["labelBudget"], f"{split_name} label budget")
    expect_equal(manifest_json.get("targetSemantics"), contract["pilot"]["targetSemantics"], f"{split_name} target semantics")
    expect_equal(manifest_json.get("scoreTransform"), contract["pilot"]["scoreTransform"], f"{split_name} score transform")
    expect_equal(manifest_json.get("resultLossEnabled"), contract["pilot"]["resultLossEnabled"], f"{split_name} result loss")
    expect_equal(manifest_json.get("unknownResultsIgnored"), contract["pilot"]["unknownResultsIgnored"], f"{split_name} unknown-result policy")

    declared_binary = manifest_json.get("binary")
    declared_sidecar = manifest_json.get("sidecar")
    if not isinstance(declared_binary, dict) or not isinstance(declared_sidecar, dict):
        fail(f"{split_name} manifest lacks nested binary/sidecar identity")
    for label, declared, frozen in (
        (f"{split_name} manifest binary", declared_binary, binary),
        (f"{split_name} manifest sidecar", declared_sidecar, sidecar),
    ):
        expect_equal(declared.get("path"), frozen["path"], f"{label} path")
        expect_equal(declared.get("bytes"), frozen["bytes"], f"{label} bytes")
        expect_equal(declared.get("sha256"), frozen["sha256"], f"{label} hash")
    expect_equal(binary["bytes"], expected_records * contract["pilot"]["entryBytes"], f"{split_name} physical length")
    return {"manifest": manifest, "binary": binary, "sidecar": sidecar}


def validate_static_clone(contract: dict[str, Any]) -> dict[str, Any]:
    static = contract["initialization"]["lanes"][1].get("staticClone")
    if not isinstance(static, dict):
        fail("static-clone lane is missing its immutable source")
    binary = artifact(static["binary"], "static-clone binary")
    sidecar = artifact(static["sidecar"], "static-clone sidecar")
    records = static.get("records")
    entry_bytes = static.get("entryBytes")
    expect_equal(binary["bytes"], records * entry_bytes, "static-clone physical length")
    # The sidecar is intentionally checked as an independent stream.  This
    # prevents a binary-only copy from being mistaken for the frozen lane.
    lines = 0
    try:
        with relpath(sidecar["path"]).open("r", encoding="utf-8") as stream:
            for line in stream:
                if not line.strip():
                    fail("static-clone sidecar contains a blank line")
                json.loads(line)
                lines += 1
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot validate static-clone sidecar: {exc}")
    expect_equal(lines, records, "static-clone sidecar records")
    return {"binary": binary, "sidecar": sidecar, "records": records}


def validate_contract_shape(contract: dict[str, Any]) -> None:
    expect_equal(contract.get("schema"), "haitaka-anhoku-r5-fixed-data-exposure-contract-v1", "R5 contract schema")
    expect_equal(contract.get("phase"), "R5", "R5 phase")
    expect_equal(contract.get("ruleset"), "anhoku", "R5 ruleset")
    expect_equal(contract.get("status"), "frozen-for-preflight", "R5 contract status")
    expect_equal(contract.get("disallowedInputs"), [
        "out/anhoku-reboot-r4-replacement-v1-failed",
        "out/anhoku-reboot-r4-replacement-v2-failed",
        "out/anhoku-reboot-r4-replacement-v3-failed",
        "out/anhoku-reboot-r4-replacement-v4-failed",
        "out/anhoku-reboot-r4-replacement-v5-failed",
        "out/anhoku-reboot-r4-replacement-v6-failed",
        "out/anhoku-reboot-r4-replacement-v7-failed",
    ], "failed-R4 denylist")

    initialization = contract.get("initialization")
    if not isinstance(initialization, dict):
        fail("initialization object is missing")
    lanes = initialization.get("lanes")
    if not isinstance(lanes, list) or not lanes:
        fail("initialization lanes are missing")
    if len(lanes) > initialization.get("maximumRecipeLanes", 0):
        fail("initialization lane count exceeds the frozen maximum")
    ids = [lane.get("id") for lane in lanes if isinstance(lane, dict)]
    if len(ids) != len(set(ids)):
        fail("initialization lane IDs are not unique")
    if set(ids) != {"scratch", "static-clone-search-finetune"}:
        fail("R5 must compare exactly scratch and the frozen static-clone lane")
    if any(lane.get("kind") not in {"scratch", "static-clone-full-precision"} for lane in lanes):
        fail("R5 contains an unapproved initialization kind")
    if initialization.get("quantizedDiagnostic", {}).get("auditOnly") is not True:
        fail("quantized import is not audit-only")
    if initialization.get("c16Baseline", {}).get("path") != initialization["quantizedDiagnostic"].get("path"):
        fail("C/16 baseline and diagnostic do not share the frozen identity")

    optimization = contract.get("optimization")
    if not isinstance(optimization, dict):
        fail("optimization object is missing")
    expect_equal(optimization.get("randomFenSkipping"), 0, "random FEN skipping")
    expect_equal(optimization.get("filtered"), False, "smart filtering")
    expect_equal(optimization.get("numWorkers"), 1, "loader worker count")
    expect_equal(optimization.get("batchSize"), 16384, "batch size")
    expect_equal(optimization.get("validationRecords"), 60000, "validation records")
    expect_equal(optimization.get("developmentRecords"), 60000, "development records")
    expect_equal(optimization.get("trainRecordsPerPass"), 880000, "training records per pass")
    expect_equal(optimization.get("selectionHorizonIsNotOnePass"), True, "multi-pass selection")
    if len(optimization.get("initialLearningRateCandidates", [])) * len(lanes) > initialization["maximumRecipeLanes"]:
        fail("learning-rate and initialization Cartesian product exceeds the lane ceiling")
    decay = optimization.get("decay")
    if not isinstance(decay, dict):
        fail("step decay is missing")
    expect_equal(decay.get("kind"), "step-by-optimizer-update", "decay kind")
    if not (decay.get("stepUpdates", 0) > 0 and 0 < decay.get("gamma", 0) < 1):
        fail("step decay is not bounded")
    plateau = optimization.get("plateau")
    if not isinstance(plateau, dict) or plateau.get("patienceValidations", 0) <= 0:
        fail("plateau rule is missing")
    if not (0 < optimization.get("minimumUpdates", 0) <= optimization.get("maximumUpdates", 0)):
        fail("update bounds are invalid")
    if optimization.get("checkpointEveryUpdates", 0) <= 0 or optimization.get("validateEveryUpdates", 0) <= 0:
        fail("checkpoint/validation cadence is invalid")

    architecture = contract.get("architecture")
    expect_equal(architecture.get("featureGeometryFrozen"), True, "feature geometry freeze")
    for width_name in ("currentWidth", "compactWidth", "debugWidth"):
        width = architecture.get(width_name)
        if not isinstance(width, dict) or not all(isinstance(width.get(k), int) and width[k] > 0 for k in ("l1", "l2", "l3")):
            fail(f"{width_name} is invalid")

    block = contract.get("offlineSelection", {}).get("oneShotStageBlock")
    if not isinstance(block, dict) or block.get("status") != "closed" or block.get("dataMayNotBeGeneratedBeforeOpen") is not True:
        fail("R5 one-shot stage block is not closed")
    expect_equal(contract.get("trainer", {}).get("finiteExposureRequired"), True, "finite exposure requirement")
    expect_equal(contract.get("trainer", {}).get("validationMustNotCycle"), True, "finite validation requirement")
    expect_equal(contract.get("trainer", {}).get("randomSamplingMustBeZero"), True, "deterministic sampling requirement")
    expect_equal(contract.get("trainer", {}).get("productionTrainingRequiresGpuApproval"), True, "GPU approval requirement")


def validate_trainer_identity(contract: dict[str, Any]) -> dict[str, Any]:
    trainer = contract["trainer"]
    checkout = ROOT / "../engine/variant-nnue-pytorch"
    if not checkout.exists():
        fail(f"pinned trainer checkout is missing: {checkout}")
    try:
        revision = subprocess.check_output(
            ["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True
        ).strip()
    except (OSError, subprocess.CalledProcessError) as exc:
        fail(f"cannot inspect pinned trainer checkout: {exc}")
    expect_equal(revision, trainer["pinnedRevision"], "trainer revision")
    patch = artifact(trainer["phase7Patch"], "Phase 7.1 trainer patch")
    r5_patch = artifact(trainer["r5Patch"], "R5 trainer patch")
    return {
        "checkout": str(checkout.relative_to(ROOT.parent)),
        "revision": revision,
        "phase7Patch": patch,
        "r5Patch": r5_patch,
    }


def preflight(contract_path: Path, output_dir: Path) -> Path:
    contract = load_json(contract_path)
    validate_contract_shape(contract)
    if output_dir.exists():
        try:
            entries = list(output_dir.iterdir())
        except OSError as exc:
            fail(f"cannot inspect output directory {output_dir}: {exc}")
        if entries:
            fail(f"fresh R5 preflight refuses non-empty output directory: {output_dir}")
    gate, pilot = validate_r4_authorization(contract)

    splits = contract["pilot"].get("splits")
    if not isinstance(splits, dict) or set(splits) != set(EXPECTED_SPLITS):
        fail("R5 pilot split declaration is not exactly training/validation/development")
    split_artifacts = {
        name: validate_split(contract, name, splits[name], pilot)
        for name in EXPECTED_SPLITS
    }
    static_clone = validate_static_clone(contract)
    diagnostic = artifact(contract["initialization"]["quantizedDiagnostic"], "C/16 diagnostic")
    baseline = artifact(contract["initialization"]["c16Baseline"], "C/16 baseline")
    expect_equal(diagnostic, baseline, "C/16 identity")
    trainer_identity = validate_trainer_identity(contract)

    batch_size = contract["optimization"]["batchSize"]
    train_records = contract["optimization"]["trainRecordsPerPass"]
    held_out_records = contract["optimization"]["validationRecords"]
    exposure = {
        "trainRecordsPerPass": train_records,
        "trainBatchesPerPass": math.ceil(train_records / batch_size),
        "trainFinalBatchRecords": train_records % batch_size or batch_size,
        "validationRecordsPerEvaluation": held_out_records,
        "validationBatchesPerEvaluation": math.ceil(held_out_records / batch_size),
        "validationFinalBatchRecords": held_out_records % batch_size or batch_size,
        "randomFenSkipping": 0,
        "filtered": False,
        "loaderWorkers": 1,
        "cyclic": False,
        "exactPhysicalPass": True,
    }
    lane_ids = [lane["id"] for lane in contract["initialization"]["lanes"]]
    report = {
        "schema": "haitaka-anhoku-r5-preflight-report-v1",
        "contract": {
            "path": str(contract_path.relative_to(ROOT)),
            "sha256": sha256_file(contract_path),
        },
        "authorization": {
            "replacementR4Gate": {
                "path": contract["authorization"]["replacementR4Gate"]["path"],
                "sha256": contract["authorization"]["replacementR4Gate"]["sha256"],
                "passed": gate["passed"],
                "r5Authorized": gate["r5Authorized"],
            },
            "pilotManifest": {
                "path": contract["authorization"]["pilotManifest"]["path"],
                "sha256": contract["authorization"]["pilotManifest"]["sha256"],
                "records": pilot["records"],
            },
        },
        "inputs": {
            "splits": split_artifacts,
            "staticClone": static_clone,
            "c16Baseline": baseline,
            "trainer": trainer_identity,
        },
        "exposure": exposure,
        "lanes": {
            "ids": lane_ids,
            "learningRates": contract["optimization"]["initialLearningRateCandidates"],
            "recipeCount": len(lane_ids) * len(contract["optimization"]["initialLearningRateCandidates"]),
            "selectionSeed": contract["initialization"]["selectionSeed"],
            "replicationSeeds": contract["initialization"]["replicationSeeds"],
        },
        "stageBlock": contract["offlineSelection"]["oneShotStageBlock"],
        "trainingStarted": False,
        "readyForTraining": True,
        "r5Authorized": False,
        "passed": False,
        "nextAction": "obtain explicit Vast.ai GPU permission, then run the dedicated R5 trainer",
    }
    try:
        output_dir.mkdir(parents=True, exist_ok=False)
        report_path = output_dir / "r5-preflight-report.json"
        report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    except OSError as exc:
        fail(f"cannot write preflight report: {exc}")
    return report_path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command", choices=("preflight",), help="prepare and validate the immutable R5 pilot"
    )
    parser.add_argument(
        "--contract",
        type=Path,
        default=ROOT / "r0/anhoku-reboot/r5-fixed-data-exposure-contract.json",
    )
    parser.add_argument("--output-dir", type=Path, default=ROOT / "out/anhoku-reboot-r5")
    args = parser.parse_args()
    contract_path = args.contract if args.contract.is_absolute() else ROOT / args.contract
    output_dir = args.output_dir if args.output_dir.is_absolute() else ROOT / args.output_dir
    try:
        path = preflight(contract_path, output_dir)
    except PreflightError as exc:
        print(f"R5 preflight FAILED: {exc}", file=sys.stderr)
        return 1
    print(f"R5 preflight ready: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
