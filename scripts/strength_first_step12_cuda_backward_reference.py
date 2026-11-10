#!/usr/bin/env python3
"""Independent scalar float32 reference for ordered feature-transformer gradients."""

from __future__ import annotations

import hashlib
import json
from fractions import Fraction
import struct
from typing import Any, Iterable

import numpy as np


REFERENCE_SCHEMA = "haitaka-anhoku-strength-first-step12-deterministic-cuda-reference-v1"


def _round_even(numerator: int, denominator: int) -> int:
    quotient, remainder = divmod(numerator, denominator)
    twice = remainder * 2
    if twice > denominator or (twice == denominator and quotient & 1):
        quotient += 1
    return quotient


def _floor_log2(value: Fraction) -> int:
    exponent = value.numerator.bit_length() - value.denominator.bit_length()
    if exponent >= 0:
        lower = value.denominator << exponent
        if value.numerator < lower:
            exponent -= 1
    elif value.numerator << (-exponent) < value.denominator:
        exponent -= 1
    return exponent


def _fraction_from_bits(value: int) -> Fraction:
    exponent = (value >> 23) & 0xFF
    mantissa = value & 0x7FFFFF
    if exponent == 0xFF:
        raise ValueError("nonfinite binary32 input")
    if exponent == 0:
        return Fraction(mantissa, 1 << 149)
    significand = (1 << 23) + mantissa
    if exponent >= 127:
        return Fraction(significand << (exponent - 127), 1 << 23)
    return Fraction(significand, 1 << (23 + 127 - exponent))


def _bits_from_fraction(value: Fraction, negative_zero: bool = False) -> int:
    negative = value < 0
    magnitude = -value if negative else value
    sign = 0x80000000 if negative or (magnitude == 0 and negative_zero) else 0
    if magnitude == 0:
        return sign
    exponent = _floor_log2(magnitude)
    if exponent < -126:
        scaled = magnitude * (1 << 149)
        numerator, denominator = scaled.numerator, scaled.denominator
        mantissa = _round_even(numerator, denominator)
        if mantissa == 0:
            return sign
        if mantissa >= (1 << 23):
            return sign | (1 << 23)
        return sign | mantissa
    if exponent > 127:
        return sign | 0x7F800000
    shift = 23 - exponent
    if shift >= 0:
        scaled = magnitude * (1 << shift)
    else:
        scaled = magnitude / (1 << (-shift))
    significand = _round_even(scaled.numerator, scaled.denominator)
    if significand >= (1 << 24):
        significand >>= 1
        exponent += 1
    if exponent > 127:
        return sign | 0x7F800000
    return sign | ((exponent + 127) << 23) | (significand - (1 << 23))


def _bits_from_value(value: Any) -> int:
    scalar = np.float32(value)
    return struct.unpack("<I", scalar.tobytes())[0]


def _value_from_bits(value: int) -> np.float32:
    return np.frombuffer(struct.pack("<I", value), dtype="<f4")[0]


def _add_bits(left: int, right: int) -> int:
    left_sign = bool(left & 0x80000000)
    right_sign = bool(right & 0x80000000)
    left_value = _fraction_from_bits(left & 0x7FFFFFFF)
    right_value = _fraction_from_bits(right & 0x7FFFFFFF)
    signed = (-left_value if left_sign else left_value) + (-right_value if right_sign else right_value)
    return _bits_from_fraction(signed, negative_zero=left_sign and right_sign)


def _mul_bits(left: int, right: int) -> int:
    left_sign = bool(left & 0x80000000)
    right_sign = bool(right & 0x80000000)
    left_value = _fraction_from_bits(left & 0x7FFFFFFF)
    right_value = _fraction_from_bits(right & 0x7FFFFFFF)
    return _bits_from_fraction((-left_value if left_sign else left_value) * (-right_value if right_sign else right_value), negative_zero=left_sign ^ right_sign)


def f32(value: Any) -> np.float32:
    """Round a scalar through the binary32 encoding."""
    return _value_from_bits(_bits_from_value(value))


