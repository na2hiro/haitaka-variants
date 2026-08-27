# Anhoku NNUE Handcrafted-Strength Execution Plan

- Status: Phase 8D-A.2 adaptive-label recovery implemented; v3 trajectory, cross-host, and calibration gates are next
- Created: 2026-08-17
- Last checked: 2026-08-27
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

Phase 8B trained seed 80 on matched 50,000-node root and qsearch-leaf data.
Against C/16, root scored `+3.73 Elo`, CI `[-7.54, +15.01]`, and leaf scored
`+1.70 Elo`, CI `[-9.89, +13.28]`, in 1,024 games each. Both narrowly pass the
written `-10 Elo` non-inferiority floor, but neither establishes an improvement.
Against handcrafted, root remained at `-115.73 Elo`, CI
`[-137.00, -94.45]`, and leaf at `-130.30 Elo`, CI
`[-151.55, -109.06]`. Root is the provisional Phase 8 policy because it has
the better strength point estimates and retains more matched candidates, not
because root-over-leaf superiority has been proven. See the
[Phase 8B result](../docs/nnue-training-anhoku-v0.6-phase8b.md).

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

Phase 6 SIMD is complete on `strengthen`; it accelerated scalar NNUE inference
but did not make NNUE faster than handcrafted in real games. In the Phase 8B
root-versus-handcrafted match, root recorded 17,272 main-search NPS and
handcrafted 34,875 NPS over nearly equal elapsed time. Phase 8B therefore
satisfies the earlier condition for one literal equal-node diagnosis: its
result will decide whether profiler-guided runtime work or data-policy work is
next. Do not generalize that authorization into an open-ended performance or
equal-node campaign.

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
- retain every candidate `.ckpt` until that seed's strength selection is final;
  after selection, permanently preserve the selected winner `.ckpt` together
  with its exported `.nnue`, even when storage-saver cleanup removes rejected
  checkpoints;
- treat the winner `.ckpt` as the full-precision continuation artifact. An
  exported quantized `.nnue` is not a substitute for it and must not be the
  only surviving warm-start source;
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

## Review Checkpoint (2026-08-24)

Phases 1–8A are complete on `strengthen`, and the Phase 8B seed-80 measurement
is complete with documented protocol exceptions. Further general engine
optimization is not the next assignment.

Phase 8B established:

- root and leaf used identical attempted-candidate identities, and their train
  and OOD-v2 incomplete-label rates were below 1%;
- both policies were statistically non-inferior to C/16 under the written
  `-10 Elo` floor, but neither was significantly stronger than C/16;
- qsearch-leaf validation loss was lower, but it did not predict fixed-time
  strength and is not comparable to root loss because leaf filtering changes
  the dataset and target distribution;
- root is the provisional continuation policy. Its preference over leaf is
  weak, but it is the better resource allocation because both strength point
  estimates are higher and it retains 256,725 rather than 236,555 accepted
  records from the matched candidates; the later GR2 audit showed those files
  contain heavily repeated deterministic trajectories, so they are not unique
  position counts;
- root still loses decisively to handcrafted, so 10M generation and default
  promotion are not authorized;
- SIMD improved NNUE relative to its scalar path, but root still searched only
  17,272 main NPS versus handcrafted's 34,875 in the decisive fixed-time match;
  fixed-time Elo therefore does not isolate evaluator quality;
- the 40-game root OOD-v2 split is not selection quality: it contains only 9
  of 12 reserved IDs, has 38.17% black positions, and has 64.99% wins among
  decisive outcomes;
- the run used more checkpoint-ranking and handcrafted games than planned,
  while no preserved tactical-suite report or winning full-precision `.ckpt`
  is present locally. These are acceptance exceptions, not reasons to rerun
  completed strength games.

### Intervention priority after Phase 8R

| Priority | Intervention | Expected strength upside | Confidence in decision value | Decision |
| ---: | --- | --- | --- | --- |
| 0 | Literal equal-node root-vs-handcrafted diagnosis | None directly | Complete | Phase 8R proved an evaluation-quality deficit: `-36.78 Elo [-51.30, -22.26]` |
| 1 | One searched-stochastic rollout policy with a packed-board uniqueness gate | Moderate to high | High | Run Phase 8D now; deterministic depth-1 trajectories collapsed 1.115M records to only 6,832 boards |
| 2A | Root-only 1M, three equal-budget seeds, direct 1M-vs-262k match | Small to moderate after diversity repair | Medium-high after Phase 8D | Return to Phase 8C only with a selected rollout and a genuinely unique 262k prefix |
| 2B | One profiler-selected NNUE runtime hotspot | Potentially material after quality approaches parity | Medium-high after a Phase 8C equal-node gate | Defer Phase 8P until the 1M winner is within `-10 Elo` non-inferiority at equal nodes while still losing at 100 ms |
| 3 | Same-policy 10M | Potentially moderate, currently unsupported | Low before repaired Phase 8C | Allow only after a reproducible unique-data 1M scale gain while equal-node quality remains the bottleneck |
| 4 | One Feature V2 relation | Potentially material | Medium-low before the data-policy diagnosis | Defer until Phase 8D fails or Phase 9/10 identifies representation limits |
| 5 | More qsearch-leaf, label-node, LR, or lambda lanes | Low on current evidence | High confidence to deprioritize | Do not schedule without a new falsifiable hypothesis |

