# NNUE Donor Strengthening Execution Plan (Superseded)

Status: superseded on 2026-08-17 by
[Anhoku NNUE Handcrafted-Strength Execution Plan](anhoku-nnue-handcrafted-strength-plan.md).

This plan was written before its main implementation assumptions changed. It
must not be used as the active task list.

Completed since it was written:

- donor-aware feature families, model loading, extraction, and verification;
- incremental donor accumulator updates;
- separate rollout and label search depths with shard identity checks;
- automatic intermediate-checkpoint export and corrected paired ranking;
- fixed-movetime self-play and handcrafted benchmark reporting.

The assumptions that plain `HalfKAv2^` is still in use, that full-refresh donor
evaluation is the expected v1 runtime, and that movetime evaluation is not yet
available are stale. The latest v0.5.1 result is nevertheless `-110.08 Elo`
against handcrafted, so the next work is data correction, stronger tactically
resolved labels, vectorized dense inference, and controlled retraining—not a
repeat of the donor v1 implementation.

Use the active plan for task ordering, file-level scope, tests, experiment
gates, and promotion criteria. The latest evidence is recorded in
[Anhoku v0.5 / v0.5.1 corrected NNUE selection](../docs/nnue-training-anhoku-v0.5-corrected.md).
