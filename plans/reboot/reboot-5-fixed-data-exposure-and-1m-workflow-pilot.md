## Phase R5: fixed-data exposure and 1M workflow pilot

### Question

Can the existing DonorSingleEff family learn a reproducible signal when trained
for enough optimizer updates, and is the current width a sensible deployment
point?

### R5-A: exposure and learning-rate curve

Use only the 1M finite-record pilot identified by a passing replacement R4 gate
report, and a small number of predeclared schedules. The failed R4 v1 filtered
payload is ineligible. Training must run through multiple passes and checkpoints
until development loss plateaus; one pass at batch 16,384 is only about 61
optimizer updates.

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

### CPU/pre-GPU execution closeout (2026-09-04)

The fixed-data execution surface is prepared and has not started ML training.

- Frozen contract: `r0/anhoku-reboot/r5-fixed-data-exposure-contract.json`,
  SHA-256 `de0fabb790b7c212e4ee1db0da05632d9b66f6cfa023614dfd8287ca1b25880a`.
  It binds the passing replacement-R4 report and its sole eligible 1M pilot,
  denies every failed replacement payload, compares only scratch with the
  immutable 64k static-clone lane, and retains C/16 as an audit-only baseline.
  No historical full-precision checkpoint was admitted as a warm start because
  the available checkpoints do not have defensible matching objective/data
  provenance.
- The preflight report is
  `out/anhoku-reboot-r5-preflight-v2/r5-preflight-report.json`, SHA-256
  `fd18474d1822eb3b48e17275ee67e2a246ce9b35a7b542c953a91010113ef103`.
  It independently hashes the training, validation, development, and
  static-clone payloads; confirms exact finite physical exposure; and records
  four recipes (two learning rates × two initialization lanes). Training,
  stage-block opening, and strength games are all false/closed in the report.
- `scripts/r5_trainer.py` is a dedicated CUDA-only runner. It uses the pinned
  disposable trainer context, no filtering/skipping, one loader worker,
  non-cyclic streams with final partial batches, the frozen normalized robust
  Huber objective, step warmup/decay, plateau stopping, checkpoint metrics, and
  separate full-precision/export artifacts. The legacy `train` command is not
  used because its manifest and objective are incompatible with replacement R4.
- `sol` added the separate `haitaka_wasm` `nnue-l1-256` compile-time profile for
  R5-C. The compact profile passed 90 `cargo test -p haitaka_wasm --features
  anhoku,nnue-l1-256` tests; the current-width profile remains the default.

The next action requires explicit permission to rent a Vast.ai GPU instance.
Until that permission is granted, no R5 training process will be launched.

### GPU execution closeout (2026-09-04)

The user approved the GPU pilot. Vast.ai offer `40819313` was rented as
instance `49809117` on an RTX 4080 SUPER at `$0.219259/hour`; the instance is
now stopped (`exited`) after the evidence was copied and hash-checked. The
contract and local preflight remain the binding data surface:

- Contract: `r0/anhoku-reboot/r5-fixed-data-exposure-contract.json`, SHA-256
  `de0fabb790b7c212e4ee1db0da05632d9b66f6cfa023614dfd8287ca1b25880a`.
- GPU preflight: `out/anhoku-reboot-r5-gpu/r5-preflight-report.json`, SHA-256
  `30cbce756aa26db374ec2b1298ba44645101f7407527522ae66146e3642d970e`.
- Consolidated status: `out/anhoku-reboot-r5-gpu/r5-phase-status.json`.

The four frozen current-width recipes completed 768 optimizer updates. None
met the three-validation plateau rule; all stopped at the frozen maximum
updates. The scratch `0.0015`, seed 17 recipe ranked first by the frozen
development metric (`0.015179697477072268`) and is the selected candidate. The
single compact diagnostic also completed at 768 updates; its development loss
was `0.01582142168659308`, with 40,858,024 parameters and 83,048,665 exported
bytes versus 80,475,560 parameters and 161,206,489 bytes for current width.

The selected current-width candidate improved the held-out development loss
over C/16 by `0.0008258709844737571`, below the preregistered `0.005` full-
precision materiality margin. Python full-precision and exported-network
reports are preserved separately. The point estimate for exported-vs-full
development degradation is `6.311560359437834e-7`, but the required grouped
stage interval cannot be formed before opening the disjoint stage block.
Native Rust search parity assertions passed, and the CPU/native latency
diagnostic is in `out/anhoku-reboot-r5-gpu/r5-runtime-diagnostic.json`;
browser-WASM latency remains a separate pending measurement.

The selected recipe was replicated with seed 29 at 768 updates (development
loss `0.015556901777924065`, validation loss `0.013678847513593738`). Seed 41
was not started: the frozen run had already reached its update ceiling without
convergence, and the deep-core sidecar has root scores but no per-legal-move
values, so the required move-regret metric is not computable without a new
explicitly frozen artifact.