This ordering distinguishes confidence that an experiment will answer the next
question from confidence that its model will be promotable. Phase 8R proves
that runtime optimization alone cannot make the current Phase 8B root stronger
than handcrafted. GR2 then proved that the supposed 262k/1M data scale was not
real: the deterministic policy repeated only a few thousand boards. Repair and
validate trajectory diversity first, then test 262k and 1M unique-data scale;
repeat the fixed equal-node diagnostic only after a reproducible stronger model
exists.

## Completed Assignment: Phase 8R-A Equal-Node Support

**Execution status (2026-08-25):** Phase 8R-A implementation, focused tests,
and documentation are complete. Phase 8R-B calibration and its 2,048-game
decision match are also complete. The equal-node result is
evaluation-quality-limited, so Phase 8P is skipped for the current 262k model
and the Phase 8C launch gate was next. The complete handoff and report are in
`docs/nnue-training-anhoku-v0.6-phase8r.md` and
`out/anhoku-v0.6-phase8r/decision/`.

Work only on `strengthen`. The following implementation-only scope was
completed before the diagnostic games. The games, 1M generation, and GPU rental
were not mixed into that implementation assignment.

1. Add a self-play `--nodes-per-move N` budget and matching `go nodes N` engine
   command. It must be mutually exclusive with fixed depth and movetime and
   use the exact `alpha-beta-plus-qsearch-v1` shared counter already used by
   fixed-node label search.
2. Apply the same fresh budget independently to every move and both evaluators.
   Preserve the last fully completed iterative-deepening result and define a
   deterministic legal fallback when depth 1 cannot complete.
3. Record requested and consumed budget nodes, alpha-beta nodes, qnodes, final
   completed depth, incomplete-iteration count, cap hits, elapsed time, NPS,
   and QNPS per side and per game. Include the node policy in report identity,
   conflict detection, resume, and merge checks.
4. Test CLI parsing and mutual exclusion, both evaluator modes, exact budget
   accounting, deterministic repetition, legal low-budget fallback, paired
   colors, report serialization, and resume/merge rejection on policy changes.
5. Write the Phase 8R-B handoff with source/model hashes and the calibration
   and match protocol below. Stop without examining match outcomes.

Phase 8R-A passes only when a match can enforce the same combined search-node
budget on NNUE and handcrafted and the machine-readable report proves it.

### Phase 8C Launch Gate After Phase 8R

Phase 8R established an evaluation-quality failure, so this launch gate is the
next assignment. Phase 8P remains deferred until a stronger model approaches
equal-node parity. Do not start 1M generation, rent a GPU, or launch Phase 8C
strength matches in this launch gate assignment.

1. Preserve and hash C/16 and both Phase 8B selected `.nnue` files. Recover and
   hash each selected step-16 `.ckpt` if the remote copy still exists. If it
   cannot be recovered, record the artifact loss explicitly; do not reconstruct
   or describe a quantized `.nnue` as the full-precision checkpoint.
2. Treat the Phase 8B audit table, dataset hashes, CPU-hours, unique counts,
   candidate identities, and protocol exceptions now in its result as the
   immutable closeout. Add actual rental time/cost only if it can be recovered;
   reuse the completed matches.
3. Run one versioned verifier/tactical suite against C/16 and both Phase 8B
   exports. A regression blocks the affected policy; loss alone cannot pass or
   fail this gate.
4. Define a deterministic stratified OOD-v2 generation mode in which all 12
   reserved IDs contribute the same number of color-swapped game pairs. Use at
   least 16 pairs per ID, report per-opening and macro-averaged loss, and keep
   this data out of training. Do not silently rebalance the production train
   set.
5. Version a root-only resumable 1M config. Require at least 1,048,576 unique
   accepted train records, rather than merely setting trainer `epoch_size` to
   1,048,576. Preserve the 50,000-node root teacher, rollout depth 1, C/16 warm
   start, LR `0.00015`, lambda `0.8`, and all Phase 8B identities that are not
   intentionally changed by extension.
6. Predeclare checkpoints, seeds 80/81/82, match openings, hashes, sequential
   stopping rules, and resource ceilings for the Phase 8C experiment below.

This launch gate passes only when the result closeout, tactical evidence,
stratified validation contract, unique-record target, and experiment config
are reviewable. Its agent must stop after writing the pass/block decision.

**Execution status (2026-08-25): PASS.** The closeout, hashes, recovered
checkpoints, verifier/tactical evidence, equal-pair OOD-v2 contract, accepted
record target, and Phase 8C config are reviewable in
`docs/nnue-training-anhoku-v0.6-phase8c-launch-gate.md` and
`out/anhoku-v0.6-phase8c-launch-gate/`. Production generation and the
multi-machine handoff were intentionally not started; the next assignment is
to freeze one source revision and run the recorded shard allocation.

