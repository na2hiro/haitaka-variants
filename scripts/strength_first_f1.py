#!/usr/bin/env python3
"""CPU preparation CLI for the bounded F1 loss ablation.

No command in this module imports a device runtime, rents an instance, trains,
or dispatches an engine search. GPU and live commands require later F1-owned
implementations and fresh grants; they are deliberately absent for now.
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import sys
from typing import Any

import strength_first_f1_core as f1


def freeze_inputs() -> dict[str, Any]:
    bound = f1.frozen_inputs()
    config = json.loads(f1.RECIPE.read_text())
    training_audit, labels = f1._audit_split("training", config, collect_labels=True)
    validation_audit, _ = f1._audit_split("validation", config, collect_labels=False)
    calibration = f1.calibrate(labels)
    calibration_path = f1.F1_ROOT / "calibration.json"
    existing_calibration = json.loads(calibration_path.read_text()) if calibration_path.exists() else {}
    calibration.update({
        "schema": "haitaka-anhoku-strength-first-f1-calibration-v1",
        "experimentId": f1.EXPERIMENT_ID,
        "createdAtUtc": existing_calibration.get("createdAtUtc", f1.utc_now()),
        "data": bound["trainingBinary"],
        "sidecar": bound["trainingSidecar"],
        "source": bound["trainingSource"],
        "selectionDataUsed": False,
        "validationDataUsed": False,
        "outcomesUsed": False,
    })
    calibration["trainingAudit"] = training_audit
    f1.immutable_json(calibration_path, calibration)
    contract_path = f1.F1_ROOT / "contract.json"
    existing_contract = json.loads(contract_path.read_text()) if contract_path.exists() else {}
    contract = {
        "schema": "haitaka-anhoku-strength-first-f1-contract-v1",
        "experimentId": f1.EXPERIMENT_ID,
        "createdAtUtc": existing_contract.get("createdAtUtc", f1.utc_now()),
        "runbook": f1.artifact(f1.ROOT / "plans/anhoku-nnue-further-strengthening-runbook.md"),
        "inputs": bound,
        "dataAudit": {"training": training_audit, "validation": validation_audit},
        "calibration": {"path": str(calibration_path.relative_to(f1.ROOT)), "sha256": f1.sha256(calibration_path)},
        "hypothesis": "distribution-scaled probability-space teacher regression",
        "arms": {"I": "immutable-Step10-u768", "C": "fresh-Huber", "T": "fresh-sigmoid-space"},
        "seed": 2026092501,
        "temperatureRule": "median-nonzero-abs-clipped-training-label-divided-by-ln3-float64-v1",
        "loss": {"predictionRuntimeScoreScale": 600.0, "targetClip": 12000.0, "normalizedScoreScale": 12000.0, "huberDelta": 0.1, "probabilityPrefactor": "8*(tau/12000)^2", "resultFieldsUsed": False},
        "training": {"stopUpdates": 1024, "scheduleHorizonUpdates": 4096, "checkpointUpdates": list(f1.UPDATES), "validationSelector": "common-Huber-global-minimum-plus-1e-12-earliest"},
        "openings": {"development": {"seed": 2026092502, "pairs": 128}, "holdout": {"seed": 2026092503, "pairs": 512}, "proposalCapPerPhase": 100000, "withinPhaseDeduplication": False},
        "grants": {"gpuFixtureAuthorized": False, "gpuTrainingAuthorized": False, "rentalAuthorized": False, "scoredGamesAuthorized": False, "deploymentAuthorized": False},
        "execution": {"gpuOccurred": False, "trainingOccurred": False, "gamesOccurred": False},
    }
    f1.immutable_json(contract_path, contract)
    return {"contract": f1.artifact(contract_path), "calibration": f1.artifact(calibration_path), "audit": contract["dataAudit"]}


def math_fixture() -> dict[str, Any]:
    from strength_first_f1_core import F1Error
    tau = 257.0 / math.log(3)
    checks = 0
    for raw, teacher in ((0, 0), (0.1, 50), (-0.1, -50), (2, 2500), (-2, -2500), (100, 30000), (-100, -30000)):
        values = f1.losses_and_raw_gradients(raw, teacher, tau)
        inverted = f1.losses_and_raw_gradients(-raw, -teacher, tau)
        for name in ("huber", "probability"):
            assert math.isclose(values[name], inverted[name], rel_tol=1e-12, abs_tol=1e-18)
            checks += 1
        for name in ("huberRawGradient", "probabilityRawGradient"):
            assert math.isclose(values[name], -inverted[name], rel_tol=1e-12, abs_tol=1e-18)
            checks += 1
        epsilon = 1e-5
        for name, gradient in (("huber", "huberRawGradient"), ("probability", "probabilityRawGradient")):
            finite_difference = (f1.losses_and_raw_gradients(raw + epsilon, teacher, tau)[name] - f1.losses_and_raw_gradients(raw - epsilon, teacher, tau)[name]) / (2 * epsilon)
            assert math.isclose(finite_difference, values[gradient], rel_tol=1e-5, abs_tol=1e-10)
            checks += 1
    assert f1.losses_and_raw_gradients(0, 0, tau)["probability"] == 0
    assert f1.losses_and_raw_gradients(0, 30000, tau)["target"] == 12000
    assert f1.losses_and_raw_gradients(0, -30000, tau)["target"] == -12000
    for bad in (float("nan"), float("inf"), -float("inf")):
        try:
            f1.losses_and_raw_gradients(0, bad, tau)
        except F1Error:
            checks += 1
        else:
            raise AssertionError("nonfinite teacher accepted")
    assert f1.calibrate([0, -10, 20, 30])["medianNonzeroAbsoluteScore"] == 20
    assert f1.calibrate([-10, 20])["medianNonzeroAbsoluteScore"] == 15
    for labels in ([], [0, 0], [float("nan")], [float("inf")]):
        try:
            f1.calibrate(labels)
        except F1Error:
            checks += 1
        else:
            raise AssertionError("degenerate temperature input accepted")
    near_zero = f1.losses_and_raw_gradients(1e-6, 0, tau)
    assert math.isclose(near_zero["probability"], near_zero["huber"], rel_tol=1e-7)
    dummy = {"path": "unused-by-selector-math", "bytes": 1, "sha256": "0" * 64}
    candidates = [{"update": update, "validationHuber": loss, "checkpoint": dummy, "export": dummy} for update, loss in zip(f1.UPDATES, (0.4, 0.3, 0.31, 0.2 + 0.5e-12, 0.2, 0.22, 0.21, 0.25))]
    assert f1.select_by_common_huber(candidates, verify_files=False)["selectedUpdate"] == 512
    candidates[3]["validationHuber"] = 0.2 + 1.5e-12
    assert f1.select_by_common_huber(candidates, verify_files=False)["selectedUpdate"] == 640
    checks += 3
    assert f1.pair_statistics([[{"valid": True, "scoreTreatment": 0.5, "genuinePlyCap": True}, {"valid": True, "scoreTreatment": 1, "genuinePlyCap": False}]])["conservativeScore"] == 0.5
    return {"schema": "haitaka-anhoku-strength-first-f1-cpu-math-fixture-v2", "createdAtUtc": f1.utc_now(), "checks": checks, "passed": True, "scope": "pure-math-and-scoring-only; real model/loader fixture recorded separately", "gpuOccurred": False, "trainingOccurred": False, "gamesOccurred": False}


def family_audit() -> dict[str, Any]:
    import strength_first_f1_openings as openings
    freeze = json.loads(openings.SOURCE_FREEZE.read_text())
    f1.require(freeze.get("status") == "passed" and freeze.get("protectedOutcomeTraversal") is False, "safe source freeze differs")
    normalizer = f1.ROOT / "haitaka_learn/src/step11_opening_gate.rs"
    f1.require(f1.sha256(normalizer) == openings.EXPECTED_NORMALIZER_HASH, "canonicalizer changed")
    inventories = {}
    family_sets = {}
    for split in ("training", "validation"):
        entry = next(item for item in freeze["entries"] if item["id"] == f"model-b-{split}")
        payload = entry["payloads"][0]
        f1.require(f1.artifact(f1.ROOT / payload["path"]) == payload, f"{split} family input changed")
        inv = entry["inventory"]
        path = Path(inv["path"])
        f1.require(path.stat().st_size == inv["bytes"] and f1.sha256(path) == inv["sha256"], f"{split} family inventory changed")
        values = set(openings._family_rows(path))
        f1.require(len(values) == (880000 if split == "training" else 60000), f"{split} family inventory count differs")
        inventories[split] = {"inventory": f1.artifact(path), "payload": payload, "families": len(values)}
        family_sets[split] = values
    overlap = len(family_sets["training"] & family_sets["validation"])
    f1.require(overlap == 0, "canonical training/validation family overlap")
    existing = f1.F1_ROOT / "preflight/family-audit.json"
    timestamp = json.loads(existing.read_text()).get("createdAtUtc") if existing.exists() else f1.utc_now()
    receipt = {"schema": "haitaka-anhoku-strength-first-f1-family-audit-v1", "createdAtUtc": timestamp, "experimentId": f1.EXPERIMENT_ID, "sourceFreeze": f1.artifact(openings.SOURCE_FREEZE), "normalizer": f1.artifact(normalizer), "inventories": inventories, "canonicalFamilyOverlap": overlap, "protectedOutcomeTraversal": False, "reusedQualifiedInventories": True, "productionTrainingOccurred": False, "gpuOccurred": False, "gamesOccurred": False}
    f1.immutable_json(existing, receipt)
    return receipt


def cpu_qualify() -> dict[str, Any]:
    """Verify archived CPU evidence, including all binary stage artifacts."""
    root = f1.F1_ROOT
    math_path = root / "preflight/cpu-math-fixture-v2.json"
    family_path = root / "preflight/family-audit.json"
    math_receipt = json.loads(math_path.read_text())
    family = json.loads(family_path.read_text())
    f1.require(math_receipt.get("passed") is True and math_receipt.get("checks") == 52, "analytic CPU fixture differs")
    f1.require(family.get("canonicalFamilyOverlap") == 0 and family.get("protectedOutcomeTraversal") is False, "family audit differs")
    real = {}
    initial = {}
    contexts = {}
    for arm in ("C", "T"):
        base = root / f"preflight/cpu-fixture/attempt-04/{arm}"
        report_path = base / "report.json"
        report = json.loads(report_path.read_text())
        f1.require(report.get("passed") is True and report.get("schema") == "haitaka-anhoku-strength-first-f1-real-cpu-fixture-v2" and all(report.get("checks", {}).values()), f"{arm} CPU resume fixture differs")
        f1.require(len(set(report["stageProcessIds"])) == 3 and report["optimizerStepMaximum"] == 12, f"{arm} process/lookahead evidence differs")
        for stage_binding in report["stages"].values():
            stage = json.loads(f1.verify_artifact(stage_binding).read_text())
            for key in ("state", "checkpoint", "export"):
                f1.verify_artifact(stage[key])
        real[arm] = f1.artifact(report_path)
        context_path = base / f"preflight/deterministic-cuda-v1/context-assembly-f1-cpu-{arm}-uninterrupted.json"
        context = json.loads(context_path.read_text())
        f1.require(context.get("status") == "built" and context.get("sameSourceHashes") is True and "evaluate.py" in context.get("sourceIdentities", {}), f"{arm} fresh context assembly differs")
        contexts[arm] = {"receipt": f1.artifact(context_path), "sourceIdentities": context["sourceIdentities"]}
        initial_path = root / f"preflight/cpu-fixture/attempt-03/{arm}/initial.json"
        init = json.loads(initial_path.read_text())
        for key in ("state", "checkpoint", "export"):
            f1.verify_artifact(init[key])
        initial[arm] = {"receipt": f1.artifact(initial_path), "identity": init["initialIdentity"]}
    f1.require(initial["C"]["identity"] == initial["T"]["identity"], "C/T CPU initial states differ")
    f1.require(contexts["C"]["sourceIdentities"] == contexts["T"]["sourceIdentities"], "C/T fresh context source hashes differ")
    output = root / "preflight/cpu-qualification-v2.json"
    prior_time = json.loads(output.read_text()).get("createdAtUtc") if output.exists() else f1.utc_now()
    receipt = {"schema": "haitaka-anhoku-strength-first-f1-cpu-qualification-v2", "experimentId": f1.EXPERIMENT_ID, "createdAtUtc": prior_time, "passed": True, "math": f1.artifact(math_path), "familyAudit": f1.artifact(family_path), "realFixture": real, "initial": initial, "freshContextAssemblies": {arm: contexts[arm]["receipt"] for arm in ("C", "T")}, "freshContextSourceHashesEqual": True, "cpuHost": "192.168.0.12", "cpuOnly": True, "gpuOccurred": False, "productionTrainingOccurred": False, "gamesOccurred": False}
    f1.immutable_json(output, receipt)
    return receipt


def select(path: Path, output: Path) -> dict[str, Any]:
    path = path.resolve()
    output = output.resolve()
    f1.require(path.parent.parent.parent == f1.F1_ROOT / "training" and path.name == "run.json", "selection input must be an F1 arm run receipt")
    value = json.loads(path.read_text())
    arm = value.get("arm")
    f1.require(arm in ("C", "T") and path == f1.F1_ROOT / f"training/arms/{arm}/run.json" and output == f1.F1_ROOT / f"training/arms/{arm}/selection.json", "selection arm/path differs")
    f1.require(value.get("experimentId") == f1.EXPERIMENT_ID and value.get("complete") is True and value.get("updates") == 1024 and value.get("scheduleHorizon") == 4096, "arm run incomplete")
    validations = value.get("validations", [])
    f1.require([item.get("update") for item in validations] == [0, *f1.UPDATES], "nine fixed validation evaluations missing")
    f1.require([item.get("update") for item in value["candidates"]] == list(f1.UPDATES), "eight eligible candidates missing")
    for candidate, validation in zip(value["candidates"], validations[1:]):
        f1.require(candidate["validationHuber"] == validation["validationHuber"] and candidate["validationProbability"] == validation["validationProbability"], "candidate validation metrics differ")
    if arm == "T":
        control = json.loads((f1.F1_ROOT / "training/arms/C/run.json").read_text())
        f1.require(control.get("complete") is True and control.get("traceSha256") == value.get("traceSha256"), "T/C exposure traces differ")
    selected = f1.select_by_common_huber(value["candidates"])
    previous = json.loads(output.read_text()) if output.exists() else {}
    receipt = {"schema": "haitaka-anhoku-strength-first-f1-selection-v1", "experimentId": f1.EXPERIMENT_ID, "arm": arm, "createdAtUtc": previous.get("createdAtUtc", f1.utc_now()), "input": f1.artifact(path), **selected}
    f1.immutable_json(output, receipt)
    return receipt


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("freeze-inputs", help="hash/audit immutable corpus and compute training-only temperature")
    sub.add_parser("math-fixture", help="run CPU-only analytic loss and scoring checks")
    sub.add_parser("family-audit", help="verify frozen training/validation canonical-family separation")
    sub.add_parser("cpu-qualify", help="verify archived analytic, family and real CPU fixtures")
    selector = sub.add_parser("select", help="select one arm's checkpoint from complete validation-only input")
    selector.add_argument("input", type=Path)
    selector.add_argument("output", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "freeze-inputs":
            result = freeze_inputs()
        elif args.command == "math-fixture":
            result = math_fixture()
            f1.immutable_json(f1.F1_ROOT / "preflight/cpu-math-fixture-v2.json", result)
        elif args.command == "family-audit":
            result = family_audit()
        elif args.command == "cpu-qualify":
            result = cpu_qualify()
        elif args.command == "select":
            result = select(args.input, args.output)
        else:
            raise AssertionError(args.command)
    except (f1.F1Error, OSError, ValueError, KeyError, AssertionError) as exc:
        print(f"F1 blocked: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