The predeclared equal-node diagnostics were completed with the R1 zero-fallback
harness, 10,000 nodes per move, one thread, 32 color-swapped pairs for C/16,
and 8 color-swapped pairs for handcrafted. The complete C/16 result was 20–44
with no draws, approximate candidate-minus-C/16 Elo `-136.969`, paired 95%
interval `[-205.870, -68.069]`, and zero missing moves/fallbacks. This is a
clear material regression against the contract's `-20` veto threshold, so the
R5 game gate vetoes the candidate. The handcrafted diagnostic was 7–9,
approximate Elo `-43.658`, with zero missing moves/fallbacks; it remains
low-sample diagnostic evidence only. Exact hashes and recovery provenance are
in `out/anhoku-reboot-r5-gpu/r5-game-audit.json`.

Therefore R5 execution is complete but R5 does not pass and R6 must not open.
The one-shot R5 stage block stayed closed and no stage data was generated.
The remaining route is a new reviewed R5 contract/evidence revision that
extends the convergence horizon and supplies the required per-legal-move
deep-teacher artifact before additional replication or stage data generation.

### First R5 repair preflight closeout (failed closed; 2026-09-04)

The first execution of this assignment stopped correctly at R5R-00. Independent
review found two specification blockers rather than an experimental result:

- requiring the reusable 256-position diagnostic to use the existing
  replacement-R4 deep core while also requiring it to be disjoint from every
  prior R4 artifact was unsatisfiable;
- the exact seed-17 and seed-29 full-precision checkpoints named in the failed-R5
  reports were no longer retained, although the hash-bound decision reports and
  both immutable exported networks remain available.

R5R-01 through R5R-04 were therefore skipped, R5R-05(false) completed, and no
labels, search nodes, training, games, holdout access, GPU rental, or new spend
occurred. The failed attempt remains immutable under
`out/anhoku-reboot-r5-repair/`; its final gate has SHA-256
`910ec0b47a47057996e82bf8c8b08376adba0badf4869c8ac1675f9467ee364b`.
It must not be overwritten or reclassified after this amendment.

The recovery decisions adopted by this amendment are:

1. replace the prior-R4 deep-core source with a fresh, reusable development
   diagnostic that is strictly disjoint from all prior R4/R5 data and matches
   and from the later stage and game suites; and
2. accept the hash-bound failed-R5 reports plus the two hash-verified immutable
   exports as sufficient negative-history evidence. The missing PT payloads are
   explicit retention gaps, not required repair inputs. They may not be
   reconstructed, substituted, used for initialization/resume, or treated as
   available full-precision models; the retained seed-29 derivative is not the
   missing reported checkpoint.

Execution restarts at R5R-00 under the versioned paths below. This amendment
changes only provenance/diagnostic data construction, not the training
hypothesis, recipe, thresholds, or one-shot holdout rules.

## R5 repair / R6-unblock worker assignment (amended; no GPU authorized)

This is the sole detailed specification for new R5 repair work. The first R5
attempt was fully executed and failed closed; the R5 objective remains
incomplete. Seeds 17/29 and their weights are diagnostic history only, C/16
remains incumbent, the original stage was never generated, and R6 remains
closed.

The repair tests one narrow hypothesis: the selected scratch, LR 0.0015,
512x16x32 recipe may need more than 768 optimizer updates. It keeps the sole
eligible replacement-R4 1M payload and every other recipe, objective, teacher,
score, serializer, search, and data-order field from the original R5 contract.
It does not authorize an architecture, objective, data-mixture, teacher,
score-scale, or 10M change, or deployment of a rejected model.

Routine inventory, fixtures, orchestration, and report assembly may go to a
Luna/max worker. Gate reviewer below means an independent Sol/high-quality
reviewer. Gate review is mandatory for statistical decisions, final
authorization, and any engine, trainer, search, serializer, WASM, or
node-accounting change.

### Contract boundary

R5R-02 must copy exact inherited values and hashes into
r0/anhoku-reboot/r5-repair-contract-v2.json; same recipe is not a sufficient
specification.