### Phase 8C Generation-Recovery Gate

**Final status (2026-08-27): GR1/GR2 COMPLETE; DATASET REJECTED BEFORE
TRAINING.** The strict extension correctly assembled all 3,400 train and 39
validation shards. Train had 1,115,026 accepted records, but only 6,832 distinct
packed boards (and the same 6,832 distinct 72-byte records). The 571,952 GR2
extension records added zero boards beyond the predecessor union. No final
dataset or `READY.json` was published.

The same audit also found incomplete-label rates of 1.549% in train and 1.582%
in OOD-v2, above the original 1% criterion. This is a second gate failure, not
a transfer or identity failure. The detailed evidence and the two-assignment
recovery protocol are in
`docs/nnue-training-anhoku-v0.6-phase8c-generation-recovery.md`.

The per-game RNG was valid but did not affect moves: `opening_random_plies=0`
disabled its only random move branch, and `uniform-rollout-v1` deterministically
played the depth-1 best move. Repeated selections from 52 fixed train openings
and their color swaps therefore replayed the same trajectories. The changing
game seed altered only the every-second-ply sampling phase. More shards under
this policy cannot fix uniqueness.

The Phase 8B root file has the same defect: 256,725 records contain only 8,641
distinct packed boards. Its completed strength matches remain observations of
the trained model, but it is no longer a valid 262k unique-data scale control.

Preserve all GR2 shards as evidence, and preserve the exact recovery
implementation only on `archive/phase8c-gr2-20260827`. The clean branch removes
the predecessor, staged-publication, ready-marker, and distributed-continuation
special cases. Do not lower the unique gate, deduplicate and upsample the
records, extend the deterministic policy, or use them as a material prefix for
a replacement dataset. Phase 8D is now the immediate assignment. The former
2% incomplete-label exception applies only to the retired dataset and does not
automatically carry into the replacement policy.

### Local Distributed Generation Policy

Phase 8 data generation is CPU-bound and should run slowly across the user's
local machines. A Vast GPU is not required for the Phase 8C launch gate,
generation recovery, or data generation. Rent one only after the extended root
1M dataset and stratified OOD-v2 set have been merged, audited, marked ready,
and bundled for training.

The completed Phase 8B root/leaf generation used one frozen source commit, one
versioned OOD-v2 suite, and matched candidate identities verified through
`scripts/phase8_prepare.py check-matched`, but its deterministic trajectories
are retired. Every replacement Phase 8D/8C machine must use the exact selected
searched-stochastic config; do not use `--ignore-identity-mismatch`. Split a
common lane count across machines, for example:

- assign non-overlapping contiguous shard ranges from the versioned Phase 8C
  root config;
- record the source commit and config hash on every machine;
- copy each complete output directory back to one coordinator;
- merge without an identity override and verify every expected shard exactly
  once.

Generation is resumable, and a lane can later be subdivided because lanes cover
contiguous shard ranges. The exact commands and shard allocation belong in the
Phase 8C handoff after its config filename and game count are frozen.

Audit the merged train and OOD-v2 manifests before creating the pretrain
bundle. Preserve every per-machine shard directory until the merged hashes and
record counts have been verified.

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
Phase 8R: equal-node quality failure (-36.78 Elo; complete)
                                      |
                                      v
Phase 8C-GR1: strict predecessor + staged merge support
                                      |
                                      v
Phase 8C-GR2: 1.115M records -> only 6,832 boards; rejected
                                      |
                                      v
Phase 8D: searched-stochastic trajectory repair + unique 262k test
                 |                                  |
         success |                                  | failure
                 v                                  v
Phase 8C: unique 1M three-seed              Phase 11: one Feature V2
       + direct scale confirmation
             |                     |                         |
 equal-node  | quality parity,     |                         | scale flat/
 deficit +   | fixed-time loss     |                         | inconclusive
 scale gain  |                     |                         |
             v                     v                         v
Phase 9: conditional 10M     Phase 8P: one hotspot          Phase 11
candidate                         |
                                 +--> remeasure fixed time

Phase 8C fixed-time win --> separate promotion confirmation; do not scale by default

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

## Phase 8: Fixed-Node Policy Selection And 1M Scale Experiment

**Status: Phase 8B measured; root provisionally selected; Phase 8R completed
with an evaluation-quality failure; Phase 8C launch gate passed.** Execute the
launch gate, generation, training, and games as separate assignments because
they have different acceptance evidence. Distributed generation is the next
assignment and was not started in the launch-gate assignment.

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

