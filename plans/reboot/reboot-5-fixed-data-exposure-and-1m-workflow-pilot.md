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

## R5 repair / R6-unblock worker assignment (planned; no GPU authorized)

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
r0/anhoku-reboot/r5-repair-contract-v1.json; same recipe is not a sufficient
specification.

| Area | Requirement |
| --- | --- |
| Inputs | Commit a867d89; passing replacement-R4 contract/report and sole 880k/60k/60k pilot split; C/16; existing deep-core; complete failed-R5 evidence. Failed R4 payloads and old checkpoints are ineligible. |
| Seeds | New from-scratch runs in strict order 41, 53, 65 after R5R-00 verifies they are unused. Never resume or initialize from seed 17/29. Seed 41 is the predeclared game model. |
| Exposure | Checkpoint/evaluate every 64 updates; minimum 1,024; hard maximum 2,048; no extension. |
| Convergence | At u at least 1,024, use the five checkpoints from u-256 through u. For both validation and development loss, max(window) - min(window) must be at most 0.00002. Require two consecutive qualifying checkpoints. |
| Selection | Lowest finite development loss through convergence, then validation loss, development MAE, earlier optimizer update, and lane ID. No convergence by 2,048 is failure. |
| Metric signs | Improvement is C/16 loss minus candidate loss; quantization degradation is exported loss minus full-precision loss; regret improvement is C/16 regret minus candidate regret. |
| Offline gates | Each seed's fresh-stage full/export point improvement must be positive. For the arithmetic mean across three fixed seeds, multiplicity-adjusted one-sided 95% simultaneous bounds require full-precision improvement above 0.005, exported improvement above zero, exported-10k-search regret improvement above zero, and quantization degradation below 0.02. Apply the 95% retained-gain gate only when the full-precision improvement lower bound is at least the original 0.01 denominator floor. |
| Reusable regret | Deterministically select 256 existing deep-core positions; label every legal child in canonical USI order at 250,000 nodes with root-oriented score and mate class/distance. The gated endpoint uses the move returned by frozen zero-fallback 10k search with the exported model. Full-static and exported-static choices are diagnostic only. |
| Fresh stage | 64 new opening_group_id values times 500 completed 10k-node root labels = 32,000 positions; four of those positions per group receive all-legal-move labels. Freeze exact group meaning and source/phase marginal quotas. |
| Fresh games | 128 new game_opening_group_id values times four color-swapped pairs = 512 pairs/1,024 games; 10k nodes/move, one thread, concurrency one, no random plies or sequential peeking. The grouped paired 95% Elo lower bound must exceed -20. |
| Statistics | Within each seed aggregate legal moves to position and positions to opening_group_id, then average the three fixed-seed group effects. Bootstrap opening_group_id jointly across seeds; do not resample seeds. The game unit is game_opening_group_id. source_game_id and canonical_board_id are leakage keys, not resampling units. Freeze the 100,000-draw bootstrap, PRNG seed, quantile convention, simultaneous family/correction, and grouped Elo estimator. |
| Completion | A complete usable label has a target from at least one completed depth and reconciled actual nodes. A later interrupted iteration with partialIteration=true remains valid. Depth-zero/null-target attempts are incomplete and may not become partial scores or artificial draws. Consume candidates in frozen order until the exact quota or a frozen replenishment ceiling; record all rejects and fail if the quota is not filled. |
| Holdout order | Freeze stage/game generator code, seeds, quotas, and overlap rules before training. Generate and hash both suites only after all six new model hashes are frozen and before stage evaluation. Keep the game suite unopened until the stage passes. No model changes or reruns after generation. |
| Independence | Stage, game, and reusable artifacts must be mutually disjoint and disjoint from prior R4/R5 data and matches by group, source_game_id, and canonical_board_id. |
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

R5R-00 through R5R-04 advance only on pass. R5R-05 always runs.

### Worker assignments

#### R5R-00 — Bind history and diagnostic design

- Owner/deps: provenance/experiment worker plus gate reviewer; none.
- Work/output: inventory the contract inputs and verify seeds 41/53/65 are
  unused. Freeze legal-move schema, orientation/mate rules, candidate order,
  replenishment ceiling, stage/game group definitions, quotas, estimands,
  power methods, and CPU diagnostic ceiling. Write
  out/anhoku-reboot-r5-repair/preflight/negative-history-inventory.json and
  out/anhoku-reboot-r5-repair/preflight/diagnostic-contract.json.
- Pass/stop: every identity resolves and the old failure, ineligible weights,
  absent stage, unused seeds, completion rules, design, and finite cost are
  explicit. Otherwise stop before generation.

#### R5R-01 — CPU-only diagnosis, power, and cost

- Owner/deps: evaluation/CPU workers plus independent auditor; R5R-00 pass.
  Core-code changes require gate review.
- Work/output: verify orientation, mate, replay, and node accounting; build and
  audit the reusable 256-position artifact; compare immutable seed-17 export,
  seed-29 export, and C/16 on exported-10k-search regret; run existing-model
  native/WASM correctness; and publish diagnostic, power, throughput, repair
  cost, peak-storage, and retention reports under
  out/anhoku-reboot-r5-repair/diagnostic/, with
  out/anhoku-reboot-r5-repair/diagnostic/diagnostic-gate.json as the index.
  Keep historical full-precision and static-choice results separate from the
  required search-regret measurement.
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
  out/anhoku-reboot-r5-repair/preflight/successor-contract-audit.json. Freeze
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
  Emit one immutable out/anhoku-reboot-r5-repair/runs/seed-{41,53,65}/ tree per
  executed seed and
  out/anhoku-reboot-r5-repair/evaluation/sequential-screen.json with checkpoint
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
  out/anhoku-reboot-r5-repair/stage/, including stage-gate.json.
- Stage pass/stop: every count, hash, quota, label, node, identity,
  independence, per-seed direction, and Contract boundary offline gate passes.
  Failure forbids replacement data, re-bootstrap, retraining, and games.
- Game/output: only after stage pass, open the hashed game suite once and run
  seed 41 versus C/16 for all 1,024 games. Preserve every game, termination
  reason, node total, and pair bin under
  out/anhoku-reboot-r5-repair/games/, including equal-node-gate.json.
- Game pass/stop: all 512 pairs, zero unknown/capped outcomes, zero
  identity/node/move/fallback faults, and grouped 95% Elo lower bound above
  -20. A harness defect invalidates and routes to reviewed repair; a clean
  failure rejects the candidate without a larger or repeated match.

#### R5R-05 — Unconditional closeout and R6 decision

- Owner/deps: provenance/cost auditor plus gate reviewer; any terminal result
  from R5R-00 through R5R-04.
- Work/output: record pass/fail/skipped tasks; reconcile CPU/GPU/node hours,
  cash, storage, invoices, and remote-instance state; verify local hash-checked
  raw evidence; preserve old R5 evidence; and write
  out/anhoku-reboot-r5-repair/closeout/cost-retention-audit.json and
  out/anhoku-reboot-r5-repair/closeout/r5-repair-final-gate.json.
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
