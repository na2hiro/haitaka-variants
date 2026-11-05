#!/usr/bin/env python3
"""Versioned width-512 capacity-comparator trainer for Anhoku Step 3.

This runner reuses the pinned context construction in ``r5_trainer.py`` while
keeping R5's historical training behavior untouched.  Candidate training is
CUDA-only and requires a separate, explicit approval receipt.  ``self-test``
is CPU-only and exercises the pure schedule, partial-batch, shuffle, and resume
semantics without importing torch.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import math
import os
import pickle
import random
import shutil
import subprocess
import sys
import tempfile
import time
import types
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable

import numpy as np

import r5_trainer as r5


ROOT = Path(__file__).resolve().parents[1]
RUNNER_VERSION = "haitaka-anhoku-strength-first-width512-trainer-v1"
CONFIG_SCHEMA = "haitaka-anhoku-strength-first-experiment-v3"
STATE_SCHEMA = "haitaka-anhoku-strength-first-resume-state-v3"
METRICS_SCHEMA = "haitaka-anhoku-strength-first-checkpoint-metrics-v1"
APPROVAL_SCHEMA = "haitaka-explicit-gpu-rental-approval-v2"
CONTINUATION_SCHEMA = "haitaka-anhoku-strength-first-continuation-receipt-v3"
THROUGHPUT_SCHEMA = "haitaka-anhoku-strength-first-throughput-probe-v2"
SCREEN_SCHEMA = "haitaka-anhoku-strength-first-screen-receipt-v2"
SCREEN_GAME_SCHEMA = "haitaka-anhoku-strength-first-screen-game-v1"
CANDIDATE_RUNTIME_SCHEMA = "haitaka-anhoku-strength-first-candidate-runtime-v1"
CANDIDATE_EXPORT_SCHEMA = "haitaka-anhoku-strength-first-candidate-export-v1"
PARITY_SCHEMA = "haitaka-anhoku-strength-first-parity-receipt-v1"
RAW_MATCH_SCHEMA = "haitaka-anhoku-strength-first-raw-match-v1"
COMPARISON_SCHEMA = "haitaka-anhoku-strength-first-comparison-receipt-v1"
SELECTION_HISTORY_SCHEMA = "haitaka-anhoku-strength-first-selection-history-v2"
SELECTION_EXPORT_SCHEMA = "haitaka-anhoku-strength-first-selection-export-receipt-v1"
TRAINING_RUN_SCHEMA = "haitaka-anhoku-strength-first-training-run-v3"
STUDENT_T_975_DF63 = 1.9983405425207418


class StrengthFirstError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise StrengthFirstError(message)


def canonical_json(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot read JSON {path}: {exc}")
    if not isinstance(value, dict):
        fail(f"JSON root is not an object: {path}")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
    except OSError as exc:
        fail(f"cannot hash {path}: {exc}")
    return digest.hexdigest()


def workspace_path(value: str) -> Path:
    path = Path(value)
    return path if path.is_absolute() else ROOT / path


def display_path(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def exact_keys(value: Any, expected: set[str], where: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{where} must be an object")
    actual = set(value)
    if actual != expected:
        fail(f"{where} keys differ: missing={sorted(expected-actual)}, unused={sorted(actual-expected)}")
    return value


def exact(value: Any, expected: Any, where: str) -> None:
    if value != expected:
        fail(f"{where} is {value!r}, required {expected!r}")


def artifact_shape(value: Any, where: str, *, with_bytes: bool) -> dict[str, Any]:
    keys = {"path", "sha256", "bytes"} if with_bytes else {"path", "sha256"}
    item = exact_keys(value, keys, where)
    if not isinstance(item["path"], str) or not isinstance(item["sha256"], str):
        fail(f"{where} path/hash types are invalid")
    if with_bytes and (not isinstance(item["bytes"], int) or item["bytes"] < 0):
        fail(f"{where}.bytes must be a nonnegative integer")
    return item


def finite_number(value: Any, where: str, *, minimum: float | None = None, maximum: float | None = None) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(float(value)):
        fail(f"{where} must be a finite number")
    result = float(value)
    if minimum is not None and result < minimum:
        fail(f"{where} must be at least {minimum}")
    if maximum is not None and result > maximum:
        fail(f"{where} must be at most {maximum}")
    return result


def nonnegative_integer(value: Any, where: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        fail(f"{where} must be a nonnegative integer")
    return value


def timestamp(value: Any, where: str) -> datetime:
    if not isinstance(value, str) or not value:
        fail(f"{where} must be a nonempty ISO-8601 timestamp")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as exc:
        fail(f"{where} is not an ISO-8601 timestamp: {exc}")
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        fail(f"{where} must include a UTC offset")
    return parsed.astimezone(timezone.utc)


def same_number(actual: Any, expected: float, where: str) -> None:
    value = finite_number(actual, where)
    if not math.isclose(value, expected, rel_tol=1e-12, abs_tol=1e-12):
        fail(f"{where} is {value!r}, recomputed {expected!r}")


def positive_integer(value: Any, where: str) -> int:
    result = nonnegative_integer(value, where)
    if result == 0:
        fail(f"{where} must be positive")
    return result


def boolean(value: Any, where: str) -> bool:
    if not isinstance(value, bool):
        fail(f"{where} must be a boolean")
    return value


def atomic_write_bytes(path: Path, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    try:
        with temporary.open("wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        fsync_directory(path.parent)
    finally:
        temporary.unlink(missing_ok=True)


def atomic_write_json(path: Path, value: Any) -> None:
    atomic_write_bytes(path, json.dumps(value, indent=2, sort_keys=True).encode("utf-8") + b"\n")


def atomic_copy_file(source: Path, target: Path) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = target.with_name(f".{target.name}.{os.getpid()}.tmp")
    try:
        with source.open("rb") as input_stream, temporary.open("wb") as output_stream:
            shutil.copyfileobj(input_stream, output_stream, 1024 * 1024)
            output_stream.flush()
            os.fsync(output_stream.fileno())
        os.replace(temporary, target)
        fsync_directory(target.parent)
    finally:
        temporary.unlink(missing_ok=True)


def fsync_directory(path: Path) -> None:
    try:
        descriptor = os.open(path, os.O_RDONLY)
    except OSError as exc:
        fail(f"cannot open directory for durability sync {path}: {exc}")
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def validate_config(config: dict[str, Any]) -> None:
    exact_keys(config, {"schema", "experimentId", "status", "authorization", "trainer", "recipe", "strengthProtocol", "stopping", "budget", "costForecast", "output"}, "config")
    exact(config["schema"], CONFIG_SCHEMA, "config.schema")
    exact(config["experimentId"], "strength-first-step3-width512-capacity-seed-20260906-v1", "config.experimentId")
    exact(config["status"], "prepared-awaiting-gpu-approval-and-throughput-probe", "config.status")

    auth = exact_keys(config["authorization"], {"candidateTrainingAuthorized", "gpuRentalAuthorized", "gpuApprovalReceipt", "paidExecutionRequiresExplicitUserApproval", "approvalReceiptSchema", "approvedAction"}, "authorization")
    exact(auth, {"candidateTrainingAuthorized": False, "gpuRentalAuthorized": False, "gpuApprovalReceipt": None, "paidExecutionRequiresExplicitUserApproval": True, "approvalReceiptSchema": APPROVAL_SCHEMA, "approvedAction": "rent-and-run-this-exact-config-through-receipt-gated-boundaries"}, "authorization")

    trainer = exact_keys(config["trainer"], {"runnerVersion", "runner", "reusedR5Runner", "pinnedTrainerRevision", "contextInputs"}, "trainer")
    exact(trainer["runnerVersion"], RUNNER_VERSION, "trainer.runnerVersion")
    artifact_shape(trainer["runner"], "trainer.runner", with_bytes=False)
    artifact_shape(trainer["reusedR5Runner"], "trainer.reusedR5Runner", with_bytes=False)
    exact(trainer["pinnedTrainerRevision"], r5.PINNED_TRAINER, "trainer.pinnedTrainerRevision")
    if not isinstance(trainer["contextInputs"], list) or len(trainer["contextInputs"]) != 7:
        fail("trainer.contextInputs must contain the seven pinned context inputs")
    for index, item in enumerate(trainer["contextInputs"]):
        artifact_shape(item, f"trainer.contextInputs[{index}]", with_bytes=False)

    recipe = exact_keys(config["recipe"], {"seed", "initialization", "featureFamily", "width", "data", "loader", "objective", "optimizer", "schedule", "evaluation", "checkpoints", "diagnostics", "export", "realCpuFixture"}, "recipe")
    exact(recipe["seed"], 20260906, "recipe.seed")
    exact(recipe["initialization"], "fresh-scratch", "recipe.initialization")
    exact(recipe["featureFamily"], "HalfKAv2^+DonorSingleEff", "recipe.featureFamily")
    exact(exact_keys(recipe["width"], {"l1", "l2", "l3"}, "recipe.width"), {"l1": 512, "l2": 16, "l3": 32}, "recipe.width")

    data = exact_keys(recipe["data"], {"training", "validation"}, "recipe.data")
    for split, records in (("training", 880000), ("validation", 60000)):
        item = exact_keys(data[split], {"records", "entryBytes", "manifest", "binary", "sidecar"}, f"recipe.data.{split}")
        exact(item["records"], records, f"recipe.data.{split}.records")
        exact(item["entryBytes"], 72, f"recipe.data.{split}.entryBytes")
        for name in ("manifest", "binary", "sidecar"):
            artifact_shape(item[name], f"recipe.data.{split}.{name}", with_bytes=True)

    loader = exact_keys(recipe["loader"], {"batchSize", "workers", "cyclic", "filtered", "randomFenSkipping", "finiteFinalPartialBatch", "trainingOrder", "validationOrder"}, "recipe.loader")
    exact(loader, {"batchSize": 16384, "workers": 1, "cyclic": False, "filtered": False, "randomFenSkipping": 0, "finiteFinalPartialBatch": True, "trainingOrder": "pcg64dxsm-seed-epoch-record-permutation-v1", "validationOrder": "fixed-source-order-once-per-evaluation-v1"}, "recipe.loader")

    objective = exact_keys(recipe["objective"], {"kind", "teacherOnly", "resultLossEnabled", "lambda", "targetScoreClip", "normalizedScoreScale", "huberDeltaNormalized", "modelOutputToRuntimeScore"}, "recipe.objective")
    exact(objective, {"kind": "robust-clipped-huber-v1", "teacherOnly": True, "resultLossEnabled": False, "lambda": 1.0, "targetScoreClip": 12000.0, "normalizedScoreScale": 12000.0, "huberDeltaNormalized": 0.1, "modelOutputToRuntimeScore": 600.0}, "recipe.objective")

    optimizer = exact_keys(recipe["optimizer"], {"kind", "betas", "epsilon", "weightDecay", "lookaheadAlpha", "lookaheadK", "nSmaThreshold", "gradientCentralizationLocation", "gradientCentralizationEnabled", "gradientCentralizationConvOnly", "parameterGroupNames", "parameterGroupLearningRateMultipliers", "weightProjection"}, "recipe.optimizer")
    exact(optimizer["kind"], "Ranger", "recipe.optimizer.kind")
    exact(optimizer["betas"], [0.9, 0.999], "recipe.optimizer.betas")
    exact(optimizer["epsilon"], 1e-7, "recipe.optimizer.epsilon")
    exact(optimizer["weightDecay"], 0.0, "recipe.optimizer.weightDecay")
    exact(optimizer["lookaheadAlpha"], 0.5, "recipe.optimizer.lookaheadAlpha")
    exact(optimizer["lookaheadK"], 6, "recipe.optimizer.lookaheadK")
    exact(optimizer["nSmaThreshold"], 5, "recipe.optimizer.nSmaThreshold")
    exact(optimizer["gradientCentralizationLocation"], False, "recipe.optimizer.gradientCentralizationLocation")
    exact(optimizer["gradientCentralizationEnabled"], False, "recipe.optimizer.gradientCentralizationEnabled")
    exact(optimizer["gradientCentralizationConvOnly"], False, "recipe.optimizer.gradientCentralizationConvOnly")
    exact(optimizer["parameterGroupNames"], ["feature-transformer", "l1-factor-weight", "l1-weight", "l1-bias", "l2-weight", "l2-bias", "output-weight", "output-bias"], "recipe.optimizer.parameterGroupNames")
    exact(optimizer["parameterGroupLearningRateMultipliers"], [1.0] * 6 + [0.1, 0.1], "recipe.optimizer.parameterGroupLearningRateMultipliers")
    exact(optimizer["weightProjection"], "pinned-model-_clip_weights-before-and-after-update-v1", "recipe.optimizer.weightProjection")

    schedule = exact_keys(recipe["schedule"], {"kind", "baseLearningRate", "minimumLearningRate", "warmupUpdates", "maximumUpdates", "firstUpdateLearningRate", "warmupEndpointLearningRate", "finalUpdateLearningRate"}, "recipe.schedule")
    exact(schedule, {"kind": "linear-warmup-then-cosine-by-optimizer-update-v1", "baseLearningRate": 0.0015, "minimumLearningRate": 0.00005, "warmupUpdates": 64, "maximumUpdates": 4096, "firstUpdateLearningRate": 0.0000234375, "warmupEndpointLearningRate": 0.0015, "finalUpdateLearningRate": 0.00005}, "recipe.schedule")

    evaluation = exact_keys(recipe["evaluation"], {"everyUpdates", "initialValidation", "validationRecordsPerEvaluation", "batchSize", "repeatWithinEvaluation"}, "recipe.evaluation")
    exact(evaluation, {"everyUpdates": 128, "initialValidation": True, "validationRecordsPerEvaluation": 60000, "batchSize": 16384, "repeatWithinEvaluation": False}, "recipe.evaluation")
    checkpoints = exact_keys(recipe["checkpoints"], {"throughputProbeUpdate", "scheduledUpdates", "retainBestValidation", "retainFinal", "atomicLastState", "lastStateEveryEvaluation", "portableFullPrecision"}, "recipe.checkpoints")
    exact(checkpoints, {"throughputProbeUpdate": 128, "scheduledUpdates": [256, 1024, 2048, 4096], "retainBestValidation": True, "retainFinal": True, "atomicLastState": True, "lastStateEveryEvaluation": True, "portableFullPrecision": True}, "recipe.checkpoints")
    diagnostics = exact_keys(recipe["diagnostics"], {"updateMagnitudeEveryUpdates", "logScoreRange", "logPhaseLoss", "logTacticalAndCheckLoss", "logDonorStateLoss", "logFeatureCoverage", "logOutputScaleAndSaturation", "logGradientMagnitude", "logUpdateMagnitude", "logExposureCounters", "logTiming"}, "recipe.diagnostics")
    exact(diagnostics["updateMagnitudeEveryUpdates"], 128, "recipe.diagnostics.updateMagnitudeEveryUpdates")
    if any(value is not True for key, value in diagnostics.items() if key != "updateMagnitudeEveryUpdates"):
        fail("every declared diagnostic is mandatory")
    export = exact_keys(recipe["export"], {"format", "automaticScheduledUpdates", "selectedCheckpointMustBeExplicit", "selectionHistorySchema", "selectionMustReferenceRunHistory", "sourceCheckpointMustBeRecordedArtifact", "atomicCommit", "candidateExportSchema", "candidateRuntimeSchema", "parityReceiptSchema", "rawMatchSchema", "comparisonReceiptSchema", "fullPrecisionIntegerReferenceNativeWasmParityRequiredBeforeGames", "quantizationErrorReportedSeparately"}, "recipe.export")
    exact(export, {"format": "pinned-serialize-py-nnue-v1", "automaticScheduledUpdates": [256, 1024, 2048, 4096], "selectedCheckpointMustBeExplicit": True, "selectionHistorySchema": SELECTION_HISTORY_SCHEMA, "selectionMustReferenceRunHistory": True, "sourceCheckpointMustBeRecordedArtifact": True, "atomicCommit": True, "candidateExportSchema": CANDIDATE_EXPORT_SCHEMA, "candidateRuntimeSchema": CANDIDATE_RUNTIME_SCHEMA, "parityReceiptSchema": PARITY_SCHEMA, "rawMatchSchema": RAW_MATCH_SCHEMA, "comparisonReceiptSchema": COMPARISON_SCHEMA, "fullPrecisionIntegerReferenceNativeWasmParityRequiredBeforeGames": True, "quantizationErrorReportedSeparately": True}, "recipe.export")

    fixture = exact_keys(recipe["realCpuFixture"], {"device", "source", "records", "entryBytes", "batchSize", "finiteBatchSizesPerEpoch", "width", "seed", "maximumUpdates", "interruptAfterUpdates", "schedule", "requirements", "qualificationLimits", "outputDirectory"}, "recipe.realCpuFixture")
    exact(fixture["device"], "cpu", "recipe.realCpuFixture.device")
    exact(fixture["source"], "first-nine-record-prefix-of-hash-bound-training-binary-v1", "recipe.realCpuFixture.source")
    exact(fixture["records"], 9, "recipe.realCpuFixture.records")
    exact(fixture["entryBytes"], 72, "recipe.realCpuFixture.entryBytes")
    exact(fixture["batchSize"], 5, "recipe.realCpuFixture.batchSize")
    exact(fixture["finiteBatchSizesPerEpoch"], [5, 4], "recipe.realCpuFixture.finiteBatchSizesPerEpoch")
    exact(exact_keys(fixture["width"], {"l1", "l2", "l3"}, "recipe.realCpuFixture.width"), {"l1": 16, "l2": 8, "l3": 8}, "recipe.realCpuFixture.width")
    exact(fixture["seed"], 20260906, "recipe.realCpuFixture.seed")
    exact(fixture["maximumUpdates"], 12, "recipe.realCpuFixture.maximumUpdates")
    exact(fixture["interruptAfterUpdates"], 1, "recipe.realCpuFixture.interruptAfterUpdates")
    exact(exact_keys(fixture["schedule"], {"kind", "baseLearningRate", "minimumLearningRate", "warmupUpdates", "maximumUpdates"}, "recipe.realCpuFixture.schedule"), {"kind": "linear-warmup-then-cosine-by-optimizer-update-v1", "baseLearningRate": 0.0015, "minimumLearningRate": 0.00005, "warmupUpdates": 2, "maximumUpdates": 12}, "recipe.realCpuFixture.schedule")
    requirements = exact_keys(fixture["requirements"], {"actualPinnedNnue", "actualRanger", "actualProductionLoader", "actualAtomicCheckpointResume", "exactUninterruptedResumeWeights", "exactUninterruptedResumePredictions", "exactUninterruptedResumeExport", "tinyOverfitLossMustDecrease", "finitePartialBatchMustBeObserved", "comprehensiveFiniteChecks", "atomicExportCommit"}, "recipe.realCpuFixture.requirements")
    if any(value is not True for value in requirements.values()):
        fail("every real CPU fixture requirement is mandatory")
    limits = exact_keys(fixture["qualificationLimits"], {"productionTrainingLoopExercised", "cudaRngAndOptimizerStateExercised", "productionMetricsStateCommitOrderingExercised", "productionContinuationAndIntermediateLegExercised", "nativeWasmParityExercised", "scope"}, "recipe.realCpuFixture.qualificationLimits")
    expected_limits = {"productionTrainingLoopExercised": False, "cudaRngAndOptimizerStateExercised": False, "productionMetricsStateCommitOrderingExercised": False, "productionContinuationAndIntermediateLegExercised": False, "nativeWasmParityExercised": False, "scope": "cpu-fixture-only-not-production-or-cuda-qualification"}
    exact(limits, expected_limits, "recipe.realCpuFixture.qualificationLimits")

    strength = exact_keys(config["strengthProtocol"], {"scheduleId", "openingSuite", "openingIds", "cycles", "pairOrder", "totalPairs", "colorSwap", "opponent", "threads", "concurrency", "randomPlies", "maximumPly", "scheduledScreens", "finalEqualNodeComparisonPairs", "finalFixedTimeScreenPairs", "fixedTimeMillisecondsPerMove", "uncertainty", "finalSelection", "executionGates"}, "strengthProtocol")
    artifact_shape(strength["openingSuite"], "strengthProtocol.openingSuite", with_bytes=True)
    expected_ids = [f"anhoku-v3-{index:03d}" for index in range(1, 65)]
    exact(strength["openingIds"], expected_ids, "strengthProtocol.openingIds")
    exact(strength["cycles"], 2, "strengthProtocol.cycles")
    exact(strength["pairOrder"], "cycle-major-then-opening-id-ascending-v1", "strengthProtocol.pairOrder")
    exact(strength["totalPairs"], 128, "strengthProtocol.totalPairs")
    exact(strength["colorSwap"], "anhoku-rotate-180-and-swap-colors-v1", "strengthProtocol.colorSwap")
    exact(strength["opponent"], "handcrafted-production-evaluator", "strengthProtocol.opponent")
    exact(exact_keys(strength["scheduledScreens"], {"updates", "pairsPerScreen", "nodesPerMove", "openingCycle"}, "strengthProtocol.scheduledScreens"), {"updates": [256, 1024, 2048], "pairsPerScreen": 64, "nodesPerMove": 10000, "openingCycle": 0}, "strengthProtocol.scheduledScreens")
    exact(strength["threads"], 1, "strengthProtocol.threads")
    exact(strength["concurrency"], 1, "strengthProtocol.concurrency")
    exact(strength["randomPlies"], 0, "strengthProtocol.randomPlies")
    exact(strength["maximumPly"], 200, "strengthProtocol.maximumPly")
    exact(strength["finalEqualNodeComparisonPairs"], 128, "strengthProtocol.finalEqualNodeComparisonPairs")
    exact(strength["finalFixedTimeScreenPairs"], 128, "strengthProtocol.finalFixedTimeScreenPairs")
    exact(strength["fixedTimeMillisecondsPerMove"], 100, "strengthProtocol.fixedTimeMillisecondsPerMove")
    uncertainty = exact_keys(strength["uncertainty"], {"independentUnit", "independentGroups", "colorSwappedGamesAggregatedWithinPair", "repeatedCyclesAggregatedWithinOpeningGroup", "repeatedCyclesAreNotIndependent", "intervalMethod", "screenStoppingBound", "screenStoppingThreshold", "sequentialUse"}, "strengthProtocol.uncertainty")
    exact(uncertainty, {"independentUnit": "opening-id", "independentGroups": 64, "colorSwappedGamesAggregatedWithinPair": True, "repeatedCyclesAggregatedWithinOpeningGroup": True, "repeatedCyclesAreNotIndependent": True, "intervalMethod": "two-sided-95-percent-student-t-interval-over-64-opening-group-mean-scores-v1", "screenStoppingBound": "upper-endpoint-of-the-two-sided-95-percent-grouped-interval", "screenStoppingThreshold": 0.45, "sequentialUse": "development-budget-heuristic-only-no-confirmatory-claim"}, "strengthProtocol.uncertainty")
    selection = exact_keys(strength["finalSelection"], {"maximumComparedCheckpoints", "candidateRoles", "deduplicateSameCheckpoint", "comparisonPairsPerCandidate", "comparisonUsesFrozenGroupedSchedule", "rankingOrder", "postSelectionScreen", "selectionHistoryRetained", "confirmatoryClaimAllowed"}, "strengthProtocol.finalSelection")
    exact(selection["maximumComparedCheckpoints"], 2, "strengthProtocol.finalSelection.maximumComparedCheckpoints")
    exact(selection["candidateRoles"], ["lowest-validation-loss", "highest-scheduled-screen-score"], "strengthProtocol.finalSelection.candidateRoles")
    exact(selection["deduplicateSameCheckpoint"], True, "strengthProtocol.finalSelection.deduplicateSameCheckpoint")
    exact(selection["comparisonPairsPerCandidate"], 128, "strengthProtocol.finalSelection.comparisonPairsPerCandidate")
    exact(selection["comparisonUsesFrozenGroupedSchedule"], True, "strengthProtocol.finalSelection.comparisonUsesFrozenGroupedSchedule")
    exact(selection["rankingOrder"], ["higher-fixed-time-score-if-measured-for-every-candidate", "higher-equal-node-score", "lower-runtime-cost", "lower-validation-loss", "earlier-optimizer-update"], "strengthProtocol.finalSelection.rankingOrder")
    exact(selection["postSelectionScreen"], "selected-checkpoint-only-128-pairs-at-100-ms-per-move", "strengthProtocol.finalSelection.postSelectionScreen")
    exact(selection["selectionHistoryRetained"], True, "strengthProtocol.finalSelection.selectionHistoryRetained")
    exact(selection["confirmatoryClaimAllowed"], False, "strengthProtocol.finalSelection.confirmatoryClaimAllowed")
    gates = exact_keys(strength["executionGates"], {"defaultFullRunAllowed", "freshRunStopUpdate", "continuationSequence", "receiptSchema", "throughputEvidenceSchema", "screenEvidenceSchema", "screenGameSchema", "receiptDecision", "receiptEvidenceMustBeHashBound", "activeLegPersistedForIntermediateResume", "noOrchestratorRequired"}, "strengthProtocol.executionGates")
    exact(gates["defaultFullRunAllowed"], False, "strengthProtocol.executionGates.defaultFullRunAllowed")
    exact(gates["freshRunStopUpdate"], 128, "strengthProtocol.executionGates.freshRunStopUpdate")
    exact(gates["receiptSchema"], CONTINUATION_SCHEMA, "strengthProtocol.executionGates.receiptSchema")
    exact(gates["throughputEvidenceSchema"], THROUGHPUT_SCHEMA, "strengthProtocol.executionGates.throughputEvidenceSchema")
    exact(gates["screenEvidenceSchema"], SCREEN_SCHEMA, "strengthProtocol.executionGates.screenEvidenceSchema")
    exact(gates["screenGameSchema"], SCREEN_GAME_SCHEMA, "strengthProtocol.executionGates.screenGameSchema")
    exact(gates["receiptDecision"], "continue", "strengthProtocol.executionGates.receiptDecision")
    exact(gates["receiptEvidenceMustBeHashBound"], True, "strengthProtocol.executionGates.receiptEvidenceMustBeHashBound")
    exact(gates["activeLegPersistedForIntermediateResume"], True, "strengthProtocol.executionGates.activeLegPersistedForIntermediateResume")
    exact(gates["noOrchestratorRequired"], True, "strengthProtocol.executionGates.noOrchestratorRequired")
    expected_gates = [(128, 256, ["throughput-probe-report"]), (256, 1024, ["throughput-probe-report", "strength-screen-u00000256"]), (1024, 2048, ["throughput-probe-report", "strength-screen-u00001024"]), (2048, 4096, ["throughput-probe-report", "strength-screen-u00002048"])]
    if not isinstance(gates["continuationSequence"], list) or len(gates["continuationSequence"]) != len(expected_gates):
        fail("strengthProtocol.executionGates.continuationSequence has the wrong length")
    for index, (item, expected_gate) in enumerate(zip(gates["continuationSequence"], expected_gates)):
        exact_keys(item, {"fromUpdate", "toUpdate", "requiredEvidenceRoles"}, f"strengthProtocol.executionGates.continuationSequence[{index}]")
        exact((item["fromUpdate"], item["toUpdate"], item["requiredEvidenceRoles"]), expected_gate, f"strengthProtocol.executionGates.continuationSequence[{index}]")

    stopping = exact_keys(config["stopping"], {"numericalOrCorrectnessFailure", "strengthRule", "singleLosingScreenStops", "budgetStopRecordedAsConvergence"}, "stopping")
    exact(stopping, {"numericalOrCorrectnessFailure": "stop-immediately", "strengthRule": "stop-after-two-successive-screens-with-paired-upper-95-score-bound-below-0.45", "singleLosingScreenStops": False, "budgetStopRecordedAsConvergence": False}, "stopping")
    budget = exact_keys(config["budget"], {"maximumFullTrainingRuns", "maximumTotalGpuHours", "maximumTotalRentalCostUsd", "initialCycleStorageBytes", "firstRentalEnvelopeGpuHours", "firstRentalEnvelopeUsd"}, "budget")
    exact(budget["maximumFullTrainingRuns"], 3, "budget.maximumFullTrainingRuns")
    exact(budget["maximumTotalGpuHours"], 24.0, "budget.maximumTotalGpuHours")
    exact(budget["maximumTotalRentalCostUsd"], 25.0, "budget.maximumTotalRentalCostUsd")
    exact(budget["initialCycleStorageBytes"], 26843545600, "budget.initialCycleStorageBytes")
    exact(budget["firstRentalEnvelopeGpuHours"], 4.0, "budget.firstRentalEnvelopeGpuHours")
    exact(budget["firstRentalEnvelopeUsd"], 5.0, "budget.firstRentalEnvelopeUsd")
    forecast = exact_keys(config["costForecast"], {"status", "historicalAnchor", "liveQuote", "probe", "firstRun", "threeRunTournament", "basis"}, "costForecast")
    anchor = exact_keys(forecast["historicalAnchor"], {"gpu", "updates", "approximateWallSeconds", "source", "historicalCombinedOfferRateUsdPerHour"}, "costForecast.historicalAnchor")
    positive_integer(anchor["updates"], "costForecast.historicalAnchor.updates")
    finite_number(anchor["approximateWallSeconds"], "costForecast.historicalAnchor.approximateWallSeconds", minimum=0.0)
    finite_number(anchor["historicalCombinedOfferRateUsdPerHour"], "costForecast.historicalAnchor.historicalCombinedOfferRateUsdPerHour", minimum=0.0)
    quote = exact_keys(forecast["liveQuote"], {"provider", "offerId", "gpu", "diskGigabytes", "combinedHourlyRateUsd", "observedAt", "validUntil", "binding", "use"}, "costForecast.liveQuote")
    positive_integer(quote["offerId"], "costForecast.liveQuote.offerId")
    finite_number(quote["diskGigabytes"], "costForecast.liveQuote.diskGigabytes", minimum=0.0)
    finite_number(quote["combinedHourlyRateUsd"], "costForecast.liveQuote.combinedHourlyRateUsd", minimum=0.0)
    if timestamp(quote["validUntil"], "costForecast.liveQuote.validUntil") <= timestamp(quote["observedAt"], "costForecast.liveQuote.observedAt"):
        fail("costForecast.liveQuote validity must end after observation")
    exact(quote["binding"], False, "costForecast.liveQuote.binding")
    probe = exact_keys(forecast["probe"], {"updates", "maximumGpuHours", "mustReplaceForecastBeforeContinuingPastUpdate", "evidenceSchema", "forecastLowerBoundMethod", "tournamentRuns", "requiredMeasuredFields"}, "costForecast.probe")
    exact(probe["updates"], 128, "costForecast.probe.updates")
    finite_number(probe["maximumGpuHours"], "costForecast.probe.maximumGpuHours", minimum=0.0, maximum=budget["firstRentalEnvelopeGpuHours"])
    exact(probe["mustReplaceForecastBeforeContinuingPastUpdate"], 256, "costForecast.probe.mustReplaceForecastBeforeContinuingPastUpdate")
    exact(probe["evidenceSchema"], THROUGHPUT_SCHEMA, "costForecast.probe.evidenceSchema")
    exact(probe["forecastLowerBoundMethod"], "elapsed-probe-plus-remaining-at-measured-rate-plus-declared-fixed-overhead-v1", "costForecast.probe.forecastLowerBoundMethod")
    exact(probe["tournamentRuns"], 3, "costForecast.probe.tournamentRuns")
    exact(probe["requiredMeasuredFields"], ["elapsedGpuHours", "updatesPerGpuHour", "revisedFirstRunGpuHours", "revisedFirstRunCostUsd", "revisedTournamentGpuHours", "revisedTournamentCostUsd"], "costForecast.probe.requiredMeasuredFields")
    first_run = exact_keys(forecast["firstRun"], {"forecastTrainingGpuHours", "reservedTotalRentedUptimeHours", "totalRentedUptimeBreakdownHours", "maximumCombinedHourlyRateUsd", "reservedComputeAndDiskUsd", "reservedTransferUsd", "reservedContingencyUsd", "totalEnvelopeUsd", "peakNewStorageBytes"}, "costForecast.firstRun")
    breakdown = exact_keys(first_run["totalRentedUptimeBreakdownHours"], {"setupBuildAndRealFixture", "throughputProbe", "candidateTrainingAndValidation", "compilationAndResumeRestarts", "parityExportAndTransfers", "paidWaitingForCpuScreens", "contingency"}, "costForecast.firstRun.totalRentedUptimeBreakdownHours")
    for name, value in first_run.items():
        if name != "totalRentedUptimeBreakdownHours":
            finite_number(value, f"costForecast.firstRun.{name}", minimum=0.0)
    for name, value in breakdown.items():
        finite_number(value, f"costForecast.firstRun.totalRentedUptimeBreakdownHours.{name}", minimum=0.0)
    if abs(sum(breakdown.values()) - first_run["reservedTotalRentedUptimeHours"]) > 1e-12:
        fail("first-run rented uptime breakdown does not sum to its reserve")
    if first_run["reservedTotalRentedUptimeHours"] > budget["firstRentalEnvelopeGpuHours"] or first_run["totalEnvelopeUsd"] > budget["firstRentalEnvelopeUsd"]:
        fail("first-run forecast exceeds the first-rental envelope")
    tournament = exact_keys(forecast["threeRunTournament"], {"forecastTrainingGpuHours", "reservedTotalRentedUptimeHours", "forecastCostUsdAtMaximumAcceptedRate", "reservedTotalCostUsd", "forecastPeakNewStorageBytes"}, "costForecast.threeRunTournament")
    for name, value in tournament.items():
        finite_number(value, f"costForecast.threeRunTournament.{name}", minimum=0.0)
    if tournament["forecastTrainingGpuHours"] > budget["maximumTotalGpuHours"] or tournament["reservedTotalCostUsd"] > budget["maximumTotalRentalCostUsd"]:
        fail("tournament forecast exceeds its frozen budget")
    output = exact_keys(config["output"], {"directory", "freshRunRefusesExistingState", "resolvedConfigCopiedBeforeTraining", "receiptsCopiedIntoRun"}, "output")
    exact(output, {"directory": "out/anhoku-strength-first/step3-width512-capacity-seed-20260906-v1", "freshRunRefusesExistingState": True, "resolvedConfigCopiedBeforeTraining": True, "receiptsCopiedIntoRun": True}, "output")


def verify_artifact(item: dict[str, Any], where: str) -> Path:
    path = workspace_path(item["path"])
    if not path.is_file():
        fail(f"{where} is missing: {path}")
    if "bytes" in item and path.stat().st_size != item["bytes"]:
        fail(f"{where} byte size differs")
    if sha256(path) != item["sha256"]:
        fail(f"{where} SHA-256 differs")
    return path


def verify_inputs(config: dict[str, Any]) -> None:
    trainer = config["trainer"]
    verify_artifact(trainer["runner"], "trainer.runner")
    verify_artifact(trainer["reusedR5Runner"], "trainer.reusedR5Runner")
    for index, item in enumerate(trainer["contextInputs"]):
        verify_artifact(item, f"trainer.contextInputs[{index}]")
    for split in ("training", "validation"):
        data = config["recipe"]["data"][split]
        for name in ("manifest", "binary", "sidecar"):
            verify_artifact(data[name], f"recipe.data.{split}.{name}")
        if data["binary"]["bytes"] != data["records"] * data["entryBytes"]:
            fail(f"{split} binary byte count is not records * entryBytes")
        Metadata(workspace_path(data["sidecar"]["path"]), data["records"])
    verify_artifact(config["strengthProtocol"]["openingSuite"], "strengthProtocol.openingSuite")


def learning_rate(schedule: dict[str, Any], update: int) -> float:
    if update < 1 or update > schedule["maximumUpdates"]:
        fail(f"schedule update {update} is outside [1,{schedule['maximumUpdates']}]")
    if update <= schedule["warmupUpdates"]:
        return schedule["baseLearningRate"] * update / schedule["warmupUpdates"]
    progress = (update - schedule["warmupUpdates"]) / (schedule["maximumUpdates"] - schedule["warmupUpdates"])
    return schedule["minimumLearningRate"] + 0.5 * (schedule["baseLearningRate"] - schedule["minimumLearningRate"]) * (1.0 + math.cos(math.pi * progress))


def epoch_permutation(records: int, seed: int, epoch: int) -> np.ndarray:
    if records <= 0 or seed < 0 or epoch < 0:
        fail("invalid epoch permutation arguments")
    generator = np.random.Generator(np.random.PCG64DXSM(np.random.SeedSequence([seed, epoch])))
    return generator.permutation(records).astype(np.int64, copy=False)


@dataclass
class Cursor:
    epoch: int = 0
    record: int = 0

    def advance(self, count: int, records: int) -> None:
        if count <= 0 or self.record + count > records:
            fail(f"invalid cursor advance {self.record}+{count}>{records}")
        self.record += count
        if self.record == records:
            self.epoch += 1
            self.record = 0


def batch_spans(records: int, batch_size: int, cursor: Cursor, updates: int) -> tuple[list[tuple[int, int, int]], Cursor]:
    current = Cursor(cursor.epoch, cursor.record)
    spans: list[tuple[int, int, int]] = []
    for _ in range(updates):
        count = min(batch_size, records - current.record)
        spans.append((current.epoch, current.record, count))
        current.advance(count, records)
    return spans, current


def materialize_epoch(source: Path, target: Path, records: int, entry_bytes: int, order: np.ndarray) -> None:
    # Always regenerate.  A same-size cache entry is not evidence that it was
    # built from this source hash and epoch permutation.
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = target.with_name(f".{target.name}.{os.getpid()}.tmp")
    original = np.memmap(source, dtype=np.uint8, mode="r", shape=(records, entry_bytes))
    try:
        with temporary.open("wb") as stream:
            for start in range(0, records, 32768):
                stream.write(original[order[start:start + 32768]].tobytes())
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, target)
    finally:
        del original
        temporary.unlink(missing_ok=True)


def finite_batches(nnue_dataset: Any, feature: str, filename: Path, records: int, batch_size: int, device: Any, loader: dict[str, Any], skip: int = 0) -> Iterable[tuple[Any, ...]]:
    dataset = nnue_dataset.SparseBatchDataset(feature, str(filename), batch_size, cyclic=loader["cyclic"], num_workers=loader["workers"], filtered=loader["filtered"], random_fen_skipping=loader["randomFenSkipping"], device=device)
    seen = 0
    for batch in dataset:
        count = int(batch[0].shape[0])
        if count <= 0 or seen + count > records:
            fail(f"finite loader overran {filename}: {seen}+{count}>{records}")
        before = seen
        seen += count
        if seen <= skip:
            continue
        if before < skip:
            fail(f"resume cursor {skip} is not at a loader batch boundary")
        yield batch
    if seen != records:
        fail(f"finite loader consumed {seen} records from {filename}, expected {records}")


class Metadata:
    PHASES = ("early", "middle", "late", "terminal-adjacent")
    DONOR_STATES = ("none", "active")

    def __init__(self, path: Path, records: int):
        self.phase = np.empty(records, dtype=np.uint8)
        self.flags = np.empty(records, dtype=np.uint8)
        self.donor = np.empty(records, dtype=np.uint8)
        phase_codes = {name: index for index, name in enumerate(self.PHASES)}
        donor_codes = {name: index for index, name in enumerate(self.DONOR_STATES)}
        count = 0
        with path.open("r", encoding="utf-8") as stream:
            for count, line in enumerate(stream, 1):
                if count > records:
                    fail(f"sidecar {path} contains more than {records} rows")
                row = json.loads(line)
                position = row["position"]
                try:
                    self.phase[count - 1] = phase_codes[position["phase"]]
                    self.donor[count - 1] = donor_codes[position["donorState"]]
                except KeyError as exc:
                    fail(f"sidecar {path} has unsupported metadata value: {exc}")
                self.flags[count - 1] = int(bool(position["tactical"])) | (int(bool(position["inCheck"])) << 1)
        if count != records:
            fail(f"sidecar {path} contains {count} rows, expected {records}")

    def labels(self, category: str, indices: np.ndarray) -> np.ndarray:
        if category == "phase":
            return self.phase[indices]
        if category == "tactical":
            return self.flags[indices] & 1
        if category == "check":
            return (self.flags[indices] >> 1) & 1
        if category == "donor":
            return self.donor[indices]
        raise AssertionError(category)


class Window:
    def __init__(self, objective: dict[str, Any]):
        self.score_clip = objective["targetScoreClip"]
        self.output_scale = objective["modelOutputToRuntimeScore"]
        self.records = 0
        self.loss_sum = 0.0
        self.absolute_score_error_sum = 0.0
        self.prediction_min = math.inf
        self.prediction_max = -math.inf
        self.target_min = math.inf
        self.target_max = -math.inf
        self.raw_output_min = math.inf
        self.raw_output_max = -math.inf
        self.prediction_saturated = 0
        self.target_clipped = 0
        self.gradient_norms: list[float] = []
        self.gradient_maxima: list[float] = []
        self.update_samples: list[dict[str, float]] = []
        self.active_rows: set[int] = set()
        self.active_feature_occurrences = 0
        self.groups: dict[str, dict[int, list[float]]] = {name: {} for name in ("phase", "tactical", "check", "donor")}

    def add(self, losses: np.ndarray, predictions: np.ndarray, targets: np.ndarray, raw: np.ndarray, metadata: Metadata, indices: np.ndarray, gradient_norm: float | None = None, gradient_max: float | None = None, update: dict[str, float] | None = None) -> None:
        for name, values in (("losses", losses), ("predictions", predictions), ("targets", targets), ("raw outputs", raw)):
            if not np.isfinite(values).all():
                fail(f"metrics window contains non-finite {name}")
        for name, value in (("gradient norm", gradient_norm), ("gradient maximum", gradient_max)):
            if value is not None:
                finite_number(value, name, minimum=0.0)
        if update is not None:
            for name, value in update.items():
                finite_number(value, f"update magnitude {name}", minimum=0.0)
        count = len(losses)
        self.records += count
        self.loss_sum += float(losses.sum())
        absolute = np.abs(predictions - targets)
        self.absolute_score_error_sum += float(absolute.sum())
        self.prediction_min = min(self.prediction_min, float(predictions.min()))
        self.prediction_max = max(self.prediction_max, float(predictions.max()))
        self.target_min = min(self.target_min, float(targets.min()))
        self.target_max = max(self.target_max, float(targets.max()))
        self.raw_output_min = min(self.raw_output_min, float(raw.min()))
        self.raw_output_max = max(self.raw_output_max, float(raw.max()))
        self.prediction_saturated += int(np.count_nonzero(np.abs(predictions) >= self.score_clip))
        self.target_clipped += int(np.count_nonzero(np.abs(targets) >= self.score_clip))
        if gradient_norm is not None:
            self.gradient_norms.append(gradient_norm)
            self.gradient_maxima.append(float(gradient_max))
        if update is not None:
            self.update_samples.append(update)
        for category in self.groups:
            labels = metadata.labels(category, indices)
            for code in np.unique(labels):
                mask = labels == code
                slot = self.groups[category].setdefault(int(code), [0.0, 0.0, 0.0])
                slot[0] += int(mask.sum())
                slot[1] += float(losses[mask].sum())
                slot[2] += float(absolute[mask].sum())

    def add_coverage(self, torch: Any, batch: tuple[Any, ...]) -> None:
        for values in (batch[2], batch[4]):
            valid = values[values >= 0]
            self.active_feature_occurrences += int(valid.numel())
            self.active_rows.update(int(item) for item in torch.unique(valid).detach().cpu().tolist())

    def report(self) -> dict[str, Any]:
        if self.records <= 0:
            fail("cannot report an empty metrics window")
        names = {"phase": self.PHASE_NAMES, "tactical": ("false", "true"), "check": ("false", "true"), "donor": Metadata.DONOR_STATES}
        grouped: dict[str, Any] = {}
        for category, slots in self.groups.items():
            grouped[category] = {}
            for code, values in sorted(slots.items()):
                count, loss_sum, error_sum = values
                grouped[category][names[category][code]] = {"records": int(count), "teacherOnlyLoss": loss_sum / count, "rawScoreMae": error_sum / count}
        return {
            "records": self.records,
            "teacherOnlyLoss": self.loss_sum / self.records,
            "rawScoreMae": self.absolute_score_error_sum / self.records,
            "scoreRange": {"prediction": [self.prediction_min, self.prediction_max], "targetAfterClip": [self.target_min, self.target_max]},
            "outputScale": {"runtimeScorePerRawUnit": self.output_scale, "rawOutputRange": [self.raw_output_min, self.raw_output_max]},
            "saturation": {"predictionAtTargetClip": self.prediction_saturated / self.records, "targetAtClip": self.target_clipped / self.records},
            "byStatus": grouped,
            "featureCoverage": {"uniqueActiveRows": len(self.active_rows), "activeFeatureOccurrences": self.active_feature_occurrences},
            "gradientMagnitude": None if not self.gradient_norms else {"l2Mean": float(np.mean(self.gradient_norms)), "l2Maximum": max(self.gradient_norms), "absoluteMaximum": max(self.gradient_maxima)},
            "updateMagnitudeSamples": self.update_samples,
        }

    PHASE_NAMES = Metadata.PHASES


def loss_tensors(torch: Any, model: Any, batch: tuple[Any, ...], objective: dict[str, Any]) -> tuple[Any, Any, Any, Any]:
    us, them, white, white_values, black, black_values, _outcome, score, psqt, stacks = batch
    raw = model(us, them, white, white_values, black, black_values, psqt, stacks)
    prediction = raw * objective["modelOutputToRuntimeScore"]
    target = score.clamp(-objective["targetScoreClip"], objective["targetScoreClip"])
    error = (prediction - target) / objective["normalizedScoreScale"]
    absolute = error.abs()
    delta = objective["huberDeltaNormalized"]
    losses = torch.where(absolute <= delta, 0.5 * error.square(), delta * (absolute - 0.5 * delta))
    for name, tensor in (("model output", raw), ("prediction", prediction), ("target", target), ("normalized error", error), ("per-record loss", losses)):
        if not bool(torch.isfinite(tensor).all()):
            fail(f"non-finite {name}")
    return losses.mean(), losses[:, 0], prediction[:, 0], target[:, 0]


def gradient_magnitude(torch: Any, model: Any) -> tuple[float, float]:
    squares = torch.zeros((), device=next(model.parameters()).device)
    maximum = 0.0
    for parameter in model.parameters():
        if parameter.grad is not None:
            gradient = parameter.grad.detach().float()
            if not bool(torch.isfinite(gradient).all()):
                fail("non-finite gradient")
            squares += gradient.square().sum()
            maximum = max(maximum, float(gradient.abs().max().cpu()))
    result = float(squares.sqrt().cpu()), maximum
    finite_number(result[0], "gradient L2 norm", minimum=0.0)
    finite_number(result[1], "gradient absolute maximum", minimum=0.0)
    return result


def assert_finite_nested(torch: Any, value: Any, where: str) -> None:
    if torch.is_tensor(value):
        if (value.is_floating_point() or value.is_complex()) and not bool(torch.isfinite(value).all()):
            fail(f"{where} contains a non-finite tensor")
        return
    if isinstance(value, dict):
        for key, item in value.items():
            assert_finite_nested(torch, item, f"{where}.{key}")
        return
    if isinstance(value, (list, tuple)):
        for index, item in enumerate(value):
            assert_finite_nested(torch, item, f"{where}[{index}]")
        return
    if isinstance(value, (float, np.floating)) and not math.isfinite(float(value)):
        fail(f"{where} contains a non-finite scalar")


def assert_finite_training_state(torch: Any, model: Any, optimizer: Any, where: str) -> None:
    assert_finite_nested(torch, model.state_dict(), f"{where}.model")
    assert_finite_nested(torch, optimizer.state_dict(), f"{where}.optimizer")


def update_magnitude(torch: Any, before: list[Any], model: Any) -> dict[str, float]:
    delta_squares = torch.zeros((), device=before[0].device)
    parameter_squares = torch.zeros((), device=before[0].device)
    maximum = 0.0
    for old, parameter in zip(before, model.parameters()):
        delta = parameter.detach().float() - old
        delta_squares += delta.square().sum()
        parameter_squares += parameter.detach().float().square().sum()
        maximum = max(maximum, float(delta.abs().max().cpu()))
    norm = float(delta_squares.sqrt().cpu())
    parameter_norm = float(parameter_squares.sqrt().cpu())
    result = {"l2": norm, "absoluteMaximum": maximum, "parameterL2": parameter_norm, "relativeL2": norm / max(parameter_norm, 1e-30)}
    for name, value in result.items():
        finite_number(value, f"update magnitude {name}", minimum=0.0)
    return result


def configured_optimizer(model: Any, recipe: dict[str, Any]) -> Any:
    configured = model.configure_optimizers()
    optimizer = configured[0][0] if isinstance(configured[0], list) else configured[0]
    declared = recipe["optimizer"]
    if type(optimizer).__name__ != declared["kind"] or len(optimizer.param_groups) != 8:
        fail("pinned model returned an optimizer that differs from the resolved contract")
    expected_parameters = [
        list(model.input.parameters()),
        [model.layer_stacks.l1_fact.weight],
        [model.layer_stacks.l1.weight],
        [model.layer_stacks.l1.bias],
        [model.layer_stacks.l2.weight],
        [model.layer_stacks.l2.bias],
        [model.layer_stacks.output.weight],
        [model.layer_stacks.output.bias],
    ]
    for name, group, expected_group in zip(declared["parameterGroupNames"], optimizer.param_groups, expected_parameters):
        if [id(item) for item in group["params"]] != [id(item) for item in expected_group]:
            fail(f"optimizer parameter group {name!r} differs from the resolved mapping")
    base = recipe["schedule"]["baseLearningRate"]
    observed = [group["lr"] / base for group in optimizer.param_groups]
    if any(abs(a - b) > 1e-12 for a, b in zip(observed, declared["parameterGroupLearningRateMultipliers"])):
        fail(f"optimizer group LR multipliers differ: {observed}")
    checks = ((optimizer.defaults["betas"], tuple(declared["betas"]), "betas"), (optimizer.defaults["eps"], declared["epsilon"], "epsilon"), (optimizer.defaults["weight_decay"], declared["weightDecay"], "weight decay"), (optimizer.alpha, declared["lookaheadAlpha"], "lookahead alpha"), (optimizer.k, declared["lookaheadK"], "lookahead k"), (optimizer.N_sma_threshold, declared["nSmaThreshold"], "N_sma threshold"), (optimizer.gc_loc, declared["gradientCentralizationLocation"], "GC location"), (optimizer.use_gc, declared["gradientCentralizationEnabled"], "GC enabled"), (optimizer.gc_conv_only, declared["gradientCentralizationConvOnly"], "GC conv-only"))
    for actual, expected, name in checks:
        if actual != expected:
            fail(f"optimizer {name} differs: {actual!r} != {expected!r}")
    return optimizer


def set_learning_rates(optimizer: Any, recipe: dict[str, Any], update: int) -> float:
    base = learning_rate(recipe["schedule"], update)
    for group, multiplier in zip(optimizer.param_groups, recipe["optimizer"]["parameterGroupLearningRateMultipliers"]):
        group["lr"] = base * multiplier
    return base


def seed_everything(torch: Any, seed: int) -> None:
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)
    if torch.cuda.is_available():
        torch.cuda.manual_seed_all(seed)
    torch.use_deterministic_algorithms(True)
    torch.backends.cudnn.benchmark = False
    torch.backends.cudnn.deterministic = True
    torch.set_num_threads(1)


def rng_state(torch: Any) -> dict[str, Any]:
    return {"python": random.getstate(), "numpy": np.random.get_state(), "torch": torch.random.get_rng_state(), "torchCuda": torch.cuda.get_rng_state_all() if torch.cuda.is_available() else None}


def restore_rng(torch: Any, state: dict[str, Any]) -> None:
    random.setstate(state["python"])
    np.random.set_state(state["numpy"])
    torch.random.set_rng_state(state["torch"].cpu())
    if state["torchCuda"] is not None:
        if not torch.cuda.is_available():
            fail("resume state contains CUDA RNG state on a CPU-only environment")
        torch.cuda.set_rng_state_all([item.cpu() for item in state["torchCuda"]])


def atomic_torch_save(torch: Any, value: Any, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    try:
        torch.save(value, temporary)
        with temporary.open("rb") as stream:
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        fsync_directory(path.parent)
    finally:
        temporary.unlink(missing_ok=True)


def save_portable_model(torch: Any, model: Any, path: Path) -> dict[str, Any]:
    if path.exists():
        # A crash may leave an immutable checkpoint just before last-state.pt
        # commits it.  Reuse it only after exact tensor comparison with the
        # deterministically replayed model.
        try:
            existing = torch.load(path, map_location="cpu", weights_only=False)
        except (OSError, RuntimeError, TypeError, ValueError) as exc:
            fail(f"cannot verify orphan checkpoint {path}: {exc}")
        current_state = model.state_dict()
        existing_state = existing.state_dict()
        if set(current_state) != set(existing_state) or any(
            not torch.equal(current_state[name].detach().cpu(), existing_state[name].detach().cpu())
            for name in current_state
        ):
            fail(f"immutable checkpoint conflicts with replayed model: {path}")
        return {"path": display_path(path), "bytes": path.stat().st_size, "sha256": sha256(path), "recoveredOrphan": True}
    portable = copy.deepcopy(model).cpu().eval()
    # The pinned model caches this as an ordinary tensor attribute, so
    # Module.cpu() does not migrate it.  It is derived on the next forward.
    portable.layer_stacks.idx_offset = None
    devices = r5.tensor_devices(torch, portable)
    if devices != {"cpu"}:
        fail(f"portable checkpoint retained unexpected tensor devices: {sorted(devices)}")
    atomic_torch_save(torch, portable, path)
    del portable
    return {"path": display_path(path), "bytes": path.stat().st_size, "sha256": sha256(path)}


def export_checkpoint(torch: Any, checkpoint: Path, output: Path, context: Path, recipe: dict[str, Any], experiment_id: str) -> dict[str, Any]:
    try:
        source = torch.load(checkpoint, map_location="cpu", weights_only=False)
    except (OSError, RuntimeError, TypeError, ValueError) as exc:
        fail(f"cannot inspect export checkpoint {checkpoint}: {exc}")
    state = source.state_dict() if hasattr(source, "state_dict") else source
    assert_finite_nested(torch, state, "export checkpoint")
    del source, state
    output.parent.mkdir(parents=True, exist_ok=True)
    existing = output.is_file()
    target = output.with_name(f".{output.stem}.{os.getpid()}.serialize.tmp{output.suffix}")
    target.unlink(missing_ok=True)
    started = time.monotonic()
    try:
        subprocess.run([sys.executable, "serialize.py", str(checkpoint), str(target), "--features", recipe["featureFamily"], "--description", f"Haitaka Anhoku {experiment_id} {recipe['width']['l1']}x{recipe['width']['l2']}x{recipe['width']['l3']}"], cwd=context, check=True)
    except (OSError, subprocess.CalledProcessError) as exc:
        target.unlink(missing_ok=True)
        fail(f"checkpoint export failed: {exc}")
    if not target.is_file() or target.stat().st_size == 0:
        target.unlink(missing_ok=True)
        fail("checkpoint serializer did not produce a nonempty network")
    with target.open("rb") as stream:
        os.fsync(stream.fileno())
    if existing:
        try:
            if sha256(target) != sha256(output):
                fail(f"immutable export conflicts with deterministic replay: {output}")
        finally:
            target.unlink(missing_ok=True)
    else:
        os.replace(target, output)
        fsync_directory(output.parent)
    serializer = context / "serialize.py"
    return {
        "sourceCheckpoint": {"path": display_path(checkpoint), "bytes": checkpoint.stat().st_size, "sha256": sha256(checkpoint)},
        "serializer": {"path": "serialize.py", "bytes": serializer.stat().st_size, "sha256": sha256(serializer), "pinnedTrainerRevision": r5.PINNED_TRAINER},
        "integerNetwork": {"path": display_path(output), "bytes": output.stat().st_size, "sha256": sha256(output)},
        "seconds": time.monotonic() - started,
        "recoveredOrphan": existing,
        "parityStatus": "pending-required-before-games",
        "quantizationErrorStatus": "pending-separate-measurement",
    }


def evaluate(torch: Any, model: Any, nnue_dataset: Any, recipe: dict[str, Any], metadata: Metadata, device: Any) -> dict[str, Any]:
    data = recipe["data"]["validation"]
    filename = workspace_path(data["binary"]["path"])
    window = Window(recipe["objective"])
    prediction_digest = hashlib.sha256()
    offset = 0
    model.eval()
    with torch.no_grad():
        for batch in finite_batches(nnue_dataset, recipe["featureFamily"], filename, data["records"], recipe["evaluation"]["batchSize"], device, recipe["loader"]):
            _mean, losses, predictions, targets = loss_tensors(torch, model, batch, recipe["objective"])
            count = int(losses.shape[0])
            indices = np.arange(offset, offset + count, dtype=np.int64)
            raw = predictions / recipe["objective"]["modelOutputToRuntimeScore"]
            np_predictions = predictions.detach().cpu().numpy().astype("<f8")
            prediction_digest.update(np_predictions.tobytes())
            window.add(losses.detach().cpu().numpy(), np_predictions, targets.detach().cpu().numpy(), raw.detach().cpu().numpy(), metadata, indices)
            window.add_coverage(torch, batch)
            offset += count
    if offset != data["records"]:
        fail("validation did not consume exactly one fixed-order pass")
    report = window.report()
    report["order"] = "fixed-source-order-once"
    report["predictionSha256"] = prediction_digest.hexdigest()
    return report


def append_committed_row(path: Path, row: dict[str, Any]) -> None:
    existing: list[dict[str, Any]] = []
    if path.exists():
        existing = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    same = [item for item in existing if item["optimizerUpdates"] == row["optimizerUpdates"]]
    if same:
        if canonical_json(same[-1]) != canonical_json(row):
            fail("metrics journal conflicts with committed last state")
        return
    if existing and existing[-1]["optimizerUpdates"] > row["optimizerUpdates"]:
        fail("metrics journal is ahead of committed last state")
    with path.open("ab") as stream:
        stream.write(canonical_json(row))
        stream.flush()
        os.fsync(stream.fileno())


def config_sha256(config_path: Path) -> str:
    return sha256(config_path)


def is_run_local_receipt_path(path: Path, config: dict[str, Any]) -> bool:
    receipt_root = (workspace_path(config["output"]["directory"]) / "receipts").resolve()
    try:
        path.resolve().relative_to(receipt_root)
        return True
    except ValueError:
        return False


def validate_approval(path: Path, config: dict[str, Any], config_path: Path, *, now: datetime | None = None) -> dict[str, Any]:
    receipt = load_json(path)
    exact_keys(receipt, {"schema", "experimentId", "configSha256", "gpuRentalApproved", "action", "approvedAt", "validUntil", "commercialEnvelope"}, "GPU approval receipt")
    exact(receipt["schema"], APPROVAL_SCHEMA, "approval.schema")
    exact(receipt["experimentId"], config["experimentId"], "approval.experimentId")
    exact(receipt["configSha256"], config_sha256(config_path), "approval.configSha256")
    exact(receipt["gpuRentalApproved"], True, "approval.gpuRentalApproved")
    exact(receipt["action"], config["authorization"]["approvedAction"], "approval.action")
    approved_at = timestamp(receipt["approvedAt"], "approval.approvedAt")
    valid_until = timestamp(receipt["validUntil"], "approval.validUntil")
    quote = config["costForecast"]["liveQuote"]
    quote_at = timestamp(quote["observedAt"], "costForecast.liveQuote.observedAt")
    quote_until = timestamp(quote["validUntil"], "costForecast.liveQuote.validUntil")
    if approved_at < quote_at or approved_at > quote_until or valid_until <= approved_at or valid_until > quote_until:
        fail("approval time is outside the exact quoted offer's validity window")
    current_time = datetime.now(timezone.utc) if now is None else now.astimezone(timezone.utc)
    if current_time > valid_until:
        fail("GPU approval receipt has expired")
    envelope = exact_keys(receipt["commercialEnvelope"], {"provider", "offerId", "gpu", "diskGigabytes", "quotedCombinedHourlyRateUsd", "maximumCombinedHourlyRateUsd", "maximumRentedUptimeHours", "maximumTotalCostUsd"}, "approval.commercialEnvelope")
    expected = {
        "provider": quote["provider"],
        "offerId": quote["offerId"],
        "gpu": quote["gpu"],
        "diskGigabytes": quote["diskGigabytes"],
        "quotedCombinedHourlyRateUsd": quote["combinedHourlyRateUsd"],
        "maximumCombinedHourlyRateUsd": config["costForecast"]["firstRun"]["maximumCombinedHourlyRateUsd"],
        "maximumRentedUptimeHours": config["budget"]["firstRentalEnvelopeGpuHours"],
        "maximumTotalCostUsd": config["budget"]["firstRentalEnvelopeUsd"],
    }
    exact(envelope, expected, "approval.commercialEnvelope")
    return {"path": str(path), "sha256": sha256(path), "approvedAt": receipt["approvedAt"], "validUntil": receipt["validUntil"], "action": receipt["action"], "configSha256": receipt["configSha256"], "commercialEnvelope": envelope}


def validate_throughput_evidence(path: Path, config: dict[str, Any], config_path: Path, *, require_continue: bool = True) -> dict[str, Any]:
    report = load_json(path)
    exact_keys(report, {"schema", "experimentId", "configSha256", "measuredAt", "sourceCheckpoint", "offer", "measurement", "revisedForecast", "decision"}, "throughput evidence")
    exact(report["schema"], THROUGHPUT_SCHEMA, "throughput.schema")
    exact(report["experimentId"], config["experimentId"], "throughput.experimentId")
    exact(report["configSha256"], config_sha256(config_path), "throughput.configSha256")
    measured_at = timestamp(report["measuredAt"], "throughput.measuredAt")
    quote = config["costForecast"]["liveQuote"]
    if measured_at < timestamp(quote["observedAt"], "quote.observedAt") or measured_at > timestamp(quote["validUntil"], "quote.validUntil"):
        fail("throughput measurement is outside the exact offer quote window")
    source = exact_keys(report["sourceCheckpoint"], {"update", "path", "sha256"}, "throughput.sourceCheckpoint")
    exact(source["update"], config["costForecast"]["probe"]["updates"], "throughput.sourceCheckpoint.update")
    source_path = verify_artifact({"path": source["path"], "sha256": source["sha256"]}, "throughput.sourceCheckpoint")
    expected_source_path = workspace_path(config["output"]["directory"]) / "checkpoints" / f"throughput-probe-u{source['update']:08d}.pt"
    if source_path.resolve() != expected_source_path.resolve():
        fail("throughput evidence does not reference this experiment's immutable probe checkpoint")
    exact(report["offer"], {"provider": quote["provider"], "offerId": quote["offerId"], "gpu": quote["gpu"], "diskGigabytes": quote["diskGigabytes"], "combinedHourlyRateUsd": quote["combinedHourlyRateUsd"], "observedAt": quote["observedAt"], "validUntil": quote["validUntil"]}, "throughput.offer")
    measurement = exact_keys(report["measurement"], {"startUpdate", "endUpdate", "elapsedGpuHours", "updatesPerGpuHour"}, "throughput.measurement")
    exact(measurement["startUpdate"], 0, "throughput.measurement.startUpdate")
    exact(measurement["endUpdate"], config["costForecast"]["probe"]["updates"], "throughput.measurement.endUpdate")
    hours = finite_number(measurement["elapsedGpuHours"], "throughput.measurement.elapsedGpuHours", minimum=1e-12, maximum=config["costForecast"]["probe"]["maximumGpuHours"])
    same_number(measurement["updatesPerGpuHour"], measurement["endUpdate"] / hours, "throughput.measurement.updatesPerGpuHour")
    revised = exact_keys(report["revisedForecast"], {"derivation", "revisedFirstRunGpuHours", "revisedFirstRunCostUsd", "revisedTournamentGpuHours", "revisedTournamentCostUsd", "maximumPlannedUpdate", "fitsFirstRunEnvelope", "fitsTournamentEnvelope"}, "throughput.revisedForecast")
    maximum_planned_update = positive_integer(revised["maximumPlannedUpdate"], "throughput.revisedForecast.maximumPlannedUpdate")
    if maximum_planned_update < measurement["endUpdate"] or maximum_planned_update > config["recipe"]["schedule"]["maximumUpdates"]:
        fail("throughput revised maximum planned update is outside the trained schedule")
    remaining_updates = maximum_planned_update - measurement["endUpdate"]
    minimum_remaining_hours = remaining_updates / float(measurement["updatesPerGpuHour"])
    fixed_overhead_sources = ["setupBuildAndRealFixture", "compilationAndResumeRestarts", "parityExportAndTransfers", "paidWaitingForCpuScreens", "contingency"]
    breakdown = config["costForecast"]["firstRun"]["totalRentedUptimeBreakdownHours"]
    declared_fixed_overhead_hours = sum(float(breakdown[name]) for name in fixed_overhead_sources)
    minimum_first_hours = hours + minimum_remaining_hours + declared_fixed_overhead_hours
    tournament_runs = config["costForecast"]["probe"]["tournamentRuns"]
    minimum_tournament_hours = minimum_first_hours * tournament_runs
    accepted_rate = config["costForecast"]["firstRun"]["maximumCombinedHourlyRateUsd"]
    minimum_first_cost = minimum_first_hours * accepted_rate
    minimum_tournament_cost = minimum_tournament_hours * accepted_rate
    derivation = exact_keys(revised["derivation"], {"method", "completedProbeGpuHours", "remainingUpdates", "measuredUpdatesPerGpuHour", "minimumRemainingMarginalGpuHours", "declaredFixedOverheadSources", "declaredFixedOverheadGpuHours", "minimumFirstRunGpuHours", "tournamentRuns", "minimumTournamentGpuHours", "pricingRateUsdPerHour", "minimumFirstRunCostUsd", "minimumTournamentCostUsd"}, "throughput.revisedForecast.derivation")
    exact(derivation["method"], config["costForecast"]["probe"]["forecastLowerBoundMethod"], "throughput.revisedForecast.derivation.method")
    same_number(derivation["completedProbeGpuHours"], hours, "throughput.revisedForecast.derivation.completedProbeGpuHours")
    exact(derivation["remainingUpdates"], remaining_updates, "throughput.revisedForecast.derivation.remainingUpdates")
    same_number(derivation["measuredUpdatesPerGpuHour"], measurement["updatesPerGpuHour"], "throughput.revisedForecast.derivation.measuredUpdatesPerGpuHour")
    same_number(derivation["minimumRemainingMarginalGpuHours"], minimum_remaining_hours, "throughput.revisedForecast.derivation.minimumRemainingMarginalGpuHours")
    exact(derivation["declaredFixedOverheadSources"], fixed_overhead_sources, "throughput.revisedForecast.derivation.declaredFixedOverheadSources")
    same_number(derivation["declaredFixedOverheadGpuHours"], declared_fixed_overhead_hours, "throughput.revisedForecast.derivation.declaredFixedOverheadGpuHours")
    same_number(derivation["minimumFirstRunGpuHours"], minimum_first_hours, "throughput.revisedForecast.derivation.minimumFirstRunGpuHours")
    exact(derivation["tournamentRuns"], tournament_runs, "throughput.revisedForecast.derivation.tournamentRuns")
    same_number(derivation["minimumTournamentGpuHours"], minimum_tournament_hours, "throughput.revisedForecast.derivation.minimumTournamentGpuHours")
    same_number(derivation["pricingRateUsdPerHour"], accepted_rate, "throughput.revisedForecast.derivation.pricingRateUsdPerHour")
    same_number(derivation["minimumFirstRunCostUsd"], minimum_first_cost, "throughput.revisedForecast.derivation.minimumFirstRunCostUsd")
    same_number(derivation["minimumTournamentCostUsd"], minimum_tournament_cost, "throughput.revisedForecast.derivation.minimumTournamentCostUsd")
    first_hours = finite_number(revised["revisedFirstRunGpuHours"], "throughput.revisedForecast.revisedFirstRunGpuHours", minimum=hours)
    first_cost = finite_number(revised["revisedFirstRunCostUsd"], "throughput.revisedForecast.revisedFirstRunCostUsd", minimum=0.0)
    tournament_hours = finite_number(revised["revisedTournamentGpuHours"], "throughput.revisedForecast.revisedTournamentGpuHours", minimum=first_hours)
    tournament_cost = finite_number(revised["revisedTournamentCostUsd"], "throughput.revisedForecast.revisedTournamentCostUsd", minimum=first_cost)
    if first_hours < minimum_first_hours or tournament_hours < minimum_tournament_hours or first_cost < minimum_first_cost or tournament_cost < minimum_tournament_cost:
        fail("throughput revised forecast is below the measured-rate lower bound")
    first_fits = first_hours <= config["budget"]["firstRentalEnvelopeGpuHours"] and first_cost <= config["budget"]["firstRentalEnvelopeUsd"]
    tournament_fits = tournament_hours <= config["budget"]["maximumTotalGpuHours"] and tournament_cost <= config["budget"]["maximumTotalRentalCostUsd"]
    exact(revised["fitsFirstRunEnvelope"], first_fits, "throughput.revisedForecast.fitsFirstRunEnvelope")
    exact(revised["fitsTournamentEnvelope"], tournament_fits, "throughput.revisedForecast.fitsTournamentEnvelope")
    expected_decision = "continue" if first_fits and tournament_fits else "stop-budget"
    exact(report["decision"], expected_decision, "throughput.decision")
    if require_continue and expected_decision != "continue":
        fail("measured throughput no longer fits the approved budgets")
    return {"schema": report["schema"], "path": str(path), "sha256": sha256(path), "decision": report["decision"], "measurement": measurement, "revisedForecast": revised, "maximumPlannedUpdate": maximum_planned_update}


def grouped_screen_statistics(scores: list[float]) -> dict[str, float]:
    if len(scores) != 64:
        fail("strength screen must contain exactly 64 independent opening-group scores")
    array = np.asarray(scores, dtype=np.float64)
    if not np.isfinite(array).all() or np.any(array < 0.0) or np.any(array > 1.0):
        fail("strength screen group scores must be finite and within [0,1]")
    mean = float(array.mean())
    standard_deviation = float(array.std(ddof=1))
    standard_error = standard_deviation / math.sqrt(len(scores))
    margin = STUDENT_T_975_DF63 * standard_error
    return {"meanScore": mean, "sampleStandardDeviation": standard_deviation, "standardError": standard_error, "criticalValue": STUDENT_T_975_DF63, "lower95": mean - margin, "upper95": mean + margin}


def validate_recorded_checkpoint(run_path: Path, config: dict[str, Any], config_path: Path, expected_checkpoint: dict[str, Any]) -> dict[str, Any]:
    run = load_json(run_path)
    required = {"schema", "experimentId", "configSha256", "complete", "stopReason", "optimizerUpdates", "examplesSeen", "equivalentPasses", "dataCursor", "bestValidation", "artifacts", "timingSeconds", "approval", "continuationReceipts", "activeLeg", "approvedMaximumPlannedUpdate", "nextContinuationRequiresNewReceipt", "selectedCheckpointExport", "paidExecutionDoesNotAuthorizeFurtherRuns"}
    exact_keys(run, required, "candidate export training run")
    exact(run["schema"], TRAINING_RUN_SCHEMA, "candidate export training run.schema")
    exact(run["experimentId"], config["experimentId"], "candidate export training run.experimentId")
    exact(run["configSha256"], config_sha256(config_path), "candidate export training run.configSha256")
    update = nonnegative_integer(run["optimizerUpdates"], "candidate export training run.optimizerUpdates")
    if expected_checkpoint["update"] > update:
        fail("candidate export checkpoint is newer than its bound training run")
    if not isinstance(run["artifacts"], list):
        fail("candidate export training run.artifacts must be a list")
    matches = []
    for artifact in run["artifacts"]:
        if not isinstance(artifact, dict) or artifact.get("role") not in {"best-validation", "throughput-probe", "scheduled-strength-screen", "final"}:
            continue
        if (artifact.get("update"), artifact.get("path"), artifact.get("sha256")) == (expected_checkpoint["update"], expected_checkpoint["path"], expected_checkpoint["sha256"]):
            matches.append(artifact)
    if len(matches) != 1:
        fail("candidate export checkpoint is not one unique checkpoint recorded by its bound training run")
    verify_artifact({"path": expected_checkpoint["path"], "sha256": expected_checkpoint["sha256"]}, "candidate export checkpoint")
    return run


def validate_candidate_export(path: Path, config: dict[str, Any], config_path: Path, expected_checkpoint: dict[str, Any]) -> dict[str, Any]:
    receipt = load_json(path)
    exact_keys(receipt, {"schema", "experimentId", "configSha256", "createdAt", "run", "checkpoint", "format", "sourceCheckpoint", "serializer", "integerNetwork", "status"}, "candidate export receipt")
    exact(receipt["schema"], CANDIDATE_EXPORT_SCHEMA, "candidate export receipt.schema")
    exact(receipt["experimentId"], config["experimentId"], "candidate export receipt.experimentId")
    exact(receipt["configSha256"], config_sha256(config_path), "candidate export receipt.configSha256")
    timestamp(receipt["createdAt"], "candidate export receipt.createdAt")
    exact(receipt["checkpoint"], expected_checkpoint, "candidate export receipt.checkpoint")
    run_item = artifact_shape(receipt["run"], "candidate export receipt.run", with_bytes=False)
    run_path = verify_artifact(run_item, "candidate export receipt.run")
    training_run = validate_recorded_checkpoint(run_path, config, config_path, expected_checkpoint)
    exact(receipt["format"], config["recipe"]["export"]["format"], "candidate export receipt.format")
    source = artifact_shape(receipt["sourceCheckpoint"], "candidate export receipt.sourceCheckpoint", with_bytes=True)
    exact(source["path"], expected_checkpoint["path"], "candidate export receipt.sourceCheckpoint.path")
    exact(source["sha256"], expected_checkpoint["sha256"], "candidate export receipt.sourceCheckpoint.sha256")
    source_path = verify_artifact(source, "candidate export receipt.sourceCheckpoint")
    serializer = exact_keys(receipt["serializer"], {"path", "bytes", "sha256", "pinnedTrainerRevision"}, "candidate export receipt.serializer")
    exact(serializer["path"], "serialize.py", "candidate export receipt.serializer.path")
    positive_integer(serializer["bytes"], "candidate export receipt.serializer.bytes")
    if not isinstance(serializer["sha256"], str) or len(serializer["sha256"]) != 64:
        fail("candidate export receipt.serializer.sha256 must be a SHA-256 hex digest")
    try:
        bytes.fromhex(serializer["sha256"])
    except ValueError:
        fail("candidate export receipt.serializer.sha256 must be hexadecimal")
    exact(serializer["pinnedTrainerRevision"], config["trainer"]["pinnedTrainerRevision"], "candidate export receipt.serializer.pinnedTrainerRevision")
    network = artifact_shape(receipt["integerNetwork"], "candidate export receipt.integerNetwork", with_bytes=True)
    network_path = verify_artifact(network, "candidate export receipt.integerNetwork")
    expected_network = workspace_path(config["output"]["directory"]) / "exports" / "candidates" / f"candidate-u{expected_checkpoint['update']:08d}.nnue"
    if network_path.resolve() != expected_network.resolve():
        fail("candidate export receipt does not reference the frozen preselection export path")
    exact(receipt["status"], "passed", "candidate export receipt.status")
    return {"path": str(path), "sha256": sha256(path), "run": {"path": str(run_path), "sha256": run_item["sha256"]}, "trainingRun": {"optimizerUpdates": training_run["optimizerUpdates"], "complete": training_run["complete"], "stopReason": training_run["stopReason"], "approvedMaximumPlannedUpdate": training_run["approvedMaximumPlannedUpdate"]}, "checkpoint": expected_checkpoint, "sourceCheckpoint": {**source, "resolvedPath": str(source_path)}, "serializer": serializer, "integerNetwork": {**network, "resolvedPath": str(network_path)}, "status": receipt["status"]}


def validate_parity_receipt(path: Path, config: dict[str, Any], config_path: Path, checkpoint: dict[str, Any], network: dict[str, Any], runtime_build: dict[str, Any]) -> dict[str, Any]:
    receipt = load_json(path)
    exact_keys(receipt, {"schema", "experimentId", "configSha256", "createdAt", "checkpoint", "integerNetwork", "runtimeBuild", "checks", "status"}, "parity receipt")
    exact(receipt["schema"], PARITY_SCHEMA, "parity receipt.schema")
    exact(receipt["experimentId"], config["experimentId"], "parity receipt.experimentId")
    exact(receipt["configSha256"], config_sha256(config_path), "parity receipt.configSha256")
    timestamp(receipt["createdAt"], "parity receipt.createdAt")
    exact(receipt["checkpoint"], checkpoint, "parity receipt.checkpoint")
    exact(receipt["integerNetwork"], network, "parity receipt.integerNetwork")
    exact(receipt["runtimeBuild"], runtime_build, "parity receipt.runtimeBuild")
    checks = exact_keys(receipt["checks"], {"fullPrecisionToIntegerMeasured", "integerReferenceMatchesNative", "integerReferenceMatchesWasm", "nativeMatchesWasm", "quantizationErrorReportedSeparately"}, "parity receipt.checks")
    if any(value is not True for value in checks.values()):
        fail("every parity and quantization-separation check must pass before games")
    exact(receipt["status"], "passed", "parity receipt.status")
    return {"path": str(path), "sha256": sha256(path), "status": receipt["status"]}


def validate_candidate_runtime(path: Path, config: dict[str, Any], config_path: Path, expected_checkpoint: dict[str, Any]) -> dict[str, Any]:
    receipt = load_json(path)
    exact_keys(receipt, {"schema", "experimentId", "configSha256", "createdAt", "checkpoint", "candidateExport", "runtimeBuild", "parityReceipt", "width", "status"}, "candidate runtime")
    exact(receipt["schema"], CANDIDATE_RUNTIME_SCHEMA, "candidate runtime.schema")
    exact(receipt["experimentId"], config["experimentId"], "candidate runtime.experimentId")
    exact(receipt["configSha256"], config_sha256(config_path), "candidate runtime.configSha256")
    timestamp(receipt["createdAt"], "candidate runtime.createdAt")
    exact(receipt["checkpoint"], expected_checkpoint, "candidate runtime.checkpoint")
    checkpoint_path = verify_artifact({"path": expected_checkpoint["path"], "sha256": expected_checkpoint["sha256"]}, "candidate runtime.checkpoint")
    export_item = artifact_shape(receipt["candidateExport"], "candidate runtime.candidateExport", with_bytes=False)
    export_path = verify_artifact(export_item, "candidate runtime.candidateExport")
    candidate_export = validate_candidate_export(export_path, config, config_path, expected_checkpoint)
    network = {key: candidate_export["integerNetwork"][key] for key in ("path", "bytes", "sha256")}
    network_path = Path(candidate_export["integerNetwork"]["resolvedPath"])
    runtime_build = artifact_shape(receipt["runtimeBuild"], "candidate runtime.runtimeBuild", with_bytes=True)
    runtime_path = verify_artifact(runtime_build, "candidate runtime.runtimeBuild")
    parity_item = artifact_shape(receipt["parityReceipt"], "candidate runtime.parityReceipt", with_bytes=False)
    parity_path = verify_artifact(parity_item, "candidate runtime.parityReceipt")
    parity = validate_parity_receipt(parity_path, config, config_path, expected_checkpoint, network, runtime_build)
    exact(receipt["width"], config["recipe"]["width"], "candidate runtime.width")
    exact(receipt["status"], "passed", "candidate runtime.status")
    return {"path": str(path), "sha256": sha256(path), "checkpoint": expected_checkpoint, "checkpointPath": str(checkpoint_path), "candidateExport": candidate_export, "integerNetwork": {**network, "resolvedPath": str(network_path)}, "runtimeBuild": {**runtime_build, "resolvedPath": str(runtime_path)}, "parityReceipt": parity, "status": receipt["status"]}


def match_schedule(config: dict[str, Any], kind: str, expected_update: int) -> dict[str, Any]:
    protocol = config["strengthProtocol"]
    if kind == "scheduled-screen":
        cycles, pairs, budget = [protocol["scheduledScreens"]["openingCycle"]], protocol["scheduledScreens"]["pairsPerScreen"], {"mode": "nodes", "value": protocol["scheduledScreens"]["nodesPerMove"]}
    elif kind == "final-equal-node":
        cycles, pairs, budget = list(range(protocol["cycles"])), protocol["finalEqualNodeComparisonPairs"], {"mode": "nodes", "value": protocol["scheduledScreens"]["nodesPerMove"]}
    elif kind == "final-fixed-time":
        cycles, pairs, budget = list(range(protocol["cycles"])), protocol["finalFixedTimeScreenPairs"], {"mode": "movetime", "value": protocol["fixedTimeMillisecondsPerMove"]}
    else:
        fail(f"unsupported raw match kind {kind!r}")
    return {"scheduleId": protocol["scheduleId"], "openingSuiteSha256": protocol["openingSuite"]["sha256"], "pairOrder": protocol["pairOrder"], "cycles": cycles, "pairs": pairs, "budget": budget, "colorSwap": protocol["colorSwap"], "opponent": protocol["opponent"], "maximumPly": protocol["maximumPly"], "candidateUpdate": expected_update}


def validate_raw_match(path: Path, config: dict[str, Any], config_path: Path, expected_kind: str, candidate_runtime_item: dict[str, Any], candidate_runtime: dict[str, Any], expected_update: int) -> dict[str, Any]:
    report = load_json(path)
    exact_keys(report, {"schema", "experimentId", "configSha256", "createdAt", "kind", "candidateRuntime", "schedule", "games", "summary", "runtimeCost", "status"}, "raw match")
    exact(report["schema"], RAW_MATCH_SCHEMA, "raw match.schema")
    exact(report["experimentId"], config["experimentId"], "raw match.experimentId")
    exact(report["configSha256"], config_sha256(config_path), "raw match.configSha256")
    timestamp(report["createdAt"], "raw match.createdAt")
    exact(report["kind"], expected_kind, "raw match.kind")
    exact(report["candidateRuntime"], candidate_runtime_item, "raw match.candidateRuntime")
    exact(report["schedule"], match_schedule(config, expected_kind, expected_update), "raw match.schedule")
    schedule = report["schedule"]
    games = report["games"]
    expected_games = schedule["pairs"] * 2
    if not isinstance(games, list) or len(games) != expected_games:
        fail(f"raw match must contain exactly {expected_games} games")
    pairs: dict[tuple[int, str], list[dict[str, Any]]] = {}
    opening_ids = config["strengthProtocol"]["openingIds"]
    for index, game in enumerate(games):
        where = f"raw match.games[{index}]"
        exact_keys(game, {"schema", "gameIndex", "pairIndex", "openingId", "openingCycle", "candidateColor", "opponent", "scoreCandidate", "result", "candidateSearchedMoves", "candidateSearchElapsedMilliseconds", "complete", "missingMoves", "illegalMoves", "emergencyFallbacks", "unsearchedMoves", "terminationReason"}, where)
        exact(game["schema"], SCREEN_GAME_SCHEMA, f"{where}.schema")
        exact(game["gameIndex"], index, f"{where}.gameIndex")
        pair_index = nonnegative_integer(game["pairIndex"], f"{where}.pairIndex")
        exact(pair_index, index // 2, f"{where}.pairIndex frozen order")
        if pair_index >= schedule["pairs"]:
            fail(f"{where}.pairIndex is outside the scheduled pairs")
        opening_index = pair_index % len(opening_ids)
        expected_cycle = schedule["cycles"][pair_index // len(opening_ids)]
        expected_id = opening_ids[opening_index]
        exact(game["openingId"], expected_id, f"{where}.openingId")
        exact(game["openingCycle"], expected_cycle, f"{where}.openingCycle")
        exact(game["candidateColor"], ("black", "white")[index % 2], f"{where}.candidateColor frozen swap order")
        exact(game["opponent"], config["strengthProtocol"]["opponent"], f"{where}.opponent")
        score = finite_number(game["scoreCandidate"], f"{where}.scoreCandidate", minimum=0.0, maximum=1.0)
        expected_result = {0.0: "candidate-loss", 0.5: "draw", 1.0: "candidate-win"}.get(score)
        if expected_result is None:
            fail(f"{where}.scoreCandidate must be 0, 0.5, or 1")
        exact(game["result"], expected_result, f"{where}.result")
        positive_integer(game["candidateSearchedMoves"], f"{where}.candidateSearchedMoves")
        finite_number(game["candidateSearchElapsedMilliseconds"], f"{where}.candidateSearchElapsedMilliseconds", minimum=0.0)
        exact(game["complete"], True, f"{where}.complete")
        for field in ("missingMoves", "illegalMoves", "emergencyFallbacks", "unsearchedMoves"):
            exact(nonnegative_integer(game[field], f"{where}.{field}"), 0, f"{where}.{field}")
        if not isinstance(game["terminationReason"], str) or not game["terminationReason"]:
            fail(f"{where}.terminationReason must be nonempty")
        pairs.setdefault((pair_index, expected_id), []).append(game)
    if len(pairs) != schedule["pairs"]:
        fail("raw match does not cover every scheduled opening pair")
    opening_scores: dict[str, list[float]] = {opening_id: [] for opening_id in opening_ids}
    for key in sorted(pairs):
        pair = pairs[key]
        if len(pair) != 2 or {item["candidateColor"] for item in pair} != {"black", "white"}:
            fail(f"raw match pair {key[0]} is not one black/white color swap")
        opening_scores[key[1]].append(sum(float(item["scoreCandidate"]) for item in pair) / 2.0)
    group_scores = [float(np.mean(opening_scores[opening_id])) for opening_id in opening_ids]
    statistics = grouped_screen_statistics(group_scores)
    summary = exact_keys(report["summary"], {"games", "pairs", "openingGroups", "groupScores", "meanScore", "sampleStandardDeviation", "standardError", "criticalValue", "lower95", "upper95", "intervalMethod", "correctnessPassed"}, "raw match.summary")
    exact(summary["games"], expected_games, "raw match.summary.games")
    exact(summary["pairs"], schedule["pairs"], "raw match.summary.pairs")
    exact(summary["openingGroups"], 64, "raw match.summary.openingGroups")
    if not isinstance(summary["groupScores"], list) or len(summary["groupScores"]) != 64:
        fail("raw match.summary.groupScores must contain 64 scores")
    for index, expected_score in enumerate(group_scores):
        same_number(summary["groupScores"][index], expected_score, f"raw match.summary.groupScores[{index}]")
    for name, expected_value in statistics.items():
        same_number(summary[name], expected_value, f"raw match.summary.{name}")
    exact(summary["intervalMethod"], config["strengthProtocol"]["uncertainty"]["intervalMethod"], "raw match.summary.intervalMethod")
    exact(summary["correctnessPassed"], True, "raw match.summary.correctnessPassed")
    runtime_cost = exact_keys(report["runtimeCost"], {"metric", "value"}, "raw match.runtimeCost")
    exact(runtime_cost["metric"], "mean-candidate-search-elapsed-milliseconds", "raw match.runtimeCost.metric")
    total_moves = sum(int(game["candidateSearchedMoves"]) for game in games)
    total_elapsed = sum(float(game["candidateSearchElapsedMilliseconds"]) for game in games)
    same_number(runtime_cost["value"], total_elapsed / total_moves, "raw match.runtimeCost.value")
    exact(report["status"], "passed", "raw match.status")
    return {"path": str(path), "sha256": sha256(path), "kind": expected_kind, "candidateRuntime": candidate_runtime, "summary": summary, "runtimeCost": runtime_cost, "status": report["status"]}


def validate_screen_evidence(path: Path, config: dict[str, Any], config_path: Path, expected_update: int, *, seen: set[str] | None = None) -> dict[str, Any]:
    digest = sha256(path)
    visited = set() if seen is None else seen
    if digest in visited:
        fail("strength-screen prior history contains a cycle")
    visited.add(digest)
    report = load_json(path)
    exact_keys(report, {"schema", "experimentId", "configSha256", "createdAt", "checkpoint", "candidateRuntime", "rawMatch", "summary", "priorScreens", "decision"}, "strength screen")
    exact(report["schema"], SCREEN_SCHEMA, "strength screen.schema")
    exact(report["experimentId"], config["experimentId"], "strength screen.experimentId")
    exact(report["configSha256"], config_sha256(config_path), "strength screen.configSha256")
    timestamp(report["createdAt"], "strength screen.createdAt")
    checkpoint = exact_keys(report["checkpoint"], {"update", "path", "sha256"}, "strength screen.checkpoint")
    exact(checkpoint["update"], expected_update, "strength screen.checkpoint.update")
    checkpoint_path = verify_artifact({"path": checkpoint["path"], "sha256": checkpoint["sha256"]}, "strength screen.checkpoint")
    expected_checkpoint_path = workspace_path(config["output"]["directory"]) / "checkpoints" / f"scheduled-u{expected_update:08d}.pt"
    if checkpoint_path.resolve() != expected_checkpoint_path.resolve():
        fail("strength screen does not reference this experiment's scheduled checkpoint")
    runtime_item = artifact_shape(report["candidateRuntime"], "strength screen.candidateRuntime", with_bytes=False)
    runtime_path = verify_artifact(runtime_item, "strength screen.candidateRuntime")
    runtime = validate_candidate_runtime(runtime_path, config, config_path, checkpoint)
    raw_item = artifact_shape(report["rawMatch"], "strength screen.rawMatch", with_bytes=False)
    raw_path = verify_artifact(raw_item, "strength screen.rawMatch")
    raw = validate_raw_match(raw_path, config, config_path, "scheduled-screen", runtime_item, runtime, expected_update)
    exact(report["summary"], raw["summary"], "strength screen.summary")
    summary = raw["summary"]
    protocol = config["strengthProtocol"]
    scheduled = protocol["scheduledScreens"]
    screen_updates = scheduled["updates"]
    if expected_update not in screen_updates:
        fail(f"strength screen update {expected_update} is not scheduled")
    position = screen_updates.index(expected_update)
    expected_prior_updates = screen_updates[:position]
    prior_items = report["priorScreens"]
    if not isinstance(prior_items, list) or len(prior_items) != len(expected_prior_updates):
        fail("strength screen prior history does not contain every earlier scheduled screen")
    prior_reports: list[dict[str, Any]] = []
    for index, (item, prior_update) in enumerate(zip(prior_items, expected_prior_updates)):
        artifact_shape(item, f"strength screen.priorScreens[{index}]", with_bytes=False)
        prior_path = verify_artifact(item, f"strength screen.priorScreens[{index}]")
        prior_reports.append(validate_screen_evidence(prior_path, config, config_path, prior_update, seen=visited))
    if any(item["decision"] != "continue" for item in prior_reports):
        fail("strength screen was produced after an earlier screen had already stopped training")
    threshold = protocol["uncertainty"]["screenStoppingThreshold"]
    previous_low = bool(prior_reports and prior_reports[-1]["upper95"] < threshold)
    current_low = summary["upper95"] < threshold
    expected_decision = "stop-two-successive-low-screens" if previous_low and current_low else "continue"
    exact(report["decision"], expected_decision, "strength screen.decision")
    visited.remove(digest)
    return {"schema": report["schema"], "path": str(path), "sha256": digest, "update": expected_update, "checkpoint": checkpoint, "candidateRuntime": runtime, "rawMatch": raw, "meanScore": summary["meanScore"], "upper95": summary["upper95"], "decision": report["decision"]}


def validate_continuation_receipt(path: Path, config: dict[str, Any], config_path: Path, from_update: int, to_update: int) -> dict[str, Any]:
    gates = config["strengthProtocol"]["executionGates"]
    matches = [item for item in gates["continuationSequence"] if item["fromUpdate"] == from_update and item["toUpdate"] == to_update]
    if len(matches) != 1:
        fail(f"continuation {from_update}->{to_update} is outside the frozen sequence")
    required_roles = matches[0]["requiredEvidenceRoles"]
    receipt = load_json(path)
    exact_keys(receipt, {"schema", "experimentId", "configSha256", "fromUpdate", "toUpdate", "decision", "createdAt", "evidence"}, "continuation receipt")
    exact(receipt["schema"], CONTINUATION_SCHEMA, "continuation receipt.schema")
    exact(receipt["experimentId"], config["experimentId"], "continuation receipt.experimentId")
    exact(receipt["configSha256"], config_sha256(config_path), "continuation receipt.configSha256")
    exact(receipt["fromUpdate"], from_update, "continuation receipt.fromUpdate")
    exact(receipt["toUpdate"], to_update, "continuation receipt.toUpdate")
    exact(receipt["decision"], gates["receiptDecision"], "continuation receipt.decision")
    timestamp(receipt["createdAt"], "continuation receipt.createdAt")
    if not isinstance(receipt["evidence"], list):
        fail("continuation receipt evidence must be a list")
    roles: list[str] = []
    evidence: list[dict[str, Any]] = []
    approved_horizon: int | None = None
    for index, item in enumerate(receipt["evidence"]):
        exact_keys(item, {"role", "path", "sha256"}, f"continuation receipt.evidence[{index}]")
        if item["role"] in roles:
            fail(f"continuation receipt duplicates evidence role {item['role']!r}")
        evidence_path = verify_artifact({"path": item["path"], "sha256": item["sha256"]}, f"continuation receipt.evidence[{index}]")
        role = item["role"]
        if role == "throughput-probe-report":
            validated = validate_throughput_evidence(evidence_path, config, config_path)
            approved_horizon = validated["maximumPlannedUpdate"]
        elif role.startswith("strength-screen-u"):
            try:
                screen_update = int(role.removeprefix("strength-screen-u"))
            except ValueError:
                fail(f"invalid strength-screen evidence role {role!r}")
            validated = validate_screen_evidence(evidence_path, config, config_path, screen_update)
            if validated["decision"] != "continue":
                fail("strength-screen stop rule forbids continuation")
        else:
            fail(f"unsupported continuation evidence role {role!r}")
        roles.append(role)
        evidence.append({"role": role, **validated})
    exact(roles, required_roles, "continuation receipt evidence role order")
    if approved_horizon is None:
        fail("every continuation leg requires persisted throughput-horizon evidence")
    if to_update > approved_horizon:
        fail(f"continuation destination {to_update} exceeds measured approved horizon {approved_horizon}")
    return {"path": str(path), "sha256": sha256(path), "fromUpdate": from_update, "toUpdate": to_update, "decision": receipt["decision"], "createdAt": receipt["createdAt"], "evidence": evidence, "approvedMaximumPlannedUpdate": approved_horizon}


def snapshot_receipt_graph(source: Path, receipt_dir: Path, memo: dict[str, Path] | None = None) -> Path:
    """Copy a validated JSON receipt graph into immutable run-local storage."""
    source = source.resolve()
    cache = {} if memo is None else memo
    source_digest = sha256(source)
    if source_digest in cache:
        return cache[source_digest]
    value = load_json(source)
    schema = value.get("schema")
    supported = {APPROVAL_SCHEMA, CONTINUATION_SCHEMA, THROUGHPUT_SCHEMA, SCREEN_SCHEMA, CANDIDATE_EXPORT_SCHEMA, CANDIDATE_RUNTIME_SCHEMA, PARITY_SCHEMA, RAW_MATCH_SCHEMA, COMPARISON_SCHEMA, SELECTION_HISTORY_SCHEMA, TRAINING_RUN_SCHEMA}
    if schema not in supported:
        fail(f"cannot snapshot unsupported receipt schema {schema!r}")

    def rewrite(item: dict[str, Any], where: str) -> dict[str, Any]:
        artifact_shape(item, where, with_bytes=False)
        child = verify_artifact(item, where)
        local = snapshot_receipt_graph(child, receipt_dir, cache)
        return {"path": str(local), "sha256": sha256(local)}

    def preserve_artifact(item: dict[str, Any], where: str) -> dict[str, Any]:
        artifact_shape(item, where, with_bytes=True)
        original = verify_artifact(item, where)
        suffix = original.suffix if original.suffix else ".bin"
        local = receipt_dir / "artifacts" / f"{item['sha256']}{suffix}"
        if local.exists():
            if local.stat().st_size != item["bytes"] or sha256(local) != item["sha256"]:
                fail(f"run-local preserved artifact conflicts for {where}")
        else:
            atomic_copy_file(original, local)
        return {"path": str(local), "bytes": item["bytes"], "sha256": item["sha256"]}

    def preserve_hash_artifact(item: dict[str, Any], where: str) -> dict[str, Any]:
        artifact_shape(item, where, with_bytes=False)
        original = verify_artifact(item, where)
        suffix = original.suffix if original.suffix else ".bin"
        local = receipt_dir / "artifacts" / f"{item['sha256']}{suffix}"
        if local.exists():
            if sha256(local) != item["sha256"]:
                fail(f"run-local preserved artifact conflicts for {where}")
        else:
            atomic_copy_file(original, local)
        return {"path": str(local), "sha256": item["sha256"]}

    def rewrite_record(item: dict[str, Any], where: str) -> dict[str, Any]:
        local = rewrite({"path": item["path"], "sha256": item["sha256"]}, where)
        return {**item, **local}

    localized = copy.deepcopy(value)
    if schema == CONTINUATION_SCHEMA:
        localized["evidence"] = [{"role": item["role"], **rewrite({"path": item["path"], "sha256": item["sha256"]}, f"continuation evidence {index}")} for index, item in enumerate(value["evidence"])]
    elif schema == SCREEN_SCHEMA:
        localized["candidateRuntime"] = rewrite(value["candidateRuntime"], "screen candidate runtime")
        localized["rawMatch"] = rewrite(value["rawMatch"], "screen raw match")
        localized["priorScreens"] = [rewrite(item, f"screen prior {index}") for index, item in enumerate(value["priorScreens"])]
    elif schema == CANDIDATE_EXPORT_SCHEMA:
        localized["run"] = rewrite(value["run"], "candidate export run")
    elif schema == CANDIDATE_RUNTIME_SCHEMA:
        localized["candidateExport"] = rewrite(value["candidateExport"], "candidate runtime export")
        localized["runtimeBuild"] = preserve_artifact(value["runtimeBuild"], "candidate runtime build")
        localized["parityReceipt"] = rewrite(value["parityReceipt"], "candidate runtime parity")
    elif schema == PARITY_SCHEMA:
        localized["runtimeBuild"] = preserve_artifact(value["runtimeBuild"], "parity runtime build")
    elif schema == RAW_MATCH_SCHEMA:
        localized["candidateRuntime"] = rewrite(value["candidateRuntime"], "raw match candidate runtime")
    elif schema == COMPARISON_SCHEMA:
        localized["candidateRuntime"] = rewrite(value["candidateRuntime"], "comparison candidate runtime")
        localized["equalNodeMatch"] = rewrite(value["equalNodeMatch"], "comparison equal-node match")
        if value["fixedTimeMatch"] is not None:
            localized["fixedTimeMatch"] = rewrite(value["fixedTimeMatch"], "comparison fixed-time match")
        localized["validationMetric"]["metricsJournal"] = preserve_hash_artifact(value["validationMetric"]["metricsJournal"], "comparison metrics journal")
    elif schema == SELECTION_HISTORY_SCHEMA:
        localized["run"] = rewrite(value["run"], "selection run")
        if value["terminalEvidence"] is not None:
            terminal = value["terminalEvidence"]
            localized["terminalEvidence"] = {"role": terminal["role"], **rewrite({"path": terminal["path"], "sha256": terminal["sha256"]}, "selection terminal evidence")}
        localized["scheduledScreens"] = [rewrite(item, f"selection screen {index}") for index, item in enumerate(value["scheduledScreens"])]
        localized["candidates"] = [{**candidate, "comparisonReceipt": rewrite(candidate["comparisonReceipt"], f"selection comparison {index}")} for index, candidate in enumerate(value["candidates"])]
    elif schema == TRAINING_RUN_SCHEMA:
        localized["approval"] = rewrite_record(value["approval"], "training run approval")
        localized["continuationReceipts"] = [rewrite_record(item, f"training run continuation {index}") for index, item in enumerate(value["continuationReceipts"])]
        if value["activeLeg"] is not None:
            localized["activeLeg"] = rewrite_record(value["activeLeg"], "training run active leg")
    safe_schema = str(schema).replace("haitaka-anhoku-strength-first-", "").replace("haitaka-explicit-", "").replace("/", "-")
    target = receipt_dir / f"{safe_schema}-{source_digest}.json"
    atomic_write_json(target, localized)
    cache[source_digest] = target
    return target


def resolve_continuation_leg(config: dict[str, Any], config_path: Path, update: int, target_update: int, active_leg: dict[str, Any] | None, supplied_receipt: Path | None, receipt_dir: Path | None = None) -> tuple[dict[str, Any], bool]:
    if target_update <= update:
        fail("continuation target must advance beyond the committed update")
    if active_leg is not None:
        exact_keys(active_leg, {"path", "sha256", "fromUpdate", "toUpdate", "decision", "createdAt", "evidence", "approvedMaximumPlannedUpdate"}, "resume activeLeg")
        if not (active_leg["fromUpdate"] < update < active_leg["toUpdate"]):
            fail("persisted active continuation leg does not contain the resume update")
        if target_update > active_leg["toUpdate"]:
            fail("an intermediate resume cannot cross the active leg destination")
        receipt_path = Path(active_leg["path"]).resolve()
        if supplied_receipt is not None and supplied_receipt.resolve() != receipt_path:
            fail("intermediate resume may only repeat the persisted active-leg receipt")
        validated = validate_continuation_receipt(receipt_path, config, config_path, active_leg["fromUpdate"], active_leg["toUpdate"])
        exact(validated["sha256"], active_leg["sha256"], "persisted active-leg receipt SHA-256")
        return validated, False
    sequence = config["strengthProtocol"]["executionGates"]["continuationSequence"]
    matches = [item for item in sequence if item["fromUpdate"] == update and update < target_update <= item["toUpdate"]]
    if len(matches) != 1:
        fail(f"requested continuation {update}->{target_update} is outside the frozen legs")
    if supplied_receipt is None:
        fail("starting a new continuation leg requires its hash-bound receipt")
    source = supplied_receipt.resolve()
    validate_continuation_receipt(source, config, config_path, update, matches[0]["toUpdate"])
    if receipt_dir is not None:
        source = snapshot_receipt_graph(source, receipt_dir)
    validated = validate_continuation_receipt(source, config, config_path, update, matches[0]["toUpdate"])
    return validated, True


def run_training(args: argparse.Namespace, config: dict[str, Any], config_path: Path) -> None:
    verify_inputs(config)
    recipe = config["recipe"]
    output = workspace_path(config["output"]["directory"])
    state_path = output / "last-state.pt"
    if args.resume:
        if not state_path.is_file():
            fail("--resume requires the experiment's last-state.pt")
    elif output.exists():
        fail(f"fresh training refuses an existing output directory: {output}")
    if not args.resume and args.approval_receipt is None:
        fail("fresh training requires --approval-receipt")
    output.mkdir(parents=True, exist_ok=args.resume)
    resolved = output / "resolved-config.json"
    if not args.resume:
        shutil.copy2(config_path, resolved)
    elif sha256(resolved) != sha256(config_path):
        fail("resume configuration differs from the copied resolved configuration")
    receipt_dir = output / "receipts"
    approval: dict[str, Any] | None = None
    if not args.resume:
        supplied_approval = args.approval_receipt.resolve()
        validate_approval(supplied_approval, config, config_path)
        local_approval = snapshot_receipt_graph(supplied_approval, receipt_dir)
        approval = validate_approval(local_approval, config, config_path)

    try:
        import torch  # type: ignore
    except ImportError as exc:
        fail(f"torch is unavailable: {exc}")
    if not torch.cuda.is_available():
        fail("candidate training requires an approved CUDA device")
    device = torch.device("cuda")
    target_update = args.stop_after_updates
    if target_update <= 0 or target_update > recipe["schedule"]["maximumUpdates"] or (target_update % recipe["evaluation"]["everyUpdates"] != 0):
        fail("--stop-after-updates must be an evaluation boundary through maximumUpdates")
    gates = config["strengthProtocol"]["executionGates"]
    if not args.resume:
        exact(target_update, gates["freshRunStopUpdate"], "fresh-run stop update")
        if args.continuation_receipt is not None:
            fail("fresh run must not provide a continuation receipt")

    seed_everything(torch, recipe["seed"])
    context_temp, context = r5.prepare_context(recipe["width"], args.cmake)
    try:
        torch, features, model_module, nnue_dataset = r5.import_trainer(context)
        feature_set = features.get_feature_set_from_name(recipe["featureFamily"])
        model = model_module.NNUE(feature_set=feature_set, lambda_=recipe["objective"]["lambda"], learning_rate=recipe["schedule"]["baseLearningRate"]).to(device)
        optimizer = configured_optimizer(model, recipe)
        cursor = Cursor()
        update = 0
        examples_seen = 0
        validation_evaluations = 0
        best = {"loss": math.inf, "update": None, "checkpoint": None}
        artifacts: list[dict[str, Any]] = []
        continuation_receipts: list[dict[str, Any]] = []
        active_leg: dict[str, Any] | None = None
        approved_horizon: int | None = None
        timing = {"shuffleSeconds": 0.0, "trainingSeconds": 0.0, "validationSeconds": 0.0, "checkpointSeconds": 0.0, "exportSeconds": 0.0}
        if args.resume:
            state = torch.load(state_path, map_location=device, weights_only=False)
            exact_keys(state, {"schema", "runnerVersion", "configSha256", "approval", "continuationReceipts", "activeLeg", "approvedMaximumPlannedUpdate", "model", "optimizer", "schedule", "rng", "optimizerUpdates", "examplesSeen", "dataCursor", "validationEvaluations", "bestValidation", "artifacts", "timing", "lastMetricsRow"}, "resume state")
            exact(state["schema"], STATE_SCHEMA, "resume state schema")
            exact(state["runnerVersion"], RUNNER_VERSION, "resume runner version")
            exact(state["configSha256"], config_sha256(config_path), "resume config SHA-256")
            local_approval_path = Path(state["approval"]["path"])
            approval = validate_approval(local_approval_path, config, config_path)
            exact(approval["sha256"], state["approval"]["sha256"], "resume approval receipt SHA-256")
            model.load_state_dict(state["model"])
            optimizer.load_state_dict(state["optimizer"])
            update = state["optimizerUpdates"]
            examples_seen = state["examplesSeen"]
            cursor = Cursor(**state["dataCursor"])
            validation_evaluations = state["validationEvaluations"]
            best = state["bestValidation"]
            artifacts = state["artifacts"]
            continuation_receipts = state["continuationReceipts"]
            active_leg = state["activeLeg"]
            approved_horizon = state["approvedMaximumPlannedUpdate"]
            if approved_horizon is not None:
                positive_integer(approved_horizon, "resume approvedMaximumPlannedUpdate")
                if update > approved_horizon:
                    fail("resume state has already crossed its measured approved horizon")
            timing = state["timing"]
            restore_rng(torch, state["rng"])
            assert_finite_training_state(torch, model, optimizer, "resumed state")
            append_committed_row(output / "metrics.jsonl", state["lastMetricsRow"])
            active_leg, is_new_leg = resolve_continuation_leg(config, config_path, update, target_update, active_leg, args.continuation_receipt, receipt_dir)
            new_horizon = active_leg["approvedMaximumPlannedUpdate"]
            if approved_horizon is not None and new_horizon < update:
                fail("new continuation receipt reduces the horizon below the committed update")
            approved_horizon = new_horizon
            if is_new_leg:
                continuation_receipts.append(active_leg)
        else:
            if approval is None:
                fail("fresh training approval was not initialized")
            assert_finite_training_state(torch, model, optimizer, "initial state")

        training_data = recipe["data"]["training"]
        train_metadata = Metadata(workspace_path(training_data["sidecar"]["path"]), training_data["records"])
        validation_data = recipe["data"]["validation"]
        validation_metadata = Metadata(workspace_path(validation_data["sidecar"]["path"]), validation_data["records"])
        if update == 0 and recipe["evaluation"]["initialValidation"]:
            started = time.monotonic()
            initial = evaluate(torch, model, nnue_dataset, recipe, validation_metadata, device)
            timing["validationSeconds"] += time.monotonic() - started
            validation_evaluations += 1
            atomic_write_json(output / "initial-validation.json", initial)

        window = Window(recipe["objective"])
        scheduled = set(recipe["checkpoints"]["scheduledUpdates"])
        cache = output / "order-cache"
        while update < target_update:
            order = epoch_permutation(training_data["records"], recipe["seed"], cursor.epoch)
            shuffled = cache / f"epoch-{cursor.epoch:04d}.bin"
            started = time.monotonic()
            materialize_epoch(workspace_path(training_data["binary"]["path"]), shuffled, training_data["records"], training_data["entryBytes"], order)
            timing["shuffleSeconds"] += time.monotonic() - started
            start_record = cursor.record
            for batch in finite_batches(nnue_dataset, recipe["featureFamily"], shuffled, training_data["records"], recipe["loader"]["batchSize"], device, recipe["loader"], skip=start_record):
                if update >= target_update:
                    break
                count = int(batch[0].shape[0])
                indices = order[cursor.record:cursor.record + count]
                next_update = update + 1
                sampled_update = next_update % recipe["diagnostics"]["updateMagnitudeEveryUpdates"] == 0
                before = [parameter.detach().float().clone() for parameter in model.parameters()] if sampled_update else []
                model.train()
                model._clip_weights()
                assert_finite_nested(torch, model.state_dict(), f"pre-update {next_update} model")
                base_lr = set_learning_rates(optimizer, recipe, next_update)
                optimizer.zero_grad(set_to_none=True)
                started = time.monotonic()
                mean_loss, losses, predictions, targets = loss_tensors(torch, model, batch, recipe["objective"])
                if not bool(torch.isfinite(mean_loss)):
                    fail(f"non-finite loss at update {next_update}")
                mean_loss.backward()
                grad_norm, grad_max = gradient_magnitude(torch, model)
                optimizer.step()
                model._clip_weights()
                assert_finite_training_state(torch, model, optimizer, f"post-update {next_update}")
                update_sample = update_magnitude(torch, before, model) if sampled_update else None
                timing["trainingSeconds"] += time.monotonic() - started
                raw = predictions / recipe["objective"]["modelOutputToRuntimeScore"]
                window.add(losses.detach().cpu().numpy(), predictions.detach().cpu().numpy(), targets.detach().cpu().numpy(), raw.detach().cpu().numpy(), train_metadata, indices, grad_norm, grad_max, update_sample)
                window.add_coverage(torch, batch)
                cursor.advance(count, training_data["records"])
                update = next_update
                examples_seen += count
                if update % recipe["evaluation"]["everyUpdates"] != 0:
                    continue

                started = time.monotonic()
                validation = evaluate(torch, model, nnue_dataset, recipe, validation_metadata, device)
                timing["validationSeconds"] += time.monotonic() - started
                validation_evaluations += 1
                checkpoint_started = time.monotonic()
                checkpoint_records: list[dict[str, Any]] = []
                if validation["teacherOnlyLoss"] < best["loss"]:
                    path = output / "checkpoints" / f"best-validation-u{update:08d}.pt"
                    record = save_portable_model(torch, model, path)
                    checkpoint_records.append({"role": "best-validation", **record})
                    best = {"loss": validation["teacherOnlyLoss"], "update": update, "checkpoint": record}
                if update == recipe["checkpoints"]["throughputProbeUpdate"]:
                    path = output / "checkpoints" / f"throughput-probe-u{update:08d}.pt"
                    record = save_portable_model(torch, model, path)
                    checkpoint_records.append({"role": "throughput-probe", **record})
                if update in scheduled:
                    path = output / "checkpoints" / f"scheduled-u{update:08d}.pt"
                    record = save_portable_model(torch, model, path)
                    checkpoint_records.append({"role": "scheduled-strength-screen", **record})
                    export_path = output / "exports" / f"scheduled-u{update:08d}.nnue"
                    exported = export_checkpoint(torch, path, export_path, context, recipe, config["experimentId"])
                    timing["exportSeconds"] += exported["seconds"]
                    artifacts.append({"role": "scheduled-export", "update": update, **exported})
                if update == recipe["schedule"]["maximumUpdates"]:
                    path = output / "checkpoints" / f"final-u{update:08d}.pt"
                    record = save_portable_model(torch, model, path)
                    checkpoint_records.append({"role": "final", **record})
                artifacts.extend({"update": update, **item} for item in checkpoint_records)
                timing["checkpointSeconds"] += time.monotonic() - checkpoint_started
                row = {
                    "schema": METRICS_SCHEMA,
                    "optimizerUpdates": update,
                    "learningRate": base_lr,
                    "groupLearningRates": [group["lr"] for group in optimizer.param_groups],
                    "exposure": {"examplesSeen": examples_seen, "equivalentPasses": examples_seen / training_data["records"], "completedPasses": cursor.epoch, "dataCursor": {"epoch": cursor.epoch, "record": cursor.record}, "validationEvaluations": validation_evaluations, "validationExamplesSeen": validation_evaluations * validation_data["records"]},
                    "trainWindow": window.report(),
                    "validation": validation,
                    "bestValidation": best,
                    "newArtifacts": checkpoint_records,
                    "activeLeg": None if active_leg is not None and update >= active_leg["toUpdate"] else active_leg,
                    "timingSeconds": dict(timing),
                }
                if active_leg is not None and update >= active_leg["toUpdate"]:
                    active_leg = None
                state = {"schema": STATE_SCHEMA, "runnerVersion": RUNNER_VERSION, "configSha256": config_sha256(config_path), "approval": approval, "continuationReceipts": continuation_receipts, "activeLeg": active_leg, "approvedMaximumPlannedUpdate": approved_horizon, "model": model.state_dict(), "optimizer": optimizer.state_dict(), "schedule": {"kind": recipe["schedule"]["kind"], "lastAppliedLearningRate": base_lr, "nextOptimizerUpdate": update + 1}, "rng": rng_state(torch), "optimizerUpdates": update, "examplesSeen": examples_seen, "dataCursor": {"epoch": cursor.epoch, "record": cursor.record}, "validationEvaluations": validation_evaluations, "bestValidation": best, "artifacts": artifacts, "timing": timing, "lastMetricsRow": row}
                assert_finite_nested(torch, state, f"committed state at update {update}")
                atomic_torch_save(torch, state, state_path)
                append_committed_row(output / "metrics.jsonl", row)
                window = Window(recipe["objective"])
                if update >= target_update:
                    break
            for old in cache.glob("epoch-*.bin"):
                if old != shuffled:
                    old.unlink(missing_ok=True)

        complete = update == recipe["schedule"]["maximumUpdates"]
        report = {"schema": TRAINING_RUN_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_sha256(config_path), "complete": complete, "stopReason": "maximum-updates" if complete else "planned-gated-boundary", "optimizerUpdates": update, "examplesSeen": examples_seen, "equivalentPasses": examples_seen / training_data["records"], "dataCursor": {"epoch": cursor.epoch, "record": cursor.record}, "bestValidation": best, "artifacts": artifacts, "timingSeconds": timing, "approval": approval, "continuationReceipts": continuation_receipts, "activeLeg": active_leg, "approvedMaximumPlannedUpdate": approved_horizon, "nextContinuationRequiresNewReceipt": not complete and active_leg is None, "selectedCheckpointExport": "required-explicitly-via-selection-history-bound-export-subcommand", "paidExecutionDoesNotAuthorizeFurtherRuns": True}
        atomic_write_json(output / "run.json", report)
        print(f"training state written to {output}; complete={complete}; update={update}")
    finally:
        context_temp.cleanup()


def checkpoint_identity(value: Any, where: str) -> dict[str, Any]:
    item = exact_keys(value, {"update", "path", "sha256"}, where)
    nonnegative_integer(item["update"], f"{where}.update")
    artifact_shape({"path": item["path"], "sha256": item["sha256"]}, where, with_bytes=False)
    return item


def validation_loss_from_journal(path: Path, update: int) -> float:
    try:
        rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot read comparison metrics journal {path}: {exc}")
    matches = [row for row in rows if isinstance(row, dict) and row.get("optimizerUpdates") == update]
    if len(matches) != 1:
        fail(f"comparison metrics journal must contain exactly one row for update {update}")
    exact(matches[0].get("schema"), METRICS_SCHEMA, "comparison metrics row.schema")
    validation = matches[0].get("validation")
    if not isinstance(validation, dict) or "teacherOnlyLoss" not in validation:
        fail("comparison metrics row lacks validation teacher-only loss")
    return finite_number(validation["teacherOnlyLoss"], "comparison validation loss", minimum=0.0)


def validate_comparison_receipt(path: Path, config: dict[str, Any], config_path: Path, expected_checkpoint: dict[str, Any]) -> dict[str, Any]:
    receipt = load_json(path)
    exact_keys(receipt, {"schema", "experimentId", "configSha256", "createdAt", "checkpoint", "candidateRuntime", "equalNodeMatch", "fixedTimeMatch", "validationMetric", "status"}, "comparison receipt")
    exact(receipt["schema"], COMPARISON_SCHEMA, "comparison receipt.schema")
    exact(receipt["experimentId"], config["experimentId"], "comparison receipt.experimentId")
    exact(receipt["configSha256"], config_sha256(config_path), "comparison receipt.configSha256")
    timestamp(receipt["createdAt"], "comparison receipt.createdAt")
    exact(receipt["checkpoint"], expected_checkpoint, "comparison receipt.checkpoint")
    runtime_item = artifact_shape(receipt["candidateRuntime"], "comparison receipt.candidateRuntime", with_bytes=False)
    runtime_path = verify_artifact(runtime_item, "comparison receipt.candidateRuntime")
    runtime = validate_candidate_runtime(runtime_path, config, config_path, expected_checkpoint)
    equal_item = artifact_shape(receipt["equalNodeMatch"], "comparison receipt.equalNodeMatch", with_bytes=False)
    equal_path = verify_artifact(equal_item, "comparison receipt.equalNodeMatch")
    equal_match = validate_raw_match(equal_path, config, config_path, "final-equal-node", runtime_item, runtime, expected_checkpoint["update"])
    fixed_item = receipt["fixedTimeMatch"]
    fixed_match = None
    if fixed_item is not None:
        artifact_shape(fixed_item, "comparison receipt.fixedTimeMatch", with_bytes=False)
        fixed_path = verify_artifact(fixed_item, "comparison receipt.fixedTimeMatch")
        fixed_match = validate_raw_match(fixed_path, config, config_path, "final-fixed-time", runtime_item, runtime, expected_checkpoint["update"])
    metric = exact_keys(receipt["validationMetric"], {"metricsJournal", "update", "teacherOnlyLoss"}, "comparison receipt.validationMetric")
    metric_item = artifact_shape(metric["metricsJournal"], "comparison receipt.validationMetric.metricsJournal", with_bytes=False)
    metric_path = verify_artifact(metric_item, "comparison receipt.validationMetric.metricsJournal")
    expected_metric_path = workspace_path(config["output"]["directory"]) / "metrics.jsonl"
    if metric_path.resolve() != expected_metric_path.resolve() and not is_run_local_receipt_path(metric_path, config):
        fail("comparison receipt does not reference this run's metrics journal")
    exact(metric["update"], expected_checkpoint["update"], "comparison receipt.validationMetric.update")
    validation_loss = validation_loss_from_journal(metric_path, expected_checkpoint["update"])
    same_number(metric["teacherOnlyLoss"], validation_loss, "comparison receipt.validationMetric.teacherOnlyLoss")
    exact(receipt["status"], "passed", "comparison receipt.status")
    chosen_runtime = fixed_match if fixed_match is not None else equal_match
    return {"path": str(path), "sha256": sha256(path), "fixedTimeScoreRate": None if fixed_match is None else fixed_match["summary"]["meanScore"], "equalNodeScoreRate": equal_match["summary"]["meanScore"], "runtimeCost": chosen_runtime["runtimeCost"]["value"], "validationLoss": validation_loss, "candidateRuntime": runtime, "equalNodeMatch": equal_match, "fixedTimeMatch": fixed_match, "status": receipt["status"]}


def validate_terminal_evidence(item: Any, config: dict[str, Any], config_path: Path, run_update: int) -> dict[str, Any]:
    terminal = exact_keys(item, {"role", "path", "sha256"}, "selection history.terminalEvidence")
    terminal_path = verify_artifact({"path": terminal["path"], "sha256": terminal["sha256"]}, "selection history.terminalEvidence")
    if terminal["role"] == "throughput-budget-stop":
        exact(run_update, config["recipe"]["checkpoints"]["throughputProbeUpdate"], "budget-terminal run update")
        evidence = validate_throughput_evidence(terminal_path, config, config_path, require_continue=False)
        exact(evidence["decision"], "stop-budget", "budget-terminal decision")
    elif terminal["role"] == f"strength-rule-stop-u{run_update:08d}":
        evidence = validate_screen_evidence(terminal_path, config, config_path, run_update)
        exact(evidence["decision"], "stop-two-successive-low-screens", "strength-terminal decision")
    else:
        fail("selection terminal evidence has an unsupported role")
    return {"role": terminal["role"], "path": str(terminal_path), "sha256": terminal["sha256"], "evidence": evidence}


def validate_selection_history(path: Path, config: dict[str, Any], config_path: Path) -> dict[str, Any]:
    history = load_json(path)
    exact_keys(history, {"schema", "experimentId", "configSha256", "createdAt", "run", "terminalEvidence", "candidates", "scheduledScreens", "rankingOrder", "selectedCheckpoint", "decision"}, "selection history")
    exact(history["schema"], SELECTION_HISTORY_SCHEMA, "selection history.schema")
    exact(history["experimentId"], config["experimentId"], "selection history.experimentId")
    exact(history["configSha256"], config_sha256(config_path), "selection history.configSha256")
    timestamp(history["createdAt"], "selection history.createdAt")
    run_artifact = artifact_shape(history["run"], "selection history.run", with_bytes=False)
    run_path = verify_artifact(run_artifact, "selection history.run")
    expected_run_path = workspace_path(config["output"]["directory"]) / "run.json"
    if run_path.resolve() != expected_run_path.resolve() and not is_run_local_receipt_path(run_path, config):
        fail("selection history does not reference this experiment's run.json")
    run = load_json(run_path)
    exact_keys(run, {"schema", "experimentId", "configSha256", "complete", "stopReason", "optimizerUpdates", "examplesSeen", "equivalentPasses", "dataCursor", "bestValidation", "artifacts", "timingSeconds", "approval", "continuationReceipts", "activeLeg", "approvedMaximumPlannedUpdate", "nextContinuationRequiresNewReceipt", "selectedCheckpointExport", "paidExecutionDoesNotAuthorizeFurtherRuns"}, "selection history run")
    exact(run["schema"], TRAINING_RUN_SCHEMA, "selection history run.schema")
    exact(run["experimentId"], config["experimentId"], "selection history run.experimentId")
    exact(run["configSha256"], config_sha256(config_path), "selection history run.configSha256")
    run_update = nonnegative_integer(run["optimizerUpdates"], "selection history run.optimizerUpdates")
    if run_update == 0 or run_update % config["recipe"]["evaluation"]["everyUpdates"] != 0:
        fail("selection history run is not at a committed evaluation boundary")
    exact(run["paidExecutionDoesNotAuthorizeFurtherRuns"], True, "selection history run.paidExecutionDoesNotAuthorizeFurtherRuns")
    run_horizon = run["approvedMaximumPlannedUpdate"]
    if run_horizon is not None:
        positive_integer(run_horizon, "selection history run.approvedMaximumPlannedUpdate")
        if run_update > run_horizon:
            fail("selection history run crossed its measured approved horizon")
    if run["complete"] is True:
        exact(run_update, config["recipe"]["schedule"]["maximumUpdates"], "complete run optimizer updates")
        exact(run["stopReason"], "maximum-updates", "complete run stopReason")
        exact(run["activeLeg"], None, "complete run activeLeg")
        exact(run["nextContinuationRequiresNewReceipt"], False, "complete run nextContinuationRequiresNewReceipt")
        exact(history["terminalEvidence"], None, "complete-run terminalEvidence")
        exact(run_horizon, config["recipe"]["schedule"]["maximumUpdates"], "complete run approvedMaximumPlannedUpdate")
        terminal = None
    elif run["complete"] is False:
        exact(run["stopReason"], "planned-gated-boundary", "terminal run stopReason")
        exact(run["activeLeg"], None, "terminal run activeLeg")
        exact(run["nextContinuationRequiresNewReceipt"], True, "terminal run nextContinuationRequiresNewReceipt")
        terminal = validate_terminal_evidence(history["terminalEvidence"], config, config_path, run_update)
        if terminal["role"] == "throughput-budget-stop":
            exact(terminal["evidence"]["maximumPlannedUpdate"], run_update, "budget-terminal maximumPlannedUpdate")
        elif run_horizon is None or run_horizon < run_update:
            fail("strength-terminal run lacks a persisted horizon covering its update")
    else:
        fail("selection history run.complete must be boolean")
    if not isinstance(run["artifacts"], list):
        fail("selection history run.artifacts must be a list")
    recorded: dict[tuple[int, str], dict[str, Any]] = {}
    for index, artifact in enumerate(run["artifacts"]):
        if not isinstance(artifact, dict) or "role" not in artifact:
            fail(f"selection history run.artifacts[{index}] lacks an artifact role")
        if artifact["role"] == "scheduled-export":
            exact_keys(artifact, {"role", "update", "sourceCheckpoint", "serializer", "integerNetwork", "seconds", "recoveredOrphan", "parityStatus", "quantizationErrorStatus"}, f"selection history run.artifacts[{index}]")
            nonnegative_integer(artifact["update"], f"run.artifacts[{index}].update")
            artifact_shape(artifact["sourceCheckpoint"], f"run.artifacts[{index}].sourceCheckpoint", with_bytes=True)
            exact_keys(artifact["serializer"], {"path", "bytes", "sha256", "pinnedTrainerRevision"}, f"run.artifacts[{index}].serializer")
            artifact_shape(artifact["integerNetwork"], f"run.artifacts[{index}].integerNetwork", with_bytes=True)
            continue
        if artifact["role"] not in {"best-validation", "throughput-probe", "scheduled-strength-screen", "final"} or not {"update", "path", "sha256"}.issubset(artifact):
            fail(f"selection history run.artifacts[{index}] has an unsupported checkpoint shape")
        key = (nonnegative_integer(artifact["update"], f"run.artifacts[{index}].update"), artifact["sha256"])
        recorded[key] = artifact

    screens = history["scheduledScreens"]
    expected_screen_updates = [update for update in config["strengthProtocol"]["scheduledScreens"]["updates"] if update <= run_update]
    if not isinstance(screens, list) or len(screens) != len(expected_screen_updates):
        fail("selection history must bind every scheduled strength screen")
    validated_screens: list[dict[str, Any]] = []
    for index, (screen, update) in enumerate(zip(screens, expected_screen_updates)):
        artifact_shape(screen, f"selection history.scheduledScreens[{index}]", with_bytes=False)
        screen_path = verify_artifact(screen, f"selection history.scheduledScreens[{index}]")
        validated_screens.append(validate_screen_evidence(screen_path, config, config_path, update))
    highest_screen = max(validated_screens, key=lambda item: (item["meanScore"], -item["update"])) if validated_screens else None

    candidates = history["candidates"]
    maximum = config["strengthProtocol"]["finalSelection"]["maximumComparedCheckpoints"]
    if not isinstance(candidates, list) or not 1 <= len(candidates) <= maximum:
        fail(f"selection history must contain one through {maximum} deduplicated candidates")
    validated_candidates: list[dict[str, Any]] = []
    observed_roles: list[str] = []
    observed_updates: set[int] = set()
    for index, candidate in enumerate(candidates):
        where = f"selection history.candidates[{index}]"
        exact_keys(candidate, {"candidateRoles", "checkpoint", "comparisonReceipt"}, where)
        roles = candidate["candidateRoles"]
        if not isinstance(roles, list) or not roles or any(role not in config["strengthProtocol"]["finalSelection"]["candidateRoles"] for role in roles):
            fail(f"{where}.candidateRoles is invalid")
        if len(set(roles)) != len(roles):
            fail(f"{where}.candidateRoles contains duplicates")
        checkpoint = checkpoint_identity(candidate["checkpoint"], f"{where}.checkpoint")
        checkpoint_path = verify_artifact({"path": checkpoint["path"], "sha256": checkpoint["sha256"]}, f"{where}.checkpoint")
        identity = (checkpoint["update"], checkpoint["sha256"])
        if identity not in recorded:
            fail(f"{where}.checkpoint is not an immutable artifact recorded by this run")
        if checkpoint["update"] in observed_updates:
            fail("selection history did not deduplicate the same checkpoint")
        observed_updates.add(checkpoint["update"])
        if "lowest-validation-loss" in roles:
            best = run["bestValidation"]
            exact(checkpoint["update"], best["update"], f"{where} lowest-validation update")
        if "highest-scheduled-screen-score" in roles and highest_screen is not None:
            exact(checkpoint["update"], highest_screen["update"], f"{where} highest-screen update")
        elif "highest-scheduled-screen-score" in roles:
            fail("selection assigned highest-screen role without an available screen")
        observed_roles.extend(roles)
        comparison_item = artifact_shape(candidate["comparisonReceipt"], f"{where}.comparisonReceipt", with_bytes=False)
        comparison_path = verify_artifact(comparison_item, f"{where}.comparisonReceipt")
        comparison = validate_comparison_receipt(comparison_path, config, config_path, checkpoint)
        export_run = comparison["candidateRuntime"]["candidateExport"]["trainingRun"]
        exact(export_run["optimizerUpdates"], run_update, f"{where} candidate export run update")
        exact(export_run["complete"], run["complete"], f"{where} candidate export run completion")
        exact(export_run["stopReason"], run["stopReason"], f"{where} candidate export run stop reason")
        exact(export_run["approvedMaximumPlannedUpdate"], run_horizon, f"{where} candidate export run horizon")
        validated_candidates.append({"candidateRoles": roles, "checkpoint": {**checkpoint, "resolvedPath": str(checkpoint_path)}, "comparison": comparison})
    expected_roles = ["lowest-validation-loss"] + (["highest-scheduled-screen-score"] if highest_screen is not None else [])
    exact(sorted(observed_roles), sorted(expected_roles), "selection history candidate role coverage")
    ranking = config["strengthProtocol"]["finalSelection"]["rankingOrder"]
    exact(history["rankingOrder"], ranking, "selection history.rankingOrder")
    use_fixed_time = all(item["comparison"]["fixedTimeScoreRate"] is not None for item in validated_candidates)
    def rank_key(item: dict[str, Any]) -> tuple[float, float, float, float, int]:
        comparison = item["comparison"]
        fixed_key = -float(comparison["fixedTimeScoreRate"]) if use_fixed_time else 0.0
        return (fixed_key, -comparison["equalNodeScoreRate"], comparison["runtimeCost"], comparison["validationLoss"], item["checkpoint"]["update"])
    winner = min(validated_candidates, key=rank_key)
    selected = checkpoint_identity(history["selectedCheckpoint"], "selection history.selectedCheckpoint")
    exact((selected["update"], selected["path"], selected["sha256"]), (winner["checkpoint"]["update"], winner["checkpoint"]["path"], winner["checkpoint"]["sha256"]), "selection history.selectedCheckpoint")
    exact(history["decision"], "selected-by-frozen-ranking", "selection history.decision")
    return {"schema": history["schema"], "path": str(path), "sha256": sha256(path), "run": {"path": str(run_path), "sha256": run_artifact["sha256"]}, "terminalEvidence": terminal, "selectedCheckpoint": selected, "usedFixedTimeRanking": use_fixed_time}


def run_explicit_export(args: argparse.Namespace, config: dict[str, Any], config_path: Path) -> None:
    verify_inputs(config)
    checkpoint = args.checkpoint.resolve()
    output = args.output.resolve()
    if not checkpoint.is_file():
        fail(f"explicit selected checkpoint is missing: {checkpoint}")
    if output.suffix != ".nnue" or output == checkpoint:
        fail("explicit export output must be a distinct .nnue path")
    source_selection = args.selection_history.resolve()
    validate_selection_history(source_selection, config, config_path)
    receipt_dir = workspace_path(config["output"]["directory"]) / "receipts" / "selected-export"
    local_selection = snapshot_receipt_graph(source_selection, receipt_dir)
    selection = validate_selection_history(local_selection, config, config_path)
    selected = selection["selectedCheckpoint"]
    if checkpoint != workspace_path(selected["path"]).resolve() or sha256(checkpoint) != selected["sha256"]:
        fail("--checkpoint is not the checkpoint selected by the bound selection history")
    context_temp, context = r5.prepare_context(config["recipe"]["width"], args.cmake)
    try:
        try:
            import torch  # type: ignore
        except ImportError as exc:
            fail(f"explicit export requires torch: {exc}")
        r5.import_trainer(context)
        record = export_checkpoint(torch, checkpoint, output, context, config["recipe"], config["experimentId"])
        report = {"schema": SELECTION_EXPORT_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_sha256(config_path), "selectionWasExplicit": True, "selectionHistory": selection, **record}
        report_path = output.with_suffix(output.suffix + ".json")
        atomic_write_json(report_path, report)
        print(f"explicit export written to {output}")
    finally:
        context_temp.cleanup()


def run_candidate_export(args: argparse.Namespace, config: dict[str, Any], config_path: Path) -> None:
    verify_inputs(config)
    checkpoint_path = args.checkpoint.resolve()
    if not checkpoint_path.is_file():
        fail(f"candidate checkpoint is missing: {checkpoint_path}")
    run_path = workspace_path(config["output"]["directory"]) / "run.json"
    run = load_json(run_path)
    matches: list[dict[str, Any]] = []
    for artifact in run.get("artifacts", []):
        if not isinstance(artifact, dict) or artifact.get("role") not in {"best-validation", "throughput-probe", "scheduled-strength-screen", "final"}:
            continue
        if workspace_path(artifact.get("path", "")).resolve() == checkpoint_path and artifact.get("sha256") == sha256(checkpoint_path):
            matches.append(artifact)
    if len(matches) != 1:
        fail("--checkpoint must be one unique checkpoint recorded by the current run.json")
    checkpoint = {"update": matches[0]["update"], "path": matches[0]["path"], "sha256": matches[0]["sha256"]}
    validate_recorded_checkpoint(run_path, config, config_path, checkpoint)
    receipt_dir = workspace_path(config["output"]["directory"]) / "receipts" / "candidate-exports" / f"u{checkpoint['update']:08d}"
    local_run = snapshot_receipt_graph(run_path, receipt_dir)
    local_run_item = {"path": str(local_run), "sha256": sha256(local_run)}
    validate_recorded_checkpoint(local_run, config, config_path, checkpoint)
    output = workspace_path(config["output"]["directory"]) / "exports" / "candidates" / f"candidate-u{checkpoint['update']:08d}.nnue"
    context_temp, context = r5.prepare_context(config["recipe"]["width"], args.cmake)
    try:
        try:
            import torch  # type: ignore
        except ImportError as exc:
            fail(f"candidate export requires torch: {exc}")
        r5.import_trainer(context)
        record = export_checkpoint(torch, checkpoint_path, output, context, config["recipe"], config["experimentId"])
        report = {"schema": CANDIDATE_EXPORT_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_sha256(config_path), "createdAt": datetime.now(timezone.utc).isoformat(), "run": local_run_item, "checkpoint": checkpoint, "format": config["recipe"]["export"]["format"], "sourceCheckpoint": record["sourceCheckpoint"], "serializer": record["serializer"], "integerNetwork": record["integerNetwork"], "status": "passed"}
        report_path = output.with_suffix(output.suffix + ".json")
        atomic_write_json(report_path, report)
        validate_candidate_export(report_path, config, config_path, checkpoint)
        print(f"preselection candidate export written to {output}")
    finally:
        context_temp.cleanup()


def fixture_recipe(config: dict[str, Any]) -> dict[str, Any]:
    recipe = copy.deepcopy(config["recipe"])
    fixture = recipe["realCpuFixture"]
    recipe["width"] = fixture["width"]
    recipe["loader"]["batchSize"] = fixture["batchSize"]
    recipe["schedule"] = fixture["schedule"]
    recipe["diagnostics"]["updateMagnitudeEveryUpdates"] = 1
    return recipe


def import_cpu_fixture_trainer(context: Path) -> tuple[Any, Any, Any, Any]:
    """Import the pinned stack without making CuPy a CPU-fixture dependency.

    The pinned ``feature_transformer.py`` imports CuPy eagerly, but touches it
    only in its CUDA kernel factories.  Candidate training calls
    ``r5.import_trainer`` directly and never sees this shim.
    """
    missing = object()
    prior = sys.modules.get("cupy", missing)
    stub = types.ModuleType("cupy")

    def reject_cuda_kernel(*_args: Any, **_kwargs: Any) -> Any:
        fail("the CPU fixture attempted to enter a CuPy CUDA kernel path")

    stub.RawKernel = reject_cuda_kernel  # type: ignore[attr-defined]
    sys.modules["cupy"] = stub
    try:
        return r5.import_trainer(context)
    finally:
        if prior is missing:
            sys.modules.pop("cupy", None)
        else:
            sys.modules["cupy"] = prior


def install_cpu_fixture_cupy_module(context: Path) -> Path:
    """Let the fixture's serializer subprocess import the pinned model on CPU."""
    module = context / "cupy.py"
    if module.exists():
        fail(f"refusing to replace an existing pinned-context module: {module}")
    module.write_text(
        "def RawKernel(*args, **kwargs):\n"
        "    raise RuntimeError('the CPU fixture attempted to enter a CuPy CUDA kernel path')\n",
        encoding="utf-8",
    )
    return module


