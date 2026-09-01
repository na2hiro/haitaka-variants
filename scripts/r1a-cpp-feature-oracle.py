#!/usr/bin/env python3
"""Dump exact R1-A real-row features from the C++ sparse data loader."""

import argparse
import ctypes
import json
from pathlib import Path


class SparseBatch(ctypes.Structure):
    _fields_ = [
        ("num_inputs", ctypes.c_int),
        ("size", ctypes.c_int),
        ("is_white", ctypes.POINTER(ctypes.c_float)),
        ("outcome", ctypes.POINTER(ctypes.c_float)),
        ("score", ctypes.POINTER(ctypes.c_float)),
        ("num_active_white_features", ctypes.c_int),
        ("num_active_black_features", ctypes.c_int),
        ("max_active_features", ctypes.c_int),
        ("white", ctypes.POINTER(ctypes.c_int)),
        ("black", ctypes.POINTER(ctypes.c_int)),
        ("white_values", ctypes.POINTER(ctypes.c_float)),
        ("black_values", ctypes.POINTER(ctypes.c_float)),
        ("psqt_indices", ctypes.POINTER(ctypes.c_int)),
        ("layer_stack_indices", ctypes.POINTER(ctypes.c_int)),
    ]


def rows(pointer, offset, width, base_rows):
    indices = sorted(pointer[offset + i] for i in range(width) if pointer[offset + i] >= 0)
    return {
        "base": [value for value in indices if value < base_rows],
        "donor": [value for value in indices if value >= base_rows],
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--library", required=True, type=Path)
    parser.add_argument("--dataset", required=True, type=Path)
    parser.add_argument("--ids", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--base-rows", required=True, type=int)
    args = parser.parse_args()

    fixture_rows = []
    for line in args.ids.read_text(encoding="utf-8").splitlines():
        fixture_id, score, is_white = line.split("\t")
        fixture_rows.append((fixture_id, int(score), int(is_white)))
    library = ctypes.cdll.LoadLibrary(str(args.library.resolve()))
    batch_pointer = ctypes.POINTER(SparseBatch)
    library.create_sparse_batch_stream.restype = ctypes.c_void_p
    library.create_sparse_batch_stream.argtypes = [
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_bool,
        ctypes.c_bool,
        ctypes.c_int,
    ]
    library.fetch_next_sparse_batch.restype = batch_pointer
    library.fetch_next_sparse_batch.argtypes = [ctypes.c_void_p]
    library.destroy_sparse_batch.argtypes = [batch_pointer]
    library.destroy_sparse_batch_stream.argtypes = [ctypes.c_void_p]

    stream = library.create_sparse_batch_stream(
        b"HalfKAv2+DonorSingleEff-R1Oracle",
        1,
        str(args.dataset.resolve()).encode("utf-8"),
        512,
        False,
        False,
        0,
    )
    if not stream:
        raise RuntimeError("C++ loader rejected the R1-A oracle feature set")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    fixture_index = 0
    try:
        with args.output.open("w", encoding="utf-8", newline="\n") as output:
            while True:
                pointer = library.fetch_next_sparse_batch(stream)
                if not pointer:
                    break
                batch = pointer.contents
                try:
                    for row_index in range(batch.size):
                        if fixture_index >= len(fixture_rows):
                            raise RuntimeError("C++ loader returned more rows than fixture IDs")
                        fixture_id, expected_score, expected_is_white = fixture_rows[fixture_index]
                        actual_score = int(batch.score[row_index])
                        actual_is_white = int(batch.is_white[row_index])
                        if actual_score != expected_score or actual_is_white != expected_is_white:
                            raise RuntimeError(
                                f"label orientation mismatch for {fixture_id}: "
                                f"score={actual_score}/{expected_score}, "
                                f"is_white={actual_is_white}/{expected_is_white}"
                            )
                        offset = row_index * batch.max_active_features
                        # Trainer White is runtime Black because the packed ABI
                        # mirrors colors; Trainer Black is runtime White.
                        record = {
                            "id": fixture_id,
                            "black": rows(
                                batch.white,
                                offset,
                                batch.max_active_features,
                                args.base_rows,
                            ),
                            "white": rows(
                                batch.black,
                                offset,
                                batch.max_active_features,
                                args.base_rows,
                            ),
                        }
                        output.write(json.dumps(record, separators=(",", ":")) + "\n")
                        fixture_index += 1
                finally:
                    library.destroy_sparse_batch(pointer)
    finally:
        library.destroy_sparse_batch_stream(stream)

    if fixture_index != len(fixture_rows):
        raise RuntimeError(
            f"C++ loader returned {fixture_index} rows for {len(fixture_rows)} fixture IDs"
        )


if __name__ == "__main__":
    main()
