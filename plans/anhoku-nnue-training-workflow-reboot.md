# Anhoku NNUE Training Workflow Reboot

- Status: Phase R0 implementation complete; gate pending the frozen production execution specification
- Created: 2026-08-31
- Last audited: 2026-09-01
- Last implementation update: 2026-09-01
- Primary ruleset: Anhoku
- Final target: a production NNUE that is stronger than the handcrafted evaluator at the target fixed time
- Historical record: `plans/anhoku-nnue-handcrafted-strength-plan.md`

## Decision

Pause the old plan's Phase 12-A representation work. Preserve the old plan and
all of its artifacts as experiment history, but supersede its future-execution
routing with this plan.

The next problem is not another feature hypothesis. The repository has not yet
run the experiment that would justify that conclusion: a diverse corpus labeled
by an independently selected handcrafted-search teacher, followed by
deterministic training to convergence and an end-to-end checkpoint/export/runtime
parity check.

The strongest current evidence instead points to a workflow failure:

1. `paths.bootstrap_nnue` selects both the training initialization and the data
   generation teacher. The Phase 8D corpus therefore used C/16 for trajectory
   search and for 50,000-node labels, then used the same C/16 network to
   initialize the student.
2. The latest deployment-size network has about 80.5 million trainable
   parameters, but the retained recipe performs only 16 optimizer updates over
   262,144 accepted examples at a learning rate ten times below the trainer's
   original default.
3. Random record skipping is seeded from `std::random_device`, validation is a
   cyclic stream that turns a 3,218-record file into seven 16,384-record batches
   (114,688 evaluated examples) for a requested size of 100,000, and the same
   random skipping is applied to validation.
4. Max-ply and ongoing games are encoded as draws. The latest train file is
   54.7% draw-labeled, yet 20% of the training objective consumes those game
   results.
5. Export verification loads 14 fixtures and runs one search smoke. It does not
   prove checkpoint-to-serialized-to-Rust prediction parity or bound the
   quantization loss.
6. The in-process timed search returns no move when depth 1 is interrupted, and
   self-play awards an immediate loss. This ended 330/1,024 games in the Phase
   8D candidate-versus-handcrafted benchmark, 381/1,024 games in the Phase 8D
   retention match, and 1,822/4,096 Phase 11 V2-versus-V1 games.
7. Node-budget self-play substitutes a lexicographically selected legal move
   when no iteration completes. Phase 8R contains 342 such fallbacks in 42/2,048
   games. Historical equal-node Elo is therefore suggestive, not clean.
8. Historical fixed-time matches ran many games concurrently—21 in the Phase
   8D handcrafted benchmark—while match identity omits resolved threads and
   hardware. A roughly 161 MB NNUE is much more cache/bandwidth sensitive than
   the tiny handcrafted evaluator, so this is not a production single-game
   runtime measurement.

Consequently, Phase 8D's training result is a weak-teacher self-distillation
result, not a test of the intended repaired handcrafted-teacher workflow. Its
fixed-time Elo is additionally contaminated by search forfeits. Phase 11 did
not establish a DonorReceiverPairV2 gain, but its fixed-time strength gate is
also non-decisional. None of this proves that feature geometry is the current
bottleneck.

## Final success contract

A model is promoted only when all of the following are true:

- The serialized network passes exact integer-reference versus Rust inference
  parity, including full-refresh and incremental paths.
- It passes the frozen verifier, tactical, portability, model-size, and latency
  gates.
- Timed and node-budget search always return the best legal root move found so
  far; the promotion match has zero missing moves, zero unsearched emergency
  fallbacks, and an explicit termination reason for every game.
- On the preregistered paired equal-node suite, the exported model's lower
  confidence bound versus handcrafted clears the frozen noninferiority margin.
  The versioned node-handicap curve and measured combined-throughput ratio must
  also place the remaining fixed-time tax inside its frozen compensable bound.
- At the production 100 ms/move target, its lower paired 95% confidence bound
  against handcrafted is above `0 Elo` on sealed openings.
- The result is reproduced by a second model trained with an independent data
  seed and training seed.
- No data, checkpoint, opening seed, or threshold used for final promotion was
  used to tune the recipe.

The final match remains the arbiter of strength. Offline metrics decide whether
a run deserves that match; they never promote a model by themselves.

## Audit findings

### What the existing evidence proves

- Historical fixed-time reports show large NNUE deficits, including roughly
  `-114 Elo`, but they are not valid strength estimates because many games ended
  as no-move search forfeits.
- Phase 8R suggests an equal-node evaluation-quality deficit at `-36.78 Elo
  [-51.30, -22.26]`, but 342 fallback moves contaminate that estimate.
- Historical telemetry shows a large throughput difference—about 17,272 NNUE
  main-search NPS versus 34,875 handcrafted NPS in one report—but the 21-game
  concurrent workload may disproportionately penalize the 161 MB NNUE. The
  production single-game tax has not been measured cleanly.
- Phase 11 V2 did not establish a gain under the matched training recipe. Its
  reported `-5.43 Elo [-11.77, +0.91]` is non-decisional because 1,822/4,096
  games ended on the no-move path.
- The early nominally large corpora were highly duplicated. The Phase 8B root
  file had only about 8,641 distinct boards.
- The Phase 8D corpus repaired board uniqueness: 279,627 records contain
  276,949 distinct packed boards.

### What has not been tested

- Diverse handcrafted-search labels plus converged training.
- A true 1M, 10M, 50M, or 100M unique-data learning curve under a fixed valid
  recipe.
- Trustworthy WDL mixing in which capped games are distinct from real draws.
- A calibrated Anhoku score transform rather than the inherited
  `sigmoid(score / 410)` transform.
- Whether DonorSingleEff is sufficient after data, optimization, export, and
  quantization are correct.
- Whether a smaller deployment network gives a better strength-per-millisecond
  tradeoff than the current roughly 161 MB network.
- A clean fixed-node or fixed-time comparison with zero fallbacks/search
  forfeits, controlled concurrency, complete match identity, and broad openings.
- A trustworthy baseline Elo for C/16, Phase 8D, or Phase 11 under the repaired
  harness.

### Severity-ranked code evidence