def install_cpu_feature_transformer(model_module: Any, torch: Any) -> tuple[Any, Any]:
    """Give the pinned sparse feature transformer a fixture-only CPU forward."""
    transformer = model_module.DoubleFeatureTransformerSlice
    original = transformer.forward

    def cpu_forward(self: Any, indices_0: Any, values_0: Any, indices_1: Any, values_1: Any) -> tuple[Any, Any]:
        def accumulate(indices: Any, values: Any) -> Any:
            if indices.is_cuda or values.is_cuda or self.weight.is_cuda or self.bias.is_cuda:
                fail("the fixture-only feature-transformer path received CUDA tensors")
            valid = indices >= 0
            safe_indices = indices.clamp_min(0).long()
            gathered = torch.nn.functional.embedding(safe_indices, self.weight)
            weighted = gathered * values.unsqueeze(-1) * valid.unsqueeze(-1)
            return weighted.sum(dim=1) + self.bias

        return accumulate(indices_0, values_0), accumulate(indices_1, values_1)

    transformer.forward = cpu_forward
    return transformer, original


def install_cpu_batch_tensors(nnue_dataset: Any, torch: Any) -> tuple[Any, Any]:
    """Avoid the pinned wrapper's unconditional CUDA-oriented pin_memory call."""
    batch_type = nnue_dataset.SparseBatch
    original = batch_type.get_tensors

    def cpu_tensors(self: Any, device: Any) -> tuple[Any, ...]:
        if str(device) != "cpu":
            fail("the fixture-only SparseBatch adapter received a non-CPU device")
        size = self.size
        width = self.max_active_features

        def tensor(pointer: Any, shape: tuple[int, ...]) -> Any:
            # Copy before the production loader destroys the C++ SparseBatch.
            array = np.ctypeslib.as_array(pointer, shape=shape).copy()
            return torch.from_numpy(array)

        white_values = tensor(self.white_values, (size, width))
        black_values = tensor(self.black_values, (size, width))
        white_indices = tensor(self.white, (size, width))
        black_indices = tensor(self.black, (size, width))
        us = tensor(self.is_white, (size, 1))
        them = 1.0 - us
        outcome = tensor(self.outcome, (size, 1))
        score = tensor(self.score, (size, 1))
        psqt = tensor(self.psqt_indices, (size,)).long()
        stacks = tensor(self.layer_stack_indices, (size,)).long()
        return us, them, white_indices, white_values, black_indices, black_values, outcome, score, psqt, stacks

    batch_type.get_tensors = cpu_tensors
    return batch_type, original


