#!/usr/bin/env python3
"""Extract deterministic Phase 11-C full-precision relation data.

This helper is intentionally narrow: Rust owns board decoding, activation
coverage, quantized-network mutation, evaluation, replay, and reporting.
"""

import argparse
import hashlib
import pathlib
import struct
import subprocess
import sys

import torch


EXPECTED_TRAINER_REVISION = "61666d9e3653e4df9881b14c23f8fdcc4bf7779b"
EXPECTED_PATCH_SHA256 = "79603cc66250e335ba242477137366f0aa8a2e530ffa36f3abfb582fafaf802f"
EXPECTED_APPLIED_DIFF_SHA256 = "87f5a9a446bb929854dbf01b38db16980e4faee73a2f86044ae725f98ee0bc4b"
EXPECTED_CHECKPOINT_SHA256 = "9d7997027791298b2d4de0a3e61acc571c48ec4c1895c222f0dc2fe292fc373b"
ROWS = 16_200
NATIVE_SLICES = 10
FT_DIMS = 512
PSQT_DIMS = 8
MAGIC = b"HTK11C-FP-V1\0\0\0\0"


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_hash(path: pathlib.Path, expected: str, label: str) -> None:
    actual = sha256(path)
    if actual != expected:
        raise RuntimeError(f"{label} hash mismatch: expected {expected}, got {actual}: {path}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--checkpoint", type=pathlib.Path, required=True)
    parser.add_argument("--trainer-checkout", type=pathlib.Path, required=True)
    parser.add_argument("--reviewed-patch", type=pathlib.Path, required=True)
    parser.add_argument("--applied-diff", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()

    require_hash(args.checkpoint, EXPECTED_CHECKPOINT_SHA256, "V2 checkpoint")
    require_hash(args.reviewed_patch, EXPECTED_PATCH_SHA256, "reviewed trainer patch")
    require_hash(args.applied_diff, EXPECTED_APPLIED_DIFF_SHA256, "applied trainer diff")
    revision = subprocess.run(
        ["git", "-C", str(args.trainer_checkout), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if revision != EXPECTED_TRAINER_REVISION:
        raise RuntimeError(
            f"trainer revision mismatch: expected {EXPECTED_TRAINER_REVISION}, got {revision}"
        )

    # Explicitly retain weights_only=False: this is a frozen, trusted checkpoint
    # and Lightning checkpoint metadata contains trainer-defined Python objects.
    checkpoint = torch.load(args.checkpoint, map_location="cpu", weights_only=False)
    state = checkpoint.get("state_dict")
    if not isinstance(state, dict) or "input.weight" not in state:
        raise RuntimeError("checkpoint does not contain state_dict[input.weight]")
    weights = state["input.weight"].detach().cpu()
    if weights.ndim != 2 or weights.shape[1] != FT_DIMS + PSQT_DIMS:
        raise RuntimeError(f"unexpected input.weight shape {tuple(weights.shape)}")
    if weights.shape[0] < ROWS:
        raise RuntimeError(f"input.weight has only {weights.shape[0]} rows")
    relation = weights[-ROWS:, :].to(dtype=torch.float32).contiguous()
    original_ft = relation[:, :FT_DIMS].mul(127).round().to(torch.int16).contiguous()
    original_psqt = relation[:, FT_DIMS:].mul(9600).round().to(torch.int32).contiguous()

    # Runtime index order is effective, native, relative color, square. Collapse
    # native slices in float32 and quantize only the resulting arithmetic mean.
    grouped = relation.reshape(10, NATIVE_SLICES, 2, 81, FT_DIMS + PSQT_DIMS)
    means = grouped.mean(dim=1).reshape(10 * 2 * 81, FT_DIMS + PSQT_DIMS)
    collapsed_ft = means[:, :FT_DIMS].mul(127).round().to(torch.int16).contiguous()
    collapsed_psqt = means[:, FT_DIMS:].mul(9600).round().to(torch.int32).contiguous()

    args.output.parent.mkdir(parents=True, exist_ok=True)
    checkpoint_digest = bytes.fromhex(EXPECTED_CHECKPOINT_SHA256)
    with args.output.open("wb") as output:
        output.write(MAGIC)
        output.write(struct.pack("<IIII", ROWS, FT_DIMS, PSQT_DIMS, NATIVE_SLICES))
        output.write(checkpoint_digest)
        output.write(relation.numpy().astype("<f4", copy=False).tobytes(order="C"))
        output.write(original_ft.numpy().astype("<i2", copy=False).tobytes(order="C"))
        output.write(original_psqt.numpy().astype("<i4", copy=False).tobytes(order="C"))
        output.write(collapsed_ft.numpy().astype("<i2", copy=False).tobytes(order="C"))
        output.write(collapsed_psqt.numpy().astype("<i4", copy=False).tobytes(order="C"))

    print(f"wrote {args.output} sha256={sha256(args.output)}", file=sys.stderr)


if __name__ == "__main__":
    main()
