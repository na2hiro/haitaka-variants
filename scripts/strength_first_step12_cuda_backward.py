#!/usr/bin/env python3
"""Deterministic CUDA backward adapter for the Step 12 feature transformer.

The pinned transformer keeps its CUDA forward and autograd classes.  This
module replaces only the unordered backward-kernel factory with ordered
segment reductions.  It is intentionally lazy: importing it on the CPU path
does not import CuPy or initialize a CUDA device.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any


ADAPTER_SCHEMA = "haitaka-anhoku-strength-first-step12-deterministic-cuda-backward-v1"
PROFILE = "ordered-segment-v1"
EXPECTED_FEATURE_TRANSFORMER_SHA256 = "a0048dcdb9cb4582e01bc1be4a7e1106700789a2d0daf8d2516bc22c6ec43ba1"
KERNEL_NAME = "step12-deterministic-cuda-backward"
COMPILE_OPTIONS = ("--std=c++14", "--fmad=false", "--ftz=false")
THREADS_PER_BLOCK = 128
MAX_INT64 = (1 << 63) - 1

_installations: dict[int, dict[str, Any]] = {}


class DeterministicCudaBackwardError(RuntimeError):
    pass


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _artifact(path: Path) -> dict[str, Any]:
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": _sha256(path)}


def _kernel_path() -> Path:
    return Path(__file__).with_name("strength_first_step12_cuda_backward.cu")


def _load_kernels(cp: Any) -> dict[str, Any]:
    source_path = _kernel_path()
    if not source_path.is_file() or source_path.is_symlink():
        raise DeterministicCudaBackwardError(f"kernel source is missing or symlinked: {source_path}")
    names = (
        "step12_row_metadata",
        "step12_build_keys",
        "step12_segments",
        "step12_reduce_weights",
        "step12_reduce_bias",
    )
    compiler = cp.cuda.compiler
    architecture = compiler._get_arch()
    architecture_option, compiled_format = compiler._get_arch_for_options_for_nvrtc(architecture)
    effective_options = list(COMPILE_OPTIONS) + [architecture_option]
    if compiler._get_nvrtc_version()[0] >= 12:
        effective_options.append("--device-as-default-execution-space")
    compiled, name_mapping = compiler.compile_using_nvrtc(
        source_path.read_text(encoding="utf-8"),
        options=COMPILE_OPTIONS,
        filename=source_path.name,
        name_expressions=names,
    )
    module = cp.cuda.function.Module()
    module.load(compiled)
    kernels = {name: module.get_function((name_mapping or {}).get(name, name)) for name in names}
    # Keep the owning module alive for the lifetime of all returned functions.
    kernels["__module__"] = module
    kernels["__compilation__"] = {
        "backend": "nvrtc",
        "loader": "cupy.cuda.compiler.compile_using_nvrtc-plus-cupy.cuda.function.Module.load-v1",
        "compiledFormat": compiled_format,
        "compiledBytes": len(compiled),
        "requestedOptions": list(COMPILE_OPTIONS),
        "effectiveOptions": effective_options,
        "architecture": architecture,
        "nameExpressions": list(names),
        "nameMapping": dict(name_mapping or {}),
        "ftzOptionCount": sum(1 for option in effective_options if "ftz" in option.lower()),
    }
    return kernels


def _ptr_array(cp: Any, pointer: int, shape: tuple[int, ...], dtype: Any, owner: Any) -> Any:
    if pointer <= 0:
        raise DeterministicCudaBackwardError("pinned CUDA kernel received a null device pointer")
    itemsize = cp.dtype(dtype).itemsize
    elements = 1
    for dimension in shape:
        if dimension < 0:
            raise DeterministicCudaBackwardError("negative CUDA tensor dimension")
        elements *= dimension
    if elements > MAX_INT64 // max(1, itemsize):
        raise DeterministicCudaBackwardError("CUDA tensor byte-size arithmetic exceeds signed int64")
    nbytes = elements * itemsize
    memory = cp.cuda.UnownedMemory(pointer, nbytes, owner)
    return cp.ndarray(shape=shape, dtype=dtype, memptr=cp.cuda.MemoryPointer(memory, 0))


def _batch_from_grid(grid: Any) -> int:
    if isinstance(grid, int):
        batch = grid
    elif isinstance(grid, (tuple, list)) and len(grid) == 1:
        batch = int(grid[0])
    else:
        raise DeterministicCudaBackwardError(f"expected one-dimensional batch grid, got {grid!r}")
    if batch <= 0:
        raise DeterministicCudaBackwardError(f"invalid CUDA batch size: {batch}")
    return batch


def _make_factory(*, cp: Any, np: Any, torch: Any, feature_count: int, kernels: dict[str, Any]):
    if feature_count <= 0:
        raise DeterministicCudaBackwardError("feature_count must be positive")

    def factory(max_active_features: int, output_size: int):
        max_active_features = int(max_active_features)
        output_size = int(output_size)
        if max_active_features <= 0 or output_size <= 0:
            raise DeterministicCudaBackwardError("invalid feature-transformer dimensions")
        stride_limit = MAX_INT64 // max(1, max_active_features)

        def launch(grid: Any, args: tuple[Any, ...]):
            if len(args) != 5:
                raise DeterministicCudaBackwardError(f"expected five CUDA pointers, got {len(args)}")
            batch_size = _batch_from_grid(grid)
            total_records = batch_size * max_active_features
            stride = total_records + 1
            max_key = (feature_count - 1) * stride + (total_records - 1)
            if stride > MAX_INT64 or max_key >= MAX_INT64 or feature_count > stride_limit or total_records > (1 << 31) - 1 or output_size > (1 << 31) - 1:
                raise DeterministicCudaBackwardError("CUDA segment key arithmetic exceeds signed int64")
            feature_indices_ptr, feature_values_ptr, weight_grad_ptr, bias_grad_ptr, output_grad_ptr = (int(value) for value in args)
            owner = object()
            indices = _ptr_array(cp, feature_indices_ptr, (batch_size, max_active_features), cp.int32, owner)
            values = _ptr_array(cp, feature_values_ptr, (batch_size, max_active_features), cp.float32, owner)
            weight_grad = _ptr_array(cp, weight_grad_ptr, (feature_count, output_size), cp.float32, owner)
            bias_grad = _ptr_array(cp, bias_grad_ptr, (output_size,), cp.float32, owner)
            output_grad = _ptr_array(cp, output_grad_ptr, (batch_size, output_size), cp.float32, owner)
            device_index = int(torch.cuda.current_device())
            cp.cuda.Device(device_index).use()
            external_stream = cp.cuda.ExternalStream(int(torch.cuda.current_stream(device_index).cuda_stream), device_id=device_index)
            try:
                with external_stream:
                    active_counts = cp.empty((batch_size,), dtype=cp.int32)
                    invalid = cp.zeros((1,), dtype=cp.int32)
                    kernels["step12_row_metadata"](
                        grid=((batch_size + THREADS_PER_BLOCK - 1) // THREADS_PER_BLOCK,),
                        block=(THREADS_PER_BLOCK, 1, 1),
                        args=(indices, values, active_counts, invalid, np.int32(batch_size), np.int32(max_active_features), np.int32(feature_count)),
                    )
                    external_stream.synchronize()
                    if int(invalid.get()[0]) != 0:
                        raise DeterministicCudaBackwardError("active CUDA feature index or value is invalid")
                    if not bool(cp.isfinite(output_grad).all().get()):
                        raise DeterministicCudaBackwardError("CUDA upstream gradient contains a nonfinite value")
                    keys = cp.empty((total_records,), dtype=cp.int64)
                    kernels["step12_build_keys"](
                        grid=((total_records + THREADS_PER_BLOCK - 1) // THREADS_PER_BLOCK,),
                        block=(THREADS_PER_BLOCK, 1, 1),
                        args=(indices, active_counts, keys, np.int32(batch_size), np.int32(max_active_features), np.int64(stride)),
                    )
                    order = cp.argsort(keys)
                    sorted_keys = keys[order]
                    segment_start = cp.full((feature_count,), -1, dtype=cp.int32)
                    segment_end = cp.full((feature_count,), -1, dtype=cp.int32)
                    kernels["step12_segments"](
                        grid=((total_records + THREADS_PER_BLOCK - 1) // THREADS_PER_BLOCK,),
                        block=(THREADS_PER_BLOCK, 1, 1),
                        args=(sorted_keys, segment_start, segment_end, np.int32(total_records), np.int32(feature_count), np.int64(stride)),
                    )
                    kernels["step12_reduce_weights"](
                        grid=(feature_count, (output_size + THREADS_PER_BLOCK - 1) // THREADS_PER_BLOCK),
                        block=(THREADS_PER_BLOCK, 1, 1),
                        args=(sorted_keys, segment_start, segment_end, values, output_grad, weight_grad, np.int32(feature_count), np.int32(output_size), np.int32(max_active_features), np.int64(stride)),
                    )
                    kernels["step12_reduce_bias"](
                        grid=((output_size + THREADS_PER_BLOCK - 1) // THREADS_PER_BLOCK,),
                        block=(THREADS_PER_BLOCK, 1, 1),
                        args=(output_grad, bias_grad, np.int32(batch_size), np.int32(output_size)),
                    )
                    external_stream.synchronize()
            finally:
                external_stream.synchronize()

        return launch

    return factory


def install(module: Any, torch: Any, *, feature_count: int, binding: dict[str, Any] | None = None) -> dict[str, Any]:
    """Install the adapter into one disposable pinned feature module."""
    module_path = Path(getattr(module, "__file__", "")).resolve()
    if not module_path.is_file() or _sha256(module_path) != EXPECTED_FEATURE_TRANSFORMER_SHA256:
        raise DeterministicCudaBackwardError("pinned feature_transformer.py source identity differs")
    if not hasattr(module, "make_feature_transformer_slice_backward_kernel"):
        raise DeterministicCudaBackwardError("pinned backward-kernel factory is missing")
    if not hasattr(module, "FeatureTransformerSliceFunction") or not hasattr(module, "DoubleFeatureTransformerSliceFunction"):
        raise DeterministicCudaBackwardError("pinned feature-transformer autograd classes are missing")
    if not torch.cuda.is_available():
        raise DeterministicCudaBackwardError("deterministic CUDA adapter requires a CUDA device")
    if int(feature_count) <= 0:
        raise DeterministicCudaBackwardError("feature_count must be positive")

    module_id = id(module)
    existing = _installations.get(module_id)
    source_artifact = _artifact(module_path)
    adapter_artifact = _artifact(Path(__file__).resolve())
    kernel_artifact = _artifact(_kernel_path())
    binding_value = {
        "schema": ADAPTER_SCHEMA,
        "profile": PROFILE,
        "featureCount": int(feature_count),
        "module": source_artifact,
        "adapter": adapter_artifact,
        "kernel": kernel_artifact,
        "compileOptions": list(COMPILE_OPTIONS),
        "compilerLoader": "cupy.cuda.compiler.compile_using_nvrtc-plus-cupy.cuda.function.Module.load-v1",
        "threadsPerBlock": THREADS_PER_BLOCK,
        "sort": {"implementation": "cupy.ndarray.argsort", "keys": "unique-int64-feature-then-ordinal"},
    }
    if not isinstance(binding, dict):
        raise DeterministicCudaBackwardError("deterministic CUDA adapter requires an approved binding")
    requested_adapter = binding.get("adapter")
    requested_kernel = binding.get("kernel")
    if not isinstance(requested_adapter, dict) or requested_adapter.get("bytes") != adapter_artifact["bytes"] or requested_adapter.get("sha256") != adapter_artifact["sha256"]:
        raise DeterministicCudaBackwardError("approved adapter binding does not match the installed adapter")
    if not isinstance(requested_kernel, dict) or requested_kernel.get("bytes") != kernel_artifact["bytes"] or requested_kernel.get("sha256") != kernel_artifact["sha256"]:
        raise DeterministicCudaBackwardError("approved kernel binding does not match the installed kernel")
    if binding.get("profile") != PROFILE:
        raise DeterministicCudaBackwardError("approved CUDA profile differs from ordered-segment-v1")
    binding_value["requestedBinding"] = binding
    binding_sha = hashlib.sha256(json.dumps(binding_value, sort_keys=True).encode("utf-8")).hexdigest()
    binding_value["bindingSha256"] = binding_sha
    if existing:
        if existing.get("binding", {}).get("bindingSha256") != binding_sha:
            raise DeterministicCudaBackwardError("conflicting deterministic CUDA adapter installation")
        if id(module.make_feature_transformer_slice_backward_kernel) != existing.get("replacementFactory", {}).get("identity"):
            raise DeterministicCudaBackwardError("installed backward factory was changed after adapter installation")
        return existing

    original_factory = module.make_feature_transformer_slice_backward_kernel
    if getattr(original_factory, "__module__", None) != module.__name__:
        raise DeterministicCudaBackwardError("backward-kernel factory is not owned by the pinned module")
    cp = __import__("cupy")
    np = __import__("numpy")
    kernels = _load_kernels(cp)
    replacement = _make_factory(cp=cp, np=np, torch=torch, feature_count=int(feature_count), kernels=kernels)
    cache = getattr(module, "_feature_transformer_slice_backward_kernel_cache", None)
    if not isinstance(cache, dict):
        raise DeterministicCudaBackwardError("pinned backward-kernel cache is not a dictionary")
    cache.clear()
    module.make_feature_transformer_slice_backward_kernel = replacement
    receipt = {
        "schema": ADAPTER_SCHEMA,
        "status": "installed",
        "profile": PROFILE,
        "binding": binding_value,
        "compilation": kernels["__compilation__"],
        "originalFactory": {"module": original_factory.__module__, "qualname": original_factory.__qualname__, "identity": id(original_factory)},
        "replacementFactory": {"module": replacement.__module__, "qualname": replacement.__qualname__, "identity": id(replacement)},
        "forwardPreserved": True,
        "autogradClassesPreserved": True,
        "cacheCleared": True,
    }
    _installations[module_id] = receipt
    return receipt
