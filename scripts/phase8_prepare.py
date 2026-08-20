#!/usr/bin/env python3
"""Preflight Phase 8 lane identity and calibrate fixed-node feasibility."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

try:
    import tomllib

    TOML_ERRORS = (tomllib.TOMLDecodeError,)

    def load_toml(handle: Any) -> dict[str, Any]:
        return tomllib.load(handle)

except ModuleNotFoundError:
    import toml

    TOML_ERRORS = (toml.TomlDecodeError,)

    def load_toml(handle: Any) -> dict[str, Any]:
        return toml.loads(handle.read().decode())


REPO_ROOT = Path(__file__).resolve().parents[1]
PHASE7_CONFIG = REPO_ROOT / "haitaka_learn.anhoku-v0.6.toml"
ROOT_CONFIG = REPO_ROOT / "haitaka_learn.anhoku-v0.6-phase8-root.toml"
LEAF_CONFIG = REPO_ROOT / "haitaka_learn.anhoku-v0.6-phase8-leaf.toml"
CALIBRATION_BASES = {
    "root-position": REPO_ROOT / "haitaka_learn.anhoku-v0.6-phase4.smoke.toml",
    "qsearch-pv-leaf": REPO_ROOT / "haitaka_learn.anhoku-v0.6-phase5.smoke.toml",
}
PHASE8_NODE_BUDGET = 50_000

TEACHER_DATA_KEYS = {
    "search_depth",
    "label_search_nodes",
    "label_search_max_depth",
    "position_policy",
    "incomplete_label_policy",
}


def read_toml(path: Path) -> dict[str, Any]:
    with path.open("rb") as handle:
        return load_toml(handle)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def non_teacher_view(config: dict[str, Any]) -> dict[str, Any]:
    view = copy.deepcopy(config)
    view.get("paths", {}).pop("output_dir", None)
    data = view.get("data", {})
    for key in TEACHER_DATA_KEYS:
        data.pop(key, None)
    view.pop("export", None)
    return view


def canonical_hash(value: Any) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def mismatch_paths(left: Any, right: Any, prefix: str = "") -> list[str]:
    if isinstance(left, dict) and isinstance(right, dict):
        paths: list[str] = []
        for key in sorted(set(left) | set(right)):
            child = f"{prefix}.{key}" if prefix else key
            if key not in left or key not in right:
                paths.append(child)
            else:
                paths.extend(mismatch_paths(left[key], right[key], child))
        return paths
    return [] if left == right else [prefix]


def lane_identity(path: Path, config: dict[str, Any]) -> dict[str, Any]:
    data = config["data"]
    return {
        "config": path.name,
        "config_sha256": sha256_file(path),
        "output_dir": config["paths"]["output_dir"],
        "label_search_depth": data.get("search_depth"),
        "label_search_nodes": data.get("label_search_nodes"),
        "label_search_max_depth": data.get("label_search_max_depth"),
        "position_policy": data.get("position_policy", "root-position"),
        "incomplete_label_policy": data.get("incomplete_label_policy", "error"),
    }


def check_configs(output: Path | None) -> int:
    configs = {
        "phase7-depth3-root": (PHASE7_CONFIG, read_toml(PHASE7_CONFIG)),
        "phase8-fixed-node-root": (ROOT_CONFIG, read_toml(ROOT_CONFIG)),
        "phase8-fixed-node-leaf": (LEAF_CONFIG, read_toml(LEAF_CONFIG)),
    }
    baseline_view = non_teacher_view(configs["phase7-depth3-root"][1])
    errors: list[str] = []

    for lane, (_, config) in configs.items():
        differences = mismatch_paths(baseline_view, non_teacher_view(config))
        if differences:
            errors.append(f"{lane}: non-teacher mismatch at {', '.join(differences)}")

    root_data = configs["phase8-fixed-node-root"][1]["data"]
    leaf_data = configs["phase8-fixed-node-leaf"][1]["data"]
    if root_data.get("position_policy") != "root-position":
        errors.append("fixed-node root lane must use position_policy=root-position")
    if leaf_data.get("position_policy") != "qsearch-pv-leaf":
        errors.append("fixed-node leaf lane must use position_policy=qsearch-pv-leaf")
    for lane, data in (("root", root_data), ("leaf", leaf_data)):
        if data.get("label_search_nodes") != PHASE8_NODE_BUDGET:
            errors.append(
                f"fixed-node {lane} lane must use the calibrated "
                f"{PHASE8_NODE_BUDGET:,}-node budget"
            )
        if data.get("label_search_max_depth") != 64:
            errors.append(f"fixed-node {lane} lane must use depth cap 64")
        if "search_depth" in data:
            errors.append(f"fixed-node {lane} lane cannot also set search_depth")
        if data.get("incomplete_label_policy") != "reject-position":
            errors.append(
                f"fixed-node {lane} lane must reject and count incomplete labels"
            )

    report = {
        "schema": "anhoku-phase8-preflight-v1",
        "config_identity_ready": not errors,
        "launch_ready": False,
        "non_teacher_sha256": canonical_hash(baseline_view),
        "training_initialization_seeds": [80, 81, 82],
        "lanes": {
            name: lane_identity(path, config)
            for name, (path, config) in configs.items()
        },
        "errors": errors,
        "launch_gates": [
            "Phase 7 explicitly approves Phase 8",
            "a representative pilot keeps the 50,000-node incomplete-label rejection rate at or below 1%",
            "the external trainer records initialization seeds 80, 81, and 82",
        ],
    }
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if output:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(rendered)
        print(f"wrote {output}")
    print(rendered, end="")
    return 0 if not errors else 1


def replace_one(text: str, pattern: str, replacement: str) -> str:
    updated, count = re.subn(pattern, replacement, text, count=1, flags=re.MULTILINE)
    if count != 1:
        raise ValueError(f"expected one match for {pattern!r}, found {count}")
    return updated


def calibration_config(
    base: str,
    output_dir: Path,
    nodes: int,
    games: int,
    positions_per_game: int,
    reject_incomplete: bool,
) -> str:
    text = replace_one(
        base,
        r'^output_dir = ".*"$',
        f'output_dir = "{output_dir.as_posix()}"',
    )
    text = replace_one(text, r"^label_search_nodes = \d+$", f"label_search_nodes = {nodes}")
    text = replace_one(text, r"^train_games = \d+$", f"train_games = {games}")
    text = replace_one(text, r"^validation_games = \d+$", f"validation_games = {games}")
    text = replace_one(
        text,
        r'^opening_suite = ".*"$',
        f'opening_suite = "{(REPO_ROOT / "haitaka_learn/openings/anhoku-v1.tsv").as_posix()}"',
    )
    text = replace_one(
        text,
        r"^max_positions_per_game = \d+$",
        f"max_positions_per_game = {positions_per_game}",
    )
    if reject_incomplete:
        if re.search(r'^position_policy = ".*"$', text, flags=re.MULTILINE):
            text = replace_one(
                text,
                r'^(position_policy = ".*")$',
                r'\1\nincomplete_label_policy = "reject-position"',
            )
        else:
            text = replace_one(
                text,
                r"^(label_search_max_depth = \d+)$",
                r"\1"
                + '\nposition_policy = "root-position"'
                + '\nincomplete_label_policy = "reject-position"',
            )
    return text


def manifest_stats(output_dir: Path) -> dict[str, Any]:
    totals = {
        "positions": 0,
        "candidate_positions": 0,
        "rejected_incomplete_label_positions": 0,
        "rejected_terminal_positions": 0,
        "rejected_mate_score_positions": 0,
        "label_searches": 0,
        "label_nodes": 0,
        "label_cpu_seconds": 0.0,
        "wall_seconds": 0.0,
    }
    for split in ("train", "validation"):
        manifest = json.loads((output_dir / "datasets" / f"{split}.json").read_text())
        totals["positions"] += manifest["sampled_positions"]
        totals["candidate_positions"] += manifest["candidate_positions"]
        totals["rejected_incomplete_label_positions"] += manifest.get(
            "rejected_incomplete_label_positions", 0
        )
        totals["rejected_terminal_positions"] += manifest.get(
            "rejected_terminal_positions", 0
        )
        totals["rejected_mate_score_positions"] += manifest.get(
            "rejected_mate_score_positions", 0
        )
        totals["label_searches"] += manifest["label_searches"]
        totals["label_nodes"] += manifest["label_search_total_nodes"]
        totals["label_cpu_seconds"] += manifest["label_search_cpu_seconds"]
        totals["wall_seconds"] += manifest["elapsed_seconds"]
    return totals


def calibrate(args: argparse.Namespace) -> int:
    calibration_base = CALIBRATION_BASES[args.position_policy]
    base = calibration_base.read_text()
    results: list[dict[str, Any]] = []
    with tempfile.TemporaryDirectory(prefix="anhoku-phase8-calibration-") as temp_name:
        temp = Path(temp_name)
        for nodes in args.nodes:
            output_dir = temp / f"nodes-{nodes}"
            config_path = temp / f"nodes-{nodes}.toml"
            config_path.write_text(
                calibration_config(
                    base,
                    output_dir,
                    nodes,
                    args.games,
                    args.positions_per_game,
                    args.reject_incomplete,
                )
            )
            command = [
                "cargo",
                "run",
                "--release",
                "-p",
                "haitaka_learn",
                "--features",
                "anhoku",
                "--",
                "generate-data",
                "--config",
                str(config_path),
                "--no-resume",
            ]
            print(f"calibrating {nodes:,} nodes ...", flush=True)
            started = time.monotonic()
            process = subprocess.run(
                command,
                cwd=REPO_ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
            )
            elapsed = time.monotonic() - started
            result: dict[str, Any] = {
                "nodes": nodes,
                "passed": process.returncode == 0,
                "process_wall_seconds": round(elapsed, 3),
            }
            if process.returncode == 0:
                result.update(manifest_stats(output_dir))
                result["all_labels_complete"] = (
                    result["rejected_incomplete_label_positions"] == 0
                )
                print(
                    f"  pass: {result['label_searches']} labels, "
                    f"{result['process_wall_seconds']:.1f}s wall",
                    flush=True,
                )
            else:
                lines = [line for line in process.stdout.splitlines() if line.strip()]
                result["failure_tail"] = lines[-12:]
                print("  fail: label search did not complete", flush=True)
            results.append(result)

    passing = [result["nodes"] for result in results if result["passed"]]
    report = {
        "schema": "anhoku-phase8-node-calibration-v1",
        "base_config": calibration_base.name,
        "base_config_sha256": sha256_file(calibration_base),
        "position_policy": args.position_policy,
        "games_per_split": args.games,
        "max_positions_per_game": args.positions_per_game,
        "incomplete_label_policy": (
            "reject-position" if args.reject_incomplete else "error"
        ),
        "budgets": results,
        "lowest_passing_budget": min(passing) if passing else None,
        "planned_budget_passed": any(
            result["nodes"] == PHASE8_NODE_BUDGET and result["passed"]
            for result in results
        ),
        "note": "A passing bounded calibration is necessary but not sufficient for the 1M launch.",
    }
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered)
        print(f"wrote {args.output}")
    print(rendered, end="")
    if args.reject_incomplete:
        return 0 if all(result["passed"] for result in results) else 2
    return 0 if report["planned_budget_passed"] else 2


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)

    check = subcommands.add_parser("check", help="verify Phase 8 lane identity")
    check.add_argument("--output", type=Path)

    calibration = subcommands.add_parser(
        "calibrate", help="run a bounded fixed-node feasibility matrix"
    )
    calibration.add_argument(
        "--nodes", type=int, nargs="+", default=[5000, 10000, 20000, 50000]
    )
    calibration.add_argument("--games", type=int, default=12)
    calibration.add_argument("--positions-per-game", type=int, default=24)
    calibration.add_argument("--reject-incomplete", action="store_true")
    calibration.add_argument(
        "--position-policy",
        choices=sorted(CALIBRATION_BASES),
        default="root-position",
    )
    calibration.add_argument("--output", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.command == "check":
        return check_configs(args.output)
    if args.command == "calibrate":
        return calibrate(args)
    raise AssertionError(args.command)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, *TOML_ERRORS) as error:
        print(f"phase8 preparation failed: {error}", file=sys.stderr)
        sys.exit(1)
