# Anhoku NNUE Handcrafted-Strength Execution Plan

- Status: Phases 1–7.1 complete; Phase 8 blocked on its cost-controlled launch gate
- Created: 2026-08-17
- Last checked: 2026-08-22
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

Phase 7.1 repaired the invalid original Phase 7.1 match evaluation and selected
the warm-start lane C step 16 as the current experimental control. Its identity
is NNUE SHA-256
`049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0`.
It scored `+27.2 Elo` with paired 95% CI `[+0.2, +54.2]` in its 256-game
selection match against corrected v0.5.1. A separate 1,024-game seed-7104
confirmation scored `+7.8 Elo`, CI `[-5.9, +21.5]`, and passed the predefined
`-10 Elo` non-inferiority gate. It then scored `-128.4 Elo`, CI
`[-150.4, -106.3]`, against handcrafted in 1,024 games. C/16 is therefore the
control for further learning experiments, not a promotion candidate.

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

Phase 6 SIMD is complete on `strengthen`; do not schedule another general NNUE
inference optimization phase before Phase 8. In the repaired C/16 versus
handcrafted match, the NNUE recorded about 16,255 main-search NPS and
handcrafted about 42,931 NPS. This remains useful match telemetry, but training,
data generation, and statistically useful paired games are now the scarce
resources. Do not add a standalone equal-node match campaign or another
performance project unless a Phase 8 result would lead to a different decision
depending on whether evaluation quality or runtime is dominant.

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

Expensive experiment phases may be split into a launch-gate assignment, a
single-seed pilot assignment, and a confirmation assignment. Passing one does
not authorize the agent to start the next. This subdivision takes precedence
over completing an entire numbered phase in one rental.

## Compute-Budget Rules

Data generation, GPU training, and paired games are the limiting resources.
Apply these rules to Phase 8 and later experiments:

- reuse preserved datasets, checkpoints, NNUE exports, opening pairs, and match
  reports whenever their identity matches; never rerun for presentation;
- generate a resumable prefix first and extend it only after its policy passes;
- start with one training seed and advance only one data policy to additional
  seeds;
- predeclare a small checkpoint schedule for strength screening instead of
  matching every saved checkpoint;
- screen each unique NNUE SHA once and map byte-identical exports back to all
  checkpoints;
- use 64-game screens only for ranking, extend at most one checkpoint per lane
  to 256 games, and reserve 1,024+ games for an independently seeded final gate;
- compare every experimental seed with the fixed C/16 control first;
- run handcrafted matches only for the overall winner after it passes the
  fixed-control gate;
- stop a match at a predeclared CI or SPRT boundary; reaching a maximum game
  count is not mandatory when the decision is already determined;
- do not run CPU matches concurrently with data generation or GPU training;
- record generation CPU-hours, GPU-hours, game-hours, and every early-stop
  decision in the result artifact.

## Review Checkpoint (2026-08-22)

Phases 1–6 are complete on `strengthen`. This includes fixed-node teacher
labeling, qsearch-PV leaf records, and vectorized NNUE inference. Phase 8
preparation and its 20,000/50,000-node data pilots are also present. Further
general engine optimization is not the next assignment.

Phase 7.1 established:

- the original 41 Phase 7.1 match reports are invalid because the overloaded
  matcher produced zero-node games and insufficient opening diversity;
- the repaired matcher passed calibration at 20 workers and all repaired
  screens, extensions, confirmation, and handcrafted matches had zero
  zero-node sides and no protocol failures;
- lowering the fresh-start LR did not solve strength: B/14 and B/16 were about
  `-74` and `-72 Elo` against corrected v0.5.1 in 256-game matches;
- warm start is the supported recipe: C/16 passed independent v0.5.1
  non-inferiority, while fresh A/B remained weaker;
- changing lambda from `0.8` to pure-score `1.0` did not establish a gain;
  do not spend another training lane on lambda without new evidence;
- ID loss improved while paired strength did not track it reliably, so loss
  may guard against regression but cannot select the winner;
- C/16 still lost decisively to handcrafted, so Phase 7.1 did not satisfy the
  project promotion condition.

Phase 8 preparation established:

- at 50,000 label nodes, incomplete-label rejection passed in train (`0.29%`)
  but failed in the two-opening validation split (`1.39%`);
