# NNUE Donor And Teacher Roadmap (Superseded)

Status: superseded on 2026-08-17 by
[Anhoku NNUE Handcrafted-Strength Execution Plan](anhoku-nnue-handcrafted-strength-plan.md).

This file described the pipeline before the donor and teacher work landed. The
following items are now complete:

- Anhoku and the other single-donor variants use
  `HalfKAv2^+DonorSingleEff` rather than plain `HalfKAv2^`.
- Antouzai uses `HalfKAv2^+DonorPairSlots`, and Anki has its eight-slot family.
- Rollout search and sampled-position label search have separate depth budgets
  and manifest/resume identity.
- Donor-family accumulators are updated incrementally; full refresh is no
  longer the current donor-family runtime plan.
- Checkpoint selection, movetime self-play, paired results, and confidence
  reporting are available.

The latest corrected Anhoku model still loses decisively to handcrafted. The
active plan therefore starts from data distribution, qsearch-leaf/fixed-node
teacher quality, SIMD inference, and a staged learning curve. Do not implement
the old Phase 1/2/3 sequence from repository history.

Historical donor implementation details are preserved in
[Donor-Family NNUE Feature Plan](done/20250602-extend-nnue-input-for-donor-rules.md).