| Severity | Finding | Evidence | Consequence |
| --- | --- | --- | --- |
| Critical | Interrupted timed search can lose without moving | Iterative search retains only completed iterations; the movetime wrapper forwards `None`; self-play awards the opponent a win. Phase 8D candidate-versus-handcrafted has 330/1,024 such games and Phase 11 has 1,822/4,096 | Historical fixed-time Elo, retention, and checkpoint selection are non-decisional |
| Critical | Initialization and generation teacher are coupled | `Teacher::from_config` in `haitaka_learn/src/dataset.rs` selects NNUE whenever `bootstrap_nnue` is present; Phase 8D and Phase 11 configs set C/16 | The latest corpus is C/16 self-distillation, contrary to the intended handcrafted-teacher interpretation |
| Critical | Training is a 16-update short-horizon experiment | Phase 11 config: batch 16,384, `max_steps = 16`, LR `0.00015`; its log reports 80.5M trainable parameters and about 50 seconds of optimization | Lack of improvement cannot diagnose data scale or representation |
| High | Loader sampling is not reproducible | `random_fen_skipping = 3`; `lib/rng.h` seeds each thread from `std::random_device` | The nominal seed does not determine the accepted examples or their order |
| High | Validation is cyclic and randomly skipped | `SparseBatchDataset` defaults to `cyclic=True`; the 3,218-record validation file becomes 114,688 evaluated examples because the requested 100,000 is rounded to seven full batches | Validation values are repeated, stochastic, and mislabeled as stable evidence |
| High | Unknown results become draws | `dataset.rs` maps max-ply and ongoing outcomes to `GameOutcome::Draw`; the audited corpus contains 153,052 draw labels | The 20% result term teaches many fabricated draws |
| High | Rollout candidate selection is not MultiPV | Legal moves are string-sorted, truncated to 16, and only then searched, with the root best forcibly inserted | Position coverage is biased by notation order and one weak policy |
| High | Score transform is uncalibrated | Current data have absolute mean score about 2,649 and p95 about +5,924, while the trainer uses `score / 410` before sigmoid | Much of the target is saturated and contributes little score-resolution signal |
| High | Export verification is only a smoke test | `haitaka_learn/src/verify.rs` evaluates 14 fixtures and one optional search | Feature/sign/scale/export errors and quantization regressions can survive |
| High | Equal-node search uses arbitrary fallbacks | Node-budget search keeps only completed iterations and the CLI substitutes the first sorted legal move; Phase 8R has 342 fallbacks in 42 games | The reported equal-node quality deficit is suggestive but not a clean estimate |
| High | Match load and identity are not deployment-faithful | `selection.threads = 0` expands to all CPUs; match identity omits resolved threads/hardware; selection uses one SFEN plus four random plies | NNUE NPS and fixed-time Elo can be distorted or combined across unlike environments |
| Medium | Warm starts come from quantized exports | `.nnue` is deserialized into a new model; optimizer state and full-precision weights are unavailable | It is not equivalent to continuing a full-precision training run |
| Medium | Resume can mask a no-op run | Existing logs contain repeated resumes that immediately stop at `max_steps=16` | A stale checkpoint can be mistaken for a fresh experiment |
| Medium | Selection exports without enforcing promotion | Anchored ranking copies the highest noisy rating and launches a handcrafted benchmark without parsing it as a rejection gate | A model can be exported even when the global strength contract fails |
| Medium | Current input loses some piece identity | Packing/runtime features coalesce Gold, Tokin, promoted Lance/Knight/Silver, while handcrafted assigns Gold 500 and promoted minors 550 and future captures demote differently | Exact handcrafted cloning is structurally impossible; collision frequency and strength impact must be measured before proposing a feature fix |

### Reproducible audit anchors

- Teacher coupling: `haitaka_learn/src/dataset.rs:1003-1137` and
  `haitaka_learn/src/dataset.rs:5078-5160`; training initialization:
  `haitaka_learn/src/trainer.rs:285-353` and `:544-607`; C/16 config:
  `haitaka_learn.anhoku-v0.6-phase8d-b1-root-262k-extension.toml:4-7`.
- Training budget: `haitaka_learn.anhoku-v0.7-phase11b-seed80-v1.toml:52-70`;
  the 80.5M-parameter/16-step log is
  `out/anhoku-v0.7-phase11b-seed80-v1/logs/vast-train-step16.log`.
- Data counts and targets:
  `out/anhoku-v0.6-phase8d-b-root-262k/artifacts/phase8d-b1-final-train-audit.json`;
  validation counts:
  `out/anhoku-v0.6-phase8d-b-root-262k/artifacts/phase8d-b1-final-validation-audit.json`.
- Loader nondeterminism: `haitaka_learn/trainer_overlay/training_data_loader.cpp:844-862`
  and `../engine/variant-nnue-pytorch/lib/rng.h:7-10`; cyclic validation:
  `../engine/variant-nnue-pytorch/train.py:12-22` and
  `../engine/variant-nnue-pytorch/nnue_dataset.py:127-168`.
- Timed no-move path: `haitaka_wasm/src/lib.rs:1845-1915`,
  `haitaka_cli/src/main.rs:1023-1041`, and
  `haitaka_cli/src/main.rs:1927-1933`. In affected JSONL, a final failed search
  is exactly identifiable as `incompleteIterations == plies + 1`.
- Phase 8D no-move evidence:
  `out/anhoku-v0.6-phase8d-b-root-262k/artifacts/selection/matches/handcrafted-benchmark/7437671b62b61397/self-play-games.jsonl`;
  Phase 11 evidence:
  `out/anhoku-v0.7-phase11b-seed80-v2/artifacts/phase11b-gate/seed80/`.
- Node fallback path: `haitaka_wasm/src/lib.rs:1166-1229` and
  `haitaka_cli/src/main.rs:1122-1145`; Phase 8R counts:
  `out/anhoku-v0.6-phase8r/decision/self-play-report.json`.
- Match concurrency/identity: `haitaka_cli/src/main.rs:1226-1232` and
  `haitaka_learn/src/selection.rs:1579-1595`.
- Current smoke verifier: `haitaka_learn/src/verify.rs:37-95`.

The target conflict is not cosmetic. In the 256-game Phase 8D trajectory audit,
all 126 draws reached the 180-ply cap. In the final train file, 129,605 of the
153,052 stored draws have `abs(teacher_score) >= 410`. Only about 36% of
decisive records have teacher-score sign aligned with their eventual stored
result. Earlier corpora do not show the same degree of reversal, so this is not
enough to declare a universal sign bug; it is enough to require explicit
orientation tests and to remove the current result term until outcomes and
policy are trustworthy.

The concise causal description of the latest run is:

> A C/16-initialized 80.5M-parameter student received 16 low-learning-rate
> updates from roughly one stochastic exposure to a 277k-board corpus whose
> trajectory policy and 50k-node labels also came from C/16, while 20% of the
> objective included many capped-as-draw results; it was then selected and
> evaluated by fixed-time matches in which roughly one third to almost one half
> of games could end as no-move search forfeits.

That experiment is a config-matched diagnostic comparison of feature variants
under those exact conditions; because loader skipping and batch order were not
deterministic, it is not a matched-example causal ablation, and it established
no feature gain. It cannot establish the ceiling of NNUE training for Anhoku.

## Workflow invariants

These rules apply to every phase.

1. **Generation cannot read training initialization.** Use separate typed
   schemas and commands for data generation and training.
2. **Every artifact has an identity.** Hash code, executable, canonical config,
   input shards, evaluator, search budget, score transform, feature family,
   checkpoint, serializer, and seeds.
3. **Corpus size and training exposure are different quantities.** Report unique
   boards, records read, records accepted, optimizer updates, and equivalent
   full passes separately.
4. **Validation is finite.** Every validation record is consumed exactly once in
   a declared order; no cycling, filtering, or random skipping is permitted.
5. **Unknown is not draw.** Capped and unfinished games are excluded from WDL
   loss until represented explicitly.
6. **Full precision, serialized integer inference, and runtime inference are
   separate measurements.** Do not use one as a proxy for another.
7. **A small-data run is a plumbing and learnability test.** It is not required
   to establish final Elo significance.
8. **One causal variable per comparison.** Data policy, teacher, objective,
   initialization, architecture, quantization, and runtime changes get separate
   lanes.
9. **Development and promotion evidence are disjoint.** Sealed data and openings
   remain sealed until the declared promotion run.
10. **Negative results route only as far as they prove.** An inconclusive tiny
    run cannot authorize a new feature family.
11. **A legal root move survives interruption.** Missing moves are harness
    errors, not game losses, and arbitrary fallbacks invalidate a strength run.
12. **Match identity includes the execution environment.** Resolved threads,
    hardware, affinity, compiler, openings, adjudication, and search versions
    are part of the reusable result identity.

## Phase overview

