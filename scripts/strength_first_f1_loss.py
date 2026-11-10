#!/usr/bin/env python3
"""F1 torch objective adapter; importing this module does not import torch."""

from __future__ import annotations

from typing import Any

from strength_first_f1_core import OUTPUT_SCALE, S, F1Error


def _finite(torch: Any, name: str, tensor: Any) -> None:
    if not bool(torch.isfinite(tensor).all()):
        raise F1Error(f"nonfinite {name}")


def per_record(torch: Any, raw: Any, source_score: Any, tau: float) -> dict[str, Any]:
    """Return both record losses in the production tensor precision.

    Preserve the qualified Huber operation order. The score is checked before
    clipping so a nonfinite source label cannot be hidden by clamp.
    """
    _finite(torch, "raw model output", raw)
    _finite(torch, "source teacher score", source_score)
    p = raw * OUTPUT_SCALE
    y = source_score.clamp(-S, S)
    e = (p - y) / S
    absolute = e.abs()
    h = torch.where(absolute <= 0.1, 0.5 * e.square(), 0.1 * (absolute - 0.05))
    sp = torch.sigmoid(p / tau)
    sy = torch.sigmoid(y / tau)
    probability = (8.0 * (tau / S) ** 2) * (sp - sy).square()
    for name, tensor in (("runtime prediction", p), ("clipped target", y), ("Huber loss", h), ("probability loss", probability)):
        _finite(torch, name, tensor)
    return {"huber": h[:, 0], "probability": probability[:, 0], "prediction": p[:, 0], "target": y[:, 0], "predictionSigmoid": sp[:, 0], "targetSigmoid": sy[:, 0]}


def forward(torch: Any, model: Any, batch: tuple[Any, ...], tau: float, arm: str) -> tuple[Any, dict[str, Any]]:
    if arm not in {"C", "T"}:
        raise F1Error(f"invalid F1 arm: {arm}")
    us, them, white, white_values, black, black_values, _outcome, score, psqt, stacks = batch
    raw = model(us, them, white, white_values, black, black_values, psqt, stacks)
    tensors = per_record(torch, raw, score, tau)
    objective = tensors["huber" if arm == "C" else "probability"].mean()
    _finite(torch, "mean objective", objective)
    return objective, tensors