**Status: measured with acceptance exceptions.** The historical protocol below
is retained so the deviations remain reviewable. The actual result used seed
80, selected step 16 in both lanes, and then compared each winner with C/16 for
1,024 games. Root and leaf passed the numerical non-inferiority gate, but
neither established a gain. Root alone advances provisionally. The automatic
selector exceeded the planned game budget, handcrafted context matches were
run early, the tactical report is absent, and the winning `.ckpt` files are not
present in the local artifact trees. Do not rerun completed matches merely to
make the execution resemble the plan.

- Generate a resumable prefix sufficient for approximately 262,144 accepted
  root records and its matched leaf records. Do not generate 1M yet.
- Train one deterministic seed for root and leaf, sequentially on one GPU.
- Save all checkpoints, but predeclare strength screens only near 65k, 164k,
  and 262k accepted positions. Offline ID/OOD-v2 loss and tactical tests may
  veto a checkpoint but may not name the winner.
- Once root and leaf winners are selected, copy each winning `.ckpt` and its
  `.nnue` to the persistent result artifacts before deleting any rental host.
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

### Phase 8R: Equal-Node Runtime/Quality Diagnosis

Phase 8B root lost `-115.73 Elo` at 100 ms while searching 17,272 main NPS
against handcrafted's 34,875. Phase 6 proved a scalar-to-SIMD speedup, not an
NNUE-over-handcrafted speed advantage. Phase 8R is therefore authorized to
separate evaluation quality from the observed runtime deficit before 1M data
generation.

Phase 8R-A is the implementation-only assignment defined above. Phase 8R-B is
a separate experiment assignment and changes no engine or match code.

#### Phase 8R-B calibration and match

**Calibration status (2026-08-25):** The prescribed 20k, 50k, and 100k
telemetry runs completed on this machine. 100k nodes/move is the smallest
candidate passing the 99.9% depth-1 completion gate for both evaluators.

**Decision status (2026-08-25):** The frozen 100k-node, 2,048-game match
completed with 904 NNUE wins, 1,120 handcrafted wins, and 24 draws. Paired Elo
was `-36.78`, with paired 95% CI `[-51.30, -22.26]`. The upper bound is below
zero, so Phase 8R is evaluation-quality-limited; skip Phase 8P for the current
262k model and proceed to the Phase 8C launch gate. The decision report is under
`out/anhoku-v0.6-phase8r/decision/`, and the full result is recorded in the
Phase 8R handoff.

The decision distribution had 342 fallbacks in 114,374 searched moves
(`0.299%`), above the calibration gate of 0.1%. They affected 42 games in 41
pairs and were concentrated in two 200-ply draws. A post-hoc sensitivity audit
that excludes every affected pair leaves 983 pairs and still gives
`-36.18 Elo [-51.05, -21.31]`. The predeclared full result remains primary;
the clean-pair result shows that fallback contamination does not change the
evaluation-quality classification.

- Use the Phase 8B root export
  `12865f59f28f6e26feffcfae2e76c576f8eb31891148a8a9c167b8b50aac972c`
  against handcrafted. Do not spend games on leaf or checkpoint selection.
- Calibrate candidate per-move combined budgets of 20,000, 50,000, and 100,000
  nodes on a fixed 32-pair telemetry set. Do not use wins, scores, or Elo to
  select the budget. Freeze the smallest budget for which, for both evaluators,
  at least 99.9% of moves complete depth 1, every move is legal, budget
  accounting is exact, and no protocol failure occurs. If neither passes,
  repair or recalibrate in a new reviewed assignment rather than silently
  changing the contract.
- After freezing the budget, discard the calibration openings from the
  decision match. Use a fresh predeclared opening seed, paired colors, four
  random opening plies, the Anhoku start SFEN, depth cap 64, and no concurrent
  generation, training, or unrelated CPU match load.
- Run exactly 2,048 decision games (1,024 opening pairs) and do not inspect
  outcome aggregates while the match is running. This avoids turning ordinary
  fixed-sample 95% intervals into unadjusted sequential boundaries. Preserve
  pentanomial bins, every per-game record, requested/consumed combined nodes,
  main nodes, qnodes, completed depths, fallbacks, cap hits, elapsed time, NPS,
  and warnings.
- Keep the completed Phase 8B 100 ms match as the fixed-time observation; do
  not rerun it for presentation. Equal-node Elo is diagnostic and cannot
  promote a default model.
- Write `docs/nnue-training-anhoku-v0.6-phase8r.md` with the calibration table,
  fixed budget identity, result, classification, resource use, and the exact
  next-phase decision.

Classify the result before further work:

- **runtime-dominant:** NNUE lower 95% CI is above `0 Elo`. Run exactly one
  Phase 8P profiler-selected hotspot assignment before Phase 8C.
- **evaluation-quality-limited:** NNUE upper 95% CI is below `0 Elo`. Skip
  runtime work for the tested model and proceed to the Phase 8C launch gate.
- **mixed/inconclusive:** the interval crosses zero at 2,048 games. Record both
  bottlenecks and proceed to Phase 8C; do not extend or alter the node budget
  without a new written boundary.

#### Phase 8P: Conditional profiler-selected runtime hotspot

