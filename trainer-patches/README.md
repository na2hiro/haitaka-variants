# Phase 7.1 trainer overlay

This directory preserves the reviewed changes for the custom trainer without
mutating the sibling checkout. The patch targets trainer base revision
`61666d9e3653e4df9881b14c23f8fdcc4bf7779b`, which is the locally available
replacement for the unavailable Phase 7 revision
`2388a9bb7bf7004eee3954ee72ff4d407a1bc1bd`.

Apply the patch in the trainer checkout with the small, documented fuzz
allowance required by this older trainer's surrounding formatting, copy
`evaluate.py` beside `train.py`, and record the resulting trainer commit or
working-tree diff in the lane manifest:

```bash
patch --fuzz=3 -p1 < /path/to/haitaka-variants/trainer-patches/variant-nnue-pytorch-phase7.1.patch
cp /path/to/haitaka-variants/trainer-patches/evaluate.py .
```

The overlay adds an explicit initial LR, step-based checkpoint and validation
intervals, max-step termination, ID/OOD TensorBoard diagnostics, and
deterministic offline checkpoint evaluation. Existing trainer defaults remain
epoch-based with LR `0.0015` when the new options are omitted.
