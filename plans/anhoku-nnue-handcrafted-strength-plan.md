# Anhoku NNUE Handcrafted-Strength Execution Plan

- Status: Phases 1–6 complete; Phase 7 pending; Phase 8 preflight prepared
- Created: 2026-08-17
- Last checked: 2026-08-20
- Primary ruleset: Anhoku
- Baseline: [Anhoku v0.5 / v0.5.1 corrected NNUE selection](../docs/nnue-training-anhoku-v0.5-corrected.md)

## Goal And Promotion Condition

Produce an Anhoku NNUE that is stronger than the handcrafted evaluator in a
paired fixed-movetime match. Improve the pipeline in an order that gives every
experiment a clear interpretation and avoids a large training run before the
current data bias and runtime deficit are addressed.

“Best NNUE checkpoint” is only an internal screening result. Promotion requires:

- the NNUE's lower 95% confidence bound against handcrafted is above `0 Elo`;
- the final match uses paired openings and the target fixed-movetime runtime;
- the gain is reproduced with a second training seed or independently generated
  dataset before changing the default model.

## Current Baseline

The corrected v0.5.1 run selected epoch 6, but scored `-110.08 Elo` against
handcrafted with a 95% CI of `[-132.29, -87.87]`. The v0.5 rerun scored
`-178.40 Elo`. These are decisive losses rather than checkpoint-selection
noise.

Already implemented and not part of this plan:

- `HalfKAv2^+DonorSingleEff` for Anhoku;
- separate label and rollout search depths;
- donor-aware incremental accumulator updates;
- movetime self-play, paired checkpoint ranking, and confidence reporting;
- preservation and selection of intermediate checkpoints.

The preserved v0.5.1 data contains 1,317,183 positions from approximately
50,000 games. A binary audit found:

- every sampled position is on an even ply because `sample_start_ply = 8` and
  `sample_every_ply = 2` use one fixed phase;
- 87.76% of labels have a positive final result relative to side to move, while
  only 3.84% are draws;
- every 16-bit teacher-move field is zero.

The generator also samples during the uniformly random opening and chooses
those opening moves from every legal move. This produces positions unlike
strong play and couples sampling to one side of the game.

Runtime speed is a separate practical-strength problem. The latest 100 ms
report recorded roughly 7,349 NNUE NPS versus 17,609 handcrafted NPS. A focused
benchmark measured incremental evaluation at about 3.34 us and identified the
scalar dense affine layers as the next optimization target.

## Industry-Aligned Principles

The execution order follows the standard NNUE workflow used by mature chess
and shogi engines:

- shuffled, diverse positions rather than long correlated game runs;
- tactically resolved qsearch-leaf examples;
- cheap rollout separated from stronger teacher labeling;
- multiple training seeds and intermediate-checkpoint evaluation;
- paired games and a statistical promotion gate;
- architecture-specific SIMD for integer inference;
- dataset growth only after a learning curve shows that scale is limiting.

References:

- [YaneuraOu NNUE training notes](https://github.com/yaneurao/YaneuraOu/wiki/NNUE)
- [Stockfish nnue-pytorch documentation](https://github.com/official-stockfish/nnue-pytorch/tree/master/docs)
- [Stockfish testing framework](https://github.com/official-stockfish/fishtest)

Stockfish-scale data is a direction, not an initial target. Establish the local
learning curve at 1M, 10M, and 50M positions first.

## Agent Assignment Contract

Each numbered phase below is one agent assignment and normally one reviewable
PR. Do not ask one agent to implement multiple phases, and do not let an agent
silently continue into the next phase after its acceptance criteria pass.

Every implementation phase must include its focused tests, documentation, and
config/manifest migration in the same PR. Every experiment phase must produce
the required machine-readable artifacts and one Markdown result document.
When a phase discovers follow-up work, record it in the plan instead of
expanding that agent's scope.

## Review Checkpoint (2026-08-20)

Phases 1–3 are complete on this branch, with implementation commits
`f187708` (Phase 1), `38f7007` (Phase 2), and `6a51121` (Phase 3). The later
`3841b5e` commit corrects the self-play sampling/label feedback discovered by
the Phase 3 pilot; it is part of the current v0.6 data path but does not claim
to complete Phase 4.

Verification completed:

- `cargo test -p haitaka_learn --features anhoku`: 77 passed;
- `cargo fmt --all -- --check` and `git diff --check` passed;
- the preserved v0.5.1 audit and v0.6 smoke/pilot audits are present under the
  ignored `out/` artifacts and match the phase result documents;
- the v0.6 suite, grouped split, shuffle, identity checks, and teacher-move
  contract are covered by focused tests and the checked-in configs/docs.

Follow-up notes:

- Phase 3's bounded-memory implementation is documented and its formula is
  tested, but the focused test uses a small fixture chunk rather than running
  a stress case at the 1,000,000-record validation cap. Add that benchmark or
  stress test before relying on the bound for a large production run.
- The corrected rollout pilot now passes the train-side balance gate, but its
  small validation split still misses the 60% decisive-outcome bound. A larger
  validation pilot and the Phase 7 gate are still required; do not start the
  1M three-seed strength experiment yet.
- Phases 4 and 5 are complete. Phase 7's root-position 1M baseline is the next
  experiment in the planned sequence.
- Phase 8's committed 20,000-node 200+40-game preparation pilots exceeded the
  incomplete-label gate in both splits (1.2% train, 2.84% validation). Leaf
  filtering also missed side/outcome balance bounds. The artifacts are valid
  for plumbing tests. The prepared lanes now use 50,000 nodes and versioned
  root/leaf-side, distance-parity, rejection-result, and per-opening telemetry;
  both lanes must be re-piloted before Phase 8 strength training.
- The 50,000-node re-pilot passed incomplete-label rejection in train (0.29%)
  but still failed validation (1.39%). Detailed telemetry ruled out a
  side/orientation implementation bug: qsearch trace parity is correlated with
  root side and opening, producing 43.10/56.90 train and 56.96/43.04 validation
  leaf-side splits. Replacement sampling after leaf rejection and the two-group
  validation set are avoidable amplifiers. Phase 8 remains gated pending an
  attempted-candidate cap and a broader or cross-validated opening holdout.

Common completion gate for implementation phases:

- run `cargo fmt` and the focused package/feature tests;
- run relevant compatibility or workspace tests for the changed boundary;
- run `git diff --check`;
- leave unrelated cleanup and the next numbered phase untouched;
- report acceptance criteria individually as passed, failed, or not run.

Phase 6 (SIMD) is code-independent from Phases 2-5 and may run in parallel in a
separate worktree. The other phases should normally land in numeric order
because they overlap in `haitaka_learn` config, dataset, and manifest code.

## Dependencies

```text
Phase 1: audit + sampling contract
    |
    +--> Phase 2: opening suite --> Phase 3: shuffle/split --> Phase 7: 1M A
    |
    +--> Phase 4: fixed-node budget --> Phase 5: qsearch leaf --> Phase 8: 1M B

Phase 6: SIMD (independent) --> Phase 8 fixed-time evaluation
Phase 7 + Phase 8 -----------> Phase 9: 10M promotion candidate
Phase 9 data-limited result --> Phase 10: 50M/100M scale confirmation
Phase 9/10 pipeline-limited result --> Phase 11: one Feature V2 experiment
```

## Phase 1: Audit, Sampling, And Teacher-Move Contract

**Status: complete.** See [the Phase 1 result](../docs/nnue-training-anhoku-v0.6-phase1.md).

This is the first agent assignment. It combines the pieces that must change
together to make the existing 72-byte dataset semantics measurable and safe.

### Scope

Add a `haitaka_learn` dataset-audit command or equivalent reusable function.
It reads a final `.bin` and manifest and emits deterministic JSON containing:

- entry count and byte-size validation;
- side-to-move and ply-parity counts;
- win/loss/draw counts relative to side to move;
- score min/max/mean/absolute mean and useful quantiles;
- mate/clamp-rate counts;
- nonzero teacher-move count;
- samples taken before the configured opening phase ends;
- config identity, seed, ruleset, feature family, and file SHA-256.

Give every game a deterministic random sampling phase in
`0..sample_every_ply`, derived from the game seed. Start sampling no earlier
than:

```text
max(sample_start_ply, opening_random_plies)
```

Add manifest identity such as `sampling_phase = "per-game-random-v1"` and
`sample_after_opening = true`. Resume and merge must reject mixed policies
unless the existing explicit identity override is used.

The current 16-bit teacher-move field cannot encode every shogi move. Until a
versioned wider record exists:

- disable trainer behavior that depends on teacher-move match rate, including
  smart capture/FEN skipping when it consumes this field;
- record `teacher_move_encoding = "unavailable"` in manifests;
- reject configs that enable a teacher-move consumer with this format;
- do not silently interpret zero as a real teacher move.

Store the v0.5.1 audit with the ignored run artifacts and copy its headline
values into the next result document. Do not change the 72-byte ABI.

Likely files:

- `haitaka_learn/src/main.rs`
- `haitaka_learn/src/dataset.rs`, or a new `dataset_audit.rs`
- `haitaka_learn/src/config.rs`
- `haitaka_learn/src/trainer.rs`
- `haitaka_learn/README.md`
- a new `haitaka_learn.anhoku-v0.6.toml`

Acceptance criteria:

- truncated and overlong files fail with an actionable error;
- a fixture covering both sides, all outcomes, and zero/nonzero moves produces
  exact audit counters;
- two audit runs produce byte-identical JSON, preferably without timestamps;
- the preserved v0.5.1 distribution is reproduced;
- 100 deterministic fixture games contain both ply parities;
- no sample precedes `opening_random_plies` under the new policy;
- resume/merge rejects sampling or teacher-move contract mismatches;
- current-format trainer invocation disables teacher-move-dependent behavior;
- legacy behavior remains available only through an explicit compatibility
  policy where required.

Out of scope: opening suites, record shuffling, grouped validation, fixed-node
search, qsearch-leaf extraction, SIMD, and a 1M run.

## Phase 2: Versioned Anhoku Opening Suite

**Status: complete.** See [the Phase 2 result](../docs/nnue-training-anhoku-v0.6-phase2.md).

This agent adds the first production-quality replacement for uniform-random
openings. It does not implement near-best stochastic search.

### Scope

Add an opening-source interface with:

- `suite`: load versioned SFEN openings and choose deterministically from the
  game seed;
- color-swapped pairs when the Anhoku transformation is valid;
- `uniform-random`: an explicitly named compatibility/smoke-test policy only.

Store opening policy, opening ID, suite SHA-256, and transformation version in
shard/final manifests and the per-game generation metadata. Resume and merge
must reject mismatched suite identity.

Add a small reviewed Anhoku suite or a deterministic script plus checked-in
source that produces it. Document how legality, duplicate positions, and color
swapping are validated.

Likely files:

- `haitaka_learn/src/config.rs`
- `haitaka_learn/src/dataset.rs`
- a checked-in opening-suite asset and its validator
- `haitaka_learn.anhoku-v0.6.toml`
- `haitaka_learn/README.md`

Acceptance criteria:

- every suite position parses and has both kings and at least one legal move;
- fixed config/seed produces the same opening sequence;
- every requested pair uses the same opening identity with opposite colors;
- suite changes invalidate resume/merge identity;
- uniform-random openings are not selected by the v0.6 production config;
- docs show how to add, validate, and version a suite.

Out of scope: searched-stochastic/MultiPV openings, dataset shuffling, teacher
changes, and training. Add searched-stochastic later only if the suite learning
curve shows insufficient policy diversity.

## Phase 3: Deterministic Shuffle And Grouped Validation

**Status: complete.** See [the Phase 3 result](../docs/nnue-training-anhoku-v0.6-phase3.md).

This agent owns dataset ordering and split leakage only.

### Scope

Replace game-order concatenation with a documented deterministic bounded-memory
record shuffle. Split train and validation by game before shuffling. Keep all
games derived from one opening ID, including its color-swapped pair, in one
split. A position, game, or opening group must never appear in both splits.

If the 72-byte ABI cannot identify games after assembly, perform grouping and
statistics while per-game buffers still exist. Record split policy/version,
split seed, shuffle policy/version, and shuffle seed in manifests and identity
checks.

Likely files:

- `haitaka_learn/src/config.rs`
- `haitaka_learn/src/dataset.rs`
- `haitaka_learn/README.md`

Acceptance criteria:

- identical inputs and seeds produce byte-identical train/validation files;
- record order differs materially from generation order;
- train/validation game and opening IDs are disjoint;
- bounded memory is demonstrated with a test or benchmark at a documented
  upper bound;
- resume/merge rejects split or shuffle mismatches;
- the audit reports group counts and zero overlap.

Out of scope: opening generation, label-search changes, training, and changing
the trainer record ABI.

## Phase 4: Fixed-Node Label Budget

**Status: complete.** See [the Phase 4 result](../docs/nnue-training-anhoku-v0.6-phase4.md).

This agent adds node-budgeted teacher labeling without changing which position
is stored.

### Scope

Add an optional fixed-node label budget while keeping rollout separate:

```toml
[data]
rollout_search_depth = 1
label_search_nodes = 5000
label_search_max_depth = 64
```

For new configs, `label_search_nodes` and depth-only `search_depth` are mutually
exclusive. Define whether the limit includes alpha-beta and qsearch nodes and
report both. The teacher must be deterministic for fixed position, evaluator,
budget, seed, and thread count. Retain depth-only mode for old datasets.

Likely files:

- `haitaka_wasm/src/lib.rs` and search-budget/result types
- `haitaka_learn/src/dataset.rs`
- `haitaka_learn/src/config.rs`
- `haitaka_learn/README.md`

Acceptance criteria:

- fixed-node search stops within a documented small overshoot;
- repeated searches return identical move, score, and counters;
- rollout continues to use its independent shallow budget;
- manifest/resume/merge identity includes budget type, nodes, depth cap, and
  node-counting version;
- legacy depth-only fixture output remains reproducible;
- telemetry reports generation CPU time and nodes per label.

Out of scope: qsearch-PV leaf storage, new opening policies, SIMD, and training.

## Phase 5: Qsearch-PV Leaf Dataset Entries

**Status: complete.** See [the Phase 5 result](../docs/nnue-training-anhoku-v0.6-phase5.md).

This agent changes the stored training position semantics and must not also
change search budgets or the binary ABI.

### Scope

Add an opt-in training trace that identifies the final position on the selected
PV after qsearch resolves captures, promotions, and required evasions. Store:

- the packed leaf position;
- its static teacher evaluation, not the backed-up root score;
- final game result relative to the leaf side to move;
- root ply and leaf distance in audit metadata.

Reject terminal kingless positions and ordinary examples with saturated mate
scores, and count every rejection. Keep the normal search API unchanged for
callers not requesting a training trace. Add a manifest field that distinguishes
root-position and qsearch-leaf datasets.

Likely files:

- `haitaka_wasm/src/lib.rs` and search result types
- `haitaka_learn/src/dataset.rs` (`Teacher` currently lives here; extracting a
  `teacher.rs` module is optional)
- `haitaka_learn/src/config.rs`

Acceptance criteria:

- repeated search produces the same PV leaf and packed record;
- crafted capture, promotion, and in-check positions reach expected quiet
  leaves;
- result orientation changes correctly when root and leaf sides differ;
- terminal and mate-saturated examples are counted and excluded;
- root/leaf policy changes invalidate resume/merge identity;
- legacy root-position generation remains reproducible.

Out of scope: hard-position mining, multiple teacher types, trainer-format v2,
SIMD, and training.

## Phase 6: Vectorized NNUE Inference

**Status: complete.** See [the Phase 6 result](../docs/nnue-training-anhoku-v0.6-phase6.md).

This is one performance-focused agent assignment and can proceed independently
in a separate worktree.

### Scope

Introduce scalar reference and optimized integer-affine kernels for:

- x86-64 AVX2;
- ARM64 NEON;
- `wasm32` SIMD128 when enabled.

Use compile-time selection where guaranteed and safe runtime feature detection
for generic native binaries. Keep scalar as the portability path and oracle.
Optimize `AffineLayer::forward_into` and `forward_single` first.

Keep the copy-based incremental accumulator. The previous apply/unapply trial
regressed and must not return without separate evidence.

Likely files:

- `haitaka_wasm/src/nnue.rs`, or a new `nnue/kernels.rs`
- `haitaka_wasm/benches/nnue.rs`
- target-feature configuration only if portable defaults remain safe

Acceptance criteria:

- optimized and scalar kernels are bit-exact over randomized/boundary inputs
  and all layer shapes in a real donor model;
- existing parse, evaluation, and search tests pass on both paths;
- Criterion records dense-layer, full-refresh, and incremental-state timings;
- native incremental evaluation improves at least 1.5x on the reference x86-64
  host, with 2x as the target;
- a 100 ms diagnostic records at least 1.5x NNUE NPS improvement without
  changing deterministic fixed-depth fixture moves;
- non-SIMD WASM builds and uses scalar fallback.

Out of scope: accumulator apply/unapply, NNUE architecture changes, training,
and unrelated search optimization.

## Phase 7: Corrected-Data 1M Baseline Experiment

This is one experiment-agent assignment after Phases 1-3. It measures data
correction without mixing in the new teacher semantics.

### Scope

- Generate approximately 1M positions with the versioned opening suite,
  randomized sampling phase, grouped split, and deterministic shuffle.
- Keep depth-3 root-position labels and all v0.5.1 training hyperparameters
  that remain compatible.
- Train three initialization seeds, export every checkpoint, and select the
  best checkpoint per seed.
- Compare against the v0.5.1 NNUE and handcrafted under identical paired
  openings and both diagnostic equal-node and 100 ms conditions.
- Write `docs/nnue-training-anhoku-v0.6-1m-data.md` with all seeds and failures.

Dataset gate:

- side-to-move share is between 45% and 55%;
- neither relative win nor loss exceeds 60% among decisive labels;
- no sample occurs during the opening phase;
- train/validation group overlap is zero;
- all manifests, audits, configs, seeds, and hashes are preserved.

Acceptance criteria:

- all three seeds complete or failures are diagnosed and preserved;
- median handcrafted Elo and seed variance are reported;
- no code changes are mixed into the experiment commit except necessary result
  documentation;
- the result explicitly decides whether Phase 8 should proceed.

Outcome bounds may be relaxed only with a written ruleset-specific audit. A
fixed-ply artifact is not an acceptable explanation.

## Phase 8: Fixed-Node Qsearch-Leaf 1M Experiment

This is one experiment-agent assignment after Phases 4, 5, and 7. Phase 6 must
also be available for the final fixed-time comparison.

### Scope

Using the same opening policy, generation seeds, split policy, and training
hyperparameters as Phase 7, compare:

- depth-3 root labels from Phase 7;
- 50,000-node root labels, using the re-pilot common budget;
- 50,000-node qsearch-leaf labels.

Node-budget searches that cannot complete depth 1 are rejected and counted
under the versioned `reject-position` policy instead of aborting a production
shard. The larger persistent 20,000-node pilot exceeded the 1% gate, so the
prepared lanes were raised to 50,000 nodes. If either re-pilot exceeds 1%,
pause before training and audit the rejected-position bias.

Train at least three initialization seeds per new lane. Record CPU-hours,
positions/second, label node distributions, validation loss, tactical-suite
results, fixed-anchor Elo, handcrafted Elo, and NNUE NPS. Write one result
document containing all lanes; do not select only the favorable seed.

Acceptance criteria:

- the same non-teacher variables are verified by hashes, not assumed;
- incomplete-label rejection counts and rates are reported for both new lanes
  and remain at or below 1%;
- median and per-seed results are reported;
- qsearch leaves advance only if they improve median handcrafted Elo or
  held-out loss without a tactical-suite regression;
- the result chooses exactly one data/label policy for Phase 9.

Out of scope: 10M generation, hyperparameter sweeps, feature changes, and
promotion of a default model.

## Phase 9: 10M Promotion Candidate

This is one end-to-end experiment-agent assignment using the single policy
selected in Phase 8.

### Scope

- Generate 10M unique positions and audit them.
- Train four initialization seeds with checkpoint export/ranking.
- Use paired fixed-anchor matches to select one checkpoint per seed.
- Use equal-node matches for evaluation-quality diagnosis.
- Use 100 ms paired matches against handcrafted for the promotion decision.
- Confirm a winning candidate with at least 2,048 paired games, extending the
  match while its confidence interval crosses zero.

SPRT is acceptable when hypotheses, alpha, beta, and maximum games are written
into the result artifact. Record pentanomial pair bins for every match.

Acceptance criteria:

- all four seeds and rejected candidates remain visible in the result;
- the winning NNUE's lower 95% confidence bound is above `0 Elo` against
  handcrafted, or the phase is explicitly recorded as not promoted;
- a second seed or independently generated dataset reproduces the gain before
  changing the default model;
- the result classifies the next bottleneck as data scale, runtime,
  representation, or inconclusive.

Out of scope: 50M generation and Feature V2 implementation.

## Phase 10: Conditional 50M/100M Scale Confirmation

Assign this phase to one experiment agent only if Phase 9 identifies data scale
as the remaining bottleneck.

### Scope And Gate

Run 50M positions with four seeds. Proceed to 100M with at least two seeds only
if held-out loss still improves with unique data, 50M improves handcrafted Elo
over 10M under the same policy, or seed variance narrows in a way consistent
with data limitation.

Use the Phase 9 promotion protocol unchanged. Do not scale merely because the
current model remains below zero Elo. The agent must end with a promote/stop
decision and a learning-curve plot or table covering 1M, 10M, and 50M/100M.

Out of scope: data-policy, teacher, architecture, and runtime code changes.

## Phase 11: One Feature V2 Experiment

This is a repeatable phase template, but each assignment selects exactly one
feature hypothesis and produces one implementation PR plus its controlled
experiment. Do not combine candidates.

Eligible hypotheses:

- receiver square x effective piece type;
- donor-to-receiver relative geometry;
- king-zone donor pressure;
- extra capacity for tactical donor relations, with compatible base weights
  initialized from the best v0.6 model where possible.

Compare against the best prior pipeline using identical data, teacher, seeds,
training budget, kernels, and openings. Report model size, latency, NPS,
validation metrics, and fixed-time Elo. A feature is retained only if its
fixed-time gain survives the same promotion protocol; otherwise revert that
feature implementation while preserving its result document.

## Required Artifacts

Every decision-making run preserves:

- source/trainer commits, full config, and hashes;
- dataset/shard manifests and audit JSON;
- opening suite/hash or policy parameters;
- training, data, split, and shuffle seeds;
- logs and checkpoint-to-NNUE mapping;
- per-game JSONL, aggregate JSON, pentanomial counts, Elo, and interval;
- nodes, qnodes, NPS/QNPS, depth, elapsed time, and runtime target;
- model hash and verification report;
- a Markdown result summary under `docs/`, including failed lanes.

## Non-Goals

- Do not revert to plain `HalfKAv2^`.
- Do not replace incremental evaluation with full refresh.
- Do not use the weak NNUE as rollout or label teacher.
- Do not promote from one seed or a 100-game match.
- Do not compare headline Elo across different openings, budgets, or runtimes.
- Do not jump to hundreds of millions of positions before 1M/10M gates identify
  data scale as the bottleneck.