| Area | Requirement |
| --- | --- |
| Inputs | Commit a867d89; passing replacement-R4 contract/report and sole 880k/60k/60k pilot split; C/16; hash-bound failed-R5 contract, preflights, decision reports, run reports, and the hash-verified seed-17/29 exports. Failed R4 payloads and all old checkpoints are ineligible. The absent reported PT files are recorded as unavailable tombstones and are not required inputs; the seed-29 derivative is not a substitute. |
| Seeds | New from-scratch runs in strict order 41, 53, 65 after R5R-00 verifies they are unused. Never resume or initialize from seed 17/29. Seed 41 is the predeclared game model. |
| Exposure | Checkpoint/evaluate every 64 updates; minimum 1,024; hard maximum 2,048; no extension. |
| Convergence | At u at least 1,024, use the five checkpoints from u-256 through u. For both validation and development loss, max(window) - min(window) must be at most 0.00002. Require two consecutive qualifying checkpoints. |
| Selection | Lowest finite development loss through convergence, then validation loss, development MAE, earlier optimizer update, and lane ID. No convergence by 2,048 is failure. |
| Metric signs | Improvement is C/16 loss minus candidate loss; quantization degradation is exported loss minus full-precision loss; regret improvement is C/16 regret minus candidate regret. |
| Offline gates | Each seed's fresh-stage full/export point improvement must be positive. For the arithmetic mean across three fixed seeds, multiplicity-adjusted one-sided 95% simultaneous bounds require full-precision improvement above 0.005, exported improvement above zero, exported-10k-search regret improvement above zero, and quantization degradation below 0.02. Apply the 95% retained-gain gate only when the full-precision improvement lower bound is at least the original 0.01 denominator floor. |
| Reusable regret | Before training, generate a fresh development-only candidate pool with the replacement-R4 position-generation policy and a new domain-separated seed. Deterministically accept exactly 64 new `reusable_opening_group_id` values with four positions each (256 parents total), subject to the frozen quotas and replenishment ceiling. Label every legal child in canonical USI order at 250,000 nodes with root-oriented score and mate class/distance. The gated endpoint uses the move returned by frozen zero-fallback 10k search with the exported model. Historical seed-17/29 exports and C/16 may be compared on it for diagnostics; new repair exports use it only for the preregistered sequential screen. Full-static choices are allowed only for newly trained checkpoints and remain diagnostic. |
| Fresh stage | 64 new opening_group_id values times 500 completed 10k-node root labels = 32,000 positions; four of those positions per group receive all-legal-move labels. Freeze exact group meaning and source/phase marginal quotas. |
| Fresh games | 128 new game_opening_group_id values times four color-swapped pairs = 512 pairs/1,024 games; 10k nodes/move, one thread, concurrency one, no random plies or sequential peeking. The grouped paired 95% Elo lower bound must exceed -20. |
| Statistics | For reusable regret, aggregate legal moves to position and positions to `reusable_opening_group_id`; for the fresh stage, aggregate positions to `opening_group_id`. Then average the three fixed-seed group effects and bootstrap the applicable group IDs jointly across seeds; do not resample seeds. The game unit is `game_opening_group_id`. `source_game_id` and `canonical_board_id` are leakage keys, not resampling units. Freeze the 100,000-draw bootstrap, PRNG seed, quantile convention, simultaneous family/correction, and grouped Elo estimator. |
| Completion | A complete usable label has a target from at least one completed depth and reconciled actual nodes. A later interrupted iteration with partialIteration=true remains valid. Depth-zero/null-target attempts are incomplete and may not become partial scores or artificial draws. Consume candidates in frozen order until the exact quota or a frozen replenishment ceiling; record all rejects and fail if the quota is not filled. |
| Holdout order | Freeze stage/game generator code, seeds, quotas, and overlap rules before training. Generate and hash both suites only after all six new model hashes are frozen and before stage evaluation. Keep the game suite unopened until the stage passes. No model changes or reruns after generation. |
| Independence | Reusable, stage, and game artifacts must be mutually disjoint and each must be disjoint from prior R4/R5 data and matches separately by semantic group identity, source_game_id, and canonical_board_id. The reusable artifact uses its own group namespace but renaming alone never establishes independence. Freeze the complete prior-history exclusion set before reusable generation; after the reusable set is frozen, add its identities to the exclusions for stage/game generation. |
| Runtime | Native full-refresh/incremental and WASM integer parity, legal moves, loading, OOM, and crashes are gates. Browser latency is diagnostic unless an existing numeric production ceiling is bound before training. |
| Cost/retention | R5R-01 publishes a repair-specific CPU/GPU/cash/storage ceiling. Retain selected, adjacent, and last-resumable checkpoints, not every checkpoint. Every paid GPU rental requires explicit user permission. CPU labeling should use mbp:mba:ubuntu:nubu in the 10:7:2:1 capacity ratio where available. |

The original 0.005 gate remains failed and unchanged. R5R-00 freezes the power
method before diagnostic results. R5R-01 must show at least 80% power for the
simultaneous primary full-precision stage bound to clear 0.005 when true
improvement is 0.01, using historical opening-group variance, and at least 80%
power for the grouped game lower bound to clear -20 Elo when true Elo is zero,
using clean paired-game variance. It also reports 80%-power minimum detectable
effects for the exported, quantization, retained-gain, and regret endpoints.
If a calculation is not reproducible or the designs are unsuitable, stop for a
separately reviewed design. No threshold or sample size changes after training.

The successor contract also freezes maximum ply, repetition/adjudication,
unknown/cap handling, valid-game inclusion, and all termination reasons. Only
rules-defined draws count; unknown or capped outcomes fail the game artifact.

### Dependency and failure flow

    pass: R5R-00 -> R5R-01 -> R5R-02 -> R5R-03 -> R5R-04 -> R5R-05
    fail: any R5R-00 through R5R-04 -------------------------> R5R-05(false)

R5R-00 through R5R-04 advance only on pass. R5R-05 always runs. All amended-run
outputs go under `out/anhoku-reboot-r5-repair-v2/`. The prior failed receipts
under `out/anhoku-reboot-r5-repair/` are read-only inputs and must never be
replaced in place.

### Worker assignments

#### R5R-00 — Bind history and diagnostic design