| Phase | Question | Expensive generation/training allowed? | Exit evidence |
| --- | --- | --- | --- |
| R0 | Can initialization, trajectory policy, and label teacher be made independent and auditable? | No | Typed configs, manifests, registry, regression tests |
| R1 | Are board packing, features, sign, score, serialization, runtime inference, and interrupted search correct? | Debug scale only | Exact oracle/integer parity and zero-fallback harness reports |
| R2 | Is training deterministic, finite, resumable, and capable of learning? | At most one debug GPU-hour | Exposure/overfit/resume/validation gates |
| R3 | Which teacher, target, search budget, and score transform are defensible? | Probe corpus only | Deep-reference calibration protocol and frozen target |
| R4 | Can we produce broad, independent, cost-effective positions and immutable splits? | Up to 1M pilot positions | Audited pilot corpus and scale forecast |
| R5 | Does the existing feature family learn under a real schedule, and what width is deployable? | 1M pilot | Replicated learning curves and non-catastrophic equal-node result |
| R6 | Does strength improve with 10M, 50M, and 100M data? | Staged, gate by gate | Nested scale curve and replicated best recipe |
| R7 | Are remaining errors distributional, objective-related, quantization-related, or representational? | Targeted only | Hard-mining or one-factor architecture evidence |
| R8 | Can runtime close the measured fixed-time tax without changing scores? | No new labels | Bit-exact optimized inference and NPS curve |
| R9 | Does the final model beat handcrafted in production conditions? | Promotion matches | Two independent positive sealed results |

## Phase R0: separate identities and freeze history

### Question

Can a generation run be configured without any possibility that a training
warm start silently changes the trajectory or labels?

### Required changes

- Replace the shared `paths.bootstrap_nnue` meaning with three explicit types:

  - `training.initial_checkpoint`: `scratch`, full-precision checkpoint, or
    explicitly marked quantized-import diagnostic.
  - `generation.trajectory_evaluator`: evaluator kind, model hash if any, and
    trajectory-search parameters.
  - `generation.label_evaluator`: evaluator kind, model hash if any,
    label-search budget, target semantics, and score-transform version.

- Prefer separate generation and training config schemas. `generate-data` must
  not deserialize or inspect a training initialization field, and must reject a
  stray training-only field rather than silently ignoring it.
- Reject ambiguous legacy generation configs rather than guessing their intent.
- At the orchestration layer, add a regression test that varies a separate
  `TrainingConfig.initial_checkpoint` while holding one
  `CombinedDataGenerationConfig` fixed and proves byte-identical generated
  shards. The generator API itself must not accept a `TrainingConfig`.
- Use stage-specific manifests rather than one impossible all-purpose manifest:

  - transitional combined data generation, allowed only for historical import
    and R1-R3 debug/probe work: current generator executable/config, separately
    typed trajectory and label evaluators, seeds, and labeled output shards, but
    no training initialization;
  - training: trainer executable/config, input dataset-manifest hash,
    initialization/optimizer state, checkpoints, and export;
  - evaluation: model, harness, openings/suite, execution environment, and raw
    results;
  - composite registry record: immutable links among the stage manifests.

  R4 retires the transitional type for production data and adds distinct
  position-generation and labeling/dataset-assembly manifests with a lossless
  unlabeled-position artifact.

- Add an experiment registry. Each run records a hypothesis, changed variable,
  controls, cost ceiling, config hash, artifacts, outcome, and what the result
  does and does not prove. Every gate is machine-decisive: before results are
  opened, register its metric direction, baseline, minimum effect or
  noninferiority margin, uncertainty/decision rule, multiplicity handling, and
  cost ceiling.
- Mark historical datasets without deleting them:

  - Phase 8B: valid for duplicated-trajectory history; invalid for a
    diverse-256k claim.
  - Phase 8D/11: valid as a C/16 self-distillation and config-matched feature
    diagnostic in which no gain was established; invalid for a matched-example
    causal ablation or handcrafted-teacher claim.

- Mark historical match evidence without deleting it:

  - every in-process movetime match produced by the current no-move-loss path:
    non-decisional for strength, retention, or checkpoint selection;
  - Phase 8R equal-node match: diagnostic only because it contains 342 arbitrary
    fallback moves;
  - historical NPS: diagnostic only until reproduced at controlled single-game
    and declared parallel loads.

- Freeze the current C/16 network, current Phase 8D V1 candidate, handcrafted
  executable, match reports, and old plan by SHA-256.
- Freeze the target production execution specification before R5 width
  selection: shipped WASM interface and required complete search path, one
  concurrent game, supported device/host classes, clock policy, cold/warm state,
  and numerical ceilings for serialized bytes, peak memory, load time, and
  per-move latency. R1 repairs and qualifies the implementation; evaluation
  manifests later identify the actual artifact.

### Gate

R0 passes only when each stage manifest exposes exactly its own identities, the
composite registry links trajectory evaluator, label teacher, and initialization
without coupling them, a test demonstrates byte-identical generation after
changing only the separate training initialization, and historical
datasets/matches carry explicit claim-validity annotations. No new data run is
authorized before this passes.

### Implementation record (2026-09-01)

Implemented on branch `reboot`:

- Added strict, separate `CombinedDataGenerationConfig` and
  `TrainingWorkflowConfig` schemas. Generation commands reject training-only
  fields, legacy search fields, ambiguous label-on-sample behavior, and the
  identity-mismatch override.
- Added independently typed trajectory and label evaluators. They use separate
  teacher instances and search workspaces, so neither evaluator can inherit the
  training initialization.
- Added explicit `training.initial_checkpoint` variants for scratch,
  full-precision weights/resume state, and a hash-checked quantized-import
  diagnostic with an explicit transform version.
- Changed the public generation and training command paths to accept only their
  corresponding typed config. `pipeline` now requires separate generation and
  training config files. The ambiguous legacy `train-select` path is disabled.
- Added combined-generation, training, and evaluation manifest schemas plus a
  composite experiment registry with preregistered gates and immutable stage
  manifest links. Generation and training publish their stage manifests.
- Added historical claim-validity annotations for Phase 8B, Phase 8D/11,
  affected movetime matches, Phase 8R, and NPS evidence. Frozen R0 artifacts
  include C/16, the Phase 8D V1 candidate, the handcrafted executable,
  representative match reports, and the superseded plan, all checked by
  SHA-256.
- Added example strict generation and training configs and documented the new
  commands in `haitaka_learn/README.md`.
- Added an orchestration regression proving byte-identical train and validation
  shards when only a separate training initialization changes. Added strict
  schema, cross-stage-field, registry, and unresolved-production-policy tests.

CPU-only verification completed:

- `cargo check -p haitaka_learn --features anhoku`
- `cargo test -p haitaka_learn --features anhoku`
- focused R0 manifest/registry/production-policy tests
- focused byte-identical generation regression
- `cargo fmt --all -- --check` and `git diff --check`

Remaining before the R0 gate can pass:

- Freeze the product-selected shipped interface and complete history-bearing
  search path in `r0/anhoku-reboot/production-execution-spec.json`.
- Freeze supported host and device classes and the exact clock and cold/warm
  protocol.
- Freeze positive ceilings for serialized bytes, peak memory, load time, and
  per-move latency.
- Run `haitaka_learn r0-gate --bundle r0/anhoku-reboot` successfully after
  those values replace the deliberately rejected `PENDING_PRODUCT_DECISION`
  placeholders.

Until that contract is supplied and the gate passes, R1 and all new data runs
remain unauthorized. No GPU rental, strength match, or production data
generation was performed during R0 implementation.

### Cost ceiling

