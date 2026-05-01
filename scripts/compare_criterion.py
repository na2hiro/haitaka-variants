#!/usr/bin/env python3
"""Compare Criterion baseline estimates and print a Markdown report."""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class BenchmarkResult:
    suite: str
    name: str
    base_ns: float
    new_ns: float

    @property
    def delta_percent(self) -> float:
        return ((self.new_ns / self.base_ns) - 1.0) * 100.0


def parse_suite(value: str) -> tuple[str, Path]:
    if "=" not in value:
        raise argparse.ArgumentTypeError("suite must use label=criterion_dir")
    label, path = value.split("=", 1)
    label = label.strip()
    if not label:
        raise argparse.ArgumentTypeError("suite label must not be empty")
    return label, Path(path)


def load_point_estimate(path: Path) -> float:
    with path.open(encoding="utf-8") as file:
        estimates = json.load(file)

    for metric in ("median", "mean"):
        point_estimate = estimates.get(metric, {}).get("point_estimate")
        if point_estimate is not None:
            return float(point_estimate)

    raise ValueError(f"{path} does not contain a median or mean point_estimate")


def benchmark_name(criterion_dir: Path, estimates_path: Path) -> str:
    benchmark_dir = estimates_path.parent.parent
    return benchmark_dir.relative_to(criterion_dir).as_posix()


def collect_results(suite: str, criterion_dir: Path) -> tuple[list[BenchmarkResult], list[str]]:
    results: list[BenchmarkResult] = []
    errors: list[str] = []

    if not criterion_dir.exists():
        errors.append(f"{suite}: missing Criterion directory {criterion_dir}")
        return results, errors

    base_estimates = sorted(criterion_dir.glob("**/base/estimates.json"))
    if not base_estimates:
        errors.append(f"{suite}: no base estimates found under {criterion_dir}")
        return results, errors

    for base_path in base_estimates:
        name = benchmark_name(criterion_dir, base_path)
        new_path = base_path.parent.parent / "new" / "estimates.json"
        if not new_path.exists():
            errors.append(f"{suite}/{name}: missing new estimates at {new_path}")
            continue

        try:
            base_ns = load_point_estimate(base_path)
            new_ns = load_point_estimate(new_path)
        except (OSError, ValueError, json.JSONDecodeError) as error:
            errors.append(f"{suite}/{name}: {error}")
            continue

        if base_ns <= 0.0:
            errors.append(f"{suite}/{name}: base estimate must be positive")
            continue

        results.append(BenchmarkResult(suite=suite, name=name, base_ns=base_ns, new_ns=new_ns))

    return results, errors


def format_duration(ns: float) -> str:
    if ns < 1_000.0:
        return f"{ns:.2f} ns"
    if ns < 1_000_000.0:
        return f"{ns / 1_000.0:.2f} us"
    if ns < 1_000_000_000.0:
        return f"{ns / 1_000_000.0:.2f} ms"
    return f"{ns / 1_000_000_000.0:.2f} s"


def classify(delta_percent: float, warning: float, significant: float) -> str:
    if delta_percent >= significant:
        return "significant"
    if delta_percent >= warning:
        return "warning"
    return "ok"


def print_report(results: list[BenchmarkResult], warning: float, significant: float) -> None:
    print("# Benchmark Comparison")
    print()
    print(
        f"Slowdowns of {warning:.1f}% or more are warnings. "
        f"Slowdowns of {significant:.1f}% or more are significant."
    )
    print()
    print("| Suite | Benchmark | Base | Head | Delta | Status |")
    print("| --- | --- | ---: | ---: | ---: | --- |")

    for result in sorted(results, key=lambda item: (item.suite, item.name)):
        delta = result.delta_percent
        print(
            "| "
            f"{result.suite} | "
            f"`{result.name}` | "
            f"{format_duration(result.base_ns)} | "
            f"{format_duration(result.new_ns)} | "
            f"{delta:+.2f}% | "
            f"{classify(delta, warning, significant)} |"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--suite",
        action="append",
        type=parse_suite,
        required=True,
        metavar="LABEL=DIR",
        help="Criterion output directory to compare.",
    )
    parser.add_argument(
        "--warning",
        type=float,
        default=5.0,
        help="Slowdown percentage that should be reported as a warning.",
    )
    parser.add_argument(
        "--significant",
        type=float,
        default=15.0,
        help="Slowdown percentage that should be reported as significant.",
    )
    args = parser.parse_args()

    if args.warning < 0.0 or args.significant < 0.0:
        print("Thresholds must be non-negative.", file=sys.stderr)
        return 2
    if args.warning > args.significant:
        print("--warning must be less than or equal to --significant.", file=sys.stderr)
        return 2

    results: list[BenchmarkResult] = []
    errors: list[str] = []
    for suite, criterion_dir in args.suite:
        suite_results, suite_errors = collect_results(suite, criterion_dir)
        results.extend(suite_results)
        errors.extend(suite_errors)

    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1
    if not results:
        print("error: no benchmark results found", file=sys.stderr)
        return 1

    print_report(results, args.warning, args.significant)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