- Owner/deps: provenance/experiment worker plus gate reviewer; none.
- Work/output: inventory the contract inputs and verify seeds 41/53/65 are
  unused. Freeze the prior-history exclusion-set manifest; the fresh reusable
  generator policy, domain-separated seed `2026090405`, semantic group
  definition, quotas, candidate order, and replenishment ceiling; and the
  legal-move schema, orientation/mate rules, stage/game group definitions,
  estimands, power methods, and CPU diagnostic ceiling. Write
  `out/anhoku-reboot-r5-repair-v2/preflight/negative-history-inventory.json` and
  `out/anhoku-reboot-r5-repair-v2/preflight/diagnostic-contract.json`.
- Pass/stop: every required executable input and every claimed historical
  report/export resolves and hash-checks. The old failure, absent PT payloads,
  ineligible weights, absent stage, unused seeds, exclusion sets, completion
  rules, design, and finite pre-GPU cost must be explicit. The reports may
  establish historical decisions but may not stand in for remeasurement of a
  missing full-precision model. Otherwise stop before generation.

#### R5R-01 — CPU-only diagnosis, power, and cost

- Owner/deps: evaluation/CPU workers plus independent auditor; R5R-00 pass.
  Core-code changes require gate review.
- Work/output: verify orientation, mate, replay, and node accounting; generate,
  label, and audit the fresh reusable 64-group/256-position artifact against the
  frozen exclusion set; then freeze its identities as exclusions for all later
  holdouts. Compare immutable seed-17 export, seed-29 export, and C/16 on
  exported-10k-search regret; run existing-export native/WASM correctness; and
  publish diagnostic, power, throughput, repair cost, peak-storage, and
  retention reports under `out/anhoku-reboot-r5-repair-v2/diagnostic/`, with
  `out/anhoku-reboot-r5-repair-v2/diagnostic/diagnostic-gate.json` as the index.
  Do not report historical full-precision remeasurement or use the seed-29
  derivative PT. Static-choice results for the historical exports are
  diagnostic and identified as exported-integer, never full precision.
- Pass/stop: require complete labels and reconciled nodes, zero correctness
  faults, both specified 80% power results, reproducible secondary sensitivity,
  and a finite affordable envelope. Historical model outcomes are diagnostic,
  not promotion gates. Otherwise route to objective/label, runtime, R4-mixture,
  or capacity review as the evidence indicates.

#### R5R-02 — Freeze the successor contract and runner

- Owner/deps: experiment/statistics lead plus gate reviewer; R5R-01 pass.
- Work/output: create the machine contract from the frozen design above without
  revising it after R5R-01; bind every group, completion, estimator,
  termination, cost, output, and retention detail; test deterministic
  stop/no-stop/resume fixtures; finish C/16 native/WASM preflight; and write
  `out/anhoku-reboot-r5-repair-v2/preflight/successor-contract-audit.json`. Freeze
  holdout policies but generate no holdout data.
- Pass/stop: no provisional field, exact runner/contract agreement, passing
  correctness, adequate power/cost, and trainingAuthorized=false pending user
  approval. Ambiguity or an unplanned recipe/data/code change fails closed.

#### R5R-03 — Train and screen unused seeds sequentially

- Owner/deps: approved GPU worker plus evaluation/runtime reviewer; R5R-02 pass
  and explicit user permission for the paid GPU rental.
- Work/output: train seed 41 from scratch and copy/hash its evidence locally.
  Start seed 53 only after seed 41 passes, and seed 65 only after seed 53
  passes. Run development, exported-10k-search regret, quantization,
  native/incremental/WASM, and browser-correctness screens after each seed.
  Never run seeds concurrently or rerun a completed result for favorability.
  Emit one immutable `out/anhoku-reboot-r5-repair-v2/runs/seed-{41,53,65}/` tree
  per executed seed and
  `out/anhoku-reboot-r5-repair-v2/evaluation/sequential-screen.json` with checkpoint
  metrics, full/export hashes, commands/environment, parity, and spend.
- Pass/stop: each seed converges by 2,048; has positive full and exported point
  improvement on validation and development; has positive exported-10k-search
  regret improvement; keeps quantization degradation below 0.02; and passes
  correctness. This same-direction screen is an early stop; only the fresh
  stage decides the 0.005 materiality bound. The first failure stops later
  seeds and all holdout work. Three passes freeze all six model hashes.

#### R5R-04 — One-shot fresh stage and game veto

- Owner/deps: data/label/evaluation workers with independent leakage,
  statistics, and game-result auditors; R5R-03 pass and immutable model hashes.
- Stage/output: instantiate and mutually audit both precommitted holdout suites
  before evaluation; hash and withhold the game manifest. Generate the exact
  root/deep stage quotas with frozen replenishment, evaluate C/16 and all three
  full/export models once, and write raw group results plus generation,
  completion, leakage, and access audits under
  `out/anhoku-reboot-r5-repair-v2/stage/`, including `stage-gate.json`.
- Stage pass/stop: every count, hash, quota, label, node, identity,
  independence, per-seed direction, and Contract boundary offline gate passes.
  Failure forbids replacement data, re-bootstrap, retraining, and games.