- qsearch-leaf side/outcome imbalance is a real opening-dependent trace-parity
  effect, not a packed-position orientation bug;
- counting accepted samples allows leaf rejection to request replacement roots
  and prevents an exact root/leaf A/B comparison;
- the two-opening holdout is too small for selection. Phase 8 remains blocked
  until a reviewed suite has at least 64 opening IDs with at least 12 held out.

## Immediate Next Assignment: Phase 8A Launch Gate

Work only on `strengthen`. Do not rent a GPU, start production generation,
train a model, or launch strength matches in this assignment.

1. Preserve C/16 as the immutable experimental control and verify its SHA-256
   shown above after transfer or extraction.
2. Expand and review the Anhoku opening suite to at least 64 IDs. Freeze at
   least 12 IDs as OOD-v2 before looking at model results; prefer multiple
   deterministic folds if one fixed holdout remains opening-sensitive.
3. Cap attempted candidate roots rather than accepted records, so terminal,
   mate, or incomplete leaf rejection cannot cause root/leaf candidate drift.
4. Avoid paying for the same 50,000-node teacher search twice. Prefer one
   deterministic generation pass that emits matched root and qsearch-leaf
   records from the same search trace. If separate output files remain, prove
   that both are derived from the same candidate/search identity.
5. Run only a bounded, sequential root/leaf re-pilot over the expanded suite.
   Keep 50,000 nodes initially; raise the budget only if the broader OOD-v2
   pilot still exceeds the 1% incomplete-label gate.
6. Demonstrate and record deterministic trainer/data seeds for the later
   single-seed pilot. Do not train in this assignment.
7. Update the Phase 8 preparation result with data quality, generation rate,
   projected 262k/1M cost, and a pass/block decision for Phase 8B.

Phase 8A passes only when root and leaf have identical candidate identity,
train and OOD-v2 satisfy their written quality gates or have a reviewed
policy-specific exception, and the projected compute cost is recorded. Its
agent must stop after writing that decision.

Common completion gate for implementation phases:

- run `cargo fmt` and the focused package/feature tests;
- run relevant compatibility or workspace tests for the changed boundary;
- run `git diff --check`;
- leave unrelated cleanup and the next numbered phase untouched;
- report acceptance criteria individually as passed, failed, or not run.

Phase 6 SIMD is complete and must remain enabled in all fixed-time evaluation
binaries. Do not reopen it without profiler evidence from a selected Phase 8
model.

## Dependencies