CPU tests only. No rented GPU, strength match, or production data generation.

### What passing proves

It proves experiment attribution is possible. It does not prove that any
teacher or model is strong.

## Phase R1: correctness ladder

### Question

Does the same legal position mean the same thing in Rust packing, the C++ data
loader, Python training, serialized NNUE inference, and the incremental runtime?

### R1-A: board, feature, and sign oracles

Build a deterministic corpus of at least 10,000 legal positions covering:

- both sides to move and color-swapped/rotated equivalents;
- captures, promotions, drops, king moves, hand-count boundaries, checks, and
  terminal-adjacent positions;
- donor activation, removal, replacement, and receiver movement;
- every reachable piece type and every output bucket.

For every position:

- require canonical packed equivalence,
  `pack(unpack(pack(board))) == pack(board)`, plus exact feature-signature
  equality. Exact decoded-board equality is intentionally not a gate because
  the current ABI coalesces Gold and promoted gold-like identities; report
  those expected identity collisions separately;
- dump active base and donor feature indices from Rust and the C++ loader and
  compare them exactly;
- test the declared color-swap and side-to-move score transformation;
- compare record label orientation with a hand-checkable depth-0/depth-1
  reference;
- compare incremental accumulator state with a full refresh after every move.

Use sentinel networks whose row weights encode the row identity. Output-only
tests are insufficient because two implementations can share the same mistaken
index mapping.

### R1-B: checkpoint/export/runtime parity

For a fixed parity corpus, record all three layers:

1. full-precision checkpoint prediction;
2. a Python integer emulation of the serialized network;
3. Rust full-refresh and Rust incremental predictions.

Required gates:

- serialized integer emulation versus Rust full refresh: zero score mismatches;
- Rust full refresh versus incremental: zero accumulator or score mismatches;
- repeat export of the same checkpoint: byte-identical network and metadata;
- full-precision-to-quantized degradation: measured by split and score bucket,
  with no hidden clamping or overflow.

Full precision is not expected to equal quantized inference exactly. The gate is
that the difference is measured and attributable. Before opening oracle
results, freeze absolute mean/tail score-delta and loss-degradation limits in
runtime units. Do not define success afterward as an unstable percentage of a
tiny full-precision gain.

### R1-C: learnability oracles

Train a small debug network on exactly representable targets:

- a constant target;
- side/sign target;
- a feature-representable material-only target whose values are defined on the
  current coalesced piece slots;
- individual sentinel feature rows.

Then run a tiny-set overfit test with the deployment feature code. On a fixed
8,192-position set, the full-precision loss must fall to at most 10% of its
initial value, and the serialized model must retain at least 90% of that loss
reduction while also passing the frozen absolute R1-B quantization limits.

Cloning the complete handcrafted static evaluator is a useful capacity
diagnostic, but not an exact correctness gate. The current input explicitly
coalesces Gold and promoted gold-like pieces even though handcrafted values and
future capture identities differ, and mobility may not be exactly representable
at the chosen width. Measure the irreducible error from feature collisions,
their frequency in real positions, relative clone error, calibration, and
residual strata rather than demanding exact scores. A frequent, high-regret
collision is direct evidence that may authorize one minimal identity-preserving
feature experiment later; its existence alone does not explain the current Elo.

### R1-D: interruption-safe search and honest self-play

Repair both timed and node-budget search before any model-selection or strength
game:

- Seed search with a legal root move, but distinguish an unsearched emergency
  move from a searched best-so-far move.
- Publish the best move after each completed root child so an interrupted
  iteration can return useful partial-root work instead of discarding the whole
  iteration.
- Keep the play and value contracts separate. A search result exposes
  `play_move_best_so_far` plus `last_completed_iteration_value`, completion
  depth, and partial-root metadata. A partial-root move may be legal for play,
  but its order-dependent partial value is never silently accepted as a
  training label. Label generation either uses the last fully completed value
  under its frozen target semantics or rejects the position as incomplete.
- Record completed iteration depth, completed root moves in the interrupted
  iteration, interruption reason, emergency-fallback use, and missing-move
  status for every search.
- Preserve full qsearch telemetry under node budgets; do not report only qnodes
  while zeroing qsearch depth, cap, check-move, and pruning counters.
- Define the node budget as the versioned combined count
  `alpha_beta_nodes + qnodes`. Tiny-budget tests must prove that consumed nodes
  never exceed the request, exact-exhaustion cases consume the declared budget,
  and in-process, USI, and production-interface paths agree without an
  arbitrary move fallback.
- A nonterminal missing move is a harness error that aborts the match. It is
  never scored as a loss.
- A promotion match requires zero missing moves and zero unsearched emergency
  fallbacks. Partial-root moves are permitted only under a frozen policy and
  must be reported separately by engine.
- Make in-process, USI, fixed-time, and node-budget behavior follow the same
  best-move contract.
- Replace `terminal_winner`'s generic non-ongoing-as-loss rule with explicit win,
  loss, and draw handling.
- Add the Anhoku repetition/perpetual-check adjudication required by the game
  rules, maintain game history in self-play, and record a versioned termination
  reason. Maximum-ply adjudication remains explicit and separate.
- Pass root game history into every search API and implement line-local
  repetition/perpetual-check detection inside alpha-beta; game-loop-only
  adjudication is insufficient because search can otherwise choose and value a
  repetition cycle incorrectly. Apply the same history contract in qsearch and
  root DFPN. Because identical boards can have different values under different
  repetition/perpetual-check histories, either add the relevant history context
  to TT identity or suppress TT probe/store at history-sensitive nodes. Add
  same-board/different-history TT tests plus golden tests for ordinary fourfold
  draw, the perpetual-check loser, terminal-on-final-max-ply precedence, and any
  applicable Anhoku entering-king rule.
- Root DFPN must use the same monotonic deadline/budget contract and expose its
  completion/interruption metadata. Unless it proves a terminal root result, it
  must not consume the entire move budget without allowing a searched legal root
  move to be published; freeze a reservation/interleaving policy and test
  deadline expiry inside DFPN.
- Add controlled tests with an artificially slow evaluator and tiny time/node
  limits that interrupt before depth 1, after one root child, and during later
  iterations.
- Freeze the monotonic clock and deadline-controller semantics. Report per-move
  requested time, elapsed time, overrun, cold/warm model state, and scheduler
  delay where measurable, along with completed-depth and completed-root-child
  distributions. Before strength play, predeclare numeric p95, p99, and maximum
  lateness limits plus an engine-symmetry/equivalence margin; any failed limit
  or systematic asymmetry invalidates the match even when every search returns
  a move.

Qualify the harness with enough independent opening groups that an
A-versus-identical-A paired interval lies wholly inside a predeclared zero-bias
equivalence margin. For distinct A and B, replay the identical pair schedule
with engine order reversed and require the sign-transformed effect estimates to
agree within a separate tolerance; the A/B strength effect itself is not
expected to be zero. Runs must have zero missing/emergency moves and pass the
time-overrun boundary. Match identity must include the resolved thread count,
CPU, affinity, compiler flags, opening-suite hash, adjudication version,
clock/controller version, search-limit version, and cold/warm protocol.

### Cost ceiling

Debug networks, at most 100,000 labels, and only harness-qualification null
games. No model-strength games.

### Failure route

Any exact-oracle failure blocks all later work. A synthetic target failure
routes to packing/features/sign/optimizer. A full-precision success followed by
serialized failure routes to quantization or serialization. Any missing move,
unsearched fallback, unexplained A=A bias, or incomplete match identity routes
to search/harness correctness; no historical match may be used as a substitute.

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

