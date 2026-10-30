## Phase R2: deterministic trainer and finite evaluation

### Question

Can one run be reproduced, resumed, and interpreted in terms of exact example
exposure and validation data?

### Required trainer changes

- Make the trainer integration a pinned, reviewable component. Use a vendored
  fork, submodule, or immutable build context; do not patch an arbitrary external
  checkout in place and restore it afterward.
- Record the trainer commit, local diff hash, dependency lock, compiler/CUDA
  versions, loader shared-library hash, serializer hash, and generated variant
  files.
- Set random skipping to zero. The merged data is already shuffled.
- Implement deterministic epoch permutations or deterministic shard scheduling
  whose sequence hash is independent of loader thread timing.
- Define an epoch as an exact record count. Log records read, records trained,
  optimizer updates, equivalent passes, and batch identity hashes.
- Implement finite validation with a supported final partial batch. Consume each
  record exactly once, with no cycling, filtering, or skipping.
- Split by opening/trajectory group before record-level shuffling. Canonical and
  color-swapped copies of a board must stay in one split.
- Save and restore optimizer, scheduler, scaler, all RNG states, and data cursor.
- Refuse a fresh run if its output directory already contains a checkpoint. A
  resume must be an explicit command and must report the starting global step.
- Use a step-based schedule with warmup and declared minimum/maximum updates.
  `max_steps = 16` remains available only for smoke tests.
- Log teacher-only loss, optional result loss, raw robust score error,
  calibrated-probability loss, Pearson/Spearman correlation, sign accuracy
  outside a near-zero band, activation/clipping rates, gradient/update norms,
  active-row counts, and quantization survival.

### Validation layers

Maintain five disjoint layers:

1. training validation, visible during optimization;
2. development test, used repeatedly to choose a recipe;
3. versioned one-shot stage blocks for R5, each R6 scale, and R7; a block is
   opened once for the predeclared stage candidate, then becomes development
   history and is never called sealed again;
4. final sealed static/search test, unopened until R9 and never used for hard
   mining or recipe changes;
5. sealed promotion opening blocks, never used for model or recipe selection.

Pre-generate and hash the stage blocks without exposing their labels. Every
stage/scale uses a disjoint source-group block so repeated scale decisions do
not tune on one fixed test.

Name metrics by split identity, not by dataloader index. A held-out opening split
must not be logged as `id_val_loss` merely because it is dataloader zero.

### Gates

- Two same-seed CPU/debug runs have the same batch hashes, metrics, and
  serialized output.
- A single uninterrupted debug run and an interrupted/resumed debug run consume
  the same records in the same order and produce the same CPU reference output.
- GPU runs consume the same data order; cross-hardware predictions and metrics
  stay within a predeclared numeric tolerance. Byte-identical GPU checkpoints
  across hardware are not required.
- Repeated finite validation produces identical metrics and reports exactly the
  physical record count.
- The R1 overfit test passes through the production loader and trainer command.

### CPU implementation and pre-GPU closeout (2026-09-02)

The CPU-valid portion of R2 is complete. The aggregate phase remains open and
its top-level `passed` field remains `false` because the cross-hardware CUDA
gate has not run.

- Replaced in-place mutation/restoration of the sibling trainer checkout with a
  disposable build context exported from pinned trainer commit
  `61666d9e3653e4df9881b14c23f8fdcc4bf7779b`, followed by the reviewed patch
  and repository-owned generated variant/feature/loader overlays. The sibling
  checkout is now only a Git object source; dirty working-tree files cannot
  affect the build.
- Added `r2-gate`, the frozen contract
  `r0/anhoku-reboot/r2-trainer-contract.json`, the strict identity config
  `haitaka_learn.anhoku-reboot-r2.training.toml`, and the deterministic trainer
  `scripts/r2-deterministic-trainer.py`. This is a compact debug learnability
  model over the production C++ sparse feature loader, not a strength candidate.
- Set random skipping to zero and loader concurrency to one. Each epoch is one
  finite physical pass, including the final partial batch. Exposure logs record
  batch hashes, physical records, optimizer updates, equivalent passes, epoch,
  record cursor, learning rate, gradient norm, and update norm.
- Split the R1-A corpus by indivisible ancestry/source group before record use.
  The split manifest reports 38 source groups, zero overlap, 6,785 training
  records, and 3,455 development-test records. Derived canonical/color-swapped
  copies are required to inherit the source root.
- Two same-seed CPU runs produced identical 96-batch schedules, metrics, and
  canonical serialized output. A run stopped at global step 37 and explicitly
  resumed restored model, optimizer, schedule step, Python/NumPy/Torch RNGs,
  and the exact epoch/record cursor; it produced the same final batch sequence,
  metrics, and output. All three canonical outputs have SHA-256
  `1db3a7f24ae1b64ffee961a1083177864e69845d09a8acf6485570238085d3b8`.