- Game/output: only after stage pass, open the hashed game suite once and run
  seed 41 versus C/16 for all 1,024 games. Preserve every game, termination
  reason, node total, and pair bin under
  `out/anhoku-reboot-r5-repair-v2/games/`, including `equal-node-gate.json`.
- Game pass/stop: all 512 pairs, zero unknown/capped outcomes, zero
  identity/node/move/fallback faults, and grouped 95% Elo lower bound above
  -20. A harness defect invalidates and routes to reviewed repair; a clean
  failure rejects the candidate without a larger or repeated match.

#### R5R-05 — Unconditional closeout and R6 decision

- Owner/deps: provenance/cost auditor plus gate reviewer; any terminal result
  from R5R-00 through R5R-04.
- Work/output: record pass/fail/skipped tasks; reconcile CPU/GPU/node hours,
  cash, storage, invoices, and remote-instance state; verify local hash-checked
  raw evidence; preserve old R5 and first-repair evidence; and write
  `out/anhoku-reboot-r5-repair-v2/closeout/cost-retention-audit.json` and
  `out/anhoku-reboot-r5-repair-v2/closeout/r5-repair-final-gate.json`.
- Pass/stop: r6Authorized=true only if R5R-00 through R5R-04 all passed. This
  permits only a separate R6-10M contract, subject to labeling at most $75,
  first-rung generation plus GPU at most $100, and retained artifacts at most
  25 GiB. It starts no work and never authorizes 50M/100M. Every other outcome
  records r6Authorized=false and the exact diagnostic route.

### Common handoff

Every receipt names task, worker/reviewer, status/timestamps, repository commit
and dirty-tree inclusion manifest, contract and dependency/input hashes,
run/seed/model role, exact commands, tool/build/environment/hardware identity,
output paths/counts/bytes/hashes, evidence for every gate, completion/anomaly
records, actual and forecast CPU/GPU/node/wall time, cash/storage, GPU-approval
receipt, provider instance ID/start-stop times/state, holdout manifest/access
state, and exact next task. Not applicable needs a reason; missing evidence and
silence fail closed.

Failure routing stays narrow: no convergence goes to optimization/capacity;
scalar gain with regret failure to objective/label alignment; independent-stage
failure to R4 mixture; full-only gain to quantization/serialization; regret
gain with game failure to score/search calibration; browser correctness failure
to runtime/R8. Architecture work remains unauthorized until these causes are
excluded.

## Second R5 repair amendment — reviewed v3 path (2026-09-05)

The v2 R5R-01 failure is retained as immutable negative evidence. A separate
Sol/high-quality review found that the failure is actionable before any repair
seed or holdout was opened: the reusable worker mishandled root no-legal-move
terminal positions, 43 qsearch-only labels had no completed target, the
implementation silently allowed a missing exported-model move to fall back to
the first legal move, the original source-game identity was renamed, and the
fixed 128-group game-power calculation was underpowered. This amendment does
not reclassify v2 and does not change the training hypothesis, recipe, seeds,
convergence limits, 0.005 stage materiality margin, -20 Elo game veto, or 80%
power requirement.

The v3 execution surface is versioned under
`out/anhoku-reboot-r5-repair-v3/`. It must bind the v2 diagnostic gate and
closeout hashes, the complete v2 generated/attempted/labeled/matched identity
set, and SHA-256 hashes of the actual worker/orchestration sources plus the
dirty-tree inclusion manifest. A git commit is preferred but not required when
the source hashes and manifest are explicit and immutable. No v2 artifact may
be overwritten.

### v3 R5R-01 corrections

- The shared root no-legal-move branch returns the same rules-defined terminal
  mate score and orientation already used below the root. It is never encoded
  as a static score or draw; mate-distance convention is frozen and fixture
  tested.
- Qsearch-only/depth-zero/null-target attempts remain incomplete. A candidate
  parent is accepted atomically only after every legal child has a completed
  target and reconciled actual nodes. Failed parents are recorded and the
  frozen rank order advances within the same phase/source quota; if necessary
  the whole source group is rejected and replenished. The exact 64 groups and
  256 parents are finalized only after this completion test.
- Label attempts are content-addressed by source identity, canonical board,
  legal move, and worker revision. Shards can stop and resume only by verifying
  the frozen input and code hashes; a partial shard is never treated as a
  completed receipt. Rejected identities are added to the later holdout
  exclusion set.
- Any missing, illegal, or emergency exported-model move is a correctness
  failure; no first-legal-move fallback is admissible. Original generated
  `source_game_id` values remain visible and are audited directly; namespace
  renaming is not an independence proof.
- Native/WASM parity is exercised with C/16, seed-17 export, and seed-29
  export, separately from the generic crate test suite.

### v3 game-power design

Before any training, replace the unsuitable 128-group historical-power
calculation with a frozen design of 2,048 independent semantic
`game_opening_group_id` values, one color-swapped pair per group, and 4,096
total 10k-node games. The match remains one thread, concurrency one, with the
same termination and zero-fallback rules. The primary is the one-sided 95%
group-level score-rate bound transformed monotonically to Elo; the 100,000-draw
PCG64-DXSM sensitivity bootstrap uses the frozen seed and quantile convention.
Both results must have power at least 0.80 and differ by at most 0.05. Include
a bounded-variance stress result using variance 0.25; at 2,048 groups the
stress result must also clear 0.80. No threshold is weakened.

