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
C16_NNUE = REPO_ROOT / "out/anhoku-v0.6-phase7.1-preserved/lane-c-step-16.nnue"
C16_SHA256 = "049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0"
OOD_V2_IDS = [f"anhoku-v2-{index:03d}" for index in range(53, 65)]

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
        "opening_suite": data.get("opening_suite"),
        "opening_suite_id": data.get("opening_suite_id"),
        "validation_opening_ids": data.get("validation_opening_ids", []),
        "max_candidate_roots_per_game": data.get("max_candidate_roots_per_game"),
    }


def check_configs(output: Path | None) -> int:
    configs = {
        "phase8-fixed-node-root": (ROOT_CONFIG, read_toml(ROOT_CONFIG)),
        "phase8-fixed-node-leaf": (LEAF_CONFIG, read_toml(LEAF_CONFIG)),
    }
    root_view = non_teacher_view(configs["phase8-fixed-node-root"][1])
    leaf_view = non_teacher_view(configs["phase8-fixed-node-leaf"][1])
    errors: list[str] = []
    differences = mismatch_paths(root_view, leaf_view)
    if differences:
        errors.append(f"root/leaf non-teacher mismatch at {', '.join(differences)}")

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
        if data.get("max_candidate_roots_per_game") != data.get(
            "max_positions_per_game"
        ):
            errors.append(
                f"fixed-node {lane} lane must cap attempted roots at max_positions_per_game"
            )

    root_data = configs["phase8-fixed-node-root"][1]["data"]
    suite_path = ROOT_CONFIG.parent / root_data["opening_suite"]
    suite_ids = []
    for raw_line in suite_path.read_text().splitlines():
        line = raw_line.split("#", 1)[0].strip()
        if line:
            suite_ids.append(line.split("\t", 1)[0].strip())
    if len(suite_ids) < 64:
        errors.append(f"OOD-v2 suite must contain at least 64 IDs, found {len(suite_ids)}")
    validation_ids = root_data.get("validation_opening_ids", [])
    if len(validation_ids) < 12:
        errors.append("OOD-v2 must freeze at least 12 validation opening IDs")
    if validation_ids != OOD_V2_IDS:
        errors.append("validation_opening_ids must be the frozen anhoku-v2-053..064 list")
    unknown_validation_ids = sorted(set(validation_ids) - set(suite_ids))
    if unknown_validation_ids:
        errors.append(f"OOD-v2 contains unknown suite IDs: {unknown_validation_ids}")

    c16_sha256 = sha256_file(C16_NNUE) if C16_NNUE.exists() else None
    if c16_sha256 != C16_SHA256:
        errors.append(
            f"C/16 control SHA-256 mismatch: expected {C16_SHA256}, found {c16_sha256}"
        )

    report = {
        "schema": "anhoku-phase8-preflight-v2",
        "config_identity_ready": not errors,
        "launch_ready": False,
        "non_teacher_sha256": canonical_hash(root_view),
        "phase7_reference_config_sha256": sha256_file(PHASE7_CONFIG),
        "c16_control": {"path": str(C16_NNUE), "sha256": c16_sha256},
        "ood_v2_ids": validation_ids,
        "training_initialization_seeds": [80, 81, 82],
        "lanes": {
            name: lane_identity(path, config)
            for name, (path, config) in configs.items()
        },
        "errors": errors,
        "launch_gates": [
            "C/16 control SHA-256 matches the preserved export",
            "the anhoku-v2 suite has at least 64 IDs and freezes 12 OOD-v2 IDs",
            "root and leaf use the same non-teacher config identity",
            "root and leaf final manifests have equal candidate identity hashes",
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


def matched_manifest_check(
    root_output: Path, leaf_output: Path, output: Path | None
) -> int:
    """Compare final root/leaf manifests after the bounded generation pass."""

    report: dict[str, Any] = {
        "schema": "anhoku-phase8-matched-manifest-v1",
        "root_output": str(root_output),
        "leaf_output": str(leaf_output),
        "splits": {},
        "matched": True,
        "quality_gate_passed": True,
        "errors": [],
    }
    shared_fields = (
        "engine_revision",
        "opening_suite_id",
        "opening_suite_sha256",
        "train_opening_ids",
        "validation_opening_ids",
        "split_policy",
        "split_seed",
        "shuffle_policy",
        "shuffle_seed",
        "candidate_roots_per_game",
        "candidate_identity_version",
    )
    for split in ("train", "validation"):
        root_manifest = json.loads(
            (root_output / "datasets" / f"{split}.json").read_text()
        )
        leaf_manifest = json.loads(
            (leaf_output / "datasets" / f"{split}.json").read_text()
        )
        errors: list[str] = []
        for field in shared_fields:
            if root_manifest.get(field) != leaf_manifest.get(field):
                errors.append(f"{field} differs")
        for field in (
            "candidate_positions",
            "candidate_identity_version",
            "candidate_identity_sha256",
        ):
            if root_manifest.get(field) != leaf_manifest.get(field):
                errors.append(f"{field} differs")
        candidate_positions = root_manifest.get("candidate_positions", 0)
        rates: dict[str, float] = {}
        for lane, manifest in (("root", root_manifest), ("leaf", leaf_manifest)):
            candidates = manifest.get("candidate_positions", 0)
            rejected = manifest.get("rejected_incomplete_label_positions", 0)
            rate = rejected / candidates if candidates else 0.0
            rates[lane] = rate
            if rate > 0.01:
                errors.append(f"{lane} incomplete-label rate is {rate:.4%} (>1%)")
        if candidate_positions == 0:
            errors.append("candidate_positions is zero")
        split_report = {
            "root_candidate_positions": root_manifest.get("candidate_positions"),
            "leaf_candidate_positions": leaf_manifest.get("candidate_positions"),
            "root_candidate_identity_sha256": root_manifest.get(
                "candidate_identity_sha256"
            ),
            "leaf_candidate_identity_sha256": leaf_manifest.get(
                "candidate_identity_sha256"
            ),
            "root_incomplete_label_rejection_rate": rates["root"],
            "leaf_incomplete_label_rejection_rate": rates["leaf"],
            "errors": errors,
        }
        report["splits"][split] = split_report
        report["errors"].extend(f"{split}: {error}" for error in errors)

    report["matched"] = not report["errors"]
    report["quality_gate_passed"] = report["matched"]
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if output:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(rendered)
        print(f"wrote {output}")
    print(rendered, end="")
    return 0 if report["matched"] else 1


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

    matched = subcommands.add_parser(
        "check-matched", help="compare generated root/leaf candidate identities"
    )
    matched.add_argument("--root-output", type=Path, required=True)
    matched.add_argument("--leaf-output", type=Path, required=True)
    matched.add_argument("--output", type=Path)

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
    if args.command == "check-matched":
        return matched_manifest_check(args.root_output, args.leaf_output, args.output)
    if args.command == "calibrate":
        return calibrate(args)
    raise AssertionError(args.command)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, *TOML_ERRORS) as error:
        print(f"phase8 preparation failed: {error}", file=sys.stderr)
        sys.exit(1)