def fixture_evaluate(torch: Any, model: Any, nnue_dataset: Any, recipe: dict[str, Any], filename: Path, records: int, device: Any) -> tuple[float, np.ndarray]:
    loss_sum = 0.0
    predictions: list[np.ndarray] = []
    seen = 0
    model.eval()
    with torch.no_grad():
        for batch in finite_batches(nnue_dataset, recipe["featureFamily"], filename, records, recipe["loader"]["batchSize"], device, recipe["loader"]):
            _mean, losses, prediction, _target = loss_tensors(torch, model, batch, recipe["objective"])
            count = int(losses.shape[0])
            loss_sum += float(losses.sum().cpu())
            predictions.append(prediction.detach().cpu().numpy().astype("<f8"))
            seen += count
    if seen != records:
        fail("real CPU fixture evaluation exposure differs")
    return loss_sum / seen, np.concatenate(predictions)


def fixture_train(
    torch: Any,
    model_module: Any,
    features: Any,
    nnue_dataset: Any,
    recipe: dict[str, Any],
    source: Path,
    order_dir: Path,
    target_update: int,
    resume_state: dict[str, Any] | None = None,
) -> tuple[Any, Any, Cursor, int, list[dict[str, Any]]]:
    device = torch.device("cpu")
    seed_everything(torch, recipe["realCpuFixture"]["seed"])
    feature_set = features.get_feature_set_from_name(recipe["featureFamily"])
    model = model_module.NNUE(feature_set=feature_set, lambda_=recipe["objective"]["lambda"], learning_rate=recipe["schedule"]["baseLearningRate"]).to(device)
    optimizer = configured_optimizer(model, recipe)
    cursor = Cursor()
    update = 0
    examples_seen = 0
    trace: list[dict[str, Any]] = []
    if resume_state is not None:
        exact(resume_state["schema"], "haitaka-anhoku-strength-first-real-fixture-state-v1", "real fixture state schema")
        model.load_state_dict(resume_state["model"])
        optimizer.load_state_dict(resume_state["optimizer"])
        cursor = Cursor(**resume_state["dataCursor"])
        update = resume_state["optimizerUpdates"]
        examples_seen = resume_state["examplesSeen"]
        trace = list(resume_state["trace"])
        restore_rng(torch, resume_state["rng"])
    records = recipe["realCpuFixture"]["records"]
    while update < target_update:
        order = epoch_permutation(records, recipe["realCpuFixture"]["seed"], cursor.epoch)
        shuffled = order_dir / f"epoch-{cursor.epoch:04d}.bin"
        materialize_epoch(source, shuffled, records, recipe["realCpuFixture"]["entryBytes"], order)
        start_record = cursor.record
        for batch in finite_batches(nnue_dataset, recipe["featureFamily"], shuffled, records, recipe["loader"]["batchSize"], device, recipe["loader"], skip=start_record):
            if update >= target_update:
                break
            count = int(batch[0].shape[0])
            indices = order[cursor.record:cursor.record + count]
            next_update = update + 1
            model.train()
            model._clip_weights()
            base_lr = set_learning_rates(optimizer, recipe, next_update)
            optimizer.zero_grad(set_to_none=True)
            loss, _losses, _predictions, _targets = loss_tensors(torch, model, batch, recipe["objective"])
            if not bool(torch.isfinite(loss)):
                fail(f"real fixture produced non-finite loss at update {next_update}")
            loss.backward()
            gradient_magnitude(torch, model)
            optimizer.step()
            model._clip_weights()
            assert_finite_training_state(torch, model, optimizer, f"real fixture post-update {next_update}")
            trace.append({"update": next_update, "epoch": cursor.epoch, "record": cursor.record, "records": count, "indicesSha256": hashlib.sha256(indices.astype("<i8").tobytes()).hexdigest(), "learningRate": base_lr, "loss": float(loss.detach().cpu())})
            cursor.advance(count, records)
            update = next_update
            examples_seen += count
            if update >= target_update:
                break
    return model, optimizer, cursor, examples_seen, trace


