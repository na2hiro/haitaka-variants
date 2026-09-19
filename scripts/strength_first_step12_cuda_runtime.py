#!/usr/bin/env python3
"""Shared authorized CUDA setup for Step 12 fixture and production."""

from __future__ import annotations

import importlib
from typing import Any


RUNTIME_SCHEMA = "haitaka-anhoku-strength-first-step12-deterministic-cuda-runtime-v1"


def resolve_feature_transformer_module(model_module: Any) -> Any:
    layer = getattr(model_module, "DoubleFeatureTransformerSlice", None)
    if layer is None:
        raise RuntimeError("pinned model does not expose the double feature-transformer layer")
    module_name = getattr(layer, "__module__", None)
    if not isinstance(module_name, str) or not module_name:
        raise RuntimeError("pinned feature-transformer function has no module identity")
    module = importlib.import_module(module_name)
    if getattr(module, "DoubleFeatureTransformerSlice", None) is not layer or not hasattr(module, "DoubleFeatureTransformerSliceFunction"):
        raise RuntimeError("resolved feature-transformer module does not own the imported layer and autograd function")
    return module


def install(model_module: Any, features: Any, torch: Any, feature_family: str, *, binding: dict[str, Any] | None = None) -> dict[str, Any]:
    """Install the same hash-bound backward adapter before model construction."""
    if not torch.cuda.is_available():
        raise RuntimeError("shared CUDA runtime requires a CUDA device")
    feature_set = features.get_feature_set_from_name(feature_family)
    if not hasattr(feature_set, "num_features") or int(feature_set.num_features) <= 0:
        raise RuntimeError("bound feature set has no valid feature count")
    transformer_module = resolve_feature_transformer_module(model_module)
    from strength_first_step12_cuda_backward import install as install_backward

    receipt = install_backward(transformer_module, torch, feature_count=int(feature_set.num_features), binding=binding)
    return {
        "schema": RUNTIME_SCHEMA,
        "status": "installed",
        "featureFamily": feature_family,
        "featureCount": int(feature_set.num_features),
        "featureTransformerModule": transformer_module.__name__,
        "adapter": receipt,
    }