### Cost ceiling

At most one GPU-hour on debug data after CPU gates pass.

### What passing proves

It proves that a declared number of deterministic updates can learn a known
target and be resumed. It does not choose the production teacher or data scale.

## Phase R3: teacher, target, and score calibration

### Question

What value should the network learn, and what is the cheapest teacher that is
reliably close to a substantially deeper independent reference?

### Probe design

Freeze `teacher-probe-v1` before comparing teachers. It must be stratified by:

- opening source and trajectory policy;
- early, middle, late, and terminal-adjacent phase;
- material imbalance and teacher score bucket;
- quiet, tactical, in-check, promotion, drop, and donor-sensitive positions;
- positions where C/16 and handcrafted disagree.

Use a tuning subset to choose candidate budgets and a label-hidden, one-shot R3
decision subset. Once opened, that subset becomes development history; it is
not the final sealed promotion test.

### Teacher candidates

Compare on the same boards:

- handcrafted search at geometrically increasing node/depth budgets;
- a substantially deeper handcrafted-search reference;
- the current C/16 50k-node teacher as a diagnostic control only;
- an external stronger compatible teacher only if its rules, score semantics,
  executable, and license can be frozen and verified.

For each candidate report:

- calibrated probability error versus the deep reference;
- score correlation and robust score error;
- top-1/top-k move agreement and deep-teacher regret of the selected move;
- sign disagreements outside the near-zero region;
- mate, incomplete-search, clipping, and node-accounting rates;
- every metric macro-averaged across the probe strata;
- measured labels/second, nodes/label, projected wall time, and projected cost.

Budget stability by itself is not enough: two shallow searches can agree and
both be wrong. Select the smallest budget that reaches the predeclared
deep-reference error/regret boundary across all important strata.

### Target semantics

On identical positions, compare without changing the trajectory or filters:

- root backed-up search value;
- qsearch-stabilized root/leaf value;
- bounded non-mate value plus an explicit mate class/distance target.

The previous root/leaf result does not decide this because its compared corpora
and filtering were not fully matched.

### Score transform and WDL policy

- Audit the current `sigmoid(score / 410)` transform by score bucket. Report the
  fraction of targets below 0.01 and above 0.99 and the corresponding gradient
  contribution.
- Compare a calibrated logistic transform with a robust clipped-score objective
  on the development split. Freeze one primary objective before R4.
- Begin strength training with teacher-only loss (`lambda = 1.0`).
- Introduce a result term only in a later, separately controlled lane after the
  record format distinguishes terminal, repetition/adjudicated, and
  unknown/capped outcomes.
- Never use the current candidate NNUE as label teacher until it has independently
  beaten handcrafted. A candidate may later propose positions for relabeling by
  the frozen stronger teacher.

### Gate

R3 publishes a protocol before opening the one-shot R3 decision results, then
freezes:

- label evaluator and hash;
- label budget and node-counting version;
- target semantics and mate policy;
- score transform and loss;
- throughput/cost model.

If no affordable teacher meets the one-shot deep-reference boundary, improve
the teacher/search or reduce label cost under a newly preregistered decision
block. Do not compensate with more weak labels.

### Cost ceiling

No production corpus. Cap the one-shot teacher decision probe at 50,000 unique
boards and publish the scale forecast before R4.

## Phase R4: position generation and immutable data

### Question

Can the workflow produce broad, independent positions cheaply, then label each
unique board once with the frozen R3 teacher?

### Two-stage pipeline

1. **Generate positions.** This stage has no training initialization and no
   label search.
2. **Canonicalize, split, deduplicate, and label.** The fixed label teacher sees
   each retained unique board once. Labels are cached and immutable.

This separation allows a cheap trajectory policy and an expensive label teacher
without conflating their identities.

### Position-source mixture

Create a predeclared mixture of independent sources:

- true root MultiPV near-best sampling, or a deterministic mixture of searched
  near-best moves and uniformly sampled legal moves;
- diverse randomized openings and legal perturbations;
- handcrafted-versus-handcrafted games at more than one search budget;
- phase-balanced samples and terminal-adjacent samples;
- later, candidate-disagreement and blunder replay positions.

Remove the current lexicographic first-16 truncation. It is acceptable not to
search every legal move; use true MultiPV or uniform candidate sampling whose
inclusion probability is defined and audited.

### Corpus tiers

- `debug`: 64k positions for parity, overfit, and resume tests.
- `pilot`: 1M unique labeled boards for recipe and exposure curves.
- `broad`: nested 10M, 50M, and 100M prefixes under one frozen generator and
  split policy.
- `deep-core`: a smaller, more expensive subset relabeled at the deep-reference
  budget for calibration and later hard-example mixing.
- `static-clone`: cheap legal positions labeled by the exact handcrafted static
  evaluator, used only as an initialization/representation lane.

At 50,000 nodes per label, 100M labels require at least `5e12` search nodes
before rollout overhead. A scale plan that ignores this is not executable.
Measure whether broad cheaper labels plus a deep-core subset dominate uniformly
expensive labeling.

### Record and manifest requirements

For every board retain or derive:

- canonical board hash and color-swap canonical hash;
- source game, opening group, trajectory policy, ply, and phase strata;
- trajectory evaluator and label evaluator identities separately;
- label budget, completed/incomplete state, root move/PV where representable,
  raw score, target transform version, mate metadata, and label hash;
- result status: terminal win/loss/draw, repetition/adjudicated, or unknown/capped;
- active feature-row counts and output bucket.

A v2 record or loss sidecar may carry the richer metadata. The initial
teacher-only baseline may keep the current binary payload only if unknown
results are ignored and all provenance is losslessly available in the sidecar.

### Data gates

- No canonical or color-swapped board crosses training validation,
  development, any one-shot stage block, final sealed static/search data, or
  promotion-opening source groups.
- No opening/trajectory group crosses a grouped split.
- Duplicate weights are capped and effective sample size is reported. Do not use
  a rigid uniqueness percentage as a substitute for coverage at large scale.
- Report source, side, ply, phase, score, material, check/tactical, result-status,
  donor-state, bucket, and active-row distributions.
- Report feature rows never observed and rows below the declared minimum update
  count; distinguish unreachable rows from coverage failures.
- Capped games are `unknown`, never `draw`.
- The current roughly 35% mate-label rejection rate is explained and either
  repaired with bounded mate targets or explicitly balanced in the retained
  distribution.
- Regenerating any shard from its manifest produces the same board and label
  hashes.

### Cost ceiling

R4 stops after the 1M pilot and publishes actual throughput plus projected 10M,
50M, and 100M cost. R5 must pass before larger generation.

## Phase R5: fixed-data exposure and 1M workflow pilot

### Question

Can the existing DonorSingleEff family learn a reproducible signal when trained
for enough optimizer updates, and is the current width a sensible deployment
point?

### R5-A: exposure and learning-rate curve

Use the same 1M pilot data and a small number of predeclared schedules. Training
must run through multiple passes and checkpoints until development loss
plateaus; one pass at batch 16,384 is only about 61 optimizer updates.

Freeze:

- warmup, initial learning-rate candidates, step-based decay, minimum updates,
  maximum updates, and plateau rule;
- checkpoint cadence in optimizer updates;
- all offline selection metrics and tie-breaking;
- the maximum number of recipe lanes.

The 16-step LR result from Phase 7.1 must not be treated as a production LR
selection: short-horizon checkpoint selection rewards staying close to the
bootstrap, not converging to a new optimum.

