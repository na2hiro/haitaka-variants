## NNUE Donor And Teacher Roadmap

### Summary

donor-rule variants should not stay on plain `HalfKAv2^` as the long-term input,
because donor relationships are core game state and are not represented
directly today. However, before the donor-feature rollout, we should do one
small teacher-pipeline upgrade so later training runs are not bottlenecked by
the current shallow single-budget labeling flow.

Recommended sequence:

- Phase 1: minimal teacher-labeling upgrade
- Phase 2: donor-feature extension
- Phase 3: controlled retraining and A/B evaluation

This is not a recommendation to postpone donor features indefinitely. It is a
recommendation to make one low-risk data-quality improvement first, then do the
real representational change immediately after.

### Why This Order

- Current donor variants still train with `HalfKAv2^` only, which is a poor fit
  for Annan, Anhoku, and Antouzai because effective movement depends on donor
  relationships.
- Current data generation uses one fixed search budget for both self-play move
  selection and sampled-position labeling. That makes label quality unnecessarily
  weak, because the full game rollout must stay cheap.
- A small teacher refactor benefits both the old feature family and the new
  donor-aware family, so it is safe preparatory work.
- The donor-feature plan remains the main strength lever for donor variants.

### Phase 1

Implement the smallest teacher-pipeline upgrade that improves label quality
without redesigning the whole trainer flow.

- Decouple rollout budget from label budget in `haitaka_learn generate-data`.
- Keep self-play move selection cheap.
- Re-search only sampled positions with a stronger budget.
- Support a separate config field for sampled-position labeling, for example:
  - `data.rollout_search_depth`
  - `data.label_search_depth`
- Keep existing `bootstrap_nnue` teacher support and use the same teacher type
  for both rollout and labeling in v1, only with different budgets.
- Preserve shard manifest compatibility by recording both rollout and label
  budgets in metadata.

Out of scope for Phase 1:

- Multi-teacher blending
- time- or node-based search budgets
- hard-position mining
- policy exploration among near-best moves

### Phase 2

Execute [extend-nnue-input-for-donor-rules.md](/home/na2hiro/proj/haitaka-variants/plans/extend-nnue-input-for-donor-rules.md).

Priority remains:

- `annan` / `anhoku` -> `HalfKAv2^+DonorSingleEff`
- `antouzai` -> `HalfKAv2^+DonorPairSlots`
- future knight-8 family -> `HalfKAv2^+DonorKnight8Slots`

Implementation guidance:

- Do not redesign the donor plan around the Phase 1 teacher work.
- Keep the donor blocks as real runtime features, not `^`-only virtual factors.
- Accept full-refresh fallback for donor-family incremental eval in v1.

### Phase 3

Run controlled retraining and self-play to isolate the effect of each change.

Required comparisons:

- Baseline: old features + old teacher pipeline
- Data-only: old features + Phase 1 teacher pipeline
- Full change: donor features + Phase 1 teacher pipeline

Success criteria:

- `Data-only` should tell us whether better labels alone move strength materially.
- `Full change` should tell us whether donor-aware inputs add strength on top of
  better labels.
- All comparisons should use fixed time controls, identical search settings, and
  the same opening/randomization policy.

### Suggested Worker Split

- Worker 1: Phase 1 `haitaka_learn` config and dataset changes
  - split rollout budget from label budget
  - update manifests/resume checks
  - add tests for metadata and sampling behavior
- Worker 2: donor feature work in trainer / runtime
  - follow the donor-family feature plan
  - keep compatibility with existing non-donor nets
- Worker 3: evaluation harness and experiment tracking
  - define A/B training matrix
  - run fixed-condition self-play comparisons
  - summarize results for promotion or rollback decisions

### Test Plan

- Phase 1
  - `generate-data` works with equal rollout/label budgets and with different
    rollout/label budgets.
  - resume rejects stale shards when either budget changes.
  - sampled positions receive labels from the configured label budget.
- Phase 2
  - all tests from the donor-feature plan pass.
- Phase 3
  - self-play harness can compare three model families under fixed conditions.
  - reports clearly distinguish feature-family changes from teacher-pipeline
    changes.

### Decision Rule

If only one large item can be scheduled next, prefer the donor-feature extension
over deeper teacher polishing. If two adjacent items can be scheduled, do the
minimal teacher refactor first and donor features immediately after.
