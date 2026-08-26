# Anhoku NNUE v0.6 Phase 8C generation recovery

Status: **GR2 COMPLETE, DATASET REJECTED BEFORE TRAINING** (2026-08-27).
This document is a result record, not an active recovery procedure.

## Result

Shard-level verification succeeded:

- the predecessor supplied 1,660 train and 39 validation shards;
- GR2 supplied train shards `1660..3399`;
- all `.bin`/`.json` lengths, ranges, config identities, bootstrap identity,
  opening-suite identity, and candidate identities matched;
- merge assembled 34,000 train games with 1,115,026 accepted records and all
  384 validation games with 15,556 records.

The readiness audit then rejected the dataset:

| Dataset | Records | Unique 72-byte records | Unique packed boards |
| --- | ---: | ---: | ---: |
| Phase 8B root train | 256,725 | 8,641 | 8,641 |
| Phase 8C predecessor train | 543,074 | 6,832 | 6,832 |
| Phase 8C full GR2 train | 1,115,026 | 6,832 | 6,832 |

GR2 added 571,952 accepted records but zero new packed boards beyond the
predecessor union. No final dataset was published and training did not start.
The extracted inputs remain forensic evidence under:

- `out/anhoku-v0.6-phase8c-gr2-root-1m/`;
- `out/anhoku-v0.6-phase8c-gr2-root-1m-mbp/`;
- `out/anhoku-v0.6-phase8c-gr2-root-1m-mba/`.

The original incomplete-label criterion also failed: train was 1.549% and
OOD-v2 validation was 1.582%, against the predeclared 1% maximum. The temporary
2% exception considered during recovery is retired with this dataset.

## Cause

This was deterministic trajectory collapse, not a transfer, shard-coverage,
teacher-identity, or RNG seed-collision failure.

The frozen config used `opening_random_plies = 0`, so its per-game RNG never
selected a move. Despite its name, `uniform-rollout-v1` performed depth-1
search and always selected the deterministic best move. Each occurrence of one
of the 52 fixed train openings and its color swap therefore followed the same
trajectory. The game seed changed only the every-second-ply sampling phase.

Generating more games under that policy could only repeat the same small set.
The Phase 8B strength matches remain measurements of the trained models, but
the root dataset is not a valid 262k-unique-position learning-curve point.

## Retired implementation

GR1 introduced a predecessor-generation contract, Phase-8C-specific staged
publication, a ready marker, and distributed continuation flags. GR2 used that
machinery to extend the deterministic dataset while preserving shard and
teacher identity. It could not repair the trajectory policy that generated the
records.

Those recovery-only code paths and the GR2 production config are intentionally
absent from the clean branch. Keeping them would permanently couple the data
pipeline to a rejected dataset, and the original uniqueness implementation
counted complete 72-byte records instead of packed boards. The historical code
and exact handoff procedure remain recoverable from branch
`archive/phase8c-gr2-20260827`; they are not current operating instructions.

## Superseding decision

Do not train on, deduplicate and upsample, extend, or use the GR2 records as a
material prefix of a replacement dataset. Phase 8D now precedes a repaired
Phase 8C:

1. add genuine searched-stochastic trajectory generation;
2. audit packed-board uniqueness on a label-free pilot;
3. require the pilot's diversity and reproducibility gates to pass;
4. generate a fresh unique-262k labeled dataset;
5. only then scale to a fresh unique-1M Phase 8C dataset.

The detailed gates and stopping rules are defined in
`plans/anhoku-nnue-handcrafted-strength-plan.md`.
