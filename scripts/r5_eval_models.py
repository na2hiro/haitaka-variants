#!/usr/bin/env python3
"""Evaluate R5 checkpoints and the C/16 serialized audit baseline.

The result is an offline teacher-fidelity report.  A serialized ``.nnue`` is
also imported into the pinned Python model solely to make the target metrics
comparable; runtime integer and Rust parity remain separate gates.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(Path(__file__).resolve().parent))
import r5_trainer as runner  # type: ignore  # noqa: E402


def model_arg(value: str) -> tuple[str, Path]:
    label, separator, path = value.partition("=")
    if not separator or not label or not path:
        raise argparse.ArgumentTypeError("model must be LABEL=PATH")
    return label, Path(path)


def verified_artifact(workspace: Path, record: dict[str, Any], label: str) -> dict[str, Any]:
    """Resolve and verify one artifact record from frozen evidence."""
    try:
        relative = record["path"]
        expected_bytes = int(record["bytes"])
        expected_hash = record["sha256"]
    except (KeyError, TypeError, ValueError) as exc:
        runner.fail(f"{label} has an invalid artifact record: {exc}")
    path = workspace / relative
    if not path.is_file():
        runner.fail(f"{label} is missing: {path}")
    actual_bytes = path.stat().st_size
    actual_hash = runner.sha256(path)
    if actual_bytes != expected_bytes or actual_hash != expected_hash:
        runner.fail(
            f"{label} identity mismatch: expected {expected_bytes} bytes/{expected_hash}, "
            f"got {actual_bytes} bytes/{actual_hash}"
        )
    return {"path": relative, "bytes": actual_bytes, "sha256": actual_hash}


def verified_deep_core(
    contract: dict[str, Any], workspace: Path
) -> tuple[dict[str, Any], dict[str, Any]]:
    """Bind deep-core to the immutable, hash-bound replacement-R4 gate."""
    gate_record = contract["authorization"]["replacementR4Gate"]
    gate_artifact = verified_artifact(workspace, gate_record, "replacement R4 gate")
    gate = runner.load_json(workspace / gate_record["path"])
    try:
        deep = gate["datasets"]["deep-core"]
        manifest_relative = contract["metrics"]["deepTeacher"]["dataset"]
    except (KeyError, TypeError) as exc:
        runner.fail(f"deep-core is not declared by the frozen R4/R5 evidence: {exc}")
    manifest_path = workspace / manifest_relative
    manifest = runner.load_json(manifest_path)
    if manifest != deep:
        runner.fail("deep-core manifest does not exactly match the hash-bound R4 gate declaration")
    records = int(deep.get("records", -1))
    entry_bytes = int(deep.get("entryBytes", -1))
    if records <= 0 or entry_bytes <= 0 or entry_bytes * records != int(deep["binary"]["bytes"]):
        runner.fail("deep-core finite binary geometry is invalid")
    binary = verified_artifact(workspace, deep["binary"], "deep-core binary")
    sidecar = verified_artifact(workspace, deep["sidecar"], "deep-core sidecar")
    return deep, {
        "gate": gate_artifact,
        "manifest": {
            "path": manifest_relative,
            "bytes": manifest_path.stat().st_size,
            "sha256": runner.sha256(manifest_path),
        },
        "binary": binary,
        "sidecar": sidecar,
        "records": records,
    }


def verified_evaluation_splits(
    contract: dict[str, Any], deep: dict[str, Any],
) -> dict[str, tuple[Path, int]]:
    pilot = contract["pilot"]["splits"]
    static = contract["initialization"]["lanes"][1]["staticClone"]
    sources = {"validation": pilot["validation"], "development": pilot["development"],
               "static-clone": static, "deep-core": deep}
    return {
        name: (runner.verified_binary(source["binary"], source["records"], name), source["records"])
        for name, source in sources.items()
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preflight-report", type=Path, required=True)
    parser.add_argument("--contract", type=Path, required=True)
    parser.add_argument("--width", choices=("current", "compact"), default="current")
    parser.add_argument("--model", action="append", type=model_arg, default=[])
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cmake", default="cmake")
    args = parser.parse_args()
    if not args.model:
        parser.error("at least one --model LABEL=PATH is required")
    labels = [label for label, _source in args.model]
    if len(labels) != len(set(labels)):
        parser.error("model labels must be unique")

    contract_path = args.contract if args.contract.is_absolute() else ROOT / args.contract
    preflight_path = (
        args.preflight_report if args.preflight_report.is_absolute() else ROOT / args.preflight_report
    )
    output_path = (args.output if args.output.is_absolute() else ROOT / args.output).resolve()
    if output_path.exists():
        runner.fail(f"fresh model evaluation refuses existing output: {output_path}")
    if not output_path.parent.is_dir():
        runner.fail(f"model evaluation output parent is missing: {output_path.parent}")
    contract = runner.verify_preflight(preflight_path, contract_path)
    width = contract["architecture"]["currentWidth" if args.width == "current" else "compactWidth"]
    workspace = ROOT
    deep, deep_identity = verified_deep_core(contract, workspace)
    split_files = verified_evaluation_splits(contract, deep)
    original_cwd = Path.cwd()
    temp, context = runner.prepare_context(width, args.cmake)
    try:
        torch, features, model_module, nnue_dataset = runner.import_trainer(context)
        device = torch.device("cuda")
        feature_set = features.get_feature_set_from_name(runner.FEATURE_SET)
        results: list[dict[str, Any]] = []
        for label, source in args.model:
            source = source if source.is_absolute() else workspace / source
            if not source.is_file():
                runner.fail(f"model is missing: {source}")
            source_format = source.suffix.lstrip(".")
            evaluation_source = source
            imported = False
            if source_format == "nnue":
                evaluation_source = context / f"imported-{label}.pt"
                runner.run(
                    [
                        sys.executable,
                        "serialize.py",
                        str(source),
                        str(evaluation_source),
                        "--features",
                        runner.FEATURE_SET,
                    ],
                    context,
                    f"import {label} serialized network",
                )
                imported = True
            elif source_format != "pt":
                runner.fail(f"unsupported model format for {label}: {source}")
            model = torch.load(evaluation_source, map_location="cpu", weights_only=False)
            model.to(device)
            model.eval()
            split_results = {
                split: runner.evaluate(torch, model, nnue_dataset, path, records, device)
                for split, (path, records) in split_files.items()
            }
            results.append({
                "label": label,
                "source": str(source.relative_to(workspace)) if source.is_relative_to(workspace) else str(source),
                "sourceBytes": source.stat().st_size,
                "sourceSha256": runner.sha256(source),
                "sourceFormat": source_format,
                "inferenceKind": "dequantized-python-import" if imported else "full-precision-checkpoint",
                "parameterCount": sum(parameter.numel() for parameter in model.parameters()),
                "splits": split_results,
            })
            del model
            if torch.cuda.is_available():
                torch.cuda.empty_cache()

        runner.seed_everything(torch, contract["initialization"]["selectionSeed"])
        untrained = model_module.NNUE(feature_set=feature_set, lambda_=1.0).to(device)
        untrained.eval()
        results.append({
            "label": "untrained-initialization",
            "source": None,
            "sourceFormat": None,
            "inferenceKind": "fresh-seeded-python-model",
            "seed": contract["initialization"]["selectionSeed"],
            "parameterCount": sum(parameter.numel() for parameter in untrained.parameters()),
            "splits": {
                split: runner.evaluate(torch, untrained, nnue_dataset, path, records, device)
                for split, (path, records) in split_files.items()
            },
        })

        report = {
            "schema": "haitaka-anhoku-r5-model-evaluation-v1",
            "contract": {"path": str(contract_path), "sha256": runner.sha256(contract_path)},
            "width": width,
            "objective": contract["optimization"]["objective"],
            "inputs": {"deepCore": deep_identity},
            "models": results,
            "runtimeIntegerAndRustParity": {
                "status": "pending-separate-runtime-gate",
                "notInferredFromPythonImport": True,
            },
            "deepTeacherMoveRegret": {
                "status": "unavailable-from-current-deep-core-artifact",
                "reason": "deep-core sidecar contains rootMove and root score but no per-legal-move deep-teacher values",
            },
        }
        temporary_output = output_path.with_name(f".{output_path.name}.{os.getpid()}.tmp")
        temporary_output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        os.replace(temporary_output, output_path)
        print(f"R5 model evaluation written to {output_path}")
    finally:
        os.chdir(original_cwd)
        temp.cleanup()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except runner.R5Error as exc:
        print(f"R5 model evaluation FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
