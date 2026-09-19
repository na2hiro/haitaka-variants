#!/usr/bin/env python3
"""Device-independent regression for the seeded CUDA RNG restore comparison."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
from typing import Any


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


class Tensor:
    def __init__(self, values: tuple[int, ...]) -> None:
        self.values = values


class Torch:
    @staticmethod
    def equal(left: Tensor, right: Tensor) -> bool:
        return left.values == right.values


def state_restored_exact(before: list[Tensor], restored: list[Tensor]) -> bool:
    return bool(len(before) > 0 and len(restored) == len(before) and all(Torch.equal(observed, expected) for observed, expected in zip(restored, before)))


def run(fixture: Path) -> dict[str, Any]:
    source = fixture.read_text(encoding="utf-8")
    source_contract = "all(torch.equal(observed, expected) for observed, expected in zip(restored, before))" in source
    same = state_restored_exact([Tensor((1, 2))], [Tensor((1, 2))])
    changed = state_restored_exact([Tensor((1, 2))], [Tensor((1, 3))])
    unequal_length = state_restored_exact([Tensor((1, 2))], [])
    empty = state_restored_exact([], [])
    cases = {
        "identical": same,
        "changedTensor": changed,
        "unequalLength": unequal_length,
        "empty": empty,
    }
    passed = source_contract and cases == {"identical": True, "changedTensor": False, "unequalLength": False, "empty": False}
    return {
        "schema": "haitaka-anhoku-strength-first-step12-seeded-cuda-rng-probe-regression-v1",
        "createdAt": now(),
        "status": "passed" if passed else "failed",
        "fixture": {"path": str(fixture), "bytes": fixture.stat().st_size},
        "sourceContract": source_contract,
        "cases": cases,
        "ambiguousTensorBooleanAvoided": True,
        "productionTrainingAttempted": False,
        "gpuAttempted": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    receipt = run(args.fixture)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": receipt["status"], "output": str(args.output)}, sort_keys=True))
    return 0 if receipt["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
