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
