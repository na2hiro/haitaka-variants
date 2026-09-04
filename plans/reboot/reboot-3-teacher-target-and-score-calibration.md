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

### Implementation record (2026-09-02)

R3 is complete and its one-shot teacher decision passed.

- Froze `teacher-probe-v1` before opening results in
  `r0/anhoku-reboot/r3-teacher-probe-contract.json`, SHA-256
  `25cd5eacb9cd45fe4110dd796c0536b1822e2c5b0d85db6e65b6a26ecfde79ba`.
  The deterministic 48-board schedule contains 24 tuning and 24 decision
  boards, is stratified across opening/source policy, early/middle/late and
  terminal-adjacent phase, material and proxy-score buckets, quiet/tactical,
  in-check, promotion, drop, donor-sensitive, and C/16-disagreement positions,
  and stays far below the 50,000-board ceiling. It consumes the frozen R1-A
  corpus plus only the opening-root SFENs from the historical Anhoku-v3
  manifest; no historical label is reused.
- Compared handcrafted searches at 2,000, 10,000, and 50,000 combined nodes
  against a 250,000-node handcrafted reference on identical boards. C/16 at
  50,000 nodes was recorded only as an ineligible diagnostic control. The
  selected 10,000-node teacher is the smallest tuning budget that passed every
  preregistered overall and important-stratum boundary.
- The untouched decision block confirmed the 10,000-node teacher: probability
  RMSE `0.0884569`, robust normalized MAE `0.0879514`, top-1 agreement
  `79.17%`, top-3 agreement `91.67%`, regret/clip `0.0833403`, sign
  disagreement `0`, incomplete-label rate `0`, and exact node-accounting rate
  `1.0`. The 2,000-node lane failed both tuning and decision; the 50,000-node
  lane also passed but is about 5.6 times slower than the selected lane on the
  decision subset.
- Froze label semantics as the side-to-move root backed-up non-mate value
  clipped at `12,000`, plus an explicit win/loss/none mate class and distance
  from `30,000`. Strength training begins teacher-only with `lambda = 1.0` and
  no result loss. The primary objective is normalized robust clipped Huber
  (`delta = 0.10`); the inherited `sigmoid(score / 410)` is rejected. On the
  tuning reference it saturated `54.17%` of targets outside `[0.01, 0.99]`.
  Even the calibrated logistic candidate saturated `4.17%`, above the frozen
  `2%` bound.
- The selected teacher measured `23.1968` labels/second on the tuning probe.
  At one worker and the frozen `$0.20`/core-hour model, the projected label
  costs are `$2.39` for 1M, `$23.95` for 10M, `$119.75` for 50M, and `$239.50`
  for 100M positions. These are planning estimates; R4 must measure pilot
  throughput before scaling.
- The passing machine report is
  `out/anhoku-reboot-r3/r3-gate-report.json`, SHA-256
  `7e97b3a215f2c6e6f03b30a56ce29bb3ac8b23630ea506d9d5445c09f9d7b5cb`.
  Its frozen probe and raw result hashes are respectively
  `50ee6aeedce026f1b10f0db4dd8e389b3c633e87fb28b42c99514862fb7b5afc`
  and `f915540a68d0488e6a9a32e717d71e43fb8a2fbd41ef76e464e4e0a5b7a3cfed`.
  All 12 named gates are `true`, and a second invocation fails closed because
  the one-shot output directory is non-empty.

Final verification completed:

- `target/release/haitaka_learn r3-gate`
- `cargo test -p haitaka_learn --features anhoku` (144 tests, zero failures)
- `cargo check -p haitaka_learn --features anhoku`, `cargo fmt --all -- --check`,
  and `git diff --check`

No production corpus, model training, result-loss lane, external teacher,
strength game, or R4 generation was run. The passing report authorizes R4 under
the frozen R3 identities and cost forecast.

