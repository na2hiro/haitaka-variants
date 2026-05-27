## NNUE Donor Strengthening Execution Plan

### Purpose

This plan turns the current NNUE training evidence into an execution sequence
that a later worker can break down into concrete implementation tasks.

The goal is to make Annan / Anhoku / Antouzai NNUE models materially stronger
than the current handcrafted evaluator under controlled self-play, without
spending large training cycles on low-leverage experiments first.

### Current Evidence

- Current donor-rule models train with plain `HalfKAv2^`.
- Plain `HalfKAv2^` only exposes physical piece-square and hand features.
- Annan, Anhoku, and Antouzai strength depends heavily on donor relationships:
  effective movement can change based on adjacent friendly pieces.
- The handcrafted evaluator is simple, but its mobility term is computed from
  legal moves, so it already sees donor-rule movement effects through movegen.
- Anhoku results are weak or near parity at shallow fixed depth, and one depth-4
  check was much worse.
- Antouzai shows a strong mid-training checkpoint, then clear regression by the
  final checkpoint.
- The recovered donor feature plan exists at
  `plans/extend-nnue-input-for-donor-rules.md` and gives a concrete feature
  architecture.

### Main Diagnosis

The biggest likely bottleneck is feature representation, not only teacher
quality.

For donor-rule variants, the network currently has to infer piece-to-piece
donor relationships from summed single-piece embeddings. That is a poor
inductive bias. A donor-aware real feature block should expose the core dynamic
state directly:

- Annan / Anhoku: `HalfKAv2^+DonorSingleEff`
- Antouzai: `HalfKAv2^+DonorPairSlots`
- future knight-8 family: `HalfKAv2^+DonorKnight8Slots`

Deeper teacher labels are still useful, but deeper labels on the old feature
family can remain capped by the missing donor signal.

### Recommended Sequence

#### Phase 0: Checkpoint Promotion And Evaluation Hygiene

Do this before another long training run.

- Export and evaluate checkpoints every fixed interval, for example every 5
  epochs.
- Promote the best checkpoint by self-play result, not the final epoch.
- Keep a machine-readable run summary with:
  - ruleset
  - feature set
  - dataset manifest hashes
  - training lambda
  - epoch
  - model path
  - self-play command
  - score
  - approximate Elo
  - total games
  - search settings
  - random seed / opening policy
- Run quick triage matches first, then confirm promising checkpoints with a
  larger paired match.
- Treat 100-game results as triage only.

Reasoning:

Antouzai already showed a strong mid-training checkpoint and a weak final
checkpoint. Failing to preserve and promote the best checkpoint can erase a real
gain even if training is working.

#### Phase 1: Donor-Aware Feature Implementation

Implement `plans/extend-nnue-input-for-donor-rules.md`.

Required emphasis:

- Keep donor features as real runtime/exported blocks, not `^` virtual factors.
- Keep standard and handicap on existing `HalfKAv2^`.
- Add family-aware model loading in Haitaka runtime.
- Add family-aware active feature generation in Haitaka runtime.
- Permit donor feature-set names in `haitaka_learn` config validation.
- Map rulesets to recommended feature sets:
  - `annan` -> `HalfKAv2^+DonorSingleEff`
  - `anhoku` -> `HalfKAv2^+DonorSingleEff`
  - `antouzai` -> `HalfKAv2^+DonorPairSlots`
- Use full-refresh fallback for donor-family NNUE search in v1 if that is the
  simplest correct implementation.

Additional tests beyond the recovered donor plan:

- Add golden crafted positions where trainer feature extraction and Haitaka
  runtime extraction must produce identical active donor features.
- Cover both black and white perspectives.
- Cover promoted donor pieces.
- Cover captures that remove a donor relation.
- Cover Antouzai left-only, right-only, and both-donor cases.
- Confirm old `HalfKAv2^` models still load and search unchanged.

Reasoning:

Hash/parser tests prove compatibility boundaries, but they do not prove that
trainer and runtime interpret donor slots identically. A mismatch there would
produce exported nets that train successfully but evaluate incorrectly.

#### Phase 2: Controlled Retraining Matrix

After donor features are implemented, run a small controlled matrix before
committing to expensive data generation.

Minimum matrix:

- Baseline: old data, old `HalfKAv2^`, best checkpoint promotion enabled.
- Donor features: comparable data, donor feature family, best checkpoint
  promotion enabled.
- Optional data-only: old `HalfKAv2^`, split rollout/label budget.

Use identical evaluation conditions:

- Same ruleset.
- Same opening/randomization policy.
- Same fixed-depth settings for diagnosis.
- Same fixed-time settings for practical strength, once the harness supports
  time control.
- Same number of games per confirmation run.

Suggested gate:

- 100 games: quick checkpoint triage.
- 400-1000 paired games: promotion decision for a candidate model.
- Fixed-depth and fixed-time results should be reported separately.

Reasoning:

Fixed depth isolates eval quality more cleanly. Fixed time captures practical
strength, especially because donor-family v1 may use full-refresh NNUE and lose
NPS.

#### Phase 3: Split Rollout And Label Budgets

Implement a minimal teacher-pipeline upgrade after or alongside donor features
if worker capacity allows.

Target behavior:

- Self-play rollout stays cheap.
- Sampled positions are re-searched with a stronger label budget.
- Shard manifests record both rollout and label budgets.
- Resume/merge rejects mismatched budget metadata.
- Existing `bootstrap_nnue` teacher support remains usable.
- The same teacher type can be used for rollout and labeling in v1, but with
  different budgets.

Recommended fields:

- `data.rollout_search_depth`
- `data.label_search_depth`

Backward compatibility:

- Existing `data.search_depth` can map to both fields when the new fields are
  absent.
- Existing manifests with only `search_depth` can be treated as equivalent to
  `rollout_search_depth == label_search_depth == search_depth` if compatibility
  is needed, or rejected with a clear migration error if strict freshness is
  preferred.

Implementation details:

- During game generation, use `rollout_search_depth` for move selection after
  opening random plies.
- When a position is sampled, run a separate search at `label_search_depth` and
  store that score in the training entry.
- Avoid reusing a shallow rollout search result as the label when the label
  depth is greater.
- Preserve cheap random opening behavior.
- Include both budgets in train and validation dataset manifests.
- Include both budgets in shard manifests.
- Include both budgets in the config hash or explicit resume/merge matching
  logic so stale shards cannot be silently reused.

Out of scope for this phase:

- Multi-teacher blending.
- Time- or node-based teacher budgets.
- Hard-position mining.
- Policy exploration among near-best moves.
- Separate teacher types for rollout and labeling.

Tests:

- `generate-data` works when rollout and label budgets are equal.
- `generate-data` works when label budget is greater than rollout budget.
- Sampled positions receive labels from the configured label budget.
- Move selection after opening random plies uses the rollout budget.
- Resume rejects stale shards when either budget changes.
- Merge rejects shards with mismatched rollout or label budget.
- Existing configs using only `data.search_depth` still behave as before.

Reasoning:

Better labels are useful, but on the old feature family they may not solve the
donor-representation problem. Once donor features exist, stronger labels should
be more valuable.

#### Phase 4: Data Diversity And Hard Positions

Do this after the representation and evaluation pipeline are stable.

Candidate improvements:

- Multiple data-generation seeds.
- More varied opening randomization.
- Mine positions where NNUE and handcrafted/search disagree.
- Oversample tactical donor positions from crafted or self-play sources.
- Keep validation data separated by seed/opening policy to detect overfitting.

Reasoning:

The current setup uses large self-play data, but limited policy diversity can
still create narrow training distributions. This should not precede donor
features, but it is likely needed for robust strength.

### Non-Goals For The First Worker Pass

- Do not redesign the NNUE architecture beyond the donor real blocks.
- Do not optimize donor-family incremental accumulator updates before proving
  full-refresh correctness and strength.
- Do not use a weak NNUE as teacher unless it already beats handcrafted under
  the target evaluation condition.
- Do not judge final strength from a single 100-game match.
- Do not mix fixed-depth and fixed-time results in one headline number.

### Risks

- Full-refresh donor NNUE may improve eval quality but reduce NPS enough to
  hurt fixed-time strength.
- Trainer and runtime donor slot ordering can silently diverge.
- Composite feature hashes or serializer behavior can make exported nets fail
  to load.
- Shallow handcrafted labels may cap final strength even with better inputs.
- Final-epoch export can regress even when earlier checkpoints are strong.

### Expected Outcome

If the diagnosis is correct:

- Antouzai should benefit most from donor-aware inputs because two-donor union
  effects are hardest for plain `HalfKAv2^`.
- Annan and Anhoku should see meaningful improvement from `DonorSingleEff`.
- Checkpoint promotion should immediately prevent losing known good mid-training
  models.
- Split rollout/label depth should become more valuable after donor features are
  available.

The objective proof remains a controlled A/B retrain plus paired self-play under
fixed settings.
