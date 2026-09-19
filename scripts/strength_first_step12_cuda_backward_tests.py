#!/usr/bin/env python3
"""Reference, contract and authorized CUDA tests for the Step 12 adapter."""

from __future__ import annotations

import argparse
import ast
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import sys
from typing import Any

import numpy as np

from strength_first_step12_cuda_backward_reference import REFERENCE_SCHEMA, case_manifest, reference_backward, reference_receipt
from strength_first_step12_trainer import validate_cuda_grant_document


ROOT = Path(__file__).resolve().parents[1]
EXPECTED_FEATURE_TRANSFORMER_SHA256 = "a0048dcdb9cb4582e01bc1be4a7e1106700789a2d0daf8d2516bc22c6ec43ba1"
TEST_SCHEMA = "haitaka-anhoku-strength-first-step12-deterministic-cuda-backward-tests-v1"


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def artifact(path: Path) -> dict[str, Any]:
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256(path)}


def write(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run_reference(output: Path) -> dict[str, Any]:
    receipt = reference_receipt()
    receipt["createdAt"] = now()
    receipt["manifestCount"] = len(case_manifest())
    write(output, receipt)
    return receipt


def run_contracts(config: Path, output: Path) -> dict[str, Any]:
    config_value = json.loads(config.read_text(encoding="utf-8"))
    trainer = config_value.get("trainer", {})
    required = ("cudaAdapter", "cudaKernel", "cudaReference", "cudaTests", "cudaRuntime")
    checks = []
    for name in required:
        item = trainer.get(name)
        path = Path(item.get("path", "")) if isinstance(item, dict) else Path("")
        checks.append({"name": name, "status": "passed" if path.is_file() and not path.is_symlink() else "failed", "artifact": artifact(path) if path.is_file() else None})
    external = Path(trainer.get("pinnedCheckout", {}).get("path", "")) / "feature_transformer.py"
    checks.append({"name": "immutable-pinned-feature-transformer", "status": "passed" if external.is_file() and sha256(external) == EXPECTED_FEATURE_TRANSFORMER_SHA256 else "failed", "artifact": artifact(external) if external.is_file() else None})
    checks.append({"name": "profile", "status": "passed" if config_value.get("recipe", {}).get("cudaDeterminism", {}).get("profile") == "ordered-segment-v1" else "failed"})
    adapter_path = Path(trainer.get("cudaAdapter", {}).get("path", ""))
    adapter_source = adapter_path.read_text(encoding="utf-8") if adapter_path.is_file() else ""
    loader_contract = {
        "directCompile": "compile_using_nvrtc" in adapter_source,
        "moduleLoad": "cp.cuda.function.Module()" in adapter_source and "module.load(compiled)" in adapter_source,
        "rawModuleBypass": "cp.RawModule" not in adapter_source,
        "explicitFtzFalse": "--ftz=false" in adapter_source,
    }
    checks.append({"name": "direct-nvrtc-loader-contract", "status": "passed" if all(loader_contract.values()) else "failed", **loader_contract})
    kernel_path = Path(trainer.get("cudaKernel", {}).get("path", ""))
    kernel_source = kernel_path.read_text(encoding="utf-8") if kernel_path.is_file() else ""
    kernel_type_contract = {
        "scopedInt32": "typedef int step12_int32;" in kernel_source,
        "scopedInt64": "typedef long long step12_int64;" in kernel_source,
        "widthAssertions": "static_assert(sizeof(step12_int32) == 4" in kernel_source and "static_assert(sizeof(step12_int64) == 8" in kernel_source,
        "explicitMaxKey": "9223372036854775807LL" in kernel_source,
        "noHostStdint": "#include <stdint.h>" not in kernel_source and "INT64_MAX" not in kernel_source,
    }
    checks.append({"name": "device-integer-abi-contract", "status": "passed" if kernel_path.is_file() and all(kernel_type_contract.values()) else "failed", "artifact": artifact(kernel_path) if kernel_path.is_file() else None, **kernel_type_contract})
    fixture_path = Path(trainer.get("fixture", {}).get("path", ""))
    fixture_source = fixture_path.read_text(encoding="utf-8") if fixture_path.is_file() else ""
    probe_start = fixture_source.find("rng_probe = _cuda_rng_probe(torch, device, fixture_seed)")
    seed_start = fixture_source.find("core.seed_everything(torch, fixture_seed)")
    seeded_probe_contract = {
        "fixturePresent": fixture_path.is_file(),
        "seededProbeSignature": "def _cuda_rng_probe(torch: Any, device: Any, seed: int)" in fixture_source,
        "seededBeforeProbe": seed_start >= 0 and probe_start > seed_start,
        "profile": '"seeded-cuda-save-restore-v2"' in fixture_source,
        "restoreReceipt": '"stateAfterRestore"' in fixture_source and '"stateRestoredExact"' in fixture_source,
        "finallyRestores": "finally:" in fixture_source and "torch.cuda.set_rng_state_all(before)" in fixture_source,
    }
    checks.append({"name": "seeded-cuda-rng-probe-contract", "status": "passed" if all(seeded_probe_contract.values()) else "failed", "artifact": artifact(fixture_path) if fixture_path.is_file() else None, **seeded_probe_contract})
    kernel_calls = []
    try:
        tree = ast.parse(adapter_source)
        for node in ast.walk(tree):
            if isinstance(node, ast.Call) and isinstance(node.func, ast.Subscript) and isinstance(node.func.value, ast.Name) and node.func.value.id == "kernels":
                kernel_calls.append({"grid": any(keyword.arg == "grid" for keyword in node.keywords), "block": any(keyword.arg == "block" for keyword in node.keywords), "args": any(keyword.arg == "args" for keyword in node.keywords)})
    except SyntaxError:
        kernel_calls = []
    checks.append({"name": "mock-raw-kernel-call-signature", "status": "passed" if len(kernel_calls) == 5 and all(item["grid"] and item["block"] and item["args"] for item in kernel_calls) else "failed", "calls": kernel_calls, "cudaImported": False})
    resolution_status = "failed"
    resolution_detail: dict[str, Any] = {"cudaImported": False}
    resolution_temp = None
    try:
        sys.path.insert(0, str(ROOT / "scripts"))
        import r5_trainer  # type: ignore
        from strength_first_step12_context import prepare_context
        from strength_first_step12_cuda_runtime import resolve_feature_transformer_module
        from strength_first_trainer import install_cpu_fixture_cupy_module
        probe_value = config_value
        resolution_temp, resolution_context = prepare_context(repo_root=ROOT, root=config.parent.parent, external=Path(probe_value["trainer"]["pinnedCheckout"]["path"]), patch_binding=probe_value["trainer"]["patchExecutable"], width=probe_value["recipe"]["width"], r5=r5_trainer, cmake=probe_value.get("environment", {}).get("cmakeExecutable", "cmake"), purpose="contracts-module-resolution")
        install_cpu_fixture_cupy_module(resolution_context)
        _probe_torch, _probe_features, model_module, _probe_dataset = r5_trainer.import_trainer(resolution_context)
        resolved = resolve_feature_transformer_module(model_module)
        resolution_status = "passed" if resolved.__name__ == "feature_transformer" and hasattr(resolved, "DoubleFeatureTransformerSliceFunction") else "failed"
        resolution_detail = {"cudaImported": False, "resolvedModule": resolved.__name__}
    except Exception as exc:
        resolution_detail = {"cudaImported": False, "error": str(exc)}
    finally:
        if resolution_temp is not None:
            resolution_temp.cleanup()
    checks.append({"name": "shared-module-resolution", "status": resolution_status, **resolution_detail})
    receipt = {"schema": TEST_SCHEMA + "-contracts", "createdAt": now(), "status": "passed" if all(item["status"] == "passed" for item in checks) else "failed", "checks": checks, "cudaImported": False, "productionStarted": False}
    write(output, receipt)
    return receipt


def _grant(config: Path, authorization_ledger: Path) -> dict[str, Any]:
    root = config.resolve().parent.parent
    grant = json.loads(authorization_ledger.resolve().read_text(encoding="utf-8"))
    if grant.get("schema") != "haitaka-anhoku-strength-first-step12-authorization-v1" or grant.get("experimentId") != "strength-first-step12-independent-replication-20260916-v1":
        raise RuntimeError("CUDA tests require the Step 12 GPU authorization schema and experiment")
    validate_cuda_grant_document(grant, observed_instance=os.environ.get("STEP12_VAST_INSTANCE_ID"))
    resource = grant["resource"]
    binding = grant.get("fixtureBinding", {})
    for role, relative in (("config", "training/config.json"), ("inputs", "training/inputs.json"), ("package", "training/package.json")):
        expected = binding.get(role, {})
        target = root / relative
        if expected.get("bytes") != target.stat().st_size or expected.get("sha256") != sha256(target):
            raise RuntimeError(f"CUDA tests grant is not bound to current {role}")
    return grant


def run_cuda(config: Path, authorization_ledger: Path, output: Path) -> dict[str, Any]:
    grant = _grant(config, authorization_ledger)
    os.environ.setdefault("CUBLAS_WORKSPACE_CONFIG", ":4096:8")
    sys.path.insert(0, str(ROOT / "scripts"))
    import torch  # type: ignore
    import r5_trainer  # type: ignore
    from strength_first_step12_context import prepare_context
    from strength_first_step12_cuda_runtime import install, resolve_feature_transformer_module

    value = json.loads(config.read_text(encoding="utf-8"))
    external = Path(value["trainer"]["pinnedCheckout"]["path"])
    temp, context = prepare_context(repo_root=ROOT, root=config.resolve().parent.parent, external=external, patch_binding=value["trainer"]["patchExecutable"], width=value["recipe"]["width"], r5=r5_trainer, cmake=value.get("environment", {}).get("cmakeExecutable", "cmake"), purpose=f"cuda-gradient-tests-{output.stem}-{os.getpid()}")
    try:
        torch, features, model_module, _dataset = r5_trainer.import_trainer(context)
        if not torch.cuda.is_available():
            raise RuntimeError("CUDA is unavailable")
        runtime = install(model_module, features, torch, value["recipe"]["featureFamily"], binding={"adapter": value["trainer"]["cudaAdapter"], "kernel": value["trainer"]["cudaKernel"], "profile": value["recipe"].get("cudaDeterminism", {}).get("profile")})
        transformer_module = resolve_feature_transformer_module(model_module)
        device = torch.device("cuda:0")
        production_feature_count = int(runtime["featureCount"])
        rows: list[dict[str, Any]] = []
        for case in case_manifest():
            expected_weight, expected_bias = reference_backward(feature_count=production_feature_count, output_size=case["outputSize"], perspectives=zip(case["indices"], case["values"], case["gradients"]))
            repeated_weight_bits: list[str] = []
            repeated_bias_bits: list[str] = []
            for repeat in range(5):
                indices = [torch.tensor(item, dtype=torch.int32, device=device) for item in case["indices"]]
                values = [torch.tensor(item, dtype=torch.float32, device=device) for item in case["values"]]
                gradients = [torch.tensor(item, dtype=torch.float32, device=device) for item in case["gradients"]]
                weight = torch.zeros(production_feature_count, case["outputSize"], dtype=torch.float32, device=device, requires_grad=True)
                bias = torch.zeros(case["outputSize"], dtype=torch.float32, device=device, requires_grad=True)
                if len(indices) == 1:
                    output_value = transformer_module.FeatureTransformerSliceFunction.apply(indices[0], values[0], weight, bias)
                    (output_value * gradients[0]).sum().backward()
                else:
                    output0, output1 = transformer_module.DoubleFeatureTransformerSliceFunction.apply(indices[0], values[0], indices[1], values[1], weight, bias)
                    ((output0 * gradients[0]).sum() + (output1 * gradients[1]).sum()).backward()
                observed_weight = weight.grad.detach().cpu().numpy().astype(np.float32)
                observed_bias = bias.grad.detach().cpu().numpy().astype(np.float32)
                exact = bool(observed_weight.astype("<f4").tobytes() == expected_weight.astype("<f4").tobytes() and observed_bias.astype("<f4").tobytes() == expected_bias.astype("<f4").tobytes())
                if not exact:
                    write(output, {"schema": TEST_SCHEMA + "-cuda", "createdAt": now(), "status": "failed", "cases": rows, "mismatch": {"name": case["name"], "repeat": repeat, "observedWeightBits": observed_weight.astype("<f4").tobytes().hex(), "expectedWeightBits": expected_weight.astype("<f4").tobytes().hex(), "observedBiasBits": observed_bias.astype("<f4").tobytes().hex(), "expectedBiasBits": expected_bias.astype("<f4").tobytes().hex()}, "productionStarted": False})
                    raise RuntimeError(f"CUDA gradient mismatch: {case['name']} repeat {repeat}")
                repeated_weight_bits.append(observed_weight.astype("<f4").tobytes().hex())
                repeated_bias_bits.append(observed_bias.astype("<f4").tobytes().hex())
            rows.append({"name": case["name"], "status": "passed", "repetitions": 5, "weightBits": repeated_weight_bits[0], "expectedWeightBits": expected_weight.astype("<f4").tobytes().hex(), "biasBits": repeated_bias_bits[0], "expectedBiasBits": expected_bias.astype("<f4").tobytes().hex(), "repeatedWeightIdentical": len(set(repeated_weight_bits)) == 1, "repeatedBiasIdentical": len(set(repeated_bias_bits)) == 1})
        invalid_indices = torch.tensor([[-2, -1, -1]], dtype=torch.int32, device=device)
        invalid_values = torch.ones((1, 3), dtype=torch.float32, device=device)
        invalid_weight = torch.zeros(production_feature_count, 2, dtype=torch.float32, device=device)
        invalid_bias = torch.zeros(2, dtype=torch.float32, device=device)
        invalid_grad = torch.ones((1, 2), dtype=torch.float32, device=device)
        invalid_rejected = False
        try:
            transformer_module.make_feature_transformer_slice_backward_kernel(3, 2)((1,), (invalid_indices.data_ptr(), invalid_values.data_ptr(), invalid_weight.data_ptr(), invalid_bias.data_ptr(), invalid_grad.data_ptr()))
        except RuntimeError:
            invalid_rejected = True
        if not invalid_rejected:
            raise RuntimeError("invalid active feature index was not rejected")

        stream_case = case_manifest()[1]
        default_stream = torch.cuda.current_stream(device)
        stream = torch.cuda.Stream(device=device)
        stream_indices = torch.tensor(stream_case["indices"][0], dtype=torch.int32, device=device)
        stream_values = torch.tensor(stream_case["values"][0], dtype=torch.float32, device=device)
        stream_grad = torch.tensor(stream_case["gradients"][0], dtype=torch.float32, device=device)
        stream_weight = torch.zeros(production_feature_count, stream_case["outputSize"], dtype=torch.float32, device=device, requires_grad=True)
        stream_bias = torch.zeros(stream_case["outputSize"], dtype=torch.float32, device=device, requires_grad=True)
        stream.wait_stream(default_stream)
        import cupy as cp
        with torch.cuda.stream(stream):
            with cp.cuda.ExternalStream(int(stream.cuda_stream), device_id=int(device.index or 0)):
                stream_output = transformer_module.FeatureTransformerSliceFunction.apply(stream_indices, stream_values, stream_weight, stream_bias)
                (stream_output * stream_grad).sum().backward()
        stream.synchronize()
        stream_expected_weight, stream_expected_bias = reference_backward(feature_count=production_feature_count, output_size=stream_case["outputSize"], perspectives=zip(stream_case["indices"], stream_case["values"], stream_case["gradients"]))
        stream_exact = bool(stream_weight.grad.detach().cpu().numpy().astype("<f4").tobytes() == stream_expected_weight.astype("<f4").tobytes() and stream_bias.grad.detach().cpu().numpy().astype("<f4").tobytes() == stream_expected_bias.astype("<f4").tobytes())
        if not stream_exact:
            raise RuntimeError("nondefault CUDA stream gradient did not match the independent reference")
        receipt = {"schema": TEST_SCHEMA + "-cuda", "createdAt": now(), "status": "passed", "grant": grant.get("fixtureBinding"), "runtime": runtime, "cases": rows, "invalidIndexRejected": invalid_rejected, "nondefaultStream": {"status": "passed", "exact": stream_exact}, "productionStarted": False, "featureCountBoundToProduction": production_feature_count}
        write(output, receipt)
        return receipt
    finally:
        temp.cleanup()


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    for name in ("reference", "contracts", "cuda"):
        subparser = subparsers.add_parser(name)
        subparser.add_argument("--config", type=Path, required=True)
        subparser.add_argument("--output", type=Path, required=True)
        if name == "cuda":
            subparser.add_argument("--authorization-ledger", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == "reference":
            run_reference(args.output)
        elif args.command == "contracts":
            result = run_contracts(args.config, args.output)
            if result["status"] != "passed":
                return 1
        elif args.command == "cuda":
            run_cuda(args.config, args.authorization_ledger, args.output)
        else:
            raise RuntimeError("choose reference, contracts or cuda")
    except Exception as exc:
        if args.command == "cuda" and args.output.is_file():
            evidence = json.loads(args.output.read_text(encoding="utf-8"))
            failure_path = args.output.with_name(args.output.stem + "-failure.json")
            write(failure_path, {"schema": TEST_SCHEMA + "-failure-envelope", "createdAt": now(), "status": "failed", "error": str(exc), "preservedEvidence": artifact(args.output), "evidenceStatus": evidence.get("status"), "mismatch": evidence.get("mismatch"), "cases": evidence.get("cases"), "productionStarted": False})
        else:
            write(args.output, {"schema": TEST_SCHEMA, "createdAt": now(), "status": "failed", "error": str(exc)})
        print(str(exc), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