def f32_add(left: Any, right: Any) -> np.float32:
    return _value_from_bits(_add_bits(_bits_from_value(left), _bits_from_value(right)))


def f32_mul(left: Any, right: Any) -> np.float32:
    return _value_from_bits(_mul_bits(_bits_from_value(left), _bits_from_value(right)))


def _perspectives(indices: Any, values: Any, gradients: Any) -> Iterable[tuple[np.ndarray, np.ndarray, np.ndarray]]:
    if len(indices) != len(values) or len(indices) != len(gradients):
        raise ValueError("feature-transformer perspective dimensions differ")
    for index_rows, value_rows, gradient_rows in zip(indices, values, gradients):
        yield np.asarray(index_rows, dtype=np.int32), np.asarray(value_rows, dtype=np.float32), np.asarray(gradient_rows, dtype=np.float32)


def reference_backward(
    *,
    feature_count: int,
    output_size: int,
    perspectives: Iterable[tuple[Any, Any, Any]],
    initial_weight_grad: Any | None = None,
    initial_bias_grad: Any | None = None,
) -> tuple[np.ndarray, np.ndarray]:
    weight = np.zeros((feature_count, output_size), dtype=np.float32) if initial_weight_grad is None else np.asarray(initial_weight_grad, dtype=np.float32).copy()
    bias = np.zeros((output_size,), dtype=np.float32) if initial_bias_grad is None else np.asarray(initial_bias_grad, dtype=np.float32).copy()
    for raw_indices, raw_values, raw_gradients in perspectives:
        indices = np.asarray(raw_indices, dtype=np.int32)
        values = np.asarray(raw_values, dtype=np.float32)
        gradients = np.asarray(raw_gradients, dtype=np.float32)
        if indices.ndim != 2 or values.shape != indices.shape or gradients.ndim != 2 or gradients.shape[0] != indices.shape[0] or gradients.shape[1] != output_size:
            raise ValueError("reference perspective shape mismatch")
        for row in range(indices.shape[0]):
            for channel in range(output_size):
                upstream = gradients[row, channel]
                if upstream == np.float32(0.0):
                    continue
                bias[channel] = f32_add(bias[channel], upstream)
                for slot in range(indices.shape[1]):
                    feature = int(indices[row, slot])
                    if feature == -1:
                        break
                    if feature < 0 or feature >= feature_count:
                        raise ValueError(f"active feature index out of range: {feature}")
                    product = f32_mul(upstream, values[row, slot])
                    weight[feature, channel] = f32_add(weight[feature, channel], product)
    return weight, bias


def bits(array: Any) -> str:
    values = np.asarray(array, dtype=np.float32)
    return hashlib.sha256(values.astype("<f4", copy=False).tobytes()).hexdigest()