Phase 8P is skipped for the Phase 8B 262k root because Phase 8R proved that
model has a material evaluation-quality deficit. It becomes authorized only
by either the historical runtime-dominant Phase 8R branch or a Phase 8C winner
whose equal-node lower 95% bound is above `-10 Elo` while its 100 ms upper 95%
bound remains below `0 Elo`.

Profile the model that triggered the phase under representative 100 ms match
positions and select exactly one measured hotspot outside the
already-vectorized affine kernels. Candidate areas include accumulator
copy/update/refresh and repeated evaluation work, but profiler wall share—not
this list—chooses the assignment.

- Preserve bit-exact evaluations and deterministic fixed-depth moves.
- Require at least 1.25x NNUE-side NPS on the representative replay set without
  a handcrafted, qsearch, verifier, tactical, or portability regression.
- Compare old and optimized binaries using the identical triggering NNUE in at
  least 1,024 paired 100 ms games. Retain the optimization only if its lower
  95% CI is above `0 Elo` and the profiler confirms the targeted cost fell.
- If Phase 8P was triggered by Phase 8C, rerun the fixed-time handcrafted
  diagnostic with the retained binary before authorizing Phase 9 or promotion.
  If it was triggered before Phase 8C, continue to the Phase 8C launch gate.
  A further runtime assignment requires new profile evidence and a written
  estimate that it can materially close the remaining fixed-time gap.

### Phase 8C: Root-Policy 1M Scale Confirmation

**Status: blocked on a successful Phase 8D unique-262k experiment.** This is a
root-only data scale experiment using the selected searched-stochastic policy.
It is intended both to produce a stronger model and to determine whether
quality scaling, representation, or runtime is the next bottleneck.

- Extend only the selected Phase 8D fixed-node root prefix to at least
  1,048,576 distinct packed-board train positions with at least 95% packed-board
  uniqueness. Do not reuse the retired deterministic Phase 8B/GR2 records and
  do not extend leaf data. Preserve the Phase 8D unique-262k root export as the
  scale control, but train all 1M runs fresh from immutable C/16.
- Train deterministic seeds 80, 81, and 82 at 1M. The Phase 8B 262k seed-80 run
  is a repeated-trajectory historical result, not a learning-curve point or a
  1M replication.
- Predeclare checkpoints near 262k, 524k, 786k, and 1M examples. Screen each
  unique NNUE hash once; byte-identical quantized exports are aliases, not
  separate candidates. Loss and tactical results may veto but not select.
- Screen checkpoints against C/16 with 64 paired games, then extend only one
  checkpoint per seed to 1,024 games. Require positive median paired Elo, at
  least two seeds with lower 95% CI greater than `-10 Elo`, and no stratified
  OOD-v2, verifier, tactical, or fixed-time NPS regression.
- Select the overall 1M root winner without using handcrafted results. Compare
  it directly with the Phase 8D unique-262k root export using paired 100 ms games.
  Start at 1,024 games and extend sequentially to at most 4,096. Declare scale
  success when the lower 95% CI is above `0 Elo`; declare scale failure when
  the upper 95% CI is below `+5 Elo`; otherwise record the result as
  inconclusive at the cap.
- Only after the reproducibility and direct scale gates pass, freeze the
  overall winner and run two handcrafted diagnostics on independent fresh,
  predeclared opening streams. Handcrafted outcomes may not change the winner.
- Run exactly 2,048 games (1,024 color-swapped pairs) at 100 ms. This measures
  target playing strength; do not inspect or stop on outcome aggregates before
  completion.
- Run exactly 2,048 games (1,024 color-swapped pairs) at the Phase 8R frozen
  budget of 100,000 combined alpha-beta-plus-qsearch nodes per move. Preserve
  the counting version and full telemetry. Audit each evaluator's fallback
  rate on the new trajectory distribution; if either exceeds 0.1%, retain the
  full predeclared result as primary and also report a clean-pair sensitivity
  analysis. Do not change the node budget after seeing outcomes.
- Use the scale gate and two diagnostics to choose the next bottleneck:
  - if the 100 ms lower 95% bound is above `0 Elo`, route to a separately
    predeclared promotion confirmation; do not generate 10M by default;
  - if the equal-node lower 95% bound is above `-10 Elo` but the 100 ms upper
    bound is below `0 Elo`, run Phase 8P on the selected 1M winner;
  - if direct scale succeeded and the equal-node upper bound remains below
    `0 Elo` without meeting the `-10 Elo` non-inferiority gate, evaluation
    quality remains limiting and Phase 9 may be authorized;
  - if reproducibility or direct scale fails or reaches its inconclusive cap,
    route to one Phase 11 representation hypothesis instead of Phase 9;
  - any other interval combination is mixed/inconclusive and requires a new
    written boundary; do not silently choose runtime work or 10M generation.