```text
Phases 1-6 complete --> Phase 7/7.1 repaired control C/16
                                      |
                                      v
Phase 8A: OOD-v2 + matched-data gate (no training)
                                      |
                                      v
Phase 8B: 262k one-seed root/leaf pilot
                                      |
                                      v
Phase 8C: selected-policy 1M confirmation
                                      |
                                      v
Phase 9: conditional 10M promotion candidate

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

**Status: complete, with Phase 7.1 repaired evaluation complete.** C/16 is the
experimental control; Phase 7 produced no promotable model.

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

**Status: blocked on Phase 8A.** Execute the launch gate, pilot, and confirmation
as separate assignments because generation, training, and games are the
bottleneck.

### Scope

After Phase 8A passes, compare:

- depth-3 root labels from Phase 7;
- 50,000-node root labels, using the re-pilot common budget;
- 50,000-node qsearch-leaf labels.

Use C/16 as the common warm start and fixed-strength control. Retain lane C's
`0.00015` initial LR and `lambda = 0.8`; do not reopen the failed lower-LR or
lambda sweep. The Phase 7 depth-3 data remains an observational control and is
not regenerated.

### Phase 8B: 262k Single-Seed Pilot

- Generate a resumable prefix sufficient for approximately 262,144 accepted
  root records and its matched leaf records. Do not generate 1M yet.
- Train one deterministic seed for root and leaf, sequentially on one GPU.
- Save all checkpoints, but predeclare strength screens only near 65k, 164k,
  and 262k accepted positions. Offline ID/OOD-v2 loss and tactical tests may
  veto a checkpoint but may not name the winner.
- Screen each of those six unique candidates against C/16 with 64 games using
  the same opening pairs. Extend only the best root and best leaf checkpoint to
  256 games.
- Advance at most one policy. It must have no verifier/tactical/OOD-v2
  regression, a positive 256-game point estimate against C/16, and paired 95%
  CI lower bound greater than `-10 Elo`.
- Do not play against handcrafted and do not train more seeds in Phase 8B.
- The Phase 8B fixed-control budget is at most 896 games: six 64-game screens
  plus two 256-game lane extensions. Stop sooner when a lane is vetoed.

If neither policy passes, stop Phase 8. Do not add seeds, increase the dataset,
or tune LR/lambda on the same labels. Use the result to choose one new teacher,
rollout, or representation hypothesis.

### Phase 8C: Selected-Policy 1M Confirmation

- Extend only the selected Phase 8B policy from its preserved prefix to 1M
  records; do not generate the rejected policy to 1M.
- Train two additional deterministic seeds, giving three total for the selected
  policy. Continue to use C/16 as both warm start and fixed match control.
- Use the same predeclared checkpoint schedule and 64-game screens. Extend only
  one checkpoint per seed to 256 games against C/16.
- Require positive median paired Elo versus C/16, at least two seeds whose
  lower CI is greater than `-10 Elo`, and no OOD-v2/tactical regression.
- Only then run the overall winner against handcrafted: start with 256 games
  and extend with a fresh opening seed to 1,024 only while its interval leaves
  the Phase 9 decision unresolved. This is a policy-selection result, not a
  promotion match.
- Equal-node games are optional diagnosis after the winner is known; omit them
  when they would not change the Phase 9 decision.
- Excluding an explicitly approved inconclusive continuation, Phase 8C may add
  at most 1,920 games: six 64-game screens for the two new seeds, two 256-game
  seed extensions, and up to 1,024 handcrafted games for the overall winner.

Record CPU-hours, GPU-hours, positions/second, label node distributions,
validation loss, tactical-suite results, fixed-control Elo, the conditional
handcrafted result, and NNUE NPS. Write one result document containing the
rejected policy and all seeds actually run.

Acceptance criteria:

- the same non-teacher variables are verified by hashes, not assumed;
- Phase 8A's OOD-v2 and matched-candidate gates pass before training;
- incomplete-label rejection counts and rates are reported for both new lanes
  and remain at or below 1%;
- Phase 8B advances no more than one policy under its fixed-control gate;
- Phase 8C reports the median and every per-seed fixed-control result;
- qsearch leaves advance only if they beat root under the same staged criteria;
- the result chooses exactly one data/label policy for Phase 9.

Out of scope: 10M generation, hyperparameter sweeps, feature changes, and
promotion of a default model.

## Phase 9: 10M Promotion Candidate

This is one end-to-end experiment-agent assignment using the single policy
selected in Phase 8. It is conditional: do not start it merely because Phase 8
finished.

### Scope

- Require Phase 8C to improve reproducibly over C/16 and to improve the
  handcrafted point estimate without an OOD-v2 or tactical regression.
- Extend the selected resumable dataset to 10M unique positions and audit it.
- Train two initialization seeds first. Run the remaining two only if at least
  one candidate can still satisfy promotion or seed variance prevents a
  decision.
- Use paired fixed-anchor matches to select one checkpoint per seed.
- Use equal-node matches only when they change the diagnosis.
- Use 100 ms paired matches against handcrafted for the promotion decision.
- Confirm a winning candidate with at least 2,048 paired games, extending the
  match while its confidence interval crosses zero.

SPRT is acceptable when hypotheses, alpha, beta, and maximum games are written
into the result artifact. Record pentanomial pair bins for every match.

Acceptance criteria:

- every seed and rejected candidate actually run remains visible in the result;
- if an early gate stops after two seeds, preserve both and document why seeds
  three and four were not run;
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

Run 50M positions with two seeds first. Add seeds three and four only if the
first two preserve the Phase 9 gain and more replication can still change the
scale decision. Proceed to 100M with at least two seeds only if held-out loss
still improves with unique data, 50M improves handcrafted Elo over 10M under
the same policy, or seed variance narrows in a way consistent with data
limitation.

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
- generation CPU-hours, GPU-hours, game-hours, and projected-versus-actual cost;
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
- Do not spend handcrafted games on every checkpoint or every seed; first use
  the fixed C/16 control and advance only the overall winner.
- Do not repeat the Phase 7.1 LR/lambda matrix without a new hypothesis.
- Do not jump to hundreds of millions of positions before 1M/10M gates identify
  data scale as the bottleneck.