def case_manifest() -> list[dict[str, Any]]:
    return [
        {"name": "single-b1-all-empty", "featureCount": 6, "outputSize": 3, "indices": [[[-1, -1, -1]]], "values": [[[7.0, -3.0, 2.0]]], "gradients": [[[1.0, -2.0, 0.5]]]},
        {"name": "single-duplicate-empty-partial", "featureCount": 7, "outputSize": 5, "indices": [[[2, 2, 0, -1], [6, -1, 5, 5], [0, 4, 4, 3], [1, 1, 1, 1]]], "values": [[[1.0, -2.0, 0.5, 9.0], [0.25, 2.0, -1.0, 3.0], [0.0, -1.5, 2.0, 0.125], [1.0, -1.0, 0.5, -0.25]]], "gradients": [[[1.0, -2.0, 0.5, -0.25, 0.125], [0.0, 1.5, -3.0, 0.25, 2.0], [-1.0, 0.25, 0.5, 1.0, -2.0], [2.0, -1.0, 0.25, 0.0, -0.5]]]},
        {"name": "double-factorized-cancellation", "featureCount": 8, "outputSize": 4, "indices": [[[0, 7, 7, 3, -1], [4, 4, 2, 2, 2], [6, -1, 1, 1, 1]], [[7, 0, 3, 3, -1], [2, 2, 4, 4, 6], [1, -1, 5, 5, 5]]], "values": [[[0.5, -0.25, 0.125, 2.0, 8.0], [1.0, -1.0, 0.25, -0.5, 0.75], [-2.0, 7.0, 1.0, -1.0, 0.5]], [[-0.5, 0.25, 2.0, -1.0, 4.0], [0.125, 0.25, -2.0, 1.0, -0.75], [1.0, 3.0, 4.0, -2.0, 0.5]]], "gradients": [[[0.5, -1.25, 2.0, -0.0], [1.0, 0.25, -0.5, 0.75], [-2.0, 0.5, 1.0, -1.0]], [[-1.0, 0.25, 0.5, 2.0], [0.75, -2.0, 1.0, 0.125], [1.0, 0.0, -0.25, 0.5]]]},
        {"name": "double-b5-factorized-no-sentinel", "featureCount": 9, "outputSize": 3, "indices": [[[0, 0, 4, -1], [1, 2, 2, 2], [3, 3, 5, 6], [7, -1, 8, 8], [4, 4, 4, 4]], [[8, 0, 4, -1], [2, 2, 1, 1], [5, 5, 3, 6], [7, 7, 8, -1], [0, 0, 0, 0]]], "values": [[[0.5, -0.25, 1.0, 9.0], [1.0, -1.0, 0.125, 0.25], [-2.0, 0.5, 0.75, -1.0], [1.0, 2.0, -3.0, 5.0], [0.25, -0.5, 0.125, 2.0]], [[-0.5, 0.25, 2.0, 8.0], [0.125, 0.25, -2.0, 1.0], [1.0, -1.0, 0.5, -0.75], [2.0, -2.0, 1.0, 6.0], [0.5, -0.25, 0.125, -1.0]]], "gradients": [[[0.5, -1.0, 2.0], [1.0, 0.25, -0.5], [-2.0, 0.5, 1.0], [0.0, -1.0, 0.25], [2.0, -0.25, -0.5]], [[-1.0, 0.25, 0.5], [0.75, -2.0, 1.0], [1.0, 0.0, -0.25], [1.0, 2.0, 0.125], [-0.5, 0.25, 1.0]]]},
    ]


def evaluate_manifest() -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    for case in case_manifest():
        perspectives = zip(case["indices"], case["values"], case["gradients"])
        weight, bias = reference_backward(feature_count=case["featureCount"], output_size=case["outputSize"], perspectives=perspectives)
        results.append({"name": case["name"], "weightShape": list(weight.shape), "biasShape": list(bias.shape), "weightSha256": bits(weight), "biasSha256": bits(bias)})
    return results


def reference_receipt() -> dict[str, Any]:
    primitive_cases = [
        {"op": "add", "left": "0x3f800000", "right": "0x3f000000", "expected": "0x3fc00000"},
        {"op": "add", "left": "0x3f800000", "right": "0x33800000", "expected": "0x3f800000"},
        {"op": "add", "left": "0x3f800000", "right": "0x33800001", "expected": "0x3f800001"},
        {"op": "add", "left": "0x00000001", "right": "0x00000001", "expected": "0x00000002"},
        {"op": "mul", "left": "0x3f800000", "right": "0xc0000000", "expected": "0xc0000000"},
        {"op": "mul", "left": "0x7f7fffff", "right": "0x40000000", "expected": "0x7f800000"},
        {"op": "add", "left": "0x80000000", "right": "0x80000000", "expected": "0x80000000"},
        {"op": "add", "left": "0x00000000", "right": "0x80000000", "expected": "0x00000000"},
    ]
    for case in primitive_cases:
        left = int(case["left"], 16)
        right = int(case["right"], 16)
        observed = _add_bits(left, right) if case["op"] == "add" else _mul_bits(left, right)
        if observed != int(case["expected"], 16):
            raise AssertionError(f"binary32 primitive mismatch: {case} observed 0x{observed:08x}")
    return {"schema": REFERENCE_SCHEMA, "status": "passed", "rounding": "independent-rational-binary32-round-to-nearest-even-after-each-operation", "primitiveCases": primitive_cases, "cases": evaluate_manifest()}