Record CPU-hours, GPU-hours, positions/second, label node distributions,
stratified per-opening and macro validation loss, tactical-suite results,
fixed-control Elo, direct 1M-versus-262k Elo, fixed-time handcrafted result,
equal-node handcrafted result, and NNUE NPS. Write one result document
containing the rejected leaf policy, the Phase 8D unique-262k scale control,
all three 1M seeds, and the next-bottleneck classification. Describe Phase 8B
as a historical repeated-trajectory comparator.

Acceptance criteria:

- the same non-teacher variables are verified by hashes, not assumed;
- the Phase 8C launch gate passes before production generation;
- Phase 8R completed with literal combined-node equality and selected the
  evaluation-quality branch; Phase 8P was skipped for the 262k model and is
  run later only if the Phase 8C dual diagnostic meets its trigger;
- distinct packed-board counts and duplicate rates are reported for train and
  OOD-v2; train has at least 1,048,576 boards and at least 95% packed-board
  uniqueness, while any repeated board with conflicting targets is reported;
- incomplete-label rejection counts and rates are reported for the replacement
  root 1M train and stratified OOD-v2 data and meet a gate frozen before
  production generation; the retired GR2 exception is not inherited silently;
- Phase 8B advances no more than one policy under its fixed-control gate;
- Phase 8C reports the median and every per-seed fixed-control result at the
  same 1M training budget;
- the direct scale-control match ends in success, failure, or capped
  inconclusive under its predeclared boundaries;
- when the reproducibility and scale gates pass, both 2,048-game (1,024-pair)
  handcrafted diagnostics finish on fresh predeclared openings and report
  complete fixed-time and combined-node telemetry;
- Phase 8P is authorized only by equal-node `-10 Elo` non-inferiority together
  with a statistically significant 100 ms loss;
- Phase 9 is authorized only by reproducible fixed-control improvement, direct
  scale success, no regression, and a remaining statistically significant
  equal-node evaluation-quality deficit;
- failed or capped scale evidence routes to Phase 11, while an unlisted mixed
  interval result stops for a new written boundary instead of defaulting to
  10M.

Out of scope: 10M generation, hyperparameter sweeps, feature changes, and
promotion of a default model.

## Phase 8D: Immediate Searched-Stochastic Trajectory Repair

**Status: in progress.** GR2 proved deterministic trajectory collapse:
1,115,026 accepted records contained only 6,832 packed boards. Phase 8D is no
longer conditional on a Phase 8C strength result; it must repair data diversity
before any further production labeling or training. This is one rollout-policy
experiment, not a teacher, lambda, LR, and feature sweep.

### Phase 8D-A: audit and label-free rollout launch gate

**Result status (2026-08-27):** the phase-independent packed-board
audit, semantic/schedule identity split, searched-stochastic rollout, label-free
trajectory audit, and matched label-calibration commands are implemented. The
audit uses two cycles of color-swapped pairs over every train/OOD opening and
gates new-board yield only after the first complete 64-ID cycle; calibration
rejects empty matched root sets. Searched-stochastic generation is regression
tested across job counts and shard lanes, and only `splitmix64-v1` is accepted.
Phase 8D-A.1 fixed the game-62 same-side-donor capture legality bug: captures
of active opposing donors now receive a post-move king-safety replay while
other moves retain the batched fast path. The exact position and dynamic
color-swap are regression tested, and the repaired game completed 180 plies
under per-ply validation. A second game-102 failure was a validator mismatch,
not an illegal move: under Anhoku, adjacent Kings are permitted when their
effective movements do not attack each other. Influence-variant SFEN validation
now uses effective attacks while standard Shogi keeps its unconditional
adjacency ban; exact-position move-generation and color-swap regressions cover
the repair. The failed pre-fix local/remote runs are invalid. The production
freeze and Phase 8D-B remain gated on rerunning the full configured telemetry,
publishing its JSON report, reproducing trajectory hashes across execution
layouts, and recording the pass/block decision. The repaired production audit
then passed: 34,210/34,492 packed boards were distinct (99.18%), the final
post-coverage tranche yielded 111.53 new boards/game, all 128 transformed
pairs matched, all 64 opening IDs had two pairs, and a 16-game `jobs=1` nubu
sample matched the local `jobs=0` trajectories, source identity, and policy
exactly. Keep `margin=80`, `temperature=40`, candidate limit 16, depth 1, and
`splitmix64-v1` fixed during label recovery; do not reopen rollout tuning.

The subsequent 128-game label calibration is blocked. It produced 126 roots
because both `anhoku-v2-048` trajectories ended after one ply, before the
ply-8 sampling origin. Across 50k, 100k, and 200k nodes, both splits had zero
incomplete and terminal labels, exact node accounting, and zero side rejection
delta, but every budget reported six train and two OOD-v2 mate labels. The
aggregate rejection counts were unchanged across budgets. The resulting
outcome rejection-rate deltas were 0.2222 train and 0.20
OOD-v2, above 0.05. More label nodes did not improve eligibility, so no budget
is selected and Phase 8D-B remains unauthorized.

