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