### R5-B: initialization lanes

With architecture, data, objective, and schedule matched, compare at most:

1. from scratch;
2. static-clone full-precision pretraining followed by search-label fine-tuning;
3. a compatible full-precision checkpoint warm start, if one exists.

A quantized `.nnue` import may be retained as a diagnostic control but cannot be
the only baseline or the source of optimizer state.

Select the initialization lane on finite training validation and the repeatedly
usable development test. Do not open a one-shot stage block yet.

### R5-C: one compact deployment diagnostic

New feature geometry remains frozen, but compare the current deployment width
with one predeclared compact width under the winning recipe. Report:

- training-validation and development teacher fidelity;
- active-row coverage and capacity indicators;
- full-precision-to-quantized gap;
- serialized bytes and full-refresh/incremental latency;
- equal-node search quality.

This is a capacity/runtime diagnostic, not an open-ended architecture sweep.
Use a separate tiny debug width for R1/R2 oracles. Select one joint
recipe-and-width candidate by the frozen development rule, replicate it with
two additional training seeds, and only then open the one-shot R5 stage block.
That block becomes development history immediately after its pass/fail decision.

### Required offline outputs

For every checkpoint report:

- train and every held-out metric versus examples seen and optimizer updates;
- the same metrics for C/16, the untrained initialization, and handcrafted
  static evaluation where meaningful;
- paired uncertainty intervals on the R5 stage-block prediction deltas,
  resampling the highest independent source group rather than individual
  positions;
- deep-teacher move regret from the fixed search benchmark, reported separately
  for full precision and the exported integer model;
- full-precision and exported-network results separately;
- the absolute full-precision improvement, absolute quantization degradation,
  and—only above its frozen denominator floor—the fraction of full-precision
  gain retained after quantization.

### Game use at 1M

The 1M run is a workflow gate, not a final-strength gate.

- Run a modest predeclared equal-node match against C/16 after offline selection,
  using only the R1-qualified zero-fallback harness.
- Elo is a veto at this phase: a clearly material regression stops the recipe.
  Failure to obtain a positive lower 95% bound does not by itself block 10M if
  offline gains replicate and the match is not materially negative.
- Run one equal-node handcrafted diagnostic to establish the remaining gap. Do
  not run a full fixed-time handcrafted promotion campaign.
- Treat all old Elo values as historical context only; do not import their pair
  bins into the repaired match stream.

### Gate to R6

R5 passes when:

- training and development curves converge reproducibly rather than stopping at
  an arbitrary corpus boundary;
- all three seeds point in the improving direction and the multiplicity-adjusted
  grouped interval for the frozen primary full-precision improvement over C/16
  clears the preregistered materiality margin on the R5 stage block;
- the grouped upper bound on absolute quantization degradation is below its
  frozen runtime-unit margin. When the full-precision improvement exceeds the
  preregistered denominator floor, the grouped lower bound on retained gain
  must also be at least 95%; below that floor the ratio is not used;
- independent of that ratio, the exported integer model's grouped lower-bound
  improvement over C/16 clears its separately preregistered nonnegative
  improvement margin on the primary stage metric;
- exported-integer deep-teacher regret clears its preregistered improvement
  margin with no failed tactical/correctness gate;
- equal-node play shows no predeclared material regression;
- match telemetry has zero missing moves and zero emergency fallbacks;
- actual data and training costs support the next scale.

If full precision succeeds and serialization fails, route to quantization. If
training loss will not fall, route to R1/R2 or capacity. If IID improves but
independent-policy data does not, route to R4 rather than architecture.

## Phase R6: nested strength-scale learning curve

### Question

Does the frozen recipe continue to improve with genuinely new data, and at what
scale does equal-node and fixed-time strength saturate?

The Fairy-Stockfish trainer guidance says that data count and depth are the main
strength factors, that at least 100M positions are usually needed for decent
results, and that depths 4-5 often work well. Its training guide historically
defines one epoch as 20M positions. These are scale references, not substitutes
for an Anhoku learning curve:

- [Fairy-Stockfish training-data guidance](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/Training-data-generation)
- [Fairy-Stockfish NNUE training guidance](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/NNUE-training)

### Scale ladder

1. Train to the frozen plateau rule on the nested 10M prefix.
2. Replicate the 10M result with a second training seed.
3. Generate/train 50M only if the preregistered grouped offline and paired-game
   decisions show a credible positive 1M-to-10M slope and the R4 cost ceiling
   remains affordable.
4. Generate/train 100M only if the corresponding 10M-to-50M slope remains
   credibly positive and the 100M forecast is affordable. An uncertain slope
   may authorize one terminal 100M probe only if that exception, its minimum
   plausible gain, and its cost cap were registered before R6 began; simply
   failing to reach the strength target never authorizes more data.
5. Reproduce the best scale with an independent generation seed, not merely a
   different optimizer seed.

Do not compare scales at the same small optimizer-step count. Compare converged
recipes and publish both loss-versus-examples-seen and loss-versus-unique-data
curves.

### Evaluation at each scale

Use the ladder:

1. finite validation and development metrics;
2. the disjoint one-shot stage block for that scale after its single
   predeclared candidate is selected; after opening, the block becomes
   development history;
3. equal-node match versus the preceding scale and handcrafted;
4. target fixed-time screen only after equal-node quality is competitive;
5. production runtime benchmark.

For offline deltas, resample the highest independent source group and apply the
registered multiplicity rule. For games, predeclare exactly one valid decision
mode: a fixed number of complete opening pairs with its grouped interval, a
validated pair/pentanomial SPRT reported by its declared decision, or an
anytime-valid confidence sequence. Never attach an ordinary post-stop 95%
interval after sequential peeking. Select one checkpoint offline, then play it;
do not rank every checkpoint by thousands of games.

### Scale decisions

- **Both offline and equal-node improve:** continue to the next affordable
  nested scale.
- **Training improves but held-out metrics flatten:** increase independent data
  diversity or regularization; do not merely repeat records.
- **Full-precision improves but quantized does not:** add quantization-aware
  training or revise scaling before more data.
- **Offline and equal-node flatten after convergence:** authorize R7 residual
  analysis.
- **Equal-node improves but fixed-time does not:** route to the compact model or
  R8 runtime work.

Every scale jump requires a measured node, wall-time, storage, GPU, and money
forecast plus an immutable artifact-retention plan.

## Phase R7: hard examples, objective alignment, and bounded architecture work

### Question

After convergence and scale are real, what explains the residual search errors?

### R7-A: hard-position replay

Mine positions from:

- candidate-versus-handcrafted games where evaluation swings precede a loss;
- candidate/deep-teacher move disagreements with high regret;
- tactical failures, rare donor configurations, and under-covered feature rows;
- independent policies, not only candidate self-play.

The candidate may propose positions but never labels them. Relabel every retained
board with the frozen stronger teacher. Cap prioritized-sampling weights and mix
hard data with a declared broad replay fraction so the network does not forget
general positions. Mine only from development games and registered disagreement
pools—never from one-shot stage blocks, the final sealed static/search set, or
sealed promotion openings.

Run one controlled broad-only versus broad-plus-hard comparison. If it improves
development regret and equal-node play, select one candidate and open a fresh
one-shot R7 stage block. Any further hard-mining round must still use development
evidence and consume a newly registered, disjoint stage block, with the maximum
number of rounds frozen before the first block is opened. An opened block becomes
development history and is never mined for its failures.

### R7-B: quantization-aware training