The v3 stage-power calculation must additionally report a conservative
variance-bound sensitivity because v2's stage estimate used only four historical
groups. If the bound fails, stop for another reviewed design; do not proceed to
training on the optimistic four-group estimate.

### v3 dependency flow

`R5R-00-v3` freezes the amendment, source/code hashes, expanded exclusions,
terminal/completion semantics, resumable worker protocol, and corrected power
methods with `trainingAuthorized=false`. `R5R-01-v3` must pass all engine,
label, identity, runtime, power, and cost gates. `R5R-02-v3` must ship the
successor contract plus deterministic holdout generators and stop/resume
fixtures before training authorization is considered. Only then may R5R-03
request a new explicit GPU-rental approval and train seeds 41, 53, and 65
sequentially. R5R-04 remains one-shot and opens games only after the fresh stage
passes. R6 remains closed unless every substantive R5 task passes.

The v2 receipts and the Sol review are required inputs to the v3 audit; they
are not promotion evidence. The v3 closeout must record the independent review
identity and may not mark R5 or R6 passed from diagnostic results alone.

## Third R5 repair amendment — v3b typed identity correction (2026-09-05)

`R5R-01-v3` failed closed before any label or model result was produced. Its
candidate selector incorrectly treated the twelve sealed source-opening strata
used by v2 as hard exclusions, leaving zero eligible candidates. This is
immutable structural-negative evidence; it does not reclassify v2 and must not
be overwritten. Sol/high adjudicated that a new opening suite is unnecessary
for this development-only diagnostic, provided the identity namespaces below
are enforced in code and in the receipts.

The corrective execution surface is
`out/anhoku-reboot-r5-repair-v3b/`. It may import the hash-checked v3
candidate shard without regeneration:

- positions SHA-256:
  `4272ef63bfd324002483dfaa5c0542e9d3de5d7b5d0a016764983d8e1abcc717`;
- generation-contract SHA-256:
  `f9e52e577f232f5c5104d3a165525b9ad230f3db0d1755ce9b97e6f98d7239e5`.

The v3b selector uses typed identities:

- `sourceOpeningGroupId` is the exact blocked-factor set
  `anhoku-v3-053` through `anhoku-v3-064`; overlap is expected, it is not a
  leakage key or resampling unit, and conditional independence is claimed only
  given this frozen set;
- raw `sourceGameId` is a hard provenance/leakage key and every prior value is
  excluded;
- `canonicalBoardId` is a hard provenance/leakage key and every prior value is
  excluded before candidate ranking/selection;
- `reusableOpeningGroupId` is the semantic resampling unit, deterministically
  bound to the raw source game and generation-contract hash; prior semantic
  group IDs remain hard exclusions.

Legacy opening-group values are classified by schema: raw generator opening
values are source-opening strata, while reusable/stage/game analysis-group
values remain hard semantic exclusions. A candidate from an unrecorded source
opening stratum fails closed. The v3b audit must report the exact allowed
stratum set and opening-SFEN hashes, expected/actual blocked-factor overlap,
hard overlap before and after filtering, within-v3b uniqueness, semantic-group
construction/contract binding, and
`conditionalIndependenceGivenSourceOpeningStratum=true`.

Before selection, freeze stratum allocation using the domain-separated
`haitaka-r5r-stratum-allocation-v1` hash: four of the twelve strata receive six
semantic groups and the remaining eight receive five, for 64 groups total.
Retain the existing four source-policy quotas and early/middle/late phase
quotas. Aggregate legal-child labels to parent, parents to
`reusableOpeningGroupId`, then aggregate groups within stratum and give each
stratum weight 1/12. Any inability to fill the frozen phase × policy × stratum
quotas within the 4,096-parent ceiling fails closed. The sensitivity bootstrap
resamples groups within each source-opening stratum and never resamples models
or seeds.

The v3b dependency flow is `R5R-00-v3b` → `R5R-01-v3b` → `R5R-02-v3b`, with
the same trainingAuthorized=false, holdoutAuthorized=false, r6Authorized=false,
and no GPU rental. Only a passing v3b diagnostic gate permits successor
contract/runner work; a fresh explicit GPU approval is still required after
R5R-02-v3b and before any training.

## Fourth R5 repair amendment — v3c quota-feasible selector (2026-09-05)

`R5R-01-v3b` also failed closed before labeling: the typed selector’s greedy
rank admission reached 248/256 parents because it exhausted the
policy-by-stratum intersections in an unfavorable order. Its preflight and
failure evidence are immutable. A temporary verification showed that the
hash-ranked candidate pool has a feasible allocation, so the selector is
repaired with a deterministic max-flow matrix over phase-feasible source-game
groups, four source-policy row quotas of 16, and the frozen source-opening
stratum column quotas of four sixes and eight fives. Rank order is then applied
within each allocated policy × stratum cell. An infeasible matrix still fails
closed.

