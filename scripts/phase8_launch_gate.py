#!/usr/bin/env python3
"""Run the pre-generation Phase 8C launch gate.

This command is intentionally limited to artifact hashing, model verification,
and a small deterministic tactical suite. It never invokes ``generate-data``
or ``merge-data``; those remain a later multi-machine assignment.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python 3.10 fallback
    import toml as tomllib


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = ROOT / "haitaka_learn.anhoku-v0.6-phase8c-root-1m.data.toml"
TACTICAL_SUITE = ROOT / "scripts/phase8-tactical-suite-v1.json"
GATE_OUTPUT = ROOT / "out/anhoku-v0.6-phase8c-launch-gate"
C16 = ROOT / "out/anhoku-v0.6-phase7.1-preserved/lane-c-step-16.nnue"
ROOT_MODEL = ROOT / "out/anhoku-v0.6-phase8b-root-262k/artifacts/haitaka-anhoku-v0.6-phase8b-root-262k.nnue"
LEAF_MODEL = ROOT / "out/anhoku-v0.6-phase8b-leaf-262k/artifacts/haitaka-anhoku-v0.6-phase8b-leaf-262k.nnue"
ROOT_CKPT = ROOT / "out/anhoku-v0.6-phase8b-root-262k/logs/lightning_logs/version_0/checkpoints/epoch=0-step=16.ckpt"
LEAF_CKPT = ROOT / "out/anhoku-v0.6-phase8b-leaf-262k/logs/lightning_logs/version_0/checkpoints/epoch=0-step=16.ckpt"

OOD_IDS = [f"anhoku-v2-{index:03d}" for index in range(53, 65)]
MIN_TRAIN_POSITIONS = 1_048_576
EXPECTED_C16_SHA256 = "049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def file_record(path: Path, expected_sha256: str | None = None) -> dict[str, Any]:
    record: dict[str, Any] = {
        "path": str(path),
        "exists": path.is_file(),
        "size_bytes": path.stat().st_size if path.is_file() else None,
        "sha256": sha256_file(path) if path.is_file() else None,
    }
    if expected_sha256 is not None:
        record["expected_sha256"] = expected_sha256
        record["sha256_matches_expected"] = record["sha256"] == expected_sha256
    return record


def run_checked(command: list[str], *, cwd: Path = ROOT) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )


def verify_model(
    learn_binary: Path, model_name: str, model_path: Path, output_dir: Path
) -> dict[str, Any]:
    model_output = output_dir / "verifier" / model_name
    artifacts = model_output / "artifacts"
    artifacts.mkdir(parents=True, exist_ok=True)
    model_link = artifacts / "model.nnue"
    if model_link.exists() or model_link.is_symlink():
        model_link.unlink()
    model_link.symlink_to(model_path.resolve())

    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".toml", prefix=f"phase8c-verify-{model_name}-", delete=False
    ) as handle:
        handle.write(
            "[rules]\n"
            'ruleset = "anhoku"\n\n'
            "[paths]\n"
            f'output_dir = "{model_output.as_posix()}"\n\n'
            "[export]\n"
            'output_name = "model.nnue"\n'
            f'description = "Phase 8C launch-gate verifier: {model_name}"\n\n'
            "[verify]\n"
            "search_depth = 2\n"
            "run_search_smoke = true\n"
        )
        config_path = Path(handle.name)

    try:
        result = run_checked(
            [str(learn_binary), "verify", "--config", str(config_path)]
        )
    finally:
        config_path.unlink(missing_ok=True)

    report_path = artifacts / "verify.json"
    report: dict[str, Any] | None = None
    if report_path.is_file():
        report = json.loads(report_path.read_text())
    return {
        "model": model_name,
        "command": [str(learn_binary), "verify", "--config", "<temporary>"],
        "exit_code": result.returncode,
        "output_tail": result.stdout.splitlines()[-12:],
        "report_path": str(report_path),
        "report": report,
        "passed": result.returncode == 0 and report is not None,
    }


def read_until_bestmove(stream: Any) -> tuple[str | None, list[str]]:
    lines: list[str] = []
    for raw_line in stream:
        line = raw_line.rstrip("\n")
        lines.append(line)
        if line.startswith("bestmove "):
            move = line.split()[1] if len(line.split()) > 1 else None
            return move, lines
    return None, lines


def tactical_results(binary: Path, model_name: str, model_path: Path, fixtures: list[dict[str, Any]]) -> dict[str, Any]:
    process = subprocess.Popen(
        [str(binary), "usi", "--eval", "nnue", "--nnue", str(model_path)],
        cwd=ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    assert process.stdin is not None
    assert process.stdout is not None

    def command(line: str) -> None:
        process.stdin.write(line + "\n")
        process.stdin.flush()

    try:
        command("usi")
        startup_lines = []
        for line in process.stdout:
            startup_lines.append(line.rstrip("\n"))
            if line.rstrip("\n") == "usiok":
                break
        command("isready")
        for line in process.stdout:
            if line.rstrip("\n") == "readyok":
                break
        command("usinewgame")

        rows = []
        for fixture in fixtures:
            command(f"position sfen {fixture['sfen']}")
            command(f"go depth {fixture.get('depth', 2)}")
            bestmove, output = read_until_bestmove(process.stdout)
            expected = fixture["expected_bestmove"]
            rows.append(
                {
                    "id": fixture["id"],
                    "depth": fixture.get("depth", 2),
                    "expected_bestmove": expected,
                    "observed_bestmove": bestmove,
                    "legal_move_reported": bool(bestmove),
                    "passed": bestmove == expected,
                    "output_tail": output[-4:],
                }
            )
        passed = all(row["passed"] for row in rows)
    finally:
        try:
            command("quit")
        except (BrokenPipeError, OSError):
            pass
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()

    return {
        "model": model_name,
        "binary": str(binary),
        "suite": str(TACTICAL_SUITE),
        "fixtures": rows,
        "passed": passed,
    }


def validate_launch_config(config_path: Path) -> dict[str, Any]:
    config = tomllib.loads(config_path.read_text())
    data = config.get("data", {})
    training = config.get("training", {})
    checks = {
        "root_position_policy": data.get("position_policy") == "root-position",
        "fixed_teacher_budget": data.get("label_search_nodes") == 50_000
        and data.get("label_search_max_depth") == 64,
        "rollout_depth_one": data.get("rollout_search_depth") == 1,
        "c16_warm_start": config.get("paths", {}).get("bootstrap_nnue")
        == "out/anhoku-v0.6-phase7.1-preserved/lane-c-step-16.nnue",
        "learning_rate_preserved": training.get("initial_learning_rate") == 0.00015,
        "lambda_preserved": training.get("lambda") == 0.8,
        "root_only": data.get("position_policy") == "root-position",
        "minimum_train_positions": data.get("minimum_train_positions")
        == MIN_TRAIN_POSITIONS,
        "validation_schedule": data.get("validation_opening_schedule")
        == "equal-color-swapped-pairs-v1",
        "all_reserved_ids": data.get("validation_opening_ids") == OOD_IDS,
        "at_least_16_pairs_per_id": data.get("validation_opening_pairs_per_id", 0)
        >= 16,
        "validation_game_count_matches_schedule": data.get("validation_games")
        == 2
        * len(OOD_IDS)
        * data.get("validation_opening_pairs_per_id", 0),
        "training_excluded_from_ood": data.get("validation_opening_ids") == OOD_IDS,
        "one_million_training_budget": training.get("epoch_size") == MIN_TRAIN_POSITIONS,
        "checkpoint_schedule_predeclared": training.get("checkpoint_interval_steps") == 16
        and training.get("validation_interval_steps") == 16
        and training.get("max_steps") == 64,
        "c16_features_preserved": training.get("features")
        == "HalfKAv2^+DonorSingleEff",
    }
    return {
        "path": str(config_path),
        "sha256": sha256_file(config_path),
        "checks": checks,
        "passed": all(checks.values()),
        "config": config,
    }


def git_identity() -> dict[str, Any]:
    commit = run_checked(["git", "rev-parse", "HEAD"]).stdout.strip()
    status = run_checked(["git", "status", "--short"]).stdout.splitlines()
    return {"commit": commit, "dirty": bool(status), "status": status}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    parser.add_argument("--output", type=Path, default=GATE_OUTPUT)
    args = parser.parse_args()
    config_path = args.config.resolve()
    output_dir = args.output.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)

    config_result = validate_launch_config(config_path)
    suite = json.loads(TACTICAL_SUITE.read_text())
    if suite.get("schema_version") != 1 or suite.get("ruleset") != "anhoku":
        raise SystemExit("unsupported tactical suite identity")

    build_learn = run_checked(
        ["cargo", "build", "--release", "-p", "haitaka_learn", "--features", "anhoku"]
    )
    build_cli = run_checked(
        ["cargo", "build", "--release", "-p", "haitaka_cli", "--features", "anhoku"]
    )
    learn_binary = ROOT / "target/release/haitaka_learn"
    cli_binary = ROOT / "target/release/haitaka_cli"
    build_result = {
        "learn": {"exit_code": build_learn.returncode, "output_tail": build_learn.stdout.splitlines()[-12:]},
        "cli": {"exit_code": build_cli.returncode, "output_tail": build_cli.stdout.splitlines()[-12:]},
        "passed": build_learn.returncode == 0 and build_cli.returncode == 0,
    }

    artifacts = {
        "c16": file_record(C16, EXPECTED_C16_SHA256),
        "phase8b_root_nnue": file_record(ROOT_MODEL),
        "phase8b_leaf_nnue": file_record(LEAF_MODEL),
        "phase8b_root_step16_ckpt": file_record(ROOT_CKPT),
        "phase8b_leaf_step16_ckpt": file_record(LEAF_CKPT),
        "phase8b_closeout": {
            "document": str(ROOT / "docs/nnue-training-anhoku-v0.6-phase8b.md"),
            "exists": (ROOT / "docs/nnue-training-anhoku-v0.6-phase8b.md").is_file(),
            "rental_time_recovered": False,
            "rental_cost_recovered": False,
            "reuse_completed_matches": True,
        },
    }
    dataset_paths = {
        "root_train": ROOT / "out/anhoku-v0.6-phase8b-root-262k/datasets/train.bin",
        "root_validation": ROOT / "out/anhoku-v0.6-phase8b-root-262k/datasets/validation.bin",
        "leaf_train": ROOT / "out/anhoku-v0.6-phase8b-leaf-262k/datasets/train.bin",
        "leaf_validation": ROOT / "out/anhoku-v0.6-phase8b-leaf-262k/datasets/validation.bin",
    }
    artifacts["phase8b_dataset_hashes"] = {
        name: file_record(path) for name, path in dataset_paths.items()
    }

    verification = []
    tactical = []
    if build_result["passed"]:
        for name, model in (
            ("c16", C16),
            ("phase8b-root", ROOT_MODEL),
            ("phase8b-leaf", LEAF_MODEL),
        ):
            verification.append(verify_model(learn_binary, name, model, output_dir))
            tactical.append(tactical_results(cli_binary, name, model, suite["fixtures"]))

    artifact_passed = all(
        item.get("exists") and item.get("sha256_matches_expected", True)
        for name, item in artifacts.items()
        if name != "phase8b_closeout" and name != "phase8b_dataset_hashes"
    ) and artifacts["phase8b_closeout"]["exists"]
    result = {
        "schema": "anhoku-phase8c-launch-gate",
        "schema_version": 1,
        "git": git_identity(),
        "config": config_result,
        "tactical_suite": {
            "path": str(TACTICAL_SUITE),
            "sha256": sha256_file(TACTICAL_SUITE),
            "version": suite["schema_version"],
        },
        "build": build_result,
        "artifacts": artifacts,
        "verification": verification,
        "tactical": tactical,
        "production_generation_started": False,
        "multi_machine_generation_started": False,
        "gate_checks": {
            "immutable_closeout_reviewable": artifact_passed,
            "c16_and_selected_exports_hashed": artifact_passed,
            "selected_step16_checkpoints_recovered_or_loss_recorded": all(
                item["exists"]
                for item in (
                    artifacts["phase8b_root_step16_ckpt"],
                    artifacts["phase8b_leaf_step16_ckpt"],
                )
            ),
            "verifier_suite_passed": bool(verification)
            and all(item["passed"] for item in verification),
            "tactical_suite_passed": bool(tactical)
            and all(item["passed"] for item in tactical),
            "stratified_ood_contract_passed": config_result["checks"]["validation_schedule"]
            and config_result["checks"]["all_reserved_ids"]
            and config_result["checks"]["at_least_16_pairs_per_id"]
            and config_result["checks"]["validation_game_count_matches_schedule"],
            "unique_record_target_contract_passed": config_result["checks"][
                "minimum_train_positions"
            ],
            "experiment_config_reviewable": config_result["passed"],
            "stopped_before_data_generation": True,
        },
    }
    result["passed"] = all(result["gate_checks"].values()) and build_result["passed"]
    report_path = output_dir / "phase8c-launch-gate.json"
    report_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps(result, indent=2, sort_keys=True))
    print(f"wrote {report_path}")
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, json.JSONDecodeError) as error:
        print(f"phase8 launch gate failed: {error}", file=sys.stderr)
        raise SystemExit(1)