Authorize QAT only when full precision consistently wins and the serialized
network loses a material fraction of that gain. The gate compares the same
checkpoint recipe with and without fake-quantization/quantization-aware
fine-tuning and requires runtime integer parity.

### R7-C: representation or capacity

New feature geometry is authorized only if all are true:

- the loader, objective, schedule, and quantization gates pass;
- train and held-out curves have converged at a meaningful scale;
- residuals form a stable, reproducible category that the current input cannot
  distinguish or the current width cannot fit;
- a written oracle shows how the proposed feature/capacity change represents
  that category;
- the comparison changes only that factor at matched data and compute.

Failure of an exactly representable R1 oracle is a correctness/trainer/oracle
failure and stays in R1/R2; it never authorizes a feature expansion. Earlier
representation work is authorized only by a proven non-representable collision
with frequent, material regret after parity passes. Otherwise, do not revive
DonorReceiverPairV2 or invent Phase 12 features from intuition.

## Phase R8: runtime and the fixed-time tax

### Question

How much equal-node strength is required to overcome NNUE's measured NPS cost,
and can inference recover that cost without changing scores?

### Required measurements

- Native and production WASM full-refresh and incremental latency by move type,
  including king moves, drops, promotions, captures, and donor changes.
- Alpha-beta NPS, qsearch NPS, and the versioned combined throughput
  `(alpha_beta_nodes + qnodes) / elapsed`, plus accumulator rebuild/update
  counts, cache behavior, and model bandwidth on the production position suite.
  Legacy main-search NPS alone is never used to derive the node handicap.
- A handcrafted-versus-handcrafted node-handicap curve using node ratios that
  match observed NNUE/handcrafted combined throughput. Publish both node
  components. This estimates the Elo tax of the runtime deficit without
  conflating evaluator quality.
- Equal-node and fixed-time results for the identical candidate network.
- Single-game production-like fixed-time performance as the primary runtime
  condition. Controlled parallel loads are separate throughput experiments;
  they never replace the target single-game result.

### Optimization rules

- Profile first and change one hotspot at a time.
- Preserve exact integer evaluator scores and, on a fixed-node search corpus,
  exact root scores, PVs, best moves, alpha-beta/qnode accounting, and declared
  trace hashes unless a search change is explicitly registered as a separate
  experiment.
- Do not retrain to validate a runtime-only change.
- Re-run accumulator differential tests and the full verifier after every
  optimization.

### Gate

Retain only changes that are score/search-contract exact, portable, pass the
frozen model-byte/peak-memory/load-time and p95/p99/max lateness ceilings, and
clear a preregistered minimum improvement in combined production throughput or
latency. The final fixed-time match, not a microbenchmark, decides whether the
tax is closed.

## Phase R9: selection and promotion

### Harness qualification

Before testing a candidate:

- A-versus-identical-A paired intervals must lie wholly inside the frozen
  zero-bias equivalence margin and pass color/order reversal and deterministic
  replay checks. Here deterministic replay means identical schedule, identity,
  raw aggregation, and fixed-node decisions; timing-noisy fixed-time games are
  tested statistically, not required to be bit-identical. For distinct A/B
  order reversals, sign-transformed estimates must agree inside their frozen
  tolerance.
- Paired-game and pentanomial accounting must be verified from raw games. Every
  scheduled pair must be complete or deterministically retried, have a unique
  index, use the identical opening/start SFEN within the pair, and reverse
  colors and engine order. Missing, duplicate, or malformed pairs fail the run;
  they are never silently excluded. Recompute all bins from raw games.
- Development match openings and sealed promotion openings must be versioned,
  broad, independent of all generation sources, and non-overlapping.
- Match executable, model hashes, hardware, worker count, time source, search
  limits, adjudication, maximum plies, seed, and stopping rule are frozen.
- Resolved threads, CPU model, affinity, compiler flags, memory configuration,
  and concurrency level are included in the match identity; a report cannot be
  resumed or combined when any differ.
- The primary qualification and promotion condition is one concurrent game on
  a controlled, quiescent production-class host. Parallel matches are separate
  throughput studies and cannot replace or be pooled with this condition.
- The match must exercise the shipped WASM artifact through its production
  interface and complete search path, including root DFPN, timers, TT behavior,
  repetition history, and fallback policy. A native in-process proxy is allowed
  for development only after an end-to-end equivalence corpus verifies its
  search decisions, scores, completion metadata, and clock semantics; it never
  substitutes for the final production-interface match.
- A maximum-ply match outcome is reported distinctly; it is not silently
  converted for training use.
- Predeclare numeric model-size, native/WASM peak-memory, load-time, per-move
  p95/p99/maximum lateness, and evaluator-symmetry bounds. A nominal 100 ms
  match that violates them is invalid.

Development games may use either a fixed number of complete pairs, a validated
pair/pentanomial LLR with preregistered `H0`, `H1`, alpha, beta, minimum and
maximum pairs, or an anytime-valid confidence sequence. The existing unpaired
W/D/L SPRT and unvalidated approximate delta-normal paired interval are not
promotion statistics. Sequentially stopped runs report their declared decision
or anytime-valid bound, never an ordinary post-stop 95% interval.

### Promotion sequence

1. From development evidence alone, designate model A as the deployment
   candidate. Train model B as a replication of the same frozen recipe with
   independent data-generation and optimizer seeds; it is not an alternative
   chosen later by whichever sealed Elo is higher.
2. Run the small fixed-node tactical/search suite and equal-node paired matches
   against C/16 and handcrafted for both models on development evidence.
3. Run the target 100 ms/move development screen for both models under a valid
   decision mode.
4. Before opening either final suite, reproduce parity, combined throughput,
   latency, verifier, tactical, memory, load-time, clock, and complete
   production-search-path gates in the shipped WASM build and on a second
   supported host. Then preregister and freeze both final model hashes.
5. Open the final sealed static/search set once for both frozen models and apply
   its preregistered, multiplicity-adjusted gates. A failure rejects the model;
   it does not reopen the recipe.
6. Run model A through the production interface on final sealed promotion block
   A using a fixed, preregistered number of complete opening pairs with no
   interim inspection.
7. Run model B on disjoint final sealed promotion block B under the same frozen
   protocol. Neither result may change the recipe; a failed model requires a
   new candidate and a fresh future block.

### Final gate

For both independent models:

- the fixed-cap lower paired 95% confidence bound versus handcrafted at
  100 ms/move is above `0 Elo`, using a coverage-validated pair/pentanomial
  method that resamples or models the highest independent opening-source group
  and applies the preregistered multiplicity rule;
- no correctness, tactical, portability, memory, or runtime gate fails;
- the final sealed static/search gate passes without a post-unseal recipe
  change;
- zero searches return no move and zero searches use an unsearched emergency
  fallback;
- every scheduled pair is valid and complete, every game has an explicit legal
  termination reason, and all p95/p99/maximum time-overrun bounds pass for each
  evaluator;
- the decisive games exercised the actual shipped production artifact,
  interface, and full search path at one-game concurrency;
- no sealed evidence was used to alter the recipe.

Only then ship the preregistered model A and update the default recommendation.
Model B is replication evidence, not a second candidate eligible for
post-unseal selection.

Enforce this in code. Selection may nominate a candidate, but promotion must
fail closed unless the machine-readable result satisfies every required bound,
identity, replication, and zero-fallback condition. Merely running a
handcrafted benchmark is not a promotion gate.

## Metrics contract

Every model report must include the following, even when a phase uses only a
subset for its decision.

### Data

- physical records, distinct full records, distinct canonical boards, effective
  sample size, duplicates and maximum multiplicity;