The source-bound execution root is
`out/anhoku-reboot-r5-repair-v3c/`. It may import the same v3 candidate shard
only after verifying the two hashes recorded in the v3b amendment. It must
create fresh v3c preflight receipts binding the max-flow helper, orchestration
script, plan, dirty-tree manifest, and imported-shard provenance. The v3 and
v3b roots remain immutable and are not rerun or overwritten. The semantic
stratum allocation audit reports group counts (not parent-row counts), exact
policy/phase quotas, 64 unique contract-bound semantic groups, and 256 unique
canonical boards.

The v3c dependency flow is `R5R-00-v3c` → `R5R-01-v3c` → `R5R-02-v3c`. All
training, holdout, GPU-rental, and R6 authorizations remain false until the
corresponding gates pass; a fresh explicit GPU approval is still required
after R5R-02-v3c and before any training.

## Fifth R5 repair amendment — v3d exact phase feasibility (2026-09-05)

The v3c diagnostic was interrupted during partial labeling after review found
that its max-flow feasibility predicate required phase presence but did not
enforce the frozen parent minimums. Its partial shards are preserved as
non-promotion evidence; no v3c gate was emitted and no model, holdout, GPU, or
training work occurred. The selector is tightened to require exactly the
available minimums for every source-game group: early ≥ 1, middle ≥ 2, and
late ≥ 1. The same predicate is used by the allocation graph and by final
selection, so an infeasible phase × policy × stratum matrix fails closed.

The new source-bound root is `out/anhoku-reboot-r5-repair-v3d/`. It may import
the already hash-verified v3 candidate shard, manifest, generation contract,
and typed exclusions, but must rerun selection and labeling under fresh v3d
receipts. The v3, v3b, and partial v3c roots remain immutable. The v3d gate
names are `R5R-00-v3d`, `R5R-01-v3d`, and `R5R-02-v3d`; all authorizations
remain closed, with fresh GPU approval required only after R5R-02-v3d.

## Sixth R5 repair amendment — v3e parent-atomic resume protocol (2026-09-05)

The v3d diagnostic failed closed during labeling after 2,812 child rows exposed
one depth-zero, qsearch-only incomplete target. The stop also proved that the
v3d shard runner was not actually resumable at its claimed granularity: it
published only mutable whole-shard streams, so a replenishment attempt or
process restart would relabel unaffected parents. The ten partial shard trees,
the incomplete row, and the absence of shard summaries or a diagnostic gate
are immutable non-promotion evidence. No v3d label may seed the corrected
cache, and no training, holdout, GPU, or game work occurred.

The corrected source-bound root is
`out/anhoku-reboot-r5-repair-v3e/`. It may import only the hash-verified v3
candidate shard and manifest, generation contract, and typed exclusions. Its
preflight freezes a separate source-binding manifest. A post-build worker
binding derives `workerRevision` from that manifest and verifies the exact
release executable, build profile/features, candidate shard and manifest,
generation contract, base exclusion set, canonically sorted model hashes, and
all search modes and constants. The current helper source must still hash to
the frozen revision immediately before build, while creating the worker
binding, and at worker startup when the source tree is available.

Each selected parent has a canonical complete-record hash covering every
provenance and selection field. A parent-cache ID binds that record hash to the
worker binding, executable, models, and search contract, but deliberately does
not bind the whole selection file so a still-selected parent can be reused
across replenishment attempts. Child label hashes are domain-separated over
the parent-cache ID, move, child SFEN, and complete result payload; child and
regret rows carry the parent-cache ID.

Parent objects are written below a same-filesystem append-only partial tree.
Files and the partial directory are flushed before an atomic rename into the
objects tree. A final object is reusable only when its manifest has verified
file hashes, byte lengths, row counts, exact legal-move count and digest,
exactly one complete label per legal move, one regret/runtime row per bound
model, reconciled nodes, zero runtime/model faults, and `passed=true`. Strict
reuse validation reconstructs every child board and label hash, recomputes
teacher-best and selected scores and both regret values, enforces positive
completed depths and search-node ceilings, and requires each runtime check to
have actually executed exactly once per model. An
invalid object at a completed ID fails closed and is never overwritten.
Concurrent writers validate the winner after a rename collision; losing and
interrupted partials remain operational evidence and count toward cost and
storage.

Replenishment uses deterministic parent-index waves of exactly ten, except the
final six-parent wave. Every
parent in the active wave finishes, all incomplete parents in that wave are
recorded in selection-index order, and no later wave starts after any such
marker. Abort state is scoped by the current selection hash, prior reject set,
worker binding, and wave number, so stale or timing-dependent markers cannot
change a later attempt. A restart completes missing members of the active wave
and reuses only validated final parent objects. Final success requires exactly
256 distinct validated cache objects matching the current selection, with no
rejected, stale, duplicate, missing-child, incomplete, or foreign-parent
object admitted.

Every command publishes an immutable start receipt before execution and a
terminal receipt after wait completion. Any unmatched start is conservatively
charged two core-hours/two wall-hours and, for a parent-label command, the
maximum 148,280,000 search nodes (593 legal children × 250,000 plus three
10,000-node model searches). Partial directories without summaries are thus
retained and costed rather than silently omitted. Completed nonzero
parent-worker commands are also charged that maximum search-node amount if
they did not yield a usable summarized object; this may double-count partial
summaries but may never undercount failed work.