- Implement a small phase-independent dataset audit that counts both distinct
  full 72-byte records and distinct packed boards from bytes `0..64`. Define
  the training minimum by packed boards. Also report duplicate multiplicity
  and repeated boards whose score, ply, or result fields disagree. Do not
  restore a Phase-8C-specific ready marker or trainer special case.
- Separate generation-semantic identity from schedule cardinality in the new
  design. A frozen policy may extend ordinary non-overlapping shard ranges
  without a cross-config predecessor exception, while any teacher, opening,
  seed, rollout, sampling, label-budget, feature, or ABI change must still
  produce a different semantic identity and fail a mixed merge.
- Rename or supersede `uniform-rollout-v1`; it is deterministic depth-1
  best-move rollout, not uniform sampling. Keep the old identity readable for
  historical manifests but do not allow it in new Phase 8D/8C production data.
- Implement one versioned searched-stochastic policy. At each rollout ply,
  score a bounded set of legal moves with cheap search, retain only moves
  within a frozen score margin of the best, and sample by a frozen temperature.
  Every policy parameter and RNG version belongs in resume/merge identity.
- Derive stochastic decisions from pair index and ply, use symmetry-canonical
  candidate ordering, and require the swapped game to reproduce the exact
  transformed move sequence. Different pairs must receive different random
  streams. `opening_random_plies` remains zero; unsearched uniform legal moves
  are not the production diversity fix.
- Add a label-free trajectory-audit mode so rollout policies can be calibrated
  without 50,000-node labels. On at most 4,096 games across all 52 train and 12
  OOD-v2 IDs, report distinct packed boards, uniqueness ratio, new-board yield
  by game-count tranche, trajectory hashes, selected-vs-best score gaps, game
  lengths/outcomes, paired symmetry, and rollout CPU cost. Complete at least
  two pairs per ID and apply the final-tranche yield gate only after the first
  complete 64-ID cycle so repeated-opening collapse is observable.
- Freeze exactly one margin/temperature using only legality, symmetry, cost,
  move-quality, and diversity telemetry. Do not use loss, NNUE training, or
  strength games to choose it. The frozen pilot must be deterministic across
  thread counts and shard partitions, have at least 95% packed-board
  uniqueness, and project at least 30 new boards/game through its final
  post-initial-coverage tranche.
- After freezing rollout, run one matched 128-game label calibration: one
  color-swapped pair from each of the 64 suite IDs, with identical candidate
  roots at 50k, 100k, and 200k combined label nodes. Freeze the smallest budget
  for which train and OOD-v2 each have at most 1% incomplete labels, exact node
  accounting, zero terminal/mate labels, and no material side/outcome rejection
  bias. Use telemetry only; do not inspect strength. If none passes, stop and
  define an adaptive-retry contract instead of accepting another post-hoc rate.

Phase 8D-A stops after source/config hashes, tests, telemetry, and the pass or
block decision are written. If no bounded near-best policy passes, do not fall
back to arbitrary opening moves; write a new trajectory hypothesis.

### Phase 8D-A.2: adaptive-label eligibility recovery

**Status: implementation complete; execution gates pending.** Candidate
eligibility is repaired without changing the frozen rollout policy, C/16
teacher, feature family, sampling cadence, or label-quality thresholds. The
implementation adds `anhoku-v3`,
`haitaka_learn.anhoku-v0.6-phase8d-a2.toml`, semantic identity v2, bounded
adaptive retry, explicit exhaustion/accounting telemetry, symmetry-coupled
calibration, and deterministic jobs/shard regressions. Do not begin 8D-B until
the v3 evidence below passes.

- Create `anhoku-v3` by replacing only the train opening currently named
  `anhoku-v2-048`. Select the replacement without label scores, loss, or
  strength results. It must be unique under the existing color-swap transform,
  parse legally in both orientations, and under the frozen rollout produce at
  least the eight matched candidate plies `8,10,...,22` in both orientations.
  Preserve the other 63 positions and the 12-ID OOD-v2 holdout boundary. The
  suite file and hash are new generation-semantic identity; no v2 shards may be
  reused.
- Implement one versioned `root-position-adaptive-retry-v1` policy. At each
  scheduled candidate ply, require exact node accounting, a complete search,
  both Kings, a non-terminal trace, and a non-mate score. A rejected attempt is
  counted by reason, side, outcome, opening ID, and root ply, does not consume
  the accepted-position quota, and advances to the next ordinary sampling ply.
  Calibration permits at most eight attempts for one accepted root per game;
  exhaustion is explicit rather than silently dropping the game. All retry
  limits and the policy version belong to semantic identity and shard manifests.
- Keep base/swapped retries symmetry-coupled in calibration: the accepted roots
  must use the same ply, be exact color-swapped boards, and either both pass or
  both retry. Add exact regressions for a first-attempt mate followed by an
  accepted root, retry exhaustion, and jobs/shard determinism.
- Rerun the full v3 trajectory audit because the suite hash changed. Require
  the existing 95% uniqueness, 30 new boards/game, complete opening coverage,
  and 128/128 symmetry gates. The cross-host `jobs=1` sample must include the
  replacement opening and at least one OOD-v2 pair; it need not duplicate all
  256 games.