- group and split overlap audit;
- source/opening/trajectory proportions;
- side, phase, ply, material, score, check/tactical, result-status, output-bucket,
  and donor-state distributions;
- active-row occurrence quantiles and uncovered reachable rows;
- teacher incomplete, mate, clamp, and rejection rates.

### Optimization

- batch size, records read, examples trained, optimizer updates, equivalent
  passes, LR by step, wall time, and peak memory;
- train/validation/development metrics by update;
- gradient and parameter-update norms, clipping, dead/saturated activations, and
  checkpoint resume origin;
- initialization and optimizer-state identities.

### Prediction

- calibrated probability loss and saturation;
- robust raw-score error, correlation, and sign accuracy by macro stratum;
- deep-teacher top-k agreement and move regret;
- full-precision, serialized Python integer, Rust full refresh, and Rust
  incremental results;
- absolute quantization degradation and, only above the frozen denominator
  floor, quantized fraction of full-precision gain.

### Search and runtime

- equal-node and fixed-time wins/losses/draws, pair completeness, raw pair bins,
  independent grouping unit, and the preregistered CI/LLR/confidence-sequence
  state;
- alpha-beta nodes, qnodes, elapsed time, component NPS, combined throughput,
  deadline lateness p50/p95/p99/max, cap hits, failures, and warnings by engine;
- completed-depth and completed-root-move distributions, interruption reasons,
  missing moves, emergency fallbacks, and game termination reasons;
- model bytes, native/WASM peak memory, load time, full-refresh latency,
  incremental latency by move type, concurrency/load state, and shipped
  production-interface/search-path measurements.

## Experiment identity contract

Every run directory contains one typed, machine-readable stage manifest. Every
stage records the repository commit, submodule/external-trainer state, generated
source hashes, its own executable and canonical config hash, input/output
artifact hashes, environment lock, all RNG seeds, scheduling version, and stage
type. It must not claim identities that the stage cannot attest. A strength run
uses a clean, rebuildable commit whenever possible. Otherwise it archives and
hashes the complete relevant source/config/script tree—including tracked
changes, untracked files, generated sources, and submodule state—with an
explicit inclusion manifest; an ordinary dirty-diff hash alone is insufficient.

Stage-specific fields are:

- **Transitional combined data generation (R0-R3 only):** current generator,
  separately typed trajectory- and label-evaluator identities, opening/source
  policy, label search/node-accounting versions, target semantics, result policy,
  and labeled shard hashes. It accepts no training initialization or training
  config and is forbidden for R4 production corpora.
- **Position generation (R4 onward):** generator and trajectory-evaluator
  identities, opening/source policy, search/node-accounting versions, and
  lossless unlabeled-position shard hashes. No training or label evaluator is
  accepted.
- **Labeling/dataset assembly (R4 onward):** labeler and label-evaluator
  identities, input-position-manifest hash, search budget, target semantics,
  score transform, result and split policies, feature/record versions, and
  labeled shard hashes.
- **Training/export:** trainer revision, fork/overlay diff, loader and serializer
  hashes, input dataset-manifest hash, training initialization, optimizer/
  scheduler/scaler states, data cursor, checkpoints, and exported network.
- **Evaluation:** model/export hash, verifier and harness hashes, opening/suite
  hash, complete search/clock/adjudication versions, production artifact and
  interface, hardware/affinity/concurrency/cold-warm identity, and raw results.

The experiment registry supplies the composite record linking these manifests.
It records the hypothesis, changed variable, controls, registered metric
direction, baseline, minimum effect or noninferiority margin, uncertainty and
multiplicity rule, cost ceiling, decision, and proof boundary. Thus a generation
manifest never needs to know a later training checkpoint merely to make the
overall experiment traceable.

Commands must fail closed on an identity mismatch. `--ignore-identity-mismatch`
is never allowed for a strength claim.

## Decision table

| Observation | Most likely class | Next action |
| --- | --- | --- |
| Timed search returns no move or any strength game uses an emergency fallback | Search/harness correctness | Invalidate the run and return to R1-D |
| Pack, feature, sign, sentinel, or integer parity fails | Correctness | Stay in R1; no training or games |
| Synthetic/tiny train loss does not fall | Loader, optimizer, objective, or capacity | Stay in R1/R2 and isolate with a smaller oracle |
| Full precision learns but export loses the gain | Quantization/serialization | Measure scaling, add QAT, or fix serializer |
| Train improves but finite validation does not | Overfit, coverage, or split distribution | Improve independent positions/regularization |
| IID improves but independent-policy OOD does not | Data-policy collapse | Return to R4 source mixture |
| Candidate teacher disagrees with deep reference | Label quality/target | Return to R3; do not scale |
| Offline deep-teacher regret improves but equal-node play does not | Objective/search mismatch | Mine search errors and revisit target semantics |
| Equal-node improves but fixed-time does not | Model width or runtime tax | Compact diagnostic and R8 profiling |
| Full-precision and quantized curves improve with data size | Healthy scale response | Continue the staged R6 ladder |
| Converged scale curve is flat with stable residuals | Capacity/representation | Authorize one evidence-backed R7 architecture test |
| Seeds disagree materially | Nondeterminism or unstable recipe | Audit order/resume, then widen seed study |

## Explicitly stop doing

- Do not start old Phase 12-A or another feature geometry experiment now.
- Do not use historical fixed-time Elo, retention, or checkpoint rankings as
  decisive evidence; their no-move forfeits require clean replays.
- Do not use the Phase 8R equal-node Elo as a clean baseline until its fallback
  path is removed and the match is replayed.
- Do not award a game loss because a nonterminal timed search returned no move.
- Do not benchmark the production fixed-time target only under all-core
  concurrent self-play.
- Do not let a warm-start field choose the trajectory or label evaluator.
- Do not use C/16 as label teacher merely because it is the bootstrap.
- Do not call a 16-update run a trained production model.
- Do not use random record skipping on an already shuffled finite corpus.
- Do not cycle or randomly filter validation.
- Do not treat capped or ongoing games as draws in the result loss.
- Do not use the inherited score sigmoid without Anhoku calibration.
- Do not truncate rollout candidates lexicographically and call it MultiPV.
- Do not select thousands of game-playing checkpoints before offline parity and
  fidelity gates pass.
- Do not reject scaling because a 262k/1M plumbing run lacks a positive Elo
  lower bound.
- Do not import a quantized network as the only from-scratch/warm-start baseline.
- Do not generate 10M-100M labels before publishing a throughput and cost model.
- Do not tune on sealed data or repeatedly reuse one promotion opening seed.

## First authorized assignment

Implement Phase R0. Alongside it, write only the R1 fixture schemas and test
specifications needed to make the next assignment unambiguous:

1. define independent `TrainingConfig` and transitional
   `CombinedDataGenerationConfig` schemas, with separately typed trajectory and
   label evaluators inside the latter;
2. make combined data generation unable to read training initialization and
   reject training-only fields;
3. add combined-generation, training, and evaluation manifests plus composite
   registry links and the initialization-independence regression test; specify
   the lossless R4 position/label artifact contract without implementing R4;
4. create the experiment registry and quarantine annotations for old corpora;
5. quarantine historical fixed-time and fallback-contaminated equal-node match
   claims with exact affected-game counts;
6. specify—but do not yet run—the deterministic R1 parity corpus,
   sentinel-network format, and interruption-safe search contract.

Stop after tests and a Phase R0 closeout document. Do not generate a new
production corpus, rent a GPU, run a strength match, change the deployed feature
family, or begin R2 in the same assignment.
