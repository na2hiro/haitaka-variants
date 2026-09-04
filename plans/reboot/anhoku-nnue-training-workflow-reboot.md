# Anhoku NNUE Training Workflow Reboot

- Status: replacement R4 passed and its sole 1M pilot remains eligible; the first R5 attempt and first repair preflight are closed failed, the R5 objective remains incomplete, C/16 is incumbent, amended R5R-00 is next, and R6/broad generation remain closed
- Created: 2026-08-31
- Last audited: 2026-09-04
- Last implementation update: 2026-09-04
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
| [R0](reboot-0-separate-identities-and-freeze-history.md) | Can initialization, trajectory policy, and label teacher be made independent and auditable? | No | Typed configs, manifests, registry, regression tests |
| [R1](reboot-1-correctness-ladder.md) | Are board packing, features, sign, score, serialization, runtime inference, and interrupted search correct? | Debug scale only | R1-A/B/C plus separate R1-D1/D2/D3 machine gates |
| [R2](reboot-2-deterministic-trainer-and-finite-evaluation.md) | Is training deterministic, finite, resumable, and capable of learning? | At most one debug GPU-hour | Exposure/overfit/resume/validation gates |
| [R3](reboot-3-teacher-target-and-score-calibration.md) | Which teacher, target, search budget, and score transform are defensible? | Probe corpus only | Deep-reference calibration protocol and frozen target |
| [R4](reboot-4-position-generation-and-immutable-data.md) | Can we produce broad, independent, cost-effective positions and immutable splits? | Up to 1M pilot positions | Audited pilot corpus and scale forecast |
| [R5](reboot-5-fixed-data-exposure-and-1m-workflow-pilot.md) | Does the existing feature family learn under a real schedule, and what width is deployable? | 1M pilot | Replicated learning curves and non-catastrophic equal-node result |
| [R6](reboot-6-nested-strength-scale-learning-curve.md) | Does strength improve with 10M, 50M, and 100M data? | Staged, gate by gate | Nested scale curve and replicated best recipe |
| [R7](reboot-7-hard-examples-objective-alignment-and-bounded-architecture-work.md) | Are remaining errors distributional, objective-related, quantization-related, or representational? | Targeted only | Hard-mining or one-factor architecture evidence |
| [R8](reboot-8-runtime-and-the-fixed-time-tax.md) | Can runtime close the measured fixed-time tax without changing scores? | No new labels | Bit-exact optimized inference and NPS curve |
| [R9](reboot-9-selection-and-promotion.md) | Does the final model beat handcrafted in production conditions? | Promotion matches | Two independent positive sealed results |


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
| Search result loses a legal move, leaks a partial value, or miscounts nodes | Interrupted-search contract | Invalidate the run and return to R1-D1 |
| Repetition, perpetual check, TT history, qsearch, DFPN, or adjudication differs by path | History/search correctness | Invalidate the run and return to R1-D2 |
| A match has a missing/emergency move, malformed pair, timing violation, or unexplained A=A bias | Match-harness correctness | Invalidate the run and return to R1-D3 |
| Pack, feature, sign, sentinel, or integer parity fails | Correctness | Stay in R1; no training or games |
| Synthetic/tiny train loss does not fall | Loader, optimizer, objective, or capacity | Stay in R1/R2 and isolate with a smaller oracle |
| Full precision learns but export loses the gain | Quantization/serialization | Measure scaling, add QAT, or fix serializer |
| Train improves but finite validation does not | Overfit, coverage, or split distribution | Improve independent positions/regularization |
| IID improves but independent-policy OOD does not | Data-policy collapse | Return to R4 source mixture |
| Candidate teacher disagrees with deep reference | Label quality/target | Return to R3; do not scale |
| Fixed-budget pilot labels cannot complete depth one | Label-completion/teacher feasibility unless correctness telemetry fails | Keep targets null; run preregistered R4-F, returning to R1 for a defect or R3 for a new teacher policy; do not train |
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

## Initial R0 assignment (complete)

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

## Current worker routing

The status line at the top of this document is authoritative. At the current
revision, R4-F and its routed R3 and replacement-R4 assignments are complete.
Preserve the original failed R4 evidence and the seven replacement tuning
failures; none is an eligible pilot. The next authorized assignment is R5,
using only
`out/anhoku-reboot-r4-replacement/datasets/pilot-1m.json` with SHA-256
`1f52e6301c84931bec1ed43939f685da051d3647bb11478ebd53c71311025c5b`.
The passing replacement report explicitly records `r5Authorized=true`; this
closeout did not begin R5. The 10M/50M/100M broad tier and strength work remain
subject to their later phase gates and are not authorized by replacement R4.

## Current worker routing supersession: amended R5 repair / R6 unblock (2026-09-04)

The preceding routing block is pre-R5 history. The first R5 attempt failed
closed: it did not converge, missed the frozen 0.005 materiality margin, and
lost the clean C/16 match 20–44. C/16 remains incumbent, the stage was not
opened, and R6/10M/50M/100M remain closed.

The sole authoritative next-work specification is the [R5 repair / R6-unblock
worker assignment](reboot-5-fixed-data-exposure-and-1m-workflow-pilot.md).
The first repair preflight failed independent review because its required
prior-R4 deep-core source contradicted its prior-data disjointness rule, and
because the exact historical seed-17/29 PT payloads were unavailable. Its
R5R-05(false) closeout is immutable and R6 remains unauthorized.

The amended assignment uses a fresh, strictly disjoint reusable diagnostic and
treats hash-bound failed-R5 reports plus immutable exports as sufficient
negative-history evidence while explicitly recording the missing PT payloads.
Restart at R5R-00 using the versioned v2 paths and follow the fail-closed chain.
No GPU rental or training is authorized until its pre-GPU gates pass and the
user explicitly approves the rental.