Before promotion, crash/resume, concurrent-writer, corrupt/partial exclusion,
and cold-versus-resumed equivalence fixtures must pass. `R5R-01-v3e` adds the
subgates `workerAndCacheProtocolBound`, `cacheKeyCoverageVerified`,
`atomicCommitAndConcurrentWriterSafe`, `partialAndCorruptObjectsExcluded`,
`deterministicAbortAndReplenishment`, `resumeEquivalentToColdRun`,
`currentSelectionCacheCoverage256`, `allLegalChildrenCompleteExactlyOnce`,
`noRejectedOrStaleCacheReuse`, and `allAttemptedWorkCosted`.

The preflight and diagnostic gate bind an immutable independent-review
receipt to the exact source, plan, and passing fixture hashes. The receipt
records the separately configured reviewer agent and requested model/effort,
but does not represent a requested configuration as a reviewer-attested model
identity.

The dependency flow is `R5R-00-v3e` → `R5R-01-v3e` → `R5R-02-v3e`. Earlier
roots remain immutable. Training, holdout, GPU-rental, game, and R6
authorizations remain false; the plan still requires a fresh explicit GPU
approval only after R5R-02-v3e passes.

## Seventh R5 repair amendment — v3f group-atomic replenishment (2026-09-05)

`R5R-01-v3e` terminated without a diagnostic gate after all sixteen selection
attempts exhausted the existing attempt ceiling. Its immutable evidence has 17
ordered incomplete-parent markers across five semantic groups, 52 committed
parent-cache objects, and 17 partial objects. Thirteen markers are distinct
boards from the same `reusableOpeningGroupId`; board-only replacement therefore
kept the same computationally infeasible resampling unit selected. No training,
holdout, GPU, game, or R6 work was authorized or started. A terminal-root
manifest must hash-bind every v3e file, including all attempts, markers, cache
objects, partials, and command receipts. Every later cost report and terminal
closeout carries cumulative v3e plus v3f CPU, wall, node, cash, and storage
costs.

The corrected source-bound execution root is
`out/anhoku-reboot-r5-repair-v3f/`; v3e remains read-only negative evidence.
Replenishment is atomic at `reusableOpeningGroupId`, the declared statistical
resampling unit. Any parent incomplete for any legal child or bound model at
the frozen budgets permanently rejects its whole semantic group. Rejection is
based only on the predeclared completion predicate, never on teacher scores,
regrets, or model favorability. The source game for every rejected group is
removed before rerunning the unchanged hash-ranked selector. Selection must
still produce exactly 64 groups × four parents, each group with one early, two
middle, and one late parent, while preserving all four policy quotas and all
twelve frozen source-opening-stratum quotas.

The replenishment ledger binds the initial five groups and 17 boards learned
from terminal v3e, every later group and board transition, every filtered-pool
hash, exact selection delta, wave marker, cache decision, contiguous wave
domain, and absence of post-abort work. The unique-candidate-parent charge is
the selector's current candidate-parent ceiling plus four for every previously
rejected semantic group and may never exceed 4,096. Ceiling exhaustion or any
stale, foreign, reordered, or tampered state writes a failure ledger and
terminal closeout before failing closed.

Only complete accepted groups enter analysis. Cache objects belonging to any
rejected group, including complete siblings of its incomplete parent, are
ineligible. Report completion attrition by reason, policy, phase, and source
opening stratum. The estimand is explicitly conditioned on computational
completion at the frozen budgets: the mean model effect over the final 64
quota-stratified semantic groups complete for every legal child and bound
model. Recompute the stratified group bootstrap, observed regret variance, and
conservative power gate from that final set; do not reuse a v3e analysis result.

The Rust worker, models, candidate shard, generation contract, base exclusion
set, search constants, and parent records remain byte-identical to the frozen
v3e worker binding. The v3f orchestration and review are bound separately.
Exactly validated v3e cache objects outside every rejected group may be copied
into the new v3f cache and reused only under that byte-identical binding; no
v3f process may write under v3e or rewrite a cache identity. A different
binding, executable, model, input, search contract, or parent record forbids
reuse.

Before `R5R-00-v3f`, fixtures must bind the already passing v3e cache protocol
and additionally pass multi-group failure, quota-preserving group refill,
rejected-sibling exclusion, unaffected-cache reuse, stale/tampered-state
rejection, resume/cold equivalence, worker-binding mismatch, and ceiling-
exhaustion checks. A fresh independent Sol/max review must bind the exact final
plan, orchestration, shared replenishment code, fixtures, unchanged worker and
WASM sources, both passing fixture gates, and the v3e terminal manifest.

The dependency flow is `R5R-00-v3f` → `R5R-01-v3f` → `R5R-02-v3f`. Training,
holdout, GPU-rental, game, and R6 authorizations remain false. A new explicit
GPU approval is still required only after `R5R-02-v3f` passes and before any
training.
