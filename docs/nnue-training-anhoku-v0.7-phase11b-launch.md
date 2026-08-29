# Anhoku v0.7 Phase 11-B Vast launch

## Decision

Phase 11-B seed 80 is ready to launch. No production training or strength game
has started. The experiment is a strict V1-versus-V2 feature ablation using the
same audited Phase 8D-B.1 positions and functionally identical C/16 starting
evaluation.

The launch must use `scripts/vast/phase11b-seed80.sh`. Do not use `cargo train`:
that wrapper runs checkpoint ranking, while Phase 11-B requires fixed step-16
exports with no games against step-4 anchors.

## Frozen inputs

| Input | Frozen value |
|---|---|
| Haitaka source | exact commit recorded in the bundle's `phase11b-input-audit/bundle-source-commit.txt`; it must contain Phase 11-A commit `c26e4fd` |
| Trainer revision | `61666d9e3653e4df9881b14c23f8fdcc4bf7779b` |
| C/16 V1 bootstrap | `049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0` |
| Train data | 279,627 records, 276,949 distinct packed boards, SHA-256 `aa2fc9decbb767d170c10a523ccefb9bb01ef3a39dc7d2e36606a34fb5e85599` |
| OOD-v2 validation | 3,218 records, 3,215 distinct packed boards, SHA-256 `36e1360e75c81af311efca4497bc611e99fd6bb01fbad8cb2be8bac605bdb2e6` |
| Training seed | 80 in both lanes |
| Budget | step 16, checkpoints and validation every 4 steps |
| Batch/order controls | batch 16,384; epoch 262,144; random-FEN skipping 3; identical data bytes and shuffle order |
| Objective/optimizer controls | lambda 0.8; initial LR 0.00015; identical remaining trainer arguments |

The only allowed lane differences are output identity and feature family:

- V1: `HalfKAv2^+DonorSingleEff`
- V2: `HalfKAv2^+DonorReceiverPairV2`

V2 starts by expanding every C/16 V1 effective-type row into its ten
receiver-native slices. Phase 11-A proved this migration is evaluation-exact;
the remote preflight additionally imports both bootstraps through the real
PyTorch serializer without taking an optimizer step.

## Instance specification

Use one on-demand x86-64 NVIDIA instance for the first run. Recommended:

- at least 16 GiB VRAM (24 GiB is comfortable); the launcher refuses less than
  12 GiB;
- at least 8 vCPUs and 32 GiB system RAM;
- at least 40 GiB free container disk;
- a CUDA/PyTorch image with SSH and reliable download bandwidth.

The two lanes run sequentially on one GPU. There is no benefit in paying for a
second GPU under this frozen config. Set `VAST_HOURLY_USD` before preflight if
the offer price should be preserved in the run metadata.

## Build the paired bundle locally

```bash
scripts/phase11b-prepare-vast-bundle.sh
```

This command rechecks all frozen hashes, proves the configs differ only in the
allowed fields, hard-links the same dataset into both isolated output trees,
validates both configs, creates the standard per-lane bundles, and combines
them into:

```text
target/pretrain-bundles/anhoku-v0.7-phase11b-seed80-paired.tgz
target/pretrain-bundles/anhoku-v0.7-phase11b-seed80-paired.tgz.sha256
```

The paired archive contains one bootstrap, both exact dataset copies, both
rewritten configs, and the Phase 8D-B.1/11-A audit evidence. It contains no old
checkpoints or training output.

## Vast setup and preflight

Clone Haitaka at the recorded launch commit and the trainer at its frozen
revision, then install the trainer environment as described in
`docs/vast-ai-nnue-training.md`. Upload and unpack the paired bundle in the
Haitaka checkout.

Before training:

```bash
cd /workspace/haitaka-variants
export VAST_HOURLY_USD='<offer price>'
scripts/vast/phase11b-seed80.sh preflight
```

The preflight fails on a source, trainer, config, dataset, bootstrap, CUDA,
VRAM, disk, or trainer/runtime parity mismatch. It compiles the real loader and
materializes both PyTorch bootstraps, then exits with:

```text
Phase 11-B preflight passed; no optimizer step has run.
```

Only after that message, start the controlled run explicitly:

```bash
scripts/vast/phase11b-seed80.sh train
```

The launcher runs V1 and then V2, supports checkpoint resume after an instance
interruption, exports the newest exact step-16 checkpoint from each lane, runs
the verifier, records hashes, and produces:

```text
anhoku-v0.7-phase11b-seed80-results.tgz
anhoku-v0.7-phase11b-seed80-results.tgz.sha256
```

Download and verify both files before destroying the instance. The result
archive retains both step-16 `.ckpt`/`.nnue` pairs, configs, manifests, logs,
GPU/preflight metadata, verifier output, and checkpoint-to-NNUE identities.

## What happens after training

Training completion does not authorize seed 81 or promotion. First run the
frozen tactical/OOD/verifier vetoes, compare V2 directly against V1 in 1,024
paired 100 ms games, and measure NNUE-side NPS. Extend once to 4,096 games only
if inconclusive. Seed 81 is authorized only if seed 80 satisfies the Phase
11-B gate in the plan.
