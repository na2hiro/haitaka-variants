#!/usr/bin/env python3
"""Generate and independently emulate the deterministic Anhoku R1-B sentinel.

The checkpoint format is deliberately small in concept but complete in bytes: it
contains every full-precision deployment parameter in a fixed little-endian
layout.  PyTorch reads those tensors for the full-precision prediction layer;
the exporter and exact-integer emulator are implemented here without importing
the Rust runtime or the trainer serializer.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import pathlib
import struct
from dataclasses import dataclass

import numpy as np
import torch


VERSION = 0x7AF32F20
HALFKAV2_HASH = 0x5F234CB8
DONOR_SINGLE_ANHOKU_HASH = 0x23627E42 ^ 0x3C6EF362
L1 = 512
PSQT_BUCKETS = 8
STACKS = 8
L2 = 16
L3 = 32
BASE_ROWS = 150_903
DONOR_ROWS = 1_620
REAL_ROWS = BASE_ROWS + DONOR_ROWS
CHECKPOINT_HEADER_BYTES = 4096
CHECKPOINT_MAGIC = b"HTK-R1B-FP-V1\0\0\0"
EXPECT_MAGIC = b"HTK-R1B-EXP-V1\0\0"
CONSTRUCTION = "haitaka-r1b-donor-single-full-graph-sentinel-v1"


def composite_hash(head: int, tail: int) -> int:
    return (head ^ ((tail << 1) & 0xFFFFFFFF) ^ (tail >> 1)) & 0xFFFFFFFF


def affine_hash(previous: int, outputs: int) -> int:
    value = (0xCC03DAE4 + outputs) & 0xFFFFFFFF
    value ^= previous >> 1
    value ^= (previous << 31) & 0xFFFFFFFF
    return value & 0xFFFFFFFF


FEATURE_HASH = composite_hash(HALFKAV2_HASH, DONOR_SINGLE_ANHOKU_HASH)
INPUT_HASH = 0xEC42E90D ^ (L1 * 2)
H1_AFFINE_HASH = affine_hash(INPUT_HASH, L2)
H1_HASH = (0x538D24C7 + H1_AFFINE_HASH) & 0xFFFFFFFF
H2_AFFINE_HASH = affine_hash(H1_HASH, L3)
H2_HASH = (0x538D24C7 + H2_AFFINE_HASH) & 0xFFFFFFFF
OUTPUT_HASH = affine_hash(H2_HASH, 1)
NETWORK_HASH = (FEATURE_HASH ^ (L1 * 2) ^ OUTPUT_HASH) & 0xFFFFFFFF
TRANSFORMER_HASH = FEATURE_HASH ^ (L1 * 2)


ARRAY_SPECS = [
    ("input_bias", (L1 + PSQT_BUCKETS,)),
    ("input_weight", (REAL_ROWS, L1 + PSQT_BUCKETS)),
    ("l1_weight", (STACKS, L2, L1 * 2)),
    ("l1_bias", (STACKS, L2)),
    ("l2_weight", (STACKS, L3, L2)),
    ("l2_bias", (STACKS, L3)),
    ("output_weight", (STACKS, 1, L3)),
    ("output_bias", (STACKS, 1)),
]


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical_json(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def mix64(value: int) -> int:
    value = (value + 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF
    value = ((value ^ (value >> 30)) * 0xBF58476D1CE4E5B9) & 0xFFFFFFFFFFFFFFFF
    value = ((value ^ (value >> 27)) * 0x94D049BB133111EB) & 0xFFFFFFFFFFFFFFFF
    return value ^ (value >> 31)


def load_fixtures(corpus_path: pathlib.Path, features_path: pathlib.Path):
    corpus = [json.loads(line) for line in corpus_path.read_text().splitlines()]
    features = [json.loads(line) for line in features_path.read_text().splitlines()]
    if len(corpus) != len(features) or len(corpus) < 10_000:
        raise RuntimeError("R1-B requires the complete R1-A parity corpus")
    rows = []
    for expected_index, (position, feature) in enumerate(zip(corpus, features)):
        expected_id = f"r1a-{expected_index:05d}"
        if position["id"] != expected_id or feature["id"] != expected_id:
            raise RuntimeError(f"unstable fixture order at {expected_index}")
        black = sorted(feature["black"]["base"] + feature["black"]["donor"])
        white = sorted(feature["white"]["base"] + feature["white"]["donor"])
        if any(value < 0 or value >= REAL_ROWS for value in black + white):
            raise RuntimeError(f"feature row outside DonorSingleEff deployment range: {expected_id}")
        rows.append((position, black, white))
    return rows


def checkpoint_layout():
    cursor = CHECKPOINT_HEADER_BYTES
    arrays = {}
    for name, shape in ARRAY_SPECS:
        count = math.prod(shape)
        arrays[name] = {"dtype": "<f4", "shape": list(shape), "offset": cursor}
        cursor += count * 4
    return arrays, cursor


def checkpoint_metadata(active_rows, positive_row, negative_row, maximum_row, minimum_row):
    arrays, total_bytes = checkpoint_layout()
    return {
        "schema": "haitaka-r1b-full-precision-checkpoint-v1",
        "construction": CONSTRUCTION,
        "featureFamily": "HalfKAv2^+DonorSingleEff",
        "realFeatureRows": REAL_ROWS,
        "framework": "PyTorch",
        "frameworkVersion": torch.__version__,
        "byteOrder": "little",
        "arrays": arrays,
        "bytes": total_bytes,
        "activeRowsInitialized": len(active_rows),
        "sentinelRows": {
            "oneHotPositive": positive_row,
            "oneHotNegative": negative_row,
            "maximumI16": maximum_row,
            "minimumI16": minimum_row,
        },
        "rounding": "IEEE-754 ties-to-even via numpy.rint",
    }


def open_checkpoint_array(path: pathlib.Path, metadata, name: str, mode="r"):
    spec = metadata["arrays"][name]
    return np.memmap(
        path,
        mode=mode,
        dtype=spec["dtype"],
        offset=spec["offset"],
        shape=tuple(spec["shape"]),
        order="C",
    )


def choose_sentinel_rows(fixtures):
    occurrences = {}
    for _, black, white in fixtures:
        for row in set(black + white):
            occurrences[row] = occurrences.get(row, 0) + 1
    unique = sorted(row for row, count in occurrences.items() if count == 1)
    all_rows = sorted(occurrences)
    selected = unique if len(unique) >= 4 else all_rows
    if len(selected) < 4:
        raise RuntimeError("parity corpus has too few active feature rows")
    return selected[0], selected[1], selected[2], selected[3]


def generate_checkpoint(path: pathlib.Path, fixtures):
    active_rows = sorted({row for _, black, white in fixtures for row in black + white})
    positive_row, negative_row, maximum_row, minimum_row = choose_sentinel_rows(fixtures)
    metadata = checkpoint_metadata(
        active_rows, positive_row, negative_row, maximum_row, minimum_row
    )
    header = canonical_json(metadata)
    if len(header) + len(CHECKPOINT_MAGIC) + 4 > CHECKPOINT_HEADER_BYTES:
        raise RuntimeError("checkpoint metadata exceeds fixed header")
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("wb") as output:
        output.truncate(metadata["bytes"])
        output.seek(0)
        output.write(CHECKPOINT_MAGIC)
        output.write(struct.pack("<I", len(header)))
        output.write(header)

    bias = open_checkpoint_array(path, metadata, "input_bias", "r+")
    bias[:] = 0.0
    boundary_quantized = [-1, 0, 63, 64, 127, 128, 8127, 8128, 8129]
    for dimension, value in enumerate(boundary_quantized):
        bias[dimension] = np.float32(value / 127.0)
    # Explicit ties-to-even cases: 0.5->0, 1.5->2, -0.5->0, -1.5->-2.
    for dimension, value in enumerate([0.5, 1.5, -0.5, -1.5], start=11):
        bias[dimension] = np.float32(value / 127.0)
    bias.flush()

    weights = open_checkpoint_array(path, metadata, "input_weight", "r+")
    # The truncated file is already all-zero. Initialize only rows exercised by
    # the fixed corpus, making every nonzero contribution independently signed.
    for row in active_rows:
        for dimension in range(16, 32):
            signature = mix64(0x4654574549474854 ^ row ^ (dimension << 40))
            quantized = int(signature % 17) - 8
            residual = (0.20 if signature & 1 else -0.20)
            weights[row, dimension] = np.float32((quantized + residual) / 127.0)
        for bucket in range(PSQT_BUCKETS):
            signature = mix64(0x5053515457454947 ^ row ^ (bucket << 44))
            quantized = int(signature % 401) - 200
            residual = (0.20 if signature & 1 else -0.20)
            weights[row, L1 + bucket] = np.float32((quantized + residual) / 9600.0)
    weights[positive_row, 15] = np.float32(63.2 / 127.0)
    weights[negative_row, 15] = np.float32(-63.2 / 127.0)
    weights[maximum_row, 9] = np.float32(32767.0 / 127.0)
    weights[minimum_row, 10] = np.float32(-32768.0 / 127.0)
    weights.flush()
    del weights

    l1_weight = open_checkpoint_array(path, metadata, "l1_weight", "r+")
    l1_bias = open_checkpoint_array(path, metadata, "l1_bias", "r+")
    l2_weight = open_checkpoint_array(path, metadata, "l2_weight", "r+")
    l2_bias = open_checkpoint_array(path, metadata, "l2_bias", "r+")
    output_weight = open_checkpoint_array(path, metadata, "output_weight", "r+")
    output_bias = open_checkpoint_array(path, metadata, "output_bias", "r+")
    for bucket in range(STACKS):
        for output in range(L2):
            bias_q = [-1, 0, 63, 64, 127, 128, 8127, 8128][output % 8]
            l1_bias[bucket, output] = np.float32((bias_q + 0.20) / 8128.0)
            for source in list(range(32)) + list(range(L1, L1 + 32)):
                signature = mix64(0x4C31574549474854 ^ bucket ^ (output << 16) ^ (source << 32))
                quantized = int(signature % 9) - 4
                residual = 0.20 if signature & 1 else -0.20
                l1_weight[bucket, output, source] = np.float32((quantized + residual) / 64.0)
        for output in range(L3):
            bias_q = [-1, 0, 63, 64, 127, 128, 8127, 8128][output % 8]
            l2_bias[bucket, output] = np.float32((bias_q + 0.20) / 8128.0)
            # The first eight outputs are exact clamp-boundary sentinels; the
            # remaining outputs retain asymmetric nonzero layer weights.
            if output >= 8:
                for source in range(L2):
                    signature = mix64(
                        0x4C32574549474854
                        ^ bucket
                        ^ (output << 16)
                        ^ (source << 32)
                    )
                    quantized = int(signature % 9) - 4
                    residual = 0.20 if signature & 1 else -0.20
                    l2_weight[bucket, output, source] = np.float32(
                        (quantized + residual) / 64.0
                    )
        output_bias[bucket, 0] = np.float32(((bucket - 3) * 19 + 0.20) / 9600.0)
        output_scale = 9600.0 / 127.0
        # Clamp-boundary neurons 0..7 are audited in the activation trace but
        # intentionally do not feed the score; this keeps the correctness
        # sentinel from manufacturing avoidable quantization degradation.
        for source in range(8, L3):
            signature = mix64(0x4F55545745494748 ^ bucket ^ (source << 32))
            quantized = int(signature % 17) - 8
            residual = 0.20 if signature & 1 else -0.20
            output_weight[bucket, 0, source] = np.float32(
                (quantized + residual) / output_scale
            )
    for array in [l1_weight, l1_bias, l2_weight, l2_bias, output_weight, output_bias]:
        array.flush()
    return metadata


def read_checkpoint_metadata(path: pathlib.Path):
    with path.open("rb") as source:
        if source.read(len(CHECKPOINT_MAGIC)) != CHECKPOINT_MAGIC:
            raise RuntimeError("wrong R1-B checkpoint magic")
        size = struct.unpack("<I", source.read(4))[0]
        metadata = json.loads(source.read(size))
    if metadata.get("schema") != "haitaka-r1b-full-precision-checkpoint-v1":
        raise RuntimeError("wrong R1-B checkpoint schema")
    if path.stat().st_size != metadata["bytes"]:
        raise RuntimeError("R1-B checkpoint length mismatch")
    return metadata


def quantize(values, scale, dtype, minimum=None, maximum=None):
    scaled = np.asarray(values, dtype=np.float32)
    clamped = 0
    if minimum is not None:
        clamped += int(np.count_nonzero(scaled < minimum))
        scaled = np.maximum(scaled, np.float32(minimum))
    if maximum is not None:
        clamped += int(np.count_nonzero(scaled > maximum))
        scaled = np.minimum(scaled, np.float32(maximum))
    rounded = np.rint(scaled * np.float32(scale))
    info = np.iinfo(dtype)
    if np.any(rounded < info.min) or np.any(rounded > info.max):
        raise RuntimeError(f"quantized value outside {dtype} range")
    return rounded.astype(dtype), clamped


def write_u32(output, value):
    output.write(struct.pack("<I", value & 0xFFFFFFFF))


def zero_bytes(output, count):
    block = b"\0" * (1024 * 1024)
    while count:
        chunk = min(count, len(block))
        output.write(block[:chunk])
        count -= chunk


def serialize_network(checkpoint: pathlib.Path, target: pathlib.Path, mode: str):
    metadata = read_checkpoint_metadata(checkpoint)
    description = f"{CONSTRUCTION}:{mode}".encode()
    clamped_weights = 0
    target.parent.mkdir(parents=True, exist_ok=True)
    with target.open("wb") as output:
        write_u32(output, VERSION)
        write_u32(output, NETWORK_HASH)
        write_u32(output, len(description))
        output.write(description)
        write_u32(output, TRANSFORMER_HASH)
        input_bias = open_checkpoint_array(checkpoint, metadata, "input_bias")
        if mode == "zero":
            zero_bytes(output, L1 * 2)
        else:
            quantized, _ = quantize(input_bias[:L1], 127.0, np.dtype("<i2"))
            output.write(quantized.tobytes(order="C"))
        weights = open_checkpoint_array(checkpoint, metadata, "input_weight")
        if mode in ("zero", "bias-only"):
            zero_bytes(output, REAL_ROWS * L1 * 2)
            zero_bytes(output, REAL_ROWS * PSQT_BUCKETS * 4)
        else:
            rows_per_chunk = 1024
            for start in range(0, REAL_ROWS, rows_per_chunk):
                chunk = weights[start : start + rows_per_chunk, :L1]
                quantized, _ = quantize(chunk, 127.0, np.dtype("<i2"))
                output.write(quantized.tobytes(order="C"))
            for start in range(0, REAL_ROWS, rows_per_chunk):
                chunk = weights[start : start + rows_per_chunk, L1:]
                quantized, _ = quantize(chunk, 9600.0, np.dtype("<i4"))
                output.write(quantized.tobytes(order="C"))
        for bucket in range(STACKS):
            write_u32(output, OUTPUT_HASH)
            for weight_name, bias_name, bias_scale, weight_scale, inputs, is_output in [
                ("l1_weight", "l1_bias", 8128.0, 64.0, L1 * 2, False),
                ("l2_weight", "l2_bias", 8128.0, 64.0, L2, False),
                ("output_weight", "output_bias", 9600.0, 9600.0 / 127.0, L3, True),
            ]:
                layer_weights = open_checkpoint_array(checkpoint, metadata, weight_name)[bucket]
                biases = open_checkpoint_array(checkpoint, metadata, bias_name)[bucket]
                padded_inputs = ((inputs + 31) // 32) * 32
                if mode == "zero":
                    zero_bytes(output, biases.size * 4)
                else:
                    qbias, _ = quantize(biases, bias_scale, np.dtype("<i4"))
                    output.write(qbias.tobytes(order="C"))
                if mode in ("zero", "bias-only"):
                    zero_bytes(output, layer_weights.shape[0] * padded_inputs)
                else:
                    maximum = 127.0 / weight_scale
                    qweight, clipped = quantize(
                        layer_weights,
                        weight_scale,
                        np.dtype("i1"),
                        -maximum,
                        maximum,
                    )
                    clamped_weights += clipped
                    if padded_inputs == inputs:
                        output.write(qweight.tobytes(order="C"))
                    else:
                        padded = np.zeros(
                            (qweight.shape[0], padded_inputs), dtype=np.dtype("i1")
                        )
                        padded[:, :inputs] = qweight
                        output.write(padded.tobytes(order="C"))
    return clamped_weights


@dataclass
class IntegerNetwork:
    bias: np.ndarray
    weights: np.ndarray
    psqt_weights: np.ndarray
    buckets: list


def read_exact(data: memoryview, offset: int, dtype, shape):
    count = math.prod(shape)
    array = np.frombuffer(data, dtype=dtype, count=count, offset=offset).reshape(shape)
    return array, offset + count * np.dtype(dtype).itemsize


def parse_network(path: pathlib.Path):
    raw = path.read_bytes()
    data = memoryview(raw)
    offset = 0
    version, network_hash, description_len = struct.unpack_from("<III", data, offset)
    offset += 12
    if version != VERSION or network_hash != NETWORK_HASH:
        raise RuntimeError("serialized network header mismatch")
    offset += description_len
    transformer_hash = struct.unpack_from("<I", data, offset)[0]
    offset += 4
    if transformer_hash != TRANSFORMER_HASH:
        raise RuntimeError("serialized transformer hash mismatch")
    bias, offset = read_exact(data, offset, "<i2", (L1,))
    weights, offset = read_exact(data, offset, "<i2", (REAL_ROWS, L1))
    psqt, offset = read_exact(data, offset, "<i4", (REAL_ROWS, PSQT_BUCKETS))
    buckets = []
    for _ in range(STACKS):
        section_hash = struct.unpack_from("<I", data, offset)[0]
        offset += 4
        if section_hash != OUTPUT_HASH:
            raise RuntimeError("serialized affine hash mismatch")
        layers = []
        for outputs, inputs in [(L2, L1 * 2), (L3, 32), (1, 32)]:
            biases, offset = read_exact(data, offset, "<i4", (outputs,))
            layer_weights, offset = read_exact(data, offset, "i1", (outputs, inputs))
            layers.append((biases, layer_weights))
        buckets.append(layers)
    if offset != len(data):
        raise RuntimeError("serialized network has trailing bytes")
    return raw, IntegerNetwork(bias, weights, psqt, buckets)


def trunc_div(value: int, divisor: int) -> int:
    return value // divisor if value >= 0 else -((-value) // divisor)


def integer_trace(network: IntegerNetwork, position, black_rows, white_rows):
    accumulator_overflows = 0
    accumulators = []
    psqts = []
    for rows in (black_rows, white_rows):
        wide = network.bias.astype(np.int64) + network.weights[rows].astype(np.int64).sum(axis=0)
        accumulator_overflows += int(np.count_nonzero((wide < -32768) | (wide > 32767)))
        accumulator = wide.astype("<i2")
        accumulators.append(accumulator)
        psqts.append(network.psqt_weights[rows].astype(np.int64).sum(axis=0).astype("<i4"))
    side_black = position["sfen"].split()[1] == "b"
    our = 0 if side_black else 1
    their = 1 - our
    transformed = np.concatenate(
        [
            np.clip(accumulators[our].astype(np.int32) >> 6, 0, 127),
            np.clip(accumulators[their].astype(np.int32) >> 6, 0, 127),
        ]
    ).astype("u1")
    bucket = int(position["outputBucket"])
    (h1_bias, h1_weight), (h2_bias, h2_weight), (out_bias, out_weight) = network.buckets[bucket]
    h1 = h1_bias.astype(np.int64) + h1_weight.astype(np.int64) @ transformed.astype(np.int64)
    h1 = h1.astype("<i4")
    h1_relu = np.clip(h1.astype(np.int64) >> 6, 0, 127).astype("u1")
    h2 = h2_bias.astype(np.int64) + h2_weight[:, :L2].astype(np.int64) @ h1_relu.astype(np.int64)
    h2 = h2.astype("<i4")
    h2_relu = np.clip(h2.astype(np.int64) >> 6, 0, 127).astype("u1")
    output = int(out_bias[0]) + int(out_weight[0].astype(np.int64) @ h2_relu.astype(np.int64))
    psqt = trunc_div(int(psqts[our][bucket]) - int(psqts[their][bucket]), 2)
    score = trunc_div(psqt + output, 16)
    clamp_counts = {
        "transformerLower": int(np.count_nonzero(np.concatenate(accumulators).astype(np.int32) < 0)),
        "transformerUpper": int(np.count_nonzero(np.concatenate(accumulators).astype(np.int32) >= 8192)),
        "hidden1Lower": int(np.count_nonzero(h1 < 0)),
        "hidden1Upper": int(np.count_nonzero(h1 >= 8192)),
        "hidden2Lower": int(np.count_nonzero(h2 < 0)),
        "hidden2Upper": int(np.count_nonzero(h2 >= 8192)),
    }
    return {
        "accumulators": accumulators,
        "psqts": psqts,
        "transformed": transformed,
        "h1": h1,
        "h1_relu": h1_relu,
        "h2": h2,
        "h2_relu": h2_relu,
        "output": output,
        "psqt": psqt,
        "score": score,
        "bucket": bucket,
        "accumulator_overflows": accumulator_overflows,
        "clamp_counts": clamp_counts,
    }


def full_precision_score(checkpoint, metadata, position, black_rows, white_rows):
    # Copy-on-write mappings satisfy PyTorch's writable-buffer contract while
    # retaining the immutable checkpoint bytes.
    bias_np = open_checkpoint_array(checkpoint, metadata, "input_bias", "c")
    weights_np = open_checkpoint_array(checkpoint, metadata, "input_weight", "c")
    bias = torch.from_numpy(np.asarray(bias_np))
    weights = torch.from_numpy(np.asarray(weights_np))
    perspectives = []
    for rows in (black_rows, white_rows):
        indices = torch.tensor(rows, dtype=torch.int64)
        perspectives.append(bias + weights.index_select(0, indices).sum(dim=0))
    side_black = position["sfen"].split()[1] == "b"
    our = 0 if side_black else 1
    their = 1 - our
    transformed = torch.clamp(
        torch.cat([perspectives[our][:L1], perspectives[their][:L1]]), 0.0, 1.0
    )
    bucket = int(position["outputBucket"])
    l1w = torch.from_numpy(
        np.asarray(open_checkpoint_array(checkpoint, metadata, "l1_weight", "c")[bucket])
    )
    l1b = torch.from_numpy(
        np.asarray(open_checkpoint_array(checkpoint, metadata, "l1_bias", "c")[bucket])
    )
    l2w = torch.from_numpy(
        np.asarray(open_checkpoint_array(checkpoint, metadata, "l2_weight", "c")[bucket])
    )
    l2b = torch.from_numpy(
        np.asarray(open_checkpoint_array(checkpoint, metadata, "l2_bias", "c")[bucket])
    )
    outw = torch.from_numpy(
        np.asarray(open_checkpoint_array(checkpoint, metadata, "output_weight", "c")[bucket])
    )
    outb = torch.from_numpy(
        np.asarray(open_checkpoint_array(checkpoint, metadata, "output_bias", "c")[bucket])
    )
    h1 = torch.clamp(torch.mv(l1w, transformed) + l1b, 0.0, 1.0)
    h2 = torch.clamp(torch.mv(l2w, h1) + l2b, 0.0, 1.0)
    output = torch.mv(outw, h2) + outb
    psqt = (perspectives[our][L1 + bucket] - perspectives[their][L1 + bucket]) * 0.5
    return float((output[0] + psqt) * 600.0)


def write_expectations(path, checkpoint, metadata, network_path, fixtures, include_full):
    raw, network = parse_network(network_path)
    counters = {
        "accumulatorOverflows": 0,
        "clamps": {
            "transformerLower": 0,
            "transformerUpper": 0,
            "hidden1Lower": 0,
            "hidden1Upper": 0,
            "hidden2Lower": 0,
            "hidden2Upper": 0,
        },
    }
    with path.open("wb") as output_file:
        output_file.write(EXPECT_MAGIC)
        output_file.write(struct.pack("<I", len(fixtures)))
        for position, black_rows, white_rows in fixtures:
            trace = integer_trace(network, position, black_rows, white_rows)
            full_score = (
                full_precision_score(checkpoint, metadata, position, black_rows, white_rows)
                if include_full
                else float(trace["score"])
            )
            output_file.write(struct.pack("<dB3x", full_score, trace["bucket"]))
            for accumulator in trace["accumulators"]:
                output_file.write(accumulator.astype("<i2", copy=False).tobytes())
            for psqt in trace["psqts"]:
                output_file.write(psqt.astype("<i4", copy=False).tobytes())
            output_file.write(trace["transformed"].tobytes())
            output_file.write(trace["h1"].astype("<i4", copy=False).tobytes())
            output_file.write(trace["h1_relu"].tobytes())
            output_file.write(trace["h2"].astype("<i4", copy=False).tobytes())
            output_file.write(trace["h2_relu"].tobytes())
            output_file.write(struct.pack("<iii", trace["output"], trace["psqt"], trace["score"]))
            counters["accumulatorOverflows"] += trace["accumulator_overflows"]
            for name, count in trace["clamp_counts"].items():
                counters["clamps"][name] += count
    del raw
    return counters


def artifact(path: pathlib.Path):
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256(path)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", required=True, type=pathlib.Path)
    parser.add_argument("--features", required=True, type=pathlib.Path)
    parser.add_argument("--limits", required=True, type=pathlib.Path)
    parser.add_argument("--output-dir", required=True, type=pathlib.Path)
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    fixtures = load_fixtures(args.corpus, args.features)
    limits = json.loads(args.limits.read_text())
    if limits.get("schema") != "haitaka-r1b-quantization-limits-v1":
        raise RuntimeError("wrong frozen R1-B limit schema")

    checkpoint_a = args.output_dir / "sentinel-checkpoint-a.r1fp"
    checkpoint_b = args.output_dir / "sentinel-checkpoint-b.r1fp"
    metadata_a = generate_checkpoint(checkpoint_a, fixtures)
    metadata_b = generate_checkpoint(checkpoint_b, fixtures)
    if metadata_a != metadata_b or sha256(checkpoint_a) != sha256(checkpoint_b):
        raise RuntimeError("repeat full-precision checkpoint generation was not byte-identical")

    network_a = args.output_dir / "sentinel-a.nnue"
    network_b = args.output_dir / "sentinel-b.nnue"
    clamped_a = serialize_network(checkpoint_a, network_a, "sentinel")
    clamped_b = serialize_network(checkpoint_a, network_b, "sentinel")
    export_metadata = {
        "schema": "haitaka-r1b-export-metadata-v1",
        "construction": CONSTRUCTION,
        "checkpointSha256": sha256(checkpoint_a),
        "featureFamily": "HalfKAv2^+DonorSingleEff",
        "description": f"{CONSTRUCTION}:sentinel",
        "rounding": "ties-to-even",
        "serializerClampedWeights": clamped_a,
    }
    export_metadata_a = args.output_dir / "sentinel-export-metadata-a.json"
    export_metadata_b = args.output_dir / "sentinel-export-metadata-b.json"
    export_metadata_a.write_bytes(canonical_json(export_metadata))
    export_metadata_b.write_bytes(canonical_json(export_metadata))
    if network_a.read_bytes() != network_b.read_bytes():
        raise RuntimeError("repeat export was not byte-identical")
    if export_metadata_a.read_bytes() != export_metadata_b.read_bytes():
        raise RuntimeError("repeat export metadata was not byte-identical")

    zero_network = args.output_dir / "zero.nnue"
    bias_network = args.output_dir / "bias-only.nnue"
    serialize_network(checkpoint_a, zero_network, "zero")
    serialize_network(checkpoint_a, bias_network, "bias-only")

    expectations = {}
    counters = {}
    for name, network, include_full in [
        ("zero", zero_network, False),
        ("bias-only", bias_network, False),
        ("sentinel", network_a, True),
    ]:
        path = args.output_dir / f"{name}-expectations.bin"
        counters[name] = write_expectations(
            path, checkpoint_a, metadata_a, network, fixtures, include_full
        )
        expectations[name] = artifact(path)

    source_path = pathlib.Path(__file__).resolve()
    construction_metadata = {
        "schema": "haitaka-r1b-sentinel-network-v1",
        "construction": CONSTRUCTION,
        "generatorSourceSha256": sha256(source_path),
        "checkpointRegenerationByteIdentical": True,
        "repeatExportByteIdentical": True,
        "repeatMetadataByteIdentical": True,
        "serializerClampedWeights": clamped_a + clamped_b,
        "patterns": {
            "zeroNetwork": True,
            "biasOnlyNetwork": True,
            "oneHotPositiveAndNegativeRows": True,
            "distinctDonorReceiverAndPerspectiveSignatures": True,
            "activationClampBoundaries": [-1, 0, 63, 64, 127, 128, 8127, 8128, 8129],
            "tiesToEven": ["0.5->0", "1.5->2", "-0.5->0", "-1.5->-2"],
            "minimumMaximumSerializedTransformerWeights": [-32768, 32767],
        },
        "checkpoint": artifact(checkpoint_a),
        "checkpointRegeneration": artifact(checkpoint_b),
        "networks": {
            "zero": artifact(zero_network),
            "bias-only": artifact(bias_network),
            "sentinel": artifact(network_a),
            "sentinelRepeat": artifact(network_b),
        },
        "exportMetadata": {
            "first": artifact(export_metadata_a),
            "repeat": artifact(export_metadata_b),
        },
        "expectations": expectations,
        "integerAudit": counters,
        "fullPrecisionPrediction": {
            "framework": "PyTorch",
            "frameworkVersion": torch.__version__,
            "device": "cpu",
            "scoreScale": 600,
        },
        "frozenLimitsSha256": sha256(args.limits),
        "corpusSha256": sha256(args.corpus),
        "featuresSha256": sha256(args.features),
    }
    metadata_path = args.output_dir / "sentinel-network-metadata.json"
    metadata_path.write_bytes(canonical_json(construction_metadata))
    print(json.dumps({"metadata": str(metadata_path), "sha256": sha256(metadata_path)}))


if __name__ == "__main__":
    main()
