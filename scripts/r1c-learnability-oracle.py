#!/usr/bin/env python3
"""CPU-only learnability and deployment-overfit oracle for Anhoku R1-C."""

from __future__ import annotations

import argparse
import collections
import hashlib
import importlib.util
import json
import math
import pathlib
import struct
import sys

import numpy as np
import torch


REAL_ROWS = 152_523
STACKS = 8
PSQT_SCORE_SCALE = 300.0
SCORE_SIGMOID_SCALE = 410.0
CHECKPOINT_HEADER_BYTES = 4096
CHECKPOINT_MAGIC = b"HTK-R1B-FP-V1\0\0\0"
MATERIAL_VALUES = np.asarray([700, 800, 400, 1000, 100, 300, 300, 500, 900], dtype=np.float64)


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_json(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def write_json(path: pathlib.Path, value: object) -> None:
    path.write_bytes(json.dumps(value, indent=2, sort_keys=True).encode() + b"\n")


def artifact(path: pathlib.Path) -> dict:
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256(path)}


def load_r1b_serializer(path: pathlib.Path):
    spec = importlib.util.spec_from_file_location("haitaka_r1b_oracle", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot import R1-B serializer from {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class BitReader:
    def __init__(self, payload: bytes):
        self.payload = payload
        self.cursor = 0

    def bit(self) -> int:
        if self.cursor >= len(self.payload) * 8:
            raise RuntimeError("packed-position decoder overflow")
        value = (self.payload[self.cursor // 8] >> (self.cursor % 8)) & 1
        self.cursor += 1
        return value

    def bits(self, count: int) -> int:
        return sum(self.bit() << shift for shift in range(count))


def decode_coalesced_material(packed_hex: str) -> tuple[int, np.ndarray]:
    reader = BitReader(bytes.fromhex(packed_hex))
    side_to_move = reader.bit()
    white_king = reader.bits(7)
    black_king = reader.bits(7)
    if white_king >= 81 or black_king >= 81 or white_king == black_king:
        raise RuntimeError("invalid packed kings")
    counts = np.zeros((2, 9), dtype=np.float64)
    for rank in reversed(range(9)):
        for file in range(9):
            square = rank * 9 + file
            if square in (white_king, black_king):
                continue
            if reader.bit():
                code = 1
                for shift in range(1, 5):
                    code |= reader.bit() << shift
                if code > 19 or code % 2 != 1:
                    raise RuntimeError("invalid packed piece code")
                color = reader.bit()
                slot = (code - 1) // 2
                if slot < 9:
                    counts[color, slot] += 1.0
    for color in range(2):
        for slot in range(10):
            count = reader.bits(5)
            if slot < 9:
                counts[color, slot] += count
    for _ in range(5):
        if reader.bit():
            raise RuntimeError("unexpected castling/en-passant bit")
    if reader.bits(6) != 0:
        raise RuntimeError("unexpected rule50 bits")
    reader.bits(17)
    signed_counts = counts[side_to_move] - counts[1 - side_to_move]
    score = int(round(float(signed_counts @ MATERIAL_VALUES)))
    return score, signed_counts


def load_fixtures(corpus_path: pathlib.Path, features_path: pathlib.Path, count: int):
    corpus = [json.loads(line) for line in corpus_path.read_text().splitlines()]
    features = [json.loads(line) for line in features_path.read_text().splitlines()]
    if len(corpus) != len(features) or len(corpus) < count:
        raise RuntimeError("R1-C requires the complete matching R1-A corpus and features")
    fixtures = []
    for index, (position, feature) in enumerate(zip(corpus[:count], features[:count])):
        expected_id = f"r1a-{index:05d}"
        if position["id"] != expected_id or feature["id"] != expected_id:
            raise RuntimeError(f"unstable R1-C fixture order at {index}")
        side_black = position["sfen"].split()[1] == "b"
        black = sorted(feature["black"]["base"] + feature["black"]["donor"])
        white = sorted(feature["white"]["base"] + feature["white"]["donor"])
        if any(row < 0 or row >= REAL_ROWS for row in black + white):
            raise RuntimeError(f"feature row outside deployment range for {expected_id}")
        material_score, material_counts = decode_coalesced_material(position["packedHex"])
        promoted_gold_like = sum(
            position["sfen"].split()[0].count(token)
            for token in ("+P", "+L", "+N", "+S", "+p", "+l", "+n", "+s")
        )
        fixtures.append(
            {
                "position": position,
                "black": black,
                "white": white,
                "side_black": side_black,
                "material_score": material_score,
                "material_counts": material_counts,
                "promoted_gold_like": promoted_gold_like,
            }
        )
    return fixtures


def train_linear_oracle(name: str, inputs: np.ndarray, targets: np.ndarray, settings: dict) -> dict:
    torch.manual_seed(int(settings["seed"]))
    # A deterministic orthonormal preconditioner preserves the complete column
    # space of each exactly representable target while preventing feature
    # frequency/collinearity from turning this optimizer plumbing test into a
    # condition-number test.  Unit-RMS columns also give all four frozen
    # oracles the same effective Adam step scale.
    u, singular_values, _ = np.linalg.svd(inputs, full_matrices=False)
    threshold = np.finfo(np.float64).eps * max(inputs.shape) * singular_values[0]
    rank = int(np.count_nonzero(singular_values > threshold))
    if rank == 0:
        raise RuntimeError(f"R1-C exact oracle {name} has rank-zero inputs")
    conditioned = u[:, :rank] * math.sqrt(inputs.shape[0])
    x = torch.tensor(conditioned, dtype=torch.float64)
    y = torch.tensor(targets, dtype=torch.float64)
    if y.ndim == 1:
        y = y[:, None]
    if y.shape[1] > 1:
        # The sentinel columns intentionally contain exact linear dependencies.
        # Train the identity in the rank-complete latent basis and apply a fixed
        # reconstruction head when measuring the frozen sixteen output targets.
        # This retains every representable direction without asking Adam to
        # invert an arbitrary correlated basis.
        training_target = x
        decoder = torch.linalg.lstsq(x, y).solution
        latent_scale = torch.nn.Parameter(torch.zeros(x.shape[1], dtype=torch.float64))
        parameters = [latent_scale]

        def predict_raw():
            return x * latent_scale
    else:
        training_target = y
        decoder = None
        model = torch.nn.Linear(x.shape[1], y.shape[1], bias=False, dtype=torch.float64)
        torch.nn.init.zeros_(model.weight)
        parameters = list(model.parameters())

        def predict_raw():
            return model(x)

    optimizer = torch.optim.Adam(parameters, lr=float(settings["learning_rate"]))
    with torch.no_grad():
        raw_prediction = predict_raw()
        prediction = raw_prediction @ decoder if decoder is not None else raw_prediction
        initial = float(torch.mean(torch.square(prediction - y)))
    for _ in range(int(settings["steps"])):
        optimizer.zero_grad(set_to_none=True)
        loss = torch.mean(torch.square(predict_raw() - training_target))
        loss.backward()
        optimizer.step()
    with torch.no_grad():
        raw_prediction = predict_raw()
        prediction = raw_prediction @ decoder if decoder is not None else raw_prediction
        final = float(torch.mean(torch.square(prediction - y)))
        maximum_error = float(torch.max(torch.abs(prediction - y)))
    ratio = final / initial if initial > 0 else math.inf
    return {
        "name": name,
        "examples": int(x.shape[0]),
        "rawInputs": int(inputs.shape[1]),
        "effectiveRank": rank,
        "inputs": int(x.shape[1]),
        "outputs": int(y.shape[1]),
        "initialLoss": initial,
        "finalLoss": final,
        "finalToInitialLossRatio": ratio,
        "maximumAbsoluteError": maximum_error,
    }


def exact_oracles(fixtures: list[dict], contract: dict) -> dict:
    settings = contract["exact_oracles"]["optimizer"]
    side = np.asarray([1.0 if fixture["side_black"] else -1.0 for fixture in fixtures])
    material = np.stack([fixture["material_counts"] for fixture in fixtures])
    sentinel_rows = contract["exact_oracles"]["sentinel_feature_rows"]
    sentinel = np.zeros((len(fixtures), len(sentinel_rows)), dtype=np.float64)
    for index, fixture in enumerate(fixtures):
        our = fixture["black"] if fixture["side_black"] else fixture["white"]
        their = fixture["white"] if fixture["side_black"] else fixture["black"]
        our_counts = collections.Counter(our)
        their_counts = collections.Counter(their)
        for column, row in enumerate(sentinel_rows):
            sentinel[index, column] = our_counts[row] - their_counts[row]
    if any(np.count_nonzero(sentinel[:, column]) == 0 for column in range(sentinel.shape[1])):
        raise RuntimeError("a frozen R1-C sentinel row is not exercised")
    results = {
        "constant": train_linear_oracle(
            "constant", np.ones((len(fixtures), 1)), np.full(len(fixtures), 0.25), settings
        ),
        "sideSign": train_linear_oracle(
            "side-sign", side[:, None], side * 0.5, settings
        ),
        "coalescedMaterial": train_linear_oracle(
            "coalesced-material", material / 20.0, np.asarray([x["material_score"] for x in fixtures]) / 1000.0, settings
        ),
        "sentinelFeatureRows": train_linear_oracle(
            "sentinel-feature-rows", sentinel, sentinel, settings
        ),
    }
    limit = float(contract["exact_oracles"]["maximum_final_to_initial_loss_ratio"])
    for result in results.values():
        result["passed"] = result["finalToInitialLossRatio"] <= limit
    return results


def deployment_matrix(fixtures: list[dict]):
    observations = []
    keys = set()
    for fixture in fixtures:
        position = fixture["position"]
        bucket = int(position["outputBucket"])
        our = fixture["black"] if fixture["side_black"] else fixture["white"]
        their = fixture["white"] if fixture["side_black"] else fixture["black"]
        signed = collections.Counter(our)
        signed.subtract(their)
        row = {(feature, bucket): count for feature, count in signed.items() if count}
        keys.update(row)
        observations.append(row)
    columns = sorted(keys)
    column_index = {key: index for index, key in enumerate(columns)}
    row_indices = []
    column_indices = []
    values = []
    for row_index, observation in enumerate(observations):
        for key, value in sorted(observation.items()):
            row_indices.append(row_index)
            column_indices.append(column_index[key])
            values.append(float(value))
    indices = torch.tensor([row_indices, column_indices], dtype=torch.int64)
    values_tensor = torch.tensor(values, dtype=torch.float64)
    matrix = torch.sparse_coo_tensor(
        indices, values_tensor, (len(fixtures), len(columns)), dtype=torch.float64
    ).coalesce()
    return matrix, columns


def sparse_mv(matrix: torch.Tensor, vector: torch.Tensor) -> torch.Tensor:
    return torch.sparse.mm(matrix, vector[:, None])[:, 0]


def train_cgls(matrix: torch.Tensor, target: torch.Tensor, maximum_iterations: int, tolerance: float):
    transpose = matrix.transpose(0, 1).coalesce()
    coefficients = torch.zeros(matrix.shape[1], dtype=torch.float64)
    residual = target.clone()
    normal = sparse_mv(transpose, residual)
    direction = normal.clone()
    gamma = torch.dot(normal, normal)
    initial_gamma = float(gamma)
    iterations = 0
    for iteration in range(maximum_iterations):
        projected = sparse_mv(matrix, direction)
        denominator = torch.dot(projected, projected)
        if float(denominator) == 0.0:
            break
        alpha = gamma / denominator
        coefficients += alpha * direction
        residual -= alpha * projected
        normal = sparse_mv(transpose, residual)
        next_gamma = torch.dot(normal, normal)
        iterations = iteration + 1
        if math.sqrt(float(next_gamma) / initial_gamma) <= tolerance:
            gamma = next_gamma
            break
        beta = next_gamma / gamma
        direction = normal + beta * direction
        gamma = next_gamma
    prediction = sparse_mv(matrix, coefficients)
    return coefficients, prediction, {
        "iterations": iterations,
        "initialNormalResidualSquared": initial_gamma,
        "finalNormalResidualSquared": float(gamma),
        "relativeNormalResidual": math.sqrt(float(gamma) / initial_gamma),
    }


def sigmoid(values: np.ndarray) -> np.ndarray:
    positive = values >= 0
    result = np.empty_like(values, dtype=np.float64)
    result[positive] = 1.0 / (1.0 + np.exp(-values[positive]))
    exponent = np.exp(values[~positive])
    result[~positive] = exponent / (1.0 + exponent)
    return result


def probability_loss(predictions: np.ndarray, targets: np.ndarray) -> float:
    delta = sigmoid(predictions / SCORE_SIGMOID_SCALE) - sigmoid(targets / SCORE_SIGMOID_SCALE)
    return float(np.mean(np.square(delta)))


def percentile(sorted_values: np.ndarray, fraction: float) -> float:
    rank = max(1, math.ceil(fraction * len(sorted_values)))
    return float(sorted_values[min(rank - 1, len(sorted_values) - 1)])


def quantization_groups(fixtures: list[dict], full: np.ndarray, quantized: np.ndarray, limits: dict):
    score_buckets = limits["score_buckets"]
    groups: dict[str, list[int]] = collections.defaultdict(list)
    for index, fixture in enumerate(fixtures):
        target = int(fixture["position"]["labelScoreSideToMove"])
        split = "even-fixture-id" if index % 2 == 0 else "odd-fixture-id"
        bucket = next(
            item["id"]
            for item in score_buckets
            if (item["minimum_inclusive"] is None or target >= item["minimum_inclusive"])
            and (item["maximum_inclusive"] is None or target <= item["maximum_inclusive"])
        )
        for name in ("all", f"split/{split}", f"score-bucket/{bucket}", f"split-score/{split}/{bucket}"):
            groups[name].append(index)
    absolute = limits["limits"]
    output = {}
    for name, indices in sorted(groups.items()):
        selected = np.asarray(indices, dtype=np.int64)
        deltas = np.sort(np.abs(full[selected] - quantized[selected]))
        targets = np.asarray(
            [fixtures[index]["position"]["labelScoreSideToMove"] for index in indices], dtype=np.float64
        )
        full_loss = probability_loss(full[selected], targets)
        quantized_loss = probability_loss(quantized[selected], targets)
        metrics = {
            "count": len(indices),
            "meanAbsoluteScoreDelta": float(np.mean(deltas)),
            "p95AbsoluteScoreDelta": percentile(deltas, 0.95),
            "p99AbsoluteScoreDelta": percentile(deltas, 0.99),
            "maximumAbsoluteScoreDelta": float(deltas[-1]),
            "fullPrecisionLoss": full_loss,
            "quantizedLoss": quantized_loss,
            "lossDegradation": quantized_loss - full_loss,
        }
        metrics["passed"] = (
            metrics["meanAbsoluteScoreDelta"] <= absolute["mean_absolute_score_delta"]
            and metrics["p99AbsoluteScoreDelta"] <= absolute["p99_absolute_score_delta"]
            and metrics["maximumAbsoluteScoreDelta"] <= absolute["maximum_absolute_score_delta"]
            and metrics["lossDegradation"] <= absolute["maximum_positive_loss_degradation"]
        )
        output[name] = metrics
    return output


def create_checkpoint(path: pathlib.Path, r1b, columns: list[tuple[int, int]], coefficients: np.ndarray):
    metadata = r1b.checkpoint_metadata([], 0, 0, 0, 0)
    metadata["construction"] = "haitaka-r1c-psqt-overfit-v1"
    header = canonical_json(metadata)
    if len(header) + len(CHECKPOINT_MAGIC) + 4 > CHECKPOINT_HEADER_BYTES:
        raise RuntimeError("R1-C checkpoint metadata exceeds fixed header")
    with path.open("wb") as output:
        output.truncate(metadata["bytes"])
        output.seek(0)
        output.write(CHECKPOINT_MAGIC)
        output.write(struct.pack("<I", len(header)))
        output.write(header)
    weights = r1b.open_checkpoint_array(path, metadata, "input_weight", "r+")
    for (row, bucket), coefficient in zip(columns, coefficients):
        weights[row, r1b.L1 + bucket] = np.float32(coefficient / PSQT_SCORE_SCALE)
    weights.flush()
    del weights
    return metadata


def residual_metrics(fixtures: list[dict], prediction: np.ndarray, target: np.ndarray):
    strata = {
        "all": np.arange(len(fixtures)),
        "identity-collision-exposed": np.asarray(
            [index for index, fixture in enumerate(fixtures) if fixture["promoted_gold_like"] > 0]
        ),
        "identity-collision-unexposed": np.asarray(
            [index for index, fixture in enumerate(fixtures) if fixture["promoted_gold_like"] == 0]
        ),
    }
    output = {}
    for name, indices in strata.items():
        if len(indices) == 0:
            output[name] = {"count": 0}
            continue
        residual = prediction[indices] - target[indices]
        output[name] = {
            "count": int(len(indices)),
            "meanResidual": float(np.mean(residual)),
            "meanAbsoluteError": float(np.mean(np.abs(residual))),
            "rootMeanSquaredError": float(np.sqrt(np.mean(np.square(residual)))),
            "maximumAbsoluteError": float(np.max(np.abs(residual))),
            "probabilityLoss": probability_loss(prediction[indices], target[indices]),
        }
    target_std = float(np.std(target))
    rmse = output["all"]["rootMeanSquaredError"]
    if target_std > 0:
        slope, intercept = np.polyfit(target, prediction, 1)
        correlation = float(np.corrcoef(target, prediction)[0, 1])
    else:
        slope, intercept, correlation = math.nan, math.nan, math.nan
    return {
        "strata": output,
        "targetStandardDeviation": target_std,
        "relativeCloneError": rmse / target_std if target_std else math.inf,
        "calibrationPredictionOnTarget": {"slope": float(slope), "intercept": float(intercept)},
        "pearsonCorrelation": correlation,
    }


def feature_collision_metrics(fixtures: list[dict], targets: np.ndarray):
    groups: dict[tuple, list[int]] = collections.defaultdict(list)
    for index, fixture in enumerate(fixtures):
        key = (
            fixture["side_black"],
            int(fixture["position"]["outputBucket"]),
            tuple(fixture["black"]),
            tuple(fixture["white"]),
        )
        groups[key].append(index)
    collisions = [indices for indices in groups.values() if len(indices) > 1]
    irreducible_sse = 0.0
    records = 0
    for indices in collisions:
        values = targets[np.asarray(indices)]
        irreducible_sse += float(np.sum(np.square(values - np.mean(values))))
        records += len(indices)
    return {
        "exactFeatureCollisionGroups": len(collisions),
        "recordsInExactFeatureCollisionGroups": records,
        "empiricalIrreducibleScoreMse": irreducible_sse / records if records else 0.0,
        "positionsWithGoldLikePromotedMinor": sum(fixture["promoted_gold_like"] > 0 for fixture in fixtures),
        "goldLikePromotedMinorInstances": sum(fixture["promoted_gold_like"] for fixture in fixtures),
        "positionFrequency": sum(fixture["promoted_gold_like"] > 0 for fixture in fixtures) / len(fixtures),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--workspace-root", required=True, type=pathlib.Path)
    parser.add_argument("--contract", required=True, type=pathlib.Path)
    parser.add_argument("--r1a-dir", required=True, type=pathlib.Path)
    parser.add_argument("--limits", required=True, type=pathlib.Path)
    parser.add_argument("--output-dir", required=True, type=pathlib.Path)
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    torch.set_num_threads(1)
    torch.use_deterministic_algorithms(True)

    contract = json.loads(args.contract.read_text())
    if contract.get("schema") != "haitaka-r1c-learnability-contract-v1":
        raise RuntimeError("wrong R1-C contract schema")
    if contract.get("device") != "cpu" or contract.get("gpu_allowed") is not False:
        raise RuntimeError("R1-C contract must remain CPU-only")
    for identity in contract["input_artifacts"].values():
        path = args.workspace_root / identity["path"]
        if sha256(path) != identity["sha256"]:
            raise RuntimeError(f"frozen input identity mismatch: {path}")

    corpus_path = args.r1a_dir / "parity-corpus.jsonl"
    features_path = args.r1a_dir / "rust-features.jsonl"
    count = int(contract["overfit_corpus"]["positions"])
    fixtures = load_fixtures(corpus_path, features_path, count)
    exact = exact_oracles(fixtures, contract)

    matrix, columns = deployment_matrix(fixtures)
    targets = np.asarray(
        [fixture["position"]["labelScoreSideToMove"] for fixture in fixtures], dtype=np.float64
    )
    coefficients, prediction_tensor, optimizer = train_cgls(
        matrix,
        torch.tensor(targets, dtype=torch.float64),
        int(contract["deployment_overfit"]["maximum_iterations"]),
        float(contract["deployment_overfit"]["relative_normal_residual_tolerance"]),
    )
    full = prediction_tensor.numpy()

    serializer_path = args.workspace_root / contract["input_artifacts"]["r1b_independent_serializer"]["path"]
    r1b = load_r1b_serializer(serializer_path)
    checkpoint = args.output_dir / "overfit-checkpoint.r1fp"
    metadata = create_checkpoint(checkpoint, r1b, columns, coefficients.numpy())
    network = args.output_dir / "overfit.nnue"
    network_repeat = args.output_dir / "overfit-repeat.nnue"
    clamped = r1b.serialize_network(checkpoint, network, "sentinel")
    clamped_repeat = r1b.serialize_network(checkpoint, network_repeat, "sentinel")
    repeat_identical = sha256(network) == sha256(network_repeat)
    raw, integer_network = r1b.parse_network(network)
    quantized = []
    accumulator_overflows = 0
    clamp_counts = collections.Counter()
    expectations_path = args.output_dir / "runtime-expectations.jsonl"
    with expectations_path.open("w") as output:
        for fixture, full_score in zip(fixtures, full):
            trace = r1b.integer_trace(
                integer_network, fixture["position"], fixture["black"], fixture["white"]
            )
            quantized.append(trace["score"])
            accumulator_overflows += trace["accumulator_overflows"]
            clamp_counts.update(trace["clamp_counts"])
            output.write(
                json.dumps(
                    {
                        "id": fixture["position"]["id"],
                        "fullPrecisionScore": float(full_score),
                        "integerScore": int(trace["score"]),
                    },
                    sort_keys=True,
                    separators=(",", ":"),
                )
                + "\n"
            )
    del raw
    quantized_array = np.asarray(quantized, dtype=np.float64)
    limits = json.loads(args.limits.read_text())
    groups = quantization_groups(fixtures, full, quantized_array, limits)
    initial_loss = probability_loss(np.zeros_like(targets), targets)
    full_loss = probability_loss(full, targets)
    quantized_loss = probability_loss(quantized_array, targets)
    full_reduction = initial_loss - full_loss
    retention = (initial_loss - quantized_loss) / full_reduction if full_reduction > 0 else -math.inf
    collision = feature_collision_metrics(fixtures, targets)
    residual = residual_metrics(fixtures, full, targets)

    training_metadata_path = args.output_dir / "overfit-training-metadata.json"
    training_metadata = {
        "schema": "haitaka-r1c-overfit-training-v1",
        "device": "cpu",
        "framework": "PyTorch",
        "frameworkVersion": torch.__version__,
        "dtype": "float64",
        "positions": len(fixtures),
        "activeBucketedFeatureCoefficients": len(columns),
        "sparseDesignNonzeros": int(matrix._nnz()),
        "optimizer": optimizer,
        "checkpointFormat": metadata["schema"],
        "checkpointConstruction": metadata["construction"],
        "contractSha256": sha256(args.contract),
    }
    write_json(training_metadata_path, training_metadata)

    maximum_ratio = float(contract["deployment_overfit"]["maximum_full_precision_final_to_initial_loss_ratio"])
    minimum_retention = float(contract["deployment_overfit"]["minimum_serialized_loss_reduction_retention"])
    results = {
        "schema": "haitaka-r1c-python-oracle-results-v1",
        "positions": len(fixtures),
        "exactOracles": exact,
        "deploymentOverfit": {
            "initialLoss": initial_loss,
            "fullPrecisionLoss": full_loss,
            "serializedLoss": quantized_loss,
            "fullPrecisionFinalToInitialLossRatio": full_loss / initial_loss,
            "serializedLossReductionRetention": retention,
            "optimizer": optimizer,
        },
        "quantizationByGroup": groups,
        "serializerClampedWeights": clamped + clamped_repeat,
        "accumulatorOverflows": accumulator_overflows,
        "activationClampCounts": dict(sorted(clamp_counts.items())),
        "repeatExportByteIdentical": repeat_identical,
        "collisionDiagnostic": collision,
        "cloneDiagnostic": residual,
        "artifacts": {
            "checkpoint": artifact(checkpoint),
            "network": artifact(network),
            "networkRepeat": artifact(network_repeat),
            "runtimeExpectations": artifact(expectations_path),
            "trainingMetadata": artifact(training_metadata_path),
        },
    }
    results["gates"] = {
        "exactOraclesLearn": all(result["passed"] for result in exact.values()),
        "fullPrecisionOverfit": full_loss / initial_loss <= maximum_ratio,
        "serializedRetainsReduction": retention >= minimum_retention,
        "absoluteQuantizationLimits": all(group["passed"] for group in groups.values()),
        "repeatExportByteIdentical": repeat_identical,
        "noSerializerWeightClamping": clamped + clamped_repeat == 0,
        "noAccumulatorOverflow": accumulator_overflows == 0,
        "collisionDiagnosticComplete": collision["positionsWithGoldLikePromotedMinor"] > 0,
    }
    results["passed"] = all(results["gates"].values())
    results_path = args.output_dir / "python-oracle-results.json"
    write_json(results_path, results)
    if not results["passed"]:
        raise RuntimeError(f"R1-C Python oracle failed; see {results_path}")
    print(json.dumps({"results": str(results_path), "sha256": sha256(results_path)}))


if __name__ == "__main__":
    main()