- A fresh invocation against the interrupted run directory was rejected. The
  resume report begins at global step 37 and ends at 96 after the same 24,210
  records (`3.5681650700073693` equivalent passes) as the uninterrupted run.
- Finite validation consumed exactly 3,455 records in 14 batches, including the
  partial final batch, and repeated byte-identically. The frozen known-target
  validation loss is `0.006075570953772767`, below the `0.01` limit; the final
  batch loss is `0.0038043612148612738` versus `0.014798738993704319` initially,
  below the frozen `0.30` ratio. Full metrics include teacher/result separation,
  robust raw error, calibrated-probability loss, Pearson/Spearman, sign accuracy,
  activation/clipping, active rows, gradients/updates, and quantization survival.
- The CPU helper report passes every CPU gate at
  `out/anhoku-reboot-r2/cpu/r2-cpu-gate-report.json`, SHA-256
  `4a16af3325ad7cf3854e3a4e4f2aa9a7e741b944e5efacd9400fd655172c9092`.
  The pre-GPU aggregate is
  `out/anhoku-reboot-r2/r2-pre-gpu-gate-report.json`, SHA-256
  `705396948095c7448bfb89eb557f75ce56f63ca8942eb174a58280a7cd9ddd6e`;
  it records `cpuGatesPassed=true`, `gpuAuthorized=true`, `gpuUsed=false`, and
  `passed=false`.
- The self-contained rental handoff is
  `out/anhoku-reboot-r2/anhoku-r2-vast-bundle.tgz` (435,088 bytes), SHA-256
  `a453ec5da5f55fc86397bc7f33eaa4a615e74175d8a5293f97d32f2a81b2aeec`.
  On a CUDA 12.8 instance, extract it, enter `vast-bundle`, install
  `r2-gpu-requirements.txt`, and run `bash run-r2-gpu.sh`. The bundle records
  the exact resolved CPU environment separately from the pinned GPU package
  contract. The gate requires identical batch hashes and bounds both
  every validation prediction and every declared metric to absolute delta
  `0.0002`; GPU checkpoint byte identity is deliberately not required.

The CPU closeout intentionally stopped before CUDA work. The rented-instance
execution is recorded below; no production-label generation or strength game
was run.

### CUDA cross-hardware closeout (2026-09-02)

The authorized debug GPU gate completed on a verified Vast.ai RTX 3060 (12 GB,
CUDA 12.8, `torch 2.8.0+cu128`). The first two launcher attempts exposed two
environment issues before any training batches ran: the bare shared-library
name was not loadable through `ctypes`, and deterministic cuBLAS requires
`CUBLAS_WORKSPACE_CONFIG` to be set before Python starts. The final run used the
absolute loader path and `CUBLAS_WORKSPACE_CONFIG=:4096:8`; the generated
launcher now emits both requirements.

- The machine consumed the exact production split and 96-batch schedule. CPU
  and GPU batch hashes match exactly (`batchHashesMatchCpu=true`).
- Maximum validation prediction absolute delta was
  `2.384185791015625e-07`, below the frozen `0.0002` tolerance. Validation
  teacher-loss delta was `3.591464785290821e-09`; every declared metric stayed
  within tolerance.
- The passing machine-readable report is
  `out/anhoku-reboot-r2/gpu-rerun3/r2-gpu-gate-report.json`, SHA-256
  `7ae6b8142908d2d3fd1d9a70aa2b8b54b843186d8d8d7492455873d079b35d67`.
  Its run artifact is `gpu/run.json`, SHA-256
  `a4d8c278e2065bef192b2f62aead427c7985d119d3c287a29666a44010c47894`.
  GPU checkpoint and serialized output hashes are
  `7afb2b16209a409573c1c0646464c6f3a615e7fc15543dea5b306007228f7dcc` and
  `9beea7f6ec1533e303151574aac83f26476a4ea6cf7c0156d17af38d7a9db1a3`.
- The GPU gate does not require byte-identical checkpoints across hardware;
  it requires exact data-order parity and bounded numerical/metric deltas.

R2's deterministic CPU, resume, finite-validation, production-loader
learnability, and CUDA cross-hardware gates now pass. This authorizes the next
declared phase (R3 teacher/target calibration), but does not authorize
production labels or strength matches without the R3 freezes.

### Cost ceiling

At most one GPU-hour on debug data after CPU gates pass.

### What passing proves

It proves that a declared number of deterministic updates can learn a known
target and be resumed. It does not choose the production teacher or data scale.