- Recalibrate sequentially. Test 50k first on one color-swapped pair per all 64
  IDs. Run 100k only if 50k fails for incomplete search or node accounting,
  and 200k only for the same reason at 100k; mate/terminal attempts trigger
  position retry, not a larger budget. Select the smallest passing budget.
- A calibration pass requires exactly 128 accepted roots, at least one matched
  accepted pair from every opening ID, zero accepted incomplete/terminal/mate
  labels, exact accounting, zero exhausted games, exact accepted-root symmetry,
  and no missing requested slots by side or outcome. Report raw retry bias as
  telemetry, and cap cost at a mean of 1.25 attempts per accepted root overall
  and 1.50 in either split. This replaces the old rejection-rate-delta gate only
  because rejected slots are now deterministically replaced; it does not relax
  admissibility of stored labels.
- If no budget passes by 200k, or if retry cost/coverage fails, stop and review
  the position policy or opening source. Do not relax the thresholds, inspect
  strength, or begin Phase 8D-B data generation.

### Phase 8D-B: unique-262k strength test

- Use the Phase 8D-A frozen root-label budget and keep C/16 bootstrap, features,
  lambda, LR, sampling, train opening groups, and OOD-v2 groups unchanged.
  Regenerate both train and OOD-v2 under the frozen stochastic policy; none of
  the deterministic Phase 8B/GR2 records count toward the minimum.
- Generate until there are at least 262,144 distinct packed train boards and
  the final dataset remains at least 95% unique. Predeclare a record ceiling
  from the label-free yield before generation. Report full-record and board
  counts and all incomplete-label/balance telemetry before training.
- Train seed 80 from immutable C/16 and preserve every checkpoint and unique
  export. Loss and the repaired OOD-v2 set may veto but may not select among
  checkpoints; use the fixed C/16 screen for selection.
- Compare the selected candidate directly with the Phase 8B repeated-trajectory
  root export in paired 100 ms games, starting at 1,024 and extending to at most
  4,096 under a written sequential boundary. This comparison measures the
  combined benefit of repaired trajectory policy and effective data size; it
  is not described as a pure 262k scale test.
- Retain the policy only when the lower 95% bound versus the Phase 8B root is
  above `0 Elo`, the point estimate versus C/16 is positive, and there is no
  verifier, tactical, OOD-v2, NPS, or material generation-cost regression.
- On success, use this unique-262k dataset/export as the new Phase 8C prefix and
  scale control, then run the three-seed unique-1M protocol. On failure, stop
  production rollout tuning and review the trajectory telemetry before
  authorizing one Phase 11 feature hypothesis. An inconclusive match may be
  extended only under its written boundary; do not try several temperatures
  against strength outcomes.

This phase has higher expected value than more label nodes or runtime work:
the current pipeline effectively trained on fewer than 10k boards, so position
diversity is now the proven first-order bottleneck.

## Phase 9: 10M Promotion Candidate

This is one end-to-end experiment-agent assignment using the single policy
selected in Phase 8. It is conditional: do not start it merely because Phase 8
finished.

### Scope

- Require the selected 1M policy to improve reproducibly over C/16, win its
  direct 1M-versus-262k scale match, retain a statistically significant
  equal-node evaluation-quality deficit, and avoid OOD-v2, tactical, verifier,
  and NPS regressions. It must be the Phase 8D searched-stochastic policy that
  subsequently passed the full three-seed repaired Phase 8C protocol.
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
- the selected winner `.ckpt` for every completed seed, its matching `.nnue`,
  both SHA-256 values, and enough trainer metadata to reload the checkpoint;
- a local archive containing those winner pairs before a remote instance is
  stopped or destroyed; verify the archive checksum after transfer;
- generation CPU-hours, GPU-hours, game-hours, and projected-versus-actual cost;
- per-game JSONL, aggregate JSON, pentanomial counts, Elo, and interval;
- nodes, qnodes, NPS/QNPS, depth, elapsed time, and runtime target;
- for node-limited matches, the counting version and requested/consumed budget,
  incomplete iterations, fallbacks, and cap hits per side;
- model hash and verification report;
- a Markdown result summary under `docs/`, including failed lanes.

## Non-Goals

- Do not revert to plain `HalfKAv2^`.
- Do not replace incremental evaluation with full refresh.
- Do not use the weak NNUE as rollout or label teacher.
- Do not promote from one seed or a 100-game match.
- Do not promote from equal-node Elo; fixed-time play remains the target.
- Do not compare headline Elo across different openings, budgets, or runtimes.
- Do not spend handcrafted games on every checkpoint or every seed; first use
  the fixed C/16 control and advance only the overall winner.
- Do not repeat the Phase 7.1 LR/lambda matrix without a new hypothesis.
- Do not jump to hundreds of millions of positions before 1M/10M gates identify
  data scale as the bottleneck.