def nested_equal(torch: Any, left: Any, right: Any) -> bool:
    if torch.is_tensor(left) or torch.is_tensor(right):
        return bool(torch.is_tensor(left) and torch.is_tensor(right) and torch.equal(left.detach().cpu(), right.detach().cpu()))
    if isinstance(left, dict) or isinstance(right, dict):
        return bool(isinstance(left, dict) and isinstance(right, dict) and set(left) == set(right) and all(nested_equal(torch, left[key], right[key]) for key in left))
    if isinstance(left, (list, tuple)) or isinstance(right, (list, tuple)):
        return bool(type(left) is type(right) and len(left) == len(right) and all(nested_equal(torch, a, b) for a, b in zip(left, right)))
    return left == right


def run_real_cpu_fixture(args: argparse.Namespace, config: dict[str, Any]) -> None:
    verify_inputs(config)
    fixture = config["recipe"]["realCpuFixture"]
    output = args.output_dir.resolve() if args.output_dir is not None else workspace_path(fixture["outputDirectory"])
    if output.exists():
        fail(f"real CPU fixture refuses existing output: {output}")
    training_binary = workspace_path(config["recipe"]["data"]["training"]["binary"]["path"])
    expected_bytes = fixture["records"] * fixture["entryBytes"]
    with training_binary.open("rb") as stream:
        payload = stream.read(expected_bytes)
    if len(payload) != expected_bytes:
        fail("training binary is too short for the real CPU fixture")

    try:
        import torch  # type: ignore
    except ImportError as exc:
        fail(f"real CPU fixture requires torch: {exc}")
    recipe = fixture_recipe(config)
    context_temp, context = r5.prepare_context(fixture["width"], args.cmake)
    patched_transformer: tuple[Any, Any] | None = None
    patched_batch: tuple[Any, Any] | None = None
    try:
        torch, features, model_module, nnue_dataset = import_cpu_fixture_trainer(context)
        patched_transformer = install_cpu_feature_transformer(model_module, torch)
        patched_batch = install_cpu_batch_tensors(nnue_dataset, torch)
        # Dependency and pinned-context import failures leave no experiment
        # directory that could be mistaken for a scientific fixture attempt.
        output.mkdir(parents=True)
        source = output / "fixture.bin"
        source.write_bytes(payload)
        seed_everything(torch, fixture["seed"])
        feature_set = features.get_feature_set_from_name(recipe["featureFamily"])
        initial_model = model_module.NNUE(feature_set=feature_set, lambda_=recipe["objective"]["lambda"], learning_rate=recipe["schedule"]["baseLearningRate"]).to("cpu")
        initial_model._clip_weights()
        initial_loss, _initial_predictions = fixture_evaluate(torch, initial_model, nnue_dataset, recipe, source, fixture["records"], torch.device("cpu"))
        del initial_model

        uninterrupted = fixture_train(torch, model_module, features, nnue_dataset, recipe, source, output / "orders-uninterrupted", fixture["maximumUpdates"])
        partial = fixture_train(torch, model_module, features, nnue_dataset, recipe, source, output / "orders-resume", fixture["interruptAfterUpdates"])
        partial_model, partial_optimizer, partial_cursor, partial_examples, partial_trace = partial
        resume_state = {"schema": "haitaka-anhoku-strength-first-real-fixture-state-v1", "model": partial_model.state_dict(), "optimizer": partial_optimizer.state_dict(), "schedule": {"kind": recipe["schedule"]["kind"], "nextOptimizerUpdate": fixture["interruptAfterUpdates"] + 1}, "rng": rng_state(torch), "optimizerUpdates": fixture["interruptAfterUpdates"], "examplesSeen": partial_examples, "dataCursor": {"epoch": partial_cursor.epoch, "record": partial_cursor.record}, "trace": partial_trace}
        resume_path = output / "actual-resume-state.pt"
        atomic_torch_save(torch, resume_state, resume_path)
        del partial_model, partial_optimizer, resume_state
        loaded = torch.load(resume_path, map_location="cpu", weights_only=False)
        resumed = fixture_train(torch, model_module, features, nnue_dataset, recipe, source, output / "orders-resume", fixture["maximumUpdates"], loaded)

        uninterrupted_model, uninterrupted_optimizer, uninterrupted_cursor, uninterrupted_examples, uninterrupted_trace = uninterrupted
        resumed_model, resumed_optimizer, resumed_cursor, resumed_examples, resumed_trace = resumed
        uninterrupted_loss, uninterrupted_predictions = fixture_evaluate(torch, uninterrupted_model, nnue_dataset, recipe, source, fixture["records"], torch.device("cpu"))
        resumed_loss, resumed_predictions = fixture_evaluate(torch, resumed_model, nnue_dataset, recipe, source, fixture["records"], torch.device("cpu"))
        weights_equal = nested_equal(torch, uninterrupted_model.state_dict(), resumed_model.state_dict())
        optimizers_equal = nested_equal(torch, uninterrupted_optimizer.state_dict(), resumed_optimizer.state_dict())
        predictions_equal = np.array_equal(uninterrupted_predictions, resumed_predictions)
        traces_equal = uninterrupted_trace == resumed_trace
        cursors_equal = uninterrupted_cursor == resumed_cursor and uninterrupted_examples == resumed_examples
        partial_observed = fixture["finiteBatchSizesPerEpoch"][-1] in [item["records"] for item in uninterrupted_trace]

        uninterrupted_checkpoint = output / "uninterrupted-final.pt"
        resumed_checkpoint = output / "resumed-final.pt"
        save_portable_model(torch, uninterrupted_model, uninterrupted_checkpoint)
        save_portable_model(torch, resumed_model, resumed_checkpoint)
        # ``serialize.py`` starts a new interpreter, so the temporary
        # ``sys.modules`` shim used for the pinned-stack import is not visible
        # there.  This module exists only in the disposable fixture context.
        install_cpu_fixture_cupy_module(context)
        uninterrupted_export = export_checkpoint(torch, uninterrupted_checkpoint, output / "uninterrupted-final.nnue", context, recipe, config["experimentId"] + "-real-fixture")
        resumed_export = export_checkpoint(torch, resumed_checkpoint, output / "resumed-final.nnue", context, recipe, config["experimentId"] + "-real-fixture")
        exports_equal = uninterrupted_export["integerNetwork"]["sha256"] == resumed_export["integerNetwork"]["sha256"]
        loss_decreased = resumed_loss < initial_loss
        passed = all((weights_equal, optimizers_equal, predictions_equal, traces_equal, cursors_equal, partial_observed, exports_equal, loss_decreased, resumed_loss == uninterrupted_loss))
        reported_fixture = copy.deepcopy(fixture)
        reported_fixture["outputDirectory"] = display_path(output)
        report = {
            "schema": "haitaka-anhoku-strength-first-real-cpu-fixture-v1",
            "runnerVersion": RUNNER_VERSION,
            "passed": passed,
            "actualComponents": {
                "pinnedNnue": True,
                "optimizer": type(resumed_optimizer).__name__,
                "productionLoader": True,
                "device": "cpu",
                "fixtureOnlyCuPyImportStub": True,
                "fixtureOnlyCpuFeatureTransformer": True,
                "fixtureOnlyCpuTensorTransfer": True,
            },
            "fixture": reported_fixture,
            "exposure": {"initialRecords": fixture["records"], "uninterruptedExamples": uninterrupted_examples, "resumedExamples": resumed_examples, "finalCursor": {"epoch": resumed_cursor.epoch, "record": resumed_cursor.record}, "observedBatchSizes": [item["records"] for item in resumed_trace]},
            "loss": {"initial": initial_loss, "uninterruptedFinal": uninterrupted_loss, "resumedFinal": resumed_loss, "decreased": loss_decreased},
            "resumeEquivalence": {"weightsExact": weights_equal, "optimizerExact": optimizers_equal, "predictionsExact": predictions_equal, "traceExact": traces_equal, "cursorExact": cursors_equal},
            "partialBatchObserved": partial_observed,
            "exportsExact": exports_equal,
            "uninterruptedExport": uninterrupted_export,
            "resumedExport": resumed_export,
            "resumeState": {"path": display_path(resume_path), "bytes": resume_path.stat().st_size, "sha256": sha256(resume_path)},
            "qualificationLimits": fixture["qualificationLimits"],
        }
        (output / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        if not passed:
            fail(f"real CPU fixture failed; inspect {output / 'report.json'}")
        print(f"real CPU fixture passed: {output / 'report.json'}")
    finally:
        if patched_batch is not None:
            batch_type, original = patched_batch
            batch_type.get_tensors = original
        if patched_transformer is not None:
            transformer, original = patched_transformer
            transformer.forward = original
        context_temp.cleanup()


def synthetic_run(seed: int, updates: int, state: dict[str, Any] | None = None) -> dict[str, Any]:
    records, batch_size, dimensions = 23, 6, 5
    features = np.arange(records * dimensions, dtype=np.float64).reshape(records, dimensions) / 100.0
    targets = np.sin(np.arange(records, dtype=np.float64))
    if state is None:
        generator = np.random.Generator(np.random.PCG64DXSM(seed))
        weights = generator.normal(0.0, 0.01, dimensions)
        momentum = np.zeros(dimensions)
        cursor = Cursor()
        step = 0
        trace: list[str] = []
    else:
        weights = state["weights"].copy()
        momentum = state["momentum"].copy()
        cursor = Cursor(**state["cursor"])
        step = state["step"]
        trace = list(state["trace"])
    while step < updates:
        order = epoch_permutation(records, seed, cursor.epoch)
        count = min(batch_size, records - cursor.record)
        indices = order[cursor.record:cursor.record + count]
        trace.append(hashlib.sha256(indices.astype("<i8").tobytes()).hexdigest())
        prediction = features[indices] @ weights
        gradient = features[indices].T @ (prediction - targets[indices]) / count
        momentum = 0.9 * momentum + gradient
        weights -= 0.01 * momentum
        cursor.advance(count, records)
        step += 1
    return {"weights": weights, "momentum": momentum, "cursor": {"epoch": cursor.epoch, "record": cursor.record}, "step": step, "trace": trace}


def self_test(config: dict[str, Any], config_path: Path) -> None:
    validate_config(config)
    def expect_failure(action: Any, message: str) -> None:
        try:
            action()
        except StrengthFirstError:
            return
        fail(message)

    schedule = config["recipe"]["schedule"]
    assert learning_rate(schedule, 1) == schedule["firstUpdateLearningRate"]
    assert learning_rate(schedule, 64) == schedule["warmupEndpointLearningRate"]
    assert abs(learning_rate(schedule, 4096) - schedule["finalUpdateLearningRate"]) < 1e-15
    spans, cursor = batch_spans(10, 4, Cursor(0, 8), 4)
    assert spans == [(0, 8, 2), (1, 0, 4), (1, 4, 4), (1, 8, 2)] and cursor == Cursor(2, 0)
    first = epoch_permutation(31, 20260906, 3)
    assert np.array_equal(first, epoch_permutation(31, 20260906, 3))
    assert not np.array_equal(first, epoch_permutation(31, 20260906, 4))
    assert sorted(first.tolist()) == list(range(31))
    uninterrupted = synthetic_run(20260906, 19)
    interrupted = synthetic_run(20260906, 7)
    with tempfile.TemporaryDirectory(prefix="strength-first-self-test-") as directory:
        state_path = Path(directory) / "state.pkl"
        temporary = state_path.with_suffix(".tmp")
        temporary.write_bytes(pickle.dumps(interrupted))
        os.replace(temporary, state_path)
        resumed = synthetic_run(20260906, 19, pickle.loads(state_path.read_bytes()))
    assert np.array_equal(uninterrupted["weights"], resumed["weights"])
    assert np.array_equal(uninterrupted["momentum"], resumed["momentum"])
    assert uninterrupted["cursor"] == resumed["cursor"] and uninterrupted["trace"] == resumed["trace"]
    expected_statistics = grouped_screen_statistics([0.5] * 64)
    assert expected_statistics["meanScore"] == 0.5 and expected_statistics["standardError"] == 0.0 and expected_statistics["upper95"] == 0.5
    expect_failure(lambda: grouped_screen_statistics([math.nan] * 64), "screen statistics accepted NaN")
    expect_failure(lambda: finite_number(math.inf, "self-test infinity"), "finite-number guard accepted infinity")

    with tempfile.TemporaryDirectory(prefix="strength-first-receipts-") as directory:
        root = Path(directory)
        config = copy.deepcopy(config)
        config["output"]["directory"] = str(root / "experiment")
        config_digest = config_sha256(config_path)
        quote = config["costForecast"]["liveQuote"]

        def write_payload(name: str, payload: dict[str, Any]) -> Path:
            target = root / name
            atomic_write_json(target, payload)
            return target

        approval = {"schema": APPROVAL_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_digest, "gpuRentalApproved": True, "action": config["authorization"]["approvedAction"], "approvedAt": quote["observedAt"], "validUntil": quote["validUntil"], "commercialEnvelope": {"provider": quote["provider"], "offerId": quote["offerId"], "gpu": quote["gpu"], "diskGigabytes": quote["diskGigabytes"], "quotedCombinedHourlyRateUsd": quote["combinedHourlyRateUsd"], "maximumCombinedHourlyRateUsd": config["costForecast"]["firstRun"]["maximumCombinedHourlyRateUsd"], "maximumRentedUptimeHours": config["budget"]["firstRentalEnvelopeGpuHours"], "maximumTotalCostUsd": config["budget"]["firstRentalEnvelopeUsd"]}}
        approval_path = write_payload("approval.json", approval)
        approval_record = validate_approval(approval_path, config, config_path, now=timestamp(quote["observedAt"], "self-test now"))

        checkpoints: dict[int, dict[str, Any]] = {}
        for update in (128, 256, 1024, 2048, 4096):
            filename = f"throughput-probe-u{update:08d}.pt" if update == 128 else f"scheduled-u{update:08d}.pt"
            checkpoint_path = root / "experiment" / "checkpoints" / filename
            atomic_write_bytes(checkpoint_path, f"finite-checkpoint-{update}\n".encode())
            checkpoints[update] = {"update": update, "path": str(checkpoint_path), "sha256": sha256(checkpoint_path)}

        candidate_run = {
            "schema": TRAINING_RUN_SCHEMA,
            "experimentId": config["experimentId"],
            "configSha256": config_digest,
            "complete": True,
            "stopReason": "maximum-updates",
            "optimizerUpdates": 4096,
            "examplesSeen": 1,
            "equivalentPasses": 1.0,
            "dataCursor": {"epoch": 1, "record": 0},
            "bestValidation": {"loss": 0.01, "update": 1024, "checkpoint": checkpoints[1024]},
            "artifacts": [
                {"role": "throughput-probe" if update == 128 else "scheduled-strength-screen", **checkpoints[update]}
                for update in (128, 256, 1024, 2048, 4096)
            ],
            "timingSeconds": {},
            "approval": approval_record,
            "continuationReceipts": [],
            "activeLeg": None,
            "approvedMaximumPlannedUpdate": 4096,
            "nextContinuationRequiresNewReceipt": False,
            "selectedCheckpointExport": "required-explicitly-via-selection-history-bound-export-subcommand",
            "paidExecutionDoesNotAuthorizeFurtherRuns": True,
        }
        candidate_run_path = write_payload("candidate-run.json", candidate_run)

        throughput = {
            "schema": THROUGHPUT_SCHEMA,
            "experimentId": config["experimentId"],
            "configSha256": config_digest,
            "measuredAt": quote["observedAt"],
            "sourceCheckpoint": checkpoints[128],
            "offer": {"provider": quote["provider"], "offerId": quote["offerId"], "gpu": quote["gpu"], "diskGigabytes": quote["diskGigabytes"], "combinedHourlyRateUsd": quote["combinedHourlyRateUsd"], "observedAt": quote["observedAt"], "validUntil": quote["validUntil"]},
            "measurement": {"startUpdate": 0, "endUpdate": 128, "elapsedGpuHours": 0.05, "updatesPerGpuHour": 2560.0},
            "revisedForecast": {"derivation": {"method": "elapsed-probe-plus-remaining-at-measured-rate-plus-declared-fixed-overhead-v1", "completedProbeGpuHours": 0.05, "remainingUpdates": 3968, "measuredUpdatesPerGpuHour": 2560.0, "minimumRemainingMarginalGpuHours": 1.55, "declaredFixedOverheadSources": ["setupBuildAndRealFixture", "compilationAndResumeRestarts", "parityExportAndTransfers", "paidWaitingForCpuScreens", "contingency"], "declaredFixedOverheadGpuHours": 1.75, "minimumFirstRunGpuHours": 3.35, "tournamentRuns": 3, "minimumTournamentGpuHours": 10.05, "pricingRateUsdPerHour": 0.75, "minimumFirstRunCostUsd": 2.5125, "minimumTournamentCostUsd": 7.5375}, "revisedFirstRunGpuHours": 3.5, "revisedFirstRunCostUsd": 3.0, "revisedTournamentGpuHours": 10.5, "revisedTournamentCostUsd": 9.0, "maximumPlannedUpdate": 4096, "fitsFirstRunEnvelope": True, "fitsTournamentEnvelope": True},
            "decision": "continue",
        }
        throughput_path = write_payload("throughput.json", throughput)
        validate_throughput_evidence(throughput_path, config, config_path)
        optimistic = copy.deepcopy(throughput)
        optimistic["revisedForecast"]["revisedFirstRunGpuHours"] = 3.0
        optimistic_path = write_payload("throughput-optimistic.json", optimistic)
        expect_failure(lambda: validate_throughput_evidence(optimistic_path, config, config_path), "throughput validator accepted a forecast below measured marginal time")
        limited_throughput = copy.deepcopy(throughput)
        limited_throughput["revisedForecast"].update({"maximumPlannedUpdate": 1024, "revisedFirstRunGpuHours": 2.25, "revisedFirstRunCostUsd": 2.0, "revisedTournamentGpuHours": 6.75, "revisedTournamentCostUsd": 6.0})
        limited_throughput["revisedForecast"]["derivation"].update({"remainingUpdates": 896, "minimumRemainingMarginalGpuHours": 0.35, "minimumFirstRunGpuHours": 2.15, "minimumTournamentGpuHours": 6.45, "minimumFirstRunCostUsd": 1.6125, "minimumTournamentCostUsd": 4.8375})
        limited_throughput_path = write_payload("throughput-limited-to-1024.json", limited_throughput)
        assert validate_throughput_evidence(limited_throughput_path, config, config_path)["maximumPlannedUpdate"] == 1024

        runtime_build_path = root / "runtime-build.wasm"
        atomic_write_bytes(runtime_build_path, b"matching-width-512-runtime\n")
        runtime_build = {"path": str(runtime_build_path), "bytes": runtime_build_path.stat().st_size, "sha256": sha256(runtime_build_path)}
        runtimes: dict[int, Path] = {}

        def make_runtime(update: int, bound_run_path: Path = candidate_run_path, tag: str = "") -> Path:
            network_path = root / "experiment" / "exports" / "candidates" / f"candidate-u{update:08d}.nnue"
            atomic_write_bytes(network_path, f"integer-network-{update}\n".encode())
            network = {"path": str(network_path), "bytes": network_path.stat().st_size, "sha256": sha256(network_path)}
            candidate_export = {
                "schema": CANDIDATE_EXPORT_SCHEMA,
                "experimentId": config["experimentId"],
                "configSha256": config_digest,
                "createdAt": "2026-09-06T14:15:00+09:00",
                "run": {"path": str(bound_run_path), "sha256": sha256(bound_run_path)},
                "checkpoint": checkpoints[update],
                "format": config["recipe"]["export"]["format"],
                "sourceCheckpoint": {"path": checkpoints[update]["path"], "bytes": Path(checkpoints[update]["path"]).stat().st_size, "sha256": checkpoints[update]["sha256"]},
                "serializer": {"path": "serialize.py", "bytes": 1, "sha256": "0" * 64, "pinnedTrainerRevision": config["trainer"]["pinnedTrainerRevision"]},
                "integerNetwork": network,
                "status": "passed",
            }
            candidate_export_path = write_payload(f"candidate-export-{update}{tag}.json", candidate_export)
            parity = {"schema": PARITY_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_digest, "createdAt": "2026-09-06T14:20:00+09:00", "checkpoint": checkpoints[update], "integerNetwork": network, "runtimeBuild": runtime_build, "checks": {"fullPrecisionToIntegerMeasured": True, "integerReferenceMatchesNative": True, "integerReferenceMatchesWasm": True, "nativeMatchesWasm": True, "quantizationErrorReportedSeparately": True}, "status": "passed"}
            parity_path = write_payload(f"parity-{update}{tag}.json", parity)
            runtime = {"schema": CANDIDATE_RUNTIME_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_digest, "createdAt": "2026-09-06T14:25:00+09:00", "checkpoint": checkpoints[update], "candidateExport": {"path": str(candidate_export_path), "sha256": sha256(candidate_export_path)}, "runtimeBuild": runtime_build, "parityReceipt": {"path": str(parity_path), "sha256": sha256(parity_path)}, "width": config["recipe"]["width"], "status": "passed"}
            runtime_path = write_payload(f"runtime-{update}{tag}.json", runtime)
            runtimes[update] = runtime_path
            return runtime_path

        for update in (128, 256, 1024, 2048, 4096):
            make_runtime(update)

        def make_raw_match(update: int, kind: str, pair_score: float, name: str) -> Path:
            per_game_score = pair_score
            result = {0.0: "candidate-loss", 0.5: "draw", 1.0: "candidate-win"}[per_game_score]
            games: list[dict[str, Any]] = []
            schedule_value = match_schedule(config, kind, update)
            for pair_index in range(schedule_value["pairs"]):
                opening_id = config["strengthProtocol"]["openingIds"][pair_index % 64]
                opening_cycle = schedule_value["cycles"][pair_index // 64]
                for seat, color in enumerate(("black", "white")):
                    games.append({"schema": SCREEN_GAME_SCHEMA, "gameIndex": pair_index * 2 + seat, "pairIndex": pair_index, "openingId": opening_id, "openingCycle": opening_cycle, "candidateColor": color, "opponent": config["strengthProtocol"]["opponent"], "scoreCandidate": per_game_score, "result": result, "candidateSearchedMoves": 1, "candidateSearchElapsedMilliseconds": float(update) / 1000.0 + 1.0, "complete": True, "missingMoves": 0, "illegalMoves": 0, "emergencyFallbacks": 0, "unsearchedMoves": 0, "terminationReason": "normal-terminal"})
            group_scores = [pair_score] * 64
            statistics = grouped_screen_statistics(group_scores)
            summary = {"games": len(games), "pairs": schedule_value["pairs"], "openingGroups": 64, "groupScores": group_scores, **statistics, "intervalMethod": config["strengthProtocol"]["uncertainty"]["intervalMethod"], "correctnessPassed": True}
            runtime_path = runtimes[update]
            runtime_item = {"path": str(runtime_path), "sha256": sha256(runtime_path)}
            raw = {"schema": RAW_MATCH_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_digest, "createdAt": "2026-09-06T14:28:00+09:00", "kind": kind, "candidateRuntime": runtime_item, "schedule": schedule_value, "games": games, "summary": summary, "runtimeCost": {"metric": "mean-candidate-search-elapsed-milliseconds", "value": float(update) / 1000.0 + 1.0}, "status": "passed"}
            return write_payload(name, raw)

        def make_screen(update: int, pair_score: float, priors: list[Path], name: str) -> Path:
            raw_path = make_raw_match(update, "scheduled-screen", pair_score, f"raw-{name}")
            raw = load_json(raw_path)
            prior_records = [{"path": str(item), "sha256": sha256(item)} for item in priors]
            previous_upper = None
            if priors:
                previous_upper = load_json(priors[-1])["summary"]["upper95"]
            decision = "stop-two-successive-low-screens" if previous_upper is not None and previous_upper < 0.45 and raw["summary"]["upper95"] < 0.45 else "continue"
            runtime_path = runtimes[update]
            payload = {"schema": SCREEN_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_digest, "createdAt": "2026-09-06T14:30:00+09:00", "checkpoint": checkpoints[update], "candidateRuntime": {"path": str(runtime_path), "sha256": sha256(runtime_path)}, "rawMatch": {"path": str(raw_path), "sha256": sha256(raw_path)}, "summary": raw["summary"], "priorScreens": prior_records, "decision": decision}
            return write_payload(name, payload)

        screen_256 = make_screen(256, 0.5, [], "screen-256.json")
        screen_1024 = make_screen(1024, 0.0, [screen_256], "screen-1024.json")
        screen_2048_stop = make_screen(2048, 0.0, [screen_256, screen_1024], "screen-2048-stop.json")
        stopped = validate_screen_evidence(screen_2048_stop, config, config_path, 2048)
        assert stopped["decision"] == "stop-two-successive-low-screens"
        stop_receipt = {"schema": CONTINUATION_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_digest, "fromUpdate": 2048, "toUpdate": 4096, "decision": "continue", "createdAt": "2026-09-06T14:40:00+09:00", "evidence": [{"role": "throughput-probe-report", "path": str(throughput_path), "sha256": sha256(throughput_path)}, {"role": "strength-screen-u00002048", "path": str(screen_2048_stop), "sha256": sha256(screen_2048_stop)}]}
        stop_receipt_path = write_payload("continuation-after-stop.json", stop_receipt)
        expect_failure(lambda: validate_continuation_receipt(stop_receipt_path, config, config_path, 2048, 4096), "continuation accepted two successive low screens")
        screen_2048 = make_screen(2048, 0.5, [screen_256, screen_1024], "screen-2048.json")
        assert validate_screen_evidence(screen_2048, config, config_path, 2048)["decision"] == "continue"
        horizon_receipt = {"schema": CONTINUATION_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_digest, "fromUpdate": 1024, "toUpdate": 2048, "decision": "continue", "createdAt": "2026-09-06T14:40:00+09:00", "evidence": [{"role": "throughput-probe-report", "path": str(limited_throughput_path), "sha256": sha256(limited_throughput_path)}, {"role": "strength-screen-u00001024", "path": str(screen_1024), "sha256": sha256(screen_1024)}]}
        horizon_receipt_path = write_payload("continuation-beyond-throughput-horizon.json", horizon_receipt)
        expect_failure(lambda: validate_continuation_receipt(horizon_receipt_path, config, config_path, 1024, 2048), "continuation crossed the measured maximum planned update")
        corrupted_screen = load_json(screen_256)
        corrupted_raw_path = Path(corrupted_screen["rawMatch"]["path"])
        corrupted_raw = load_json(corrupted_raw_path)
        corrupted_raw["games"][1]["candidateColor"] = "black"
        corrupted_raw_copy = write_payload("raw-screen-corrupt.json", corrupted_raw)
        corrupted_screen["rawMatch"] = {"path": str(corrupted_raw_copy), "sha256": sha256(corrupted_raw_copy)}
        corrupted_path = write_payload("screen-corrupt.json", corrupted_screen)
        expect_failure(lambda: validate_screen_evidence(corrupted_path, config, config_path, 256), "screen validator accepted a missing color swap")

        receipt = {"schema": CONTINUATION_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_digest, "fromUpdate": 256, "toUpdate": 1024, "decision": "continue", "createdAt": "2026-09-06T14:40:00+09:00", "evidence": [{"role": "throughput-probe-report", "path": str(throughput_path), "sha256": sha256(throughput_path)}, {"role": "strength-screen-u00000256", "path": str(screen_256), "sha256": sha256(screen_256)}]}
        receipt_path = write_payload("continuation-256-1024.json", receipt)
        receipt_dir = root / "experiment" / "receipts"
        active, is_new = resolve_continuation_leg(config, config_path, 256, 512, None, receipt_path, receipt_dir)
        assert is_new and active["fromUpdate"] == 256 and active["toUpdate"] == 1024
        assert Path(active["path"]).parent == receipt_dir
        mutated_receipt = copy.deepcopy(receipt)
        mutated_receipt["decision"] = "tampered-after-snapshot"
        atomic_write_json(receipt_path, mutated_receipt)
        original_screen_256 = load_json(screen_256)
        atomic_write_json(screen_256, {"deleted": True})
        atomic_write_bytes(runtime_build_path, b"tampered-runtime\n")
        recovered, is_new = resolve_continuation_leg(config, config_path, 384, 768, active, None)
        assert not is_new and recovered["sha256"] == active["sha256"]
        atomic_write_json(screen_256, original_screen_256)
        atomic_write_bytes(runtime_build_path, b"matching-width-512-runtime\n")
        expect_failure(lambda: resolve_continuation_leg(config, config_path, 384, 1152, active, None), "intermediate resume crossed its active leg")

        approval = {"schema": APPROVAL_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_digest, "gpuRentalApproved": True, "action": config["authorization"]["approvedAction"], "approvedAt": quote["observedAt"], "validUntil": quote["validUntil"], "commercialEnvelope": {"provider": quote["provider"], "offerId": quote["offerId"], "gpu": quote["gpu"], "diskGigabytes": quote["diskGigabytes"], "quotedCombinedHourlyRateUsd": quote["combinedHourlyRateUsd"], "maximumCombinedHourlyRateUsd": config["costForecast"]["firstRun"]["maximumCombinedHourlyRateUsd"], "maximumRentedUptimeHours": config["budget"]["firstRentalEnvelopeGpuHours"], "maximumTotalCostUsd": config["budget"]["firstRentalEnvelopeUsd"]}}
        approval_path = write_payload("approval.json", approval)
        validate_approval(approval_path, config, config_path, now=timestamp(quote["observedAt"], "self-test now"))
        bad_approval = copy.deepcopy(approval)
        bad_approval["commercialEnvelope"]["maximumTotalCostUsd"] = 6.0
        bad_approval_path = write_payload("approval-bad.json", bad_approval)
        expect_failure(lambda: validate_approval(bad_approval_path, config, config_path, now=timestamp(quote["observedAt"], "self-test now")), "approval validator accepted a different budget")

        metric_rows = [{"schema": METRICS_SCHEMA, "optimizerUpdates": update, "validation": {"teacherOnlyLoss": loss}} for update, loss in ((128, 0.03), (256, 0.02), (1024, 0.01), (2048, 0.015), (4096, 0.02))]
        metrics_path = root / "experiment" / "metrics.jsonl"
        atomic_write_bytes(metrics_path, b"".join(canonical_json(row) for row in metric_rows))
        metrics_item = {"path": str(metrics_path), "sha256": sha256(metrics_path)}

        def make_comparison(update: int, score: float, name: str) -> Path:
            raw_path = make_raw_match(update, "final-equal-node", score, f"raw-comparison-{name}.json")
            runtime_path = runtimes[update]
            loss = next(row["validation"]["teacherOnlyLoss"] for row in metric_rows if row["optimizerUpdates"] == update)
            comparison = {"schema": COMPARISON_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_digest, "createdAt": "2026-09-06T15:20:00+09:00", "checkpoint": checkpoints[update], "candidateRuntime": {"path": str(runtime_path), "sha256": sha256(runtime_path)}, "equalNodeMatch": {"path": str(raw_path), "sha256": sha256(raw_path)}, "fixedTimeMatch": None, "validationMetric": {"metricsJournal": metrics_item, "update": update, "teacherOnlyLoss": loss}, "status": "passed"}
            return write_payload(f"comparison-{name}.json", comparison)

        comparison_128 = make_comparison(128, 0.5, "128")
        comparison_256 = make_comparison(256, 1.0, "256")
        comparison_1024 = make_comparison(1024, 0.5, "1024")
        bad_runtime = load_json(runtimes[256])
        bad_runtime["candidateExport"] = load_json(runtimes[1024])["candidateExport"]
        bad_runtime_path = write_payload("runtime-network-mismatch.json", bad_runtime)
        expect_failure(lambda: validate_candidate_runtime(bad_runtime_path, config, config_path, checkpoints[256]), "candidate runtime accepted a network not covered by its parity receipt")
        bad_comparison = load_json(comparison_256)
        bad_comparison["equalNodeScoreRate"] = 0.0
        bad_comparison_path = write_payload("comparison-literal-injection.json", bad_comparison)
        expect_failure(lambda: validate_comparison_receipt(bad_comparison_path, config, config_path, checkpoints[256]), "comparison receipt accepted an unaudited score literal")
        comparison_raw_path = Path(load_json(comparison_256)["equalNodeMatch"]["path"])
        bad_raw = load_json(comparison_raw_path)
        bad_raw["summary"]["meanScore"] = 0.0
        bad_raw_path = write_payload("raw-comparison-summary-tamper.json", bad_raw)
        bad_summary_comparison = load_json(comparison_256)
        bad_summary_comparison["equalNodeMatch"] = {"path": str(bad_raw_path), "sha256": sha256(bad_raw_path)}
        bad_summary_path = write_payload("comparison-summary-tamper.json", bad_summary_comparison)
        expect_failure(lambda: validate_comparison_receipt(bad_summary_path, config, config_path, checkpoints[256]), "comparison accepted a score summary inconsistent with raw games")
        run = {"schema": TRAINING_RUN_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_digest, "complete": True, "stopReason": "maximum-updates", "optimizerUpdates": 4096, "examplesSeen": 1, "equivalentPasses": 1.0, "dataCursor": {"epoch": 1, "record": 0}, "bestValidation": {"loss": 0.01, "update": 1024, "checkpoint": {"path": checkpoints[1024]["path"], "bytes": Path(checkpoints[1024]["path"]).stat().st_size, "sha256": checkpoints[1024]["sha256"]}}, "artifacts": [{"role": "scheduled-strength-screen", **checkpoints[update]} for update in (256, 1024, 2048, 4096)], "timingSeconds": {}, "approval": approval_record, "continuationReceipts": [], "activeLeg": None, "approvedMaximumPlannedUpdate": 4096, "nextContinuationRequiresNewReceipt": False, "selectedCheckpointExport": "required-explicitly-via-selection-history-bound-export-subcommand", "paidExecutionDoesNotAuthorizeFurtherRuns": True}
        run_path = root / "experiment" / "run.json"
        atomic_write_json(run_path, run)
        selection = {"schema": SELECTION_HISTORY_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_digest, "createdAt": "2026-09-06T15:30:00+09:00", "run": {"path": str(run_path), "sha256": sha256(run_path)}, "terminalEvidence": None, "candidates": [{"candidateRoles": ["lowest-validation-loss"], "checkpoint": checkpoints[1024], "comparisonReceipt": {"path": str(comparison_1024), "sha256": sha256(comparison_1024)}}, {"candidateRoles": ["highest-scheduled-screen-score"], "checkpoint": checkpoints[256], "comparisonReceipt": {"path": str(comparison_256), "sha256": sha256(comparison_256)}}], "scheduledScreens": [{"path": str(item), "sha256": sha256(item)} for item in (screen_256, screen_1024, screen_2048)], "rankingOrder": config["strengthProtocol"]["finalSelection"]["rankingOrder"], "selectedCheckpoint": checkpoints[256], "decision": "selected-by-frozen-ranking"}
        selection_path = write_payload("selection.json", selection)
        selected = validate_selection_history(selection_path, config, config_path)
        assert selected["selectedCheckpoint"]["sha256"] == checkpoints[256]["sha256"]
        bad_selection = copy.deepcopy(selection)
        bad_selection["selectedCheckpoint"] = checkpoints[1024]
        bad_selection_path = write_payload("selection-bad.json", bad_selection)
        expect_failure(lambda: validate_selection_history(bad_selection_path, config, config_path), "selection history accepted the losing candidate")

        low_256 = make_screen(256, 0.0, [], "screen-low-256.json")
        low_1024_stop = make_screen(1024, 0.0, [low_256], "screen-low-1024-stop.json")
        early_run = copy.deepcopy(run)
        early_run.update({"complete": False, "stopReason": "planned-gated-boundary", "optimizerUpdates": 1024, "nextContinuationRequiresNewReceipt": True})
        early_run["artifacts"] = [{"role": "scheduled-strength-screen", **checkpoints[update]} for update in (256, 1024)]
        atomic_write_json(run_path, early_run)
        make_runtime(256, run_path, "-early")
        make_runtime(1024, run_path, "-early")
        early_comparison_256 = make_comparison(256, 1.0, "early-256")
        early_comparison_1024 = make_comparison(1024, 0.5, "early-1024")
        early_selection = copy.deepcopy(selection)
        early_selection["run"] = {"path": str(run_path), "sha256": sha256(run_path)}
        early_selection["terminalEvidence"] = {"role": "strength-rule-stop-u00001024", "path": str(low_1024_stop), "sha256": sha256(low_1024_stop)}
        early_selection["candidates"][0]["comparisonReceipt"] = {"path": str(early_comparison_1024), "sha256": sha256(early_comparison_1024)}
        early_selection["candidates"][1]["comparisonReceipt"] = {"path": str(early_comparison_256), "sha256": sha256(early_comparison_256)}
        early_selection["scheduledScreens"] = [{"path": str(item), "sha256": sha256(item)} for item in (low_256, low_1024_stop)]
        early_selection_path = write_payload("selection-early-stop.json", early_selection)
        assert validate_selection_history(early_selection_path, config, config_path)["terminalEvidence"]["role"] == "strength-rule-stop-u00001024"

        budget_stop = copy.deepcopy(throughput)
        budget_stop["revisedForecast"].update({"revisedFirstRunGpuHours": 5.0, "revisedFirstRunCostUsd": 6.0, "revisedTournamentGpuHours": 15.0, "revisedTournamentCostUsd": 18.0, "maximumPlannedUpdate": 128, "fitsFirstRunEnvelope": False, "fitsTournamentEnvelope": True})
        budget_stop["revisedForecast"]["derivation"].update({"remainingUpdates": 0, "minimumRemainingMarginalGpuHours": 0.0, "minimumFirstRunGpuHours": 1.8, "minimumTournamentGpuHours": 5.4, "minimumFirstRunCostUsd": 1.35, "minimumTournamentCostUsd": 4.05})
        budget_stop["decision"] = "stop-budget"
        budget_stop_path = write_payload("throughput-budget-stop.json", budget_stop)
        budget_run = copy.deepcopy(early_run)
        budget_run.update({"optimizerUpdates": 128, "approvedMaximumPlannedUpdate": 128, "nextContinuationRequiresNewReceipt": True, "bestValidation": {"loss": 0.03, "update": 128, "checkpoint": {"path": checkpoints[128]["path"], "bytes": Path(checkpoints[128]["path"]).stat().st_size, "sha256": checkpoints[128]["sha256"]}}, "artifacts": [{"role": "best-validation", **checkpoints[128]}]})
        atomic_write_json(run_path, budget_run)
        make_runtime(128, run_path, "-budget")
        budget_comparison_128 = make_comparison(128, 0.5, "budget-128")
        budget_selection = {"schema": SELECTION_HISTORY_SCHEMA, "experimentId": config["experimentId"], "configSha256": config_digest, "createdAt": "2026-09-06T15:30:00+09:00", "run": {"path": str(run_path), "sha256": sha256(run_path)}, "terminalEvidence": {"role": "throughput-budget-stop", "path": str(budget_stop_path), "sha256": sha256(budget_stop_path)}, "candidates": [{"candidateRoles": ["lowest-validation-loss"], "checkpoint": checkpoints[128], "comparisonReceipt": {"path": str(budget_comparison_128), "sha256": sha256(budget_comparison_128)}}], "scheduledScreens": [], "rankingOrder": config["strengthProtocol"]["finalSelection"]["rankingOrder"], "selectedCheckpoint": checkpoints[128], "decision": "selected-by-frozen-ranking"}
        budget_selection_path = write_payload("selection-budget-stop.json", budget_selection)
        assert validate_selection_history(budget_selection_path, config, config_path)["terminalEvidence"]["role"] == "throughput-budget-stop"

    mutated = copy.deepcopy(config)
    mutated["recipe"]["optimizer"]["unused"] = True
    expect_failure(lambda: validate_config(mutated), "closed-world validation accepted an unused setting")
    for path_parts, replacement in ((["strengthProtocol", "pairOrder"], "different-order"), (["strengthProtocol", "maximumPly"], 201), (["stopping", "strengthRule"], "different-rule"), (["budget", "initialCycleStorageBytes"], 1)):
        mutated = copy.deepcopy(config)
        cursor_value = mutated
        for part in path_parts[:-1]:
            cursor_value = cursor_value[part]
        cursor_value[path_parts[-1]] = replacement
        expect_failure(lambda mutated=mutated: validate_config(mutated), f"closed-world validation accepted changed {'.'.join(path_parts)}")
    print("strength-first trainer self-test passed")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=ROOT / "r0/anhoku-strength-first/step3-width512-experiment-v1.json")
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("validate")
    sub.add_parser("self-test")
    real_fixture = sub.add_parser("real-fixture")
    real_fixture.add_argument("--output-dir", type=Path)
    real_fixture.add_argument("--cmake", default="cmake")
    train = sub.add_parser("train")
    train.add_argument("--approval-receipt", type=Path)
    train.add_argument("--resume", action="store_true")
    train.add_argument("--stop-after-updates", type=int, required=True)
    train.add_argument("--continuation-receipt", type=Path)
    train.add_argument("--cmake", default="cmake")
    candidate_export = sub.add_parser("candidate-export")
    candidate_export.add_argument("--checkpoint", type=Path, required=True)
    candidate_export.add_argument("--cmake", default="cmake")
    export = sub.add_parser("export")
    export.add_argument("--checkpoint", type=Path, required=True)
    export.add_argument("--selection-history", type=Path, required=True)
    export.add_argument("--output", type=Path, required=True)
    export.add_argument("--cmake", default="cmake")
    args = parser.parse_args()
    config_path = args.config.resolve()
    config = load_json(config_path)
    validate_config(config)
    if args.command == "validate":
        verify_inputs(config)
        print(f"validated {config_path}")
    elif args.command == "self-test":
        self_test(config, config_path)
    elif args.command == "real-fixture":
        run_real_cpu_fixture(args, config)
    elif args.command == "train":
        run_training(args, config, config_path)
    elif args.command == "candidate-export":
        run_candidate_export(args, config, config_path)
    elif args.command == "export":
        run_explicit_export(args, config, config_path)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except StrengthFirstError as exc:
        print(f"strength-first trainer FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
