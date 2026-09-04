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

#### R1-A implementation record (2026-09-01)

Implemented and passed on branch `reboot`:

- Added `r1a-gate`, which builds a deterministic 10,240-position Anhoku corpus
  in stable fixture-ID order without RNG, filtering, cycling, or sampling. The
  corpus contains 10,240 distinct canonical packed boards and complete
  color-swapped/rotated pairing.
- Added an independent packed-bitstream decoder and feature-signature oracle.
  All 10,240 boards pass
  `pack(unpack(pack(board))) == pack(board)` and signature equality. The report
  separately counts the expected gold-like ABI identity collisions.
- Added a C++ loader audit selector for the real HalfKAv2 plus DonorSingleEff
  rows (excluding factorized training aliases) and a standalone dump helper.
  The 20,480 Rust/C++ perspective dumps are byte-identical.
- Added an in-memory row-identity sentinel transformer with eight independent
  bounded `i16` and `i32` signatures per row. Incremental and full-refresh
  accumulator states are exact across 10,202 move transitions and restored
  parent snapshots.
- Added independent side-to-move depth-0 and hand-checkable kings-only depth-1
  score oracles, packed-record score/side checks in the C++ loader, and exact
  color-swap score checks. All pass. The depth-0 oracle explicitly preserves
  the frozen handcrafted mobility convention: it counts move destinations,
  so optional promotion alternatives on one destination count once.
- Machine-checked coverage includes both sides, every piece type, every output
  bucket, captures, promotions, drops, king moves, checks, double checks,
  terminal-adjacent positions, empty and maximum hands, and donor/receiver
  relation gain, removal, and replacement.
- Gate artifacts and their SHA-256 identities are recorded in
  `out/anhoku-reboot-r1a/r1a-gate-report.json`; `passed` and all nine named
  gates are `true`.

CPU-only verification completed:

- `target/release/haitaka_learn r1a-gate --config haitaka_learn.anhoku-reboot-r1a.training.toml --output-dir out/anhoku-reboot-r1a`
- `cargo test -p haitaka_learn --features anhoku` (128 passed)
- `cargo test -p haitaka_wasm --features anhoku` (86 passed)
- `cargo check -p haitaka_learn --features anhoku`
- `target/release/haitaka_learn r0-gate --bundle r0/anhoku-reboot`
- `cargo fmt --all -- --check` and `git diff --check`

No GPU, labels, training, model-strength game, or production data generation
was run. R1-B, R1-C, R1-D, and R2 remain unauthorized by this result.

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

#### R1-B implementation record (2026-09-01)

Implemented and passed on branch `reboot`:

- Froze absolute quantization limits before executing the oracle in
  `r0/anhoku-reboot/r1b-quantization-limits.json`: mean absolute runtime-score
  delta at most 8, p99 at most 16, maximum at most 32, and positive
  probability-MSE degradation at most 0.005 in every declared split and score
  bucket. Serializer weight clamps and accumulator overflows must both be zero.
- Added `r1b-gate` and a source-generated complete full-precision deployment
  checkpoint whose tensors are evaluated by PyTorch on CPU. The construction
  includes zero and bias-only networks, positive/negative one-hot rows,
  perspective- and donor-asymmetric signatures, activation clamp boundaries,
  ties-to-even cases, and `i16` transformer extrema.
- Added an independent Python serializer/parser and exact-integer emulator. It
  records both accumulator perspectives, PSQT, transformed input, both affine
  pre-activations and clipped activations, output, and final score for every
  R1-A fixture. Rust compares those traces against the production parser,
  full-refresh accumulator, detected affine kernel, and incremental update.
- Checkpoint regeneration, repeat export from the same checkpoint, and repeat
  canonical export metadata are byte-identical. The two 317,797,280-byte
  full-precision checkpoints have the same SHA-256; the two 161,206,504-byte
  sentinel exports also have the same SHA-256.
- Zero, bias-only, and full sentinel cases each pass 10,240 position checks and
  10,202 move-transition checks. Across all 30,720 predictions and 30,606
  transitions there are zero Python/Rust score mismatches, zero per-layer
  activation mismatches, zero full/incremental accumulator or score mismatches,
  and zero restored-parent snapshot mismatches.
- Full-precision-to-quantized degradation is reported for the complete corpus,
  each parity split, each reference-score bucket, and every split/bucket cross.
  Overall mean/p99/maximum absolute score delta is
  `2.9240 / 12.6003 / 13.7455`; the worst group mean and p99 are `7.5947` and
  `13.1960`, and the largest positive group loss degradation is below `0.00104`.
  Every frozen absolute gate passes. Serializer weight clamps and accumulator
  overflows are both zero; activation clamp counts are explicit in the report.
- The machine-readable report and SHA-256 artifact inventory are in
  `out/anhoku-reboot-r1b/r1b-gate-report.json` and all nine named gates are
  `true`.

Final CPU-only verification completed:

- `target/release/haitaka_learn r0-gate --bundle r0/anhoku-reboot`
- `target/release/haitaka_learn r1a-gate --config haitaka_learn.anhoku-reboot-r1a.training.toml --output-dir out/anhoku-reboot-r1a`
- `target/release/haitaka_learn r1b-gate --r1a-dir out/anhoku-reboot-r1a --output-dir out/anhoku-reboot-r1b --limits r0/anhoku-reboot/r1b-quantization-limits.json --python ../engine/variant-nnue-pytorch/.venv/bin/python`
- `cargo test -p haitaka_learn -p haitaka_wasm --features anhoku` (130 and
  86 tests passed)
- `cargo check -p haitaka_learn --features anhoku`,
  `cargo fmt --all -- --check`, and `git diff --check`

No GPU, labels, training, model-strength game, or production data generation
was run. R1-C is now authorized; R1-D, R2, and production corpus generation
remain unauthorized by this result.

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

#### R1-C implementation record (2026-09-01)

Implemented and passed on branch `reboot` without a GPU:

- Froze the CPU-only oracle, exact targets, optimizer settings, 8,192-position
  subset, sentinel rows, collision fixtures, and pass thresholds before opening
  results in `r0/anhoku-reboot/r1c-learnability-contract.json`.
- Added `r1c-gate`. Its independent PyTorch helper trains constant, side/sign,
  coalesced-material, and 16 production-row sentinel targets, then fits the
  complete deployment checkpoint through the real bucketed DonorSingleEff PSQT
  path and exports it with the independent R1-B serializer.
- Every exactly representable oracle passed the frozen final/initial loss ratio
  of `0.000001`. The deployment overfit probability loss fell from
  `0.0668115660` to `0.0000232274`, or `0.03477%` of its initial value, exceeding
  the required 90% reduction by a wide margin.
- Serialized loss was `0.0000231965` and retained `100.000046%` of the measured
  full-precision reduction. Every R1-B split, score bucket, and split/bucket
  cross passed the frozen absolute score-delta and loss-degradation limits;
  serializer clamps and accumulator overflows were both zero.
- Across 8,192 positions and 8,154 move transitions, Python integer inference,
  Rust full refresh, and Rust incremental inference had zero score,
  accumulator, restored-parent, or transition mismatches. Repeat serialized
  exports were byte-identical.
- The fixed corpus contains 412 promoted gold-like instances in 112/8,192
  positions (`1.3671875%`). Eight explicit Black/White Gold-versus-promoted-minor
  pairs prove identical packed bytes and active features despite an exact
  50-unit handcrafted-material difference. This is a measured structural
  collision, not evidence that it currently limits strength.
- The machine-readable report and SHA-256 inventory are in
  `out/anhoku-reboot-r1c/r1c-gate-report.json`; all six top-level gates and all
  nested Python gates are `true`.

Final CPU-only verification completed:

- `target/release/haitaka_learn r0-gate --bundle r0/anhoku-reboot`
- `target/release/haitaka_learn r1a-gate --config haitaka_learn.anhoku-reboot-r1a.training.toml --output-dir out/anhoku-reboot-r1a-r1c-regression`
- `target/release/haitaka_learn r1b-gate --r1a-dir out/anhoku-reboot-r1a-r1c-regression --output-dir out/anhoku-reboot-r1b-r1c-regression --limits r0/anhoku-reboot/r1b-quantization-limits.json --python ../engine/variant-nnue-pytorch/.venv/bin/python`
- `target/release/haitaka_learn r1c-gate --r1a-dir out/anhoku-reboot-r1a --r1b-dir out/anhoku-reboot-r1b --output-dir out/anhoku-reboot-r1c --contract r0/anhoku-reboot/r1c-learnability-contract.json --limits r0/anhoku-reboot/r1b-quantization-limits.json --python ../engine/variant-nnue-pytorch/.venv/bin/python`
- `cargo test -p haitaka_learn -p haitaka_wasm --features anhoku`
- `cargo check -p haitaka_learn --features anhoku`, `cargo fmt --all -- --check`,
  and `git diff --check`

No GPU, labels, production data generation, self-play, or model-strength game
was run. R1-D1 is now authorized; R1-D2/D3, R2, and production corpus generation
remain unauthorized by this result.

### R1-D: interruption-safe search and honest self-play

R1-D is not one worker assignment. It is three separately authorized steps,
each ending in its own commit, machine-readable report, gate command, and
closeout entry. R1-C must pass before R1-D1 begins; R1-D2 is unauthorized until
R1-D1 passes; R1-D3 is unauthorized until R1-D2 passes. A worker assigned one
step must not implement the next step in the same assignment, except for the
minimal typed interface scaffolding required by the current step.

| Step | Scope | Prerequisite | Exit evidence |
| --- | --- | --- | --- |
| R1-D1 | Interrupted root-result contract and exact node accounting | R1-C passed | Search-result contract, forced-interruption corpus, `r1d1-gate` |
| R1-D2 | Repetition/history, TT, qsearch, DFPN, and adjudication | R1-D1 passed | History/adjudication golden corpus, `r1d2-gate` |
| R1-D3 | Match integrity, production timing, and null qualification | R1-D2 passed | Complete-pair null reports, timing/equivalence report, `r1d3-gate` |

#### R1-D1: interrupted root result and node accounting

**Objective:** every search API returns an honest, typed result under tiny time
or node budgets, without conflating a legal play move with a trustworthy label
value.

Required work:

- Seed search with a legal root move, but distinguish an unsearched emergency
  move from a searched best-so-far move.
- Publish the best move after every completed root child so an interrupted
  iteration preserves useful partial-root work instead of discarding the whole
  iteration.
- Keep the play and value contracts separate. The result type must expose at
  least `play_move_best_so_far`, whether that move was searched,
  `last_completed_iteration_value`, completed iteration depth, completed root
  moves in the interrupted iteration, partial-root state, interruption reason,
  emergency-fallback use, and missing-move state.
- A partial-root move is legal for play but its order-dependent partial value is
  never a training label. Label generation uses only the last fully completed
  value under its frozen target semantics or rejects the position as incomplete.
- Define the node budget as the versioned combined count
  `alpha_beta_nodes + qnodes`. Preserve full qsearch telemetry, including depth,
  cap, check-move, and pruning counters.
- Make in-process, USI, fixed-time, node-budget, and production-interface
  adapters expose the same result semantics. Remove lexicographic or otherwise
  arbitrary fallback substitution from these paths.
- Add deterministic forced-interruption tests with an artificially slow
  evaluator and tiny limits: before any root child finishes, immediately after
  one root child, during a later root child, between completed iterations, and
  inside qsearch.

R1-D1 is deliberately not allowed to change repetition rules, TT history
semantics, DFPN policy, game adjudication, match statistics, or promotion
thresholds; those belong to D2 and D3.

Before opening the forced-interruption results, freeze the typed result schema,
node-counting version, corpus cases, and exact assertions in
`r0/anhoku-reboot/r1d1-search-contract.json`. The worker must add `r1d1-gate`
and publish `out/anhoku-reboot-r1d1/r1d1-gate-report.json` with hashes of the
executable, contract, fixtures, and raw traces.

R1-D1 passes only when:

- every nonterminal test position returns a legal move;
- an unsearched emergency move appears only when no root child completed and is
  explicitly marked; after one completed root child, the returned play move is
  searched;
- partial-root values never enter generated labels and the completed-value field
  is exact in every interruption case;
- combined consumed nodes never exceed the request, exact-exhaustion cases match
  the declared budget, and alpha-beta plus qnodes reconciles exactly;
- qsearch telemetry is complete and internally consistent;
- all adapters agree on move/value/completeness semantics; and
- R0 and R1-A/B/C gates plus the affected Rust tests still pass.

Stop after the D1 report and closeout. Do not run self-play qualification games
or begin history/adjudication work.

##### R1-D1 implementation record (complete, 2026-09-01)

- Added the frozen `haitaka-search-root-result-v1` contract and deterministic
  forced-interruption fixtures. Search results now separate the legal play move
  from `last_completed_iteration_value`, publish root progress after every
  completed child, and explicitly identify searched moves, partial-root state,
  interruption reason, emergency use, and missing moves.
- Removed the lexicographic CLI/USI substitution path. Timed, fixed-depth,
  fixed-node, in-process, USI, CLI, and WASM-facing adapters now carry the same
  typed semantics. A legal unsearched emergency seed is used only before any
  root child completes; a searched partial-root best move supersedes it after
  the first completed child.
- Versioned combined node accounting as `alpha-beta-plus-qsearch-v2`. Admission
  counts each alpha-beta or qsearch node exactly once before its body executes;
  the combined count never exceeds the request. Interrupted node searches now
  preserve complete qsearch depth, cap, check-move, and pruning telemetry.
- Training labels and training traces use only the last fully completed
  iteration. The forced partial-root and tiny-budget cases publish no completed
  value or training trace.
- Added `r1d1-gate`. Its artificially slow deterministic fixture controller
  covers interruption before a root child, after one root child, during the
  second child, between iterations, inside qsearch, and at exact combined
  budgets of one and three nodes. The one-node case consumes exactly `1 + 0`
  alpha-beta/qsearch nodes and marks a legal emergency move; the three-node case
  consumes exactly `2 + 1`, preserves one searched root move, and publishes no
  partial label value.
- The machine-readable report and SHA-256 inventory are in
  `out/anhoku-reboot-r1d1/r1d1-gate-report.json`; all eight named gates and all
  five adapter-parity checks are `true`.

Final CPU-only verification completed:

- `target/release/haitaka_learn r0-gate --bundle r0/anhoku-reboot`
- `target/release/haitaka_learn r1a-gate --config haitaka_learn.anhoku-reboot-r1a.training.toml --output-dir out/anhoku-reboot-r1a-r1d1-regression`
- `target/release/haitaka_learn r1b-gate --r1a-dir out/anhoku-reboot-r1a-r1d1-regression --output-dir out/anhoku-reboot-r1b-r1d1-regression --limits r0/anhoku-reboot/r1b-quantization-limits.json --python ../engine/variant-nnue-pytorch/.venv/bin/python`
- `target/release/haitaka_learn r1c-gate --r1a-dir out/anhoku-reboot-r1a-r1d1-regression --r1b-dir out/anhoku-reboot-r1b-r1d1-regression --output-dir out/anhoku-reboot-r1c-r1d1-regression --contract r0/anhoku-reboot/r1c-learnability-contract.json --limits r0/anhoku-reboot/r1b-quantization-limits.json --python ../engine/variant-nnue-pytorch/.venv/bin/python`
- `target/release/haitaka_learn r1d1-gate --r1a-dir out/anhoku-reboot-r1a-r1d1-regression --r1b-dir out/anhoku-reboot-r1b-r1d1-regression --r1c-dir out/anhoku-reboot-r1c-r1d1-regression --output-dir out/anhoku-reboot-r1d1 --contract r0/anhoku-reboot/r1d1-search-contract.json --fixtures r0/anhoku-reboot/r1d1-forced-interruption-fixtures.json`
- `cargo test -p haitaka_learn -p haitaka_wasm -p haitaka_cli --features anhoku`
- `cargo check -p haitaka_learn -p haitaka_wasm -p haitaka_cli --features anhoku`,
  `cargo fmt --all -- --check`, and `git diff --check`

No GPU, labels, production data generation, self-play, model-strength game,
repetition/adjudication change, TT-history change, or DFPN-policy change was
made. R1-D2 is now authorized; R1-D3, R2, and production corpus generation
remain unauthorized by this result.

#### R1-D2: history, repetition, TT, qsearch, DFPN, and adjudication

**Objective:** identical board layouts are evaluated according to their actual
game and line history, with correct Anhoku terminal semantics throughout search
and self-play.

Required work:

- Pass root game history into every search API and maintain line-local history
  in alpha-beta, qsearch, and root DFPN. Game-loop-only repetition detection is
  insufficient.
- Implement the Anhoku repetition and perpetual-check rules, including the
  correct loser/draw result and any applicable entering-king rule.
- Because identical boards can have different values under different
  repetition counts or checking histories, either include the necessary
  repetition context in TT identity or suppress TT probe/store at
  history-sensitive nodes.
- Replace `terminal_winner`'s generic non-ongoing-as-loss behavior with explicit
  win, loss, draw, repetition/adjudication, and maximum-ply handling. Record a
  versioned termination reason and define terminal-on-final-max-ply precedence.
- Root DFPN must obey the D1 move/value result contract, use the same monotonic
  deadline and combined-node budget, and expose completion/interruption
  metadata. Unless it proves a terminal root result, it must not consume the
  complete move budget without permitting a searched legal root move to be
  published; freeze a reservation or interleaving policy.
- Add same-board/different-history fixtures and golden lines covering ordinary
  fourfold repetition, perpetual-check loss for either side, qsearch repetition,
  TT contamination attempts, DFPN repetition, deadline expiry within DFPN,
  terminal-on-final-max-ply precedence, and entering-king if applicable.

R1-D2 must not run match-equivalence games or tune timing/statistical thresholds;
those belong to D3.

Before executing the golden corpus, freeze the rules/adjudication version,
history representation, TT policy, DFPN budget policy, termination precedence,
and expected outcomes in `r0/anhoku-reboot/r1d2-history-contract.json`. Add
`r1d2-gate` and publish
`out/anhoku-reboot-r1d2/r1d2-gate-report.json` with raw trace and artifact hashes.

R1-D2 passes only when:

- every golden history produces the exact expected result, termination reason,
  score orientation, and legal move behavior in all applicable search APIs;
- same-board/different-history cases cannot contaminate one another through TT;
- alpha-beta, qsearch, DFPN, USI, in-process, and production-interface history
  semantics agree;
- DFPN interruption respects the frozen budget/reservation policy and preserves
  the D1 searched-move contract;
- all self-play outcomes distinguish real draws, adjudications, capped games,
  and unfinished games; and
- R0, R1-A/B/C, and R1-D1 gates plus the affected Rust tests still pass.

Stop after the D2 report and closeout. Do not run A=A games or begin match
qualification.

##### R1-D2 closeout (2026-09-02)

R1-D2 is complete and all machine gates pass.

- Froze `anhoku-history-adjudication-v1` before the successful golden run in
  `r0/anhoku-reboot/r1d2-history-contract.json`. The representation is an
  ordered inclusive sequence of `Board` positions; search pushes and pops the
  same history through alpha-beta, qsearch, and DFPN.
- Fourfold repetition is a draw unless exactly one side checked after every one
  of its moves over the repetition interval, in which case that checker loses.
  Score orientation is always side-to-move. Per the 2026-09-01 rules decision,
  27-point declaration is not implemented and explicit 持将棋 adjudication is a
  draw.
- Alpha-beta and DFPN transposition identities include the ordered history and
  checking context. The golden same-layout/different-history probe produced no
  cross-context TT hit.
- USI now retains every board from `position ... moves`; native handcrafted,
  native NNUE, node-budget, timed, external-engine, and self-play adapters pass
  the same root history. External self-play sends the original SFEN plus the
  complete move list instead of sending only the current layout.
- Self-play game records use schema version 3 and
  `haitaka-self-play-termination-v1`, distinguishing wins, repetition and
  持将棋 draws, perpetual-check adjudication, maximum-ply caps, engine-missing
  moves, and unfinished games. A terminal result on the final allowed ply takes
  precedence over the cap.
- Timed root DFPN is skipped below 4 ms and otherwise receives at most one
  quarter of the deadline (capped at 25 ms), reserving at least three quarters
  for alpha-beta. Node-budget search assigns DFPN zero nodes. DFPN reports
  completion, interruption reason, repetition hits, nodes, elapsed time, and
  the reserved alpha-beta time; interrupted/unproven DFPN falls through to a
  searched legal move.
- `r1d2-gate` covers ordinary repetition, either perpetual checker losing,
  root alpha-beta/qsearch agreement, TT contamination, DFPN repetition and
  zero-millisecond deadline interruption, the sub-4-ms reservation, USI parity,
  and terminal-on-final-max-ply precedence. All ten named gates are `true` in
  `out/anhoku-reboot-r1d2/r1d2-gate-report.json`; raw trace and artifact hashes
  are recorded there.

Final CPU-only verification completed:

- `target/release/haitaka_learn r0-gate`
- `target/release/haitaka_learn r1a-gate`
- `target/release/haitaka_learn r1b-gate`
- `target/release/haitaka_learn r1c-gate`
- `target/release/haitaka_learn r1d1-gate`
- `target/release/haitaka_learn r1d2-gate`
- `cargo test --workspace --features anhoku --no-fail-fast` (342 tests plus
  doctests, zero failures)
- `cargo check --workspace --features anhoku`, `cargo fmt --all -- --check`,
  and `git diff --check`

No A=A, match-equivalence, timing-qualification, or strength games were run.
R1-D3 is now authorized; R2 and production corpus generation remain
unauthorized by this result.

#### R1-D3: honest match harness and production timing qualification

**Objective:** prove that the match harness measures evaluator strength rather
than missing moves, malformed pairs, timing asymmetry, load contention, or
adjudication bugs.

Required work:

- A nonterminal missing move is a harness error that aborts the match; it is
  never scored as a loss. Qualifying runs require zero missing moves and zero
  unsearched emergency fallbacks. Searched partial-root moves are permitted only
  under the frozen D1 policy and are reported separately by engine.
- Require every scheduled pair to be complete or deterministically retried,
  uniquely indexed, based on the identical opening/start SFEN, and played with
  opposite colors and engine order. Recompute pair/pentanomial bins from raw
  games; malformed or incomplete pairs fail closed rather than being excluded.
- Exercise the R0-frozen production path: shipped WASM interface, complete
  history-bearing USI search path, one concurrent game, declared host/device
  class, and frozen cold/warm protocol. Native/in-process runs are equivalence
  diagnostics, not substitutes.
- Freeze the monotonic clock/deadline-controller version. Record requested and
  elapsed time, deadline lateness, scheduler delay where measurable, cold/warm
  state, completed depth/root-child distributions, interruption reasons,
  combined node counts, missing/emergency moves, and termination reason for
  every search.
- Before games, preregister numeric p95, p99, and maximum lateness limits,
  engine-symmetry bounds, the A=A zero-bias equivalence margin, A/B order-reversal
  tolerance, opening groups, pair count, and fixed-cap analysis. Do not use an
  ordinary confidence interval after sequential peeking.
- Qualify with enough independent opening groups that the A-versus-identical-A
  paired interval lies wholly inside the zero-bias equivalence margin. For a
  distinct deterministic A/B diagnostic, replay the identical pair schedule
  with engine order reversed and require sign-transformed estimates to agree
  within tolerance; the A/B effect itself is not expected to be zero.
- Match identity includes model/executable hashes, resolved thread count, CPU,
  affinity, compiler flags, memory configuration, concurrency, opening-suite
  hash, adjudication, clock/controller, search-limit, and cold/warm versions.

Freeze the complete protocol in `r0/anhoku-reboot/r1d3-match-contract.json`
before launching null games. Add `r1d3-gate` and publish
`out/anhoku-reboot-r1d3/r1d3-gate-report.json` plus hash-frozen raw games.

R1-D3 passes only when:

- every scheduled pair is valid and complete and all bins recompute exactly;
- every game has an explicit legal termination reason;
- qualifying games have zero missing moves and zero unsearched emergency
  fallbacks;
- each evaluator passes the frozen p95, p99, maximum lateness, and symmetry
  limits under one-game production concurrency;
- the A=A interval lies wholly inside the equivalence margin and the A/B
  sign-transformed order-reversal estimates agree within tolerance;
- the shipped production-interface result agrees with the declared native
  equivalence diagnostics; and
- R0, R1-A/B/C, R1-D1, and R1-D2 gates plus the affected Rust tests still pass.

Only null/equivalence qualification games are authorized. Stop after the D3
report and R1 closeout; do not run a candidate-versus-handcrafted strength match.

##### R1-D3 and aggregate R1 closeout (worker-reported, 2026-09-02; authorization superseded below)

The worker reported R1-D3 and the aggregate R1 gate complete. No
candidate-versus-handcrafted strength match, production corpus generation,
labels, or GPU training was run.

- Froze `haitaka-r1d3-match-contract-v1` before launching games in
  `r0/anhoku-reboot/r1d3-match-contract.json`, including eight independent
  opening groups, 16 complete color-swapped games per lane, fixed-cap analysis,
  nearest-rank timing percentiles, 8/15/25 ms p95/p99/maximum lateness limits,
  a +/-20 Elo A=A equivalence margin, and a 25 Elo A/B order-reversal tolerance.
- Added a production browser harness that builds the release Anhoku WASM with
  SIMD128, instantiates the shipped `UsiEngine` in one dedicated Chrome module
  Worker, loads the R1-B zero debug NNUE exactly once, sends `usinewgame` before
  each game, and replays the full start SFEN and move history before every
  search. The worker closeout claimed that the raw trace bound the WASM, glue,
  model, openings, contract, browser/host identity, and per-search evidence.
  The independent audit below found that this provenance claim was not actually
  encoded or enforced and therefore retracts the resulting R2 authorization.
- The A=A timing/null lane completed 16/16 games and 8/8 pairs. All eight pair
  scores were exactly 0.5, producing `0 Elo [0, 0]`. Across 96 warm 100 ms
  searches, p95/p99/maximum lateness was
  `0.1000 / 0.2000 / 0.2000 ms`; mean A/B elapsed-time difference was below
  `0.023 ms`, and cold WASM plus 161,206,500-byte NNUE load was `644.6 ms`.
- The fixed-depth A/B forward and reversed lanes each completed 16/16 games and
  8/8 pairs. Their sign-transformed paired estimates differed by `0 Elo`, within
  the frozen 25 Elo tolerance. The A/B lanes are deterministic order diagnostics,
  not model-strength evidence.
- All 48 games ended with explicit `maximum-ply-capped` qualification reasons.
  Every scheduled pair was uniquely indexed, byte-identical at its start, and
  color/engine-order swapped. Recomputed pentanomial bins exactly matched the
  browser summaries. There were zero missing moves and zero unsearched emergency
  fallbacks; 57 searched partial-root moves in the timed lane were retained and
  reported under the frozen D1 policy.
- The shipped WASM interface, native history-bearing `UsiSession`, and native
  in-process incremental adapter agreed exactly on both fixed-depth fixtures,
  including best move and typed root-result fields.
- Native self-play now treats any nonterminal missing move as a harness error
  that aborts the match instead of assigning a loss. The WASM USI path also
  emits an explicit `gameover no-legal-move` reason for a genuine terminal
  resign, so a terminal board cannot be confused with an engine failure.
- Repaired two aggregate-rerun identity defects without changing an oracle or
  threshold. R0 no longer treats mutable `target/release/haitaka_cli` as a
  historical artifact; the hash-frozen historical reports remain the evidence
  and already record their actual executable identities. Mutable R1-A/B report
  wrapper hashes were removed from R1-C's frozen artifact set; R1-C still
  requires both reports to pass, while its stable corpus, features, serializer,
  quantization limits, targets, optimizer, and pass thresholds remain
  hash-frozen and byte-identical.
- `r1d3-gate` passes all 12 named gates in
  `out/anhoku-reboot-r1d3/r1d3-gate-report.json`. The hash-frozen browser trace
  is `out/anhoku-reboot-r1d3/browser-trace.json`, and detailed timing,
  equivalence, lane, host, and pair analysis is in
  `out/anhoku-reboot-r1d3/r1d3-analysis.json`.
- All six R1 reports were rerun from the same release executable SHA-256
  `d04b6ed7fb5ae0429c807464af60ba02bde2ee287577e85ea61d715b931f411f`.
  `out/anhoku-reboot-r1d3/r1-closeout-manifest.json` hashes every report and
  records `allReportsPassing=true`, `sourceIdentityExact=true`, and
  `r2Authorized=true`.

Final CPU/browser verification completed:

- `target/release/haitaka_learn r0-gate`
- `target/release/haitaka_learn r1a-gate`
- `target/release/haitaka_learn r1b-gate`
- `target/release/haitaka_learn r1c-gate`
- `target/release/haitaka_learn r1d1-gate`
- `target/release/haitaka_learn r1d2-gate`
- `target/release/haitaka_learn r1d3-gate`
- SIMD-enabled release `wasm-pack build`, followed by the dedicated-Worker
  Chrome qualification harness
- `cargo test --workspace --features anhoku --no-fail-fast`,
  `cargo check --workspace --features anhoku`, `cargo fmt --all -- --check`,
  and `git diff --check`

The worker-reported gates exited successfully, but the post-closeout audit below
found fail-open provenance and prior-report validation. R2 authorization is
therefore retracted until the remediation gate passes. Later phases and
production corpus generation remain unauthorized.

##### Post-closeout independent audit and remediation assignment (required, 2026-09-02)

An independent Luna/max audit reran the R0 gate, rechecked the recorded R1
reports, and verified every artifact currently listed by the aggregate closeout
manifest. It did not rerun the R1 gates, builds, or browser harness. Inspection
nevertheless found that the recorded evidence chain does not yet prove that the
qualifying browser trace was produced by those artifacts:

- `out/anhoku-reboot-r1d3/browser-trace.json` records sizes and versions but not
  the SHA-256 identities of the WASM module, JavaScript glue/worker/harness,
  NNUE, opening suite, R1-D3 contract, source/build identity, or resolved search
  protocol that produced the games.
- `scripts/r1d3-browser-worker.js` does not emit those identities before play.
  `r1d3-gate` hashes files available at analysis time and checks the model byte
  length, which cannot prove that the browser consumed those same bytes.
- R1-D1/D2/D3 prior-stage checks can accept a report from its top-level
  `passed=true` value or a literal gate result without revalidating the prior
  report schema, named gates, contract identity, executable/source identity,
  and artifact hashes.
- The source-cleanliness check is based on tracked diffs and can report
  `gitDirty=false` while untracked files exist. Unrelated large archives may be
  explicitly excluded, but all relevant untracked source, generated source,
  configuration, scripts, and submodule/trainer state must be included or make
  a strength-evidence gate fail closed.

The next worker must implement only the following remediation:

1. **Bind provenance at trace production time.** Before the first browser game,
   compute and emit a versioned provenance envelope containing SHA-256 and byte
   count for the exact WASM module, generated JavaScript glue, browser worker,
   harness, NNUE, opening suite, and frozen R1-D3 contract. Also record the
   release executable/source identity, browser and host/device class, one-game
   concurrency, clock/deadline controller, adjudication, history/repetition,
   DFPN, search-limit, node-accounting, cold/warm, and model-load identities.
   Per-search records must reference that immutable envelope; the envelope is
   finalized before any result is observed.
2. **Verify the producer-bound envelope.** Make `r1d3-gate` reject a missing,
   duplicate, late, unknown-version, or mismatched provenance field. Independently
   hash every frozen input and compare it byte-for-byte with the identities
   embedded in the trace. Bind `r1d3-analysis.json` to the complete raw-trace
   hash and bind the gate report and aggregate closeout to both. A model byte
   length or a hash collected only after play is not sufficient.
3. **Make the prior-stage chain fail closed.** Replace literal prior-contract
   gates and top-level-boolean trust in R1-D1/D2/D3 with shared validation that
   checks the expected report schema/version, complete named-gate set, all gates
   true, expected contract/config hash, executable and source identity, frozen
   artifact existence/size/SHA-256, and the exact preceding-report hash. Reject
   extra or missing stage links and a report copied from another run.
4. **Make source identity rebuild-complete.** For qualification evidence, use a
   clean committed tree or a versioned inclusion/exclusion manifest that hashes
   tracked changes, relevant untracked files, generated sources, scripts,
   configs, Cargo lockfile, submodules, and external trainer revision/diff. Do
   not infer cleanliness from `git diff` alone. Record unrelated archive
   exclusions explicitly rather than silently ignoring all untracked files.
5. **Add adversarial regression tests.** At minimum, mutate each trace input
   hash, replace the NNUE with a same-size file, alter the contract/openings/glue
   after trace creation, remove or add a prior named gate, flip only top-level
   `passed`, substitute a prior report from another run, modify a relevant
   untracked script, and break each closeout link. Every case must make the
   appropriate gate fail.
6. **Regenerate rather than bless old evidence.** From the remediation commit,
   build one release executable and a fresh SIMD128 WASM package, rerun the
   dedicated-worker browser qualification to create a new producer-bound trace,
   then rerun `r0-gate`, `r1a-gate`, `r1b-gate`, `r1c-gate`, `r1d1-gate`,
   `r1d2-gate`, and `r1d3-gate`. Publish a new aggregate closeout manifest in
   which all reports use the same executable/source identity and every link is
   independently hash-verifiable.

Remediation passes only when all adversarial tests fail closed, all seven gates
exit zero on the new evidence, independent recomputation matches every embedded
identity and pair/timing statistic, the aggregate closeout records
`r2Authorized=true`, and a reviewer can reconstruct the evidence chain without
trusting a top-level boolean. Update this implementation record with exact
commands, artifact hashes, and outcomes, then stop.

The remediation cost ceiling is CPU plus the existing browser null/equivalence
qualification only. Do not generate labels or production data, rent a GPU, run
a model-strength match, modify feature geometry, or begin R2 in this assignment.

##### Provenance remediation and replacement R1 closeout (complete, 2026-09-02)

The remediation was implemented in commits `6ac48ee` and `b422aaf`; the latter
is the clean evidence-producing commit. No labels, production data, GPU work,
feature change, or model-strength game was run.

- Added `haitaka-r1-source-identity-v1` and a versioned exclusion policy. The
  recorded workspace is clean at commit
  `b422aaffeac9e5c2fe654d413e467cdf6de9d469`, tree
  `32782b58b6b53589ce78d30941ff51ec45d65683`, with no relevant untracked files.
  The three unrelated release archives are recorded individually by path,
  byte count, SHA-256, and reason. The external trainer is clean at commit
  `61666d9e3653e4df9881b14c23f8fdcc4bf7779b`; every `.idea/` exclusion is
  likewise recorded individually. `Cargo.lock`, submodule status, both Git
  diffs, generated WASM/glue, scripts, configs, policy, and producer executable
  are all identity-bound.
- The first remediated R1-D1 attempt failed closed because R1-B had created
  relevant untracked Python bytecode under `scripts/__pycache__/`. The bytecode
  was removed, all R1 Python oracle launches were changed to set
  `PYTHONDONTWRITEBYTECODE=1`, and all evidence was regenerated from the
  follow-up commit rather than reusing the earlier reports or trace.
- The browser harness now reads every frozen input once, hashes those exact
  buffers before Chrome starts, and creates envelope
  `2180ef79b89f6406b9f06727a3602dca22fdd8f2f3a07fd0fb6b8158d4a90868`.
  The Worker independently fetches and hashes every browser-consumed input,
  posts a single pre-play acknowledgement, and puts the envelope ID on every
  diagnostic and game search. The server rejects a missing, duplicate, late,
  or mismatched acknowledgement.
- Shared report validation now checks the exact schema/version, complete named
  gate set, every gate value, frozen contract/config identity, executable,
  source manifest, every recorded artifact, and exact prior-report links.
  R1-D3 additionally rejects duplicate JSON keys and unknown provenance fields.
  Raw trace, analysis, gate report, and aggregate closeout form an independently
  re-hashable chain.
- Six adversarial test functions cover mutations of all nine trace inputs,
  same-size NNUE replacement, post-trace contract/openings/glue mutation,
  missing/extra/false named gates, top-level-only pass flips, a report from
  another run, relevant untracked scripts, duplicate provenance keys, and every
  closeout link. All fail closed; the focused suites passed `4/4` and `2/2`.

The successful build and qualification commands were:

```text
cargo build --release -p haitaka_learn --features anhoku
target/release/haitaka_learn r1-source-identity --policy r0/anhoku-reboot/r1-source-identity-policy.json --external-trainer ../engine/variant-nnue-pytorch --output out/anhoku-reboot-r1d3/source-identity.json
RUSTFLAGS='-C target-feature=+simd128' wasm-pack build --target web --release --out-dir ../out/anhoku-reboot-r1d3/pkg haitaka_wasm --features anhoku
node scripts/r1d3-browser-harness.mjs --pkg-dir out/anhoku-reboot-r1d3/pkg --model out/anhoku-reboot-r1b/zero.nnue --contract r0/anhoku-reboot/r1d3-match-contract.json --openings r0/anhoku-reboot/r1d3-openings.tsv --worker scripts/r1d3-browser-worker.js --source-identity out/anhoku-reboot-r1d3/source-identity.json --release-executable target/release/haitaka_learn --output out/anhoku-reboot-r1d3/browser-trace.json --timeout-ms 300000
target/release/haitaka_learn r0-gate
target/release/haitaka_learn r1a-gate
target/release/haitaka_learn r1b-gate
target/release/haitaka_learn r1c-gate
target/release/haitaka_learn r1d1-gate
target/release/haitaka_learn r1d2-gate
target/release/haitaka_learn r1d3-gate
cargo test --workspace --features anhoku --no-fail-fast
cargo check --workspace --features anhoku
cargo fmt --all -- --check
git diff --check
```

All seven gates exited zero. The full workspace suite passed 428 tests with 11
ignored documentation tests and no failures. Independent read-only Node
recomputation re-hashed every producer input, every report artifact, and every
closeout link, and independently reproduced all pair bins and nearest-rank
timing statistics without consulting a top-level `passed` value.

Final identities:

| Artifact | SHA-256 |
| --- | --- |
| release `haitaka_learn` | `94dc040d460f16de6c2022a49f2a05f94f68f0def23e79f3efaedf358b673803` |
| source identity | `824b3255f1aa43c0ef5018ce742254b36e39db8dd2321c6999590bd78f3bbe9d` |
| WASM / glue | `35476bc49568874980f62c4746d1283fcba523ebf73ccfdb7aee4b2fabc991d2` / `e2001db5701d35c82a9e4b55a2a46771a8d72e0225d0e616cb948bbaec4e720c` |
| browser trace | `0917aac2d89852b935ec88785d181fea555537803bbf2ee734a185b31ac9fbe0` |
| R1-D3 analysis | `59d1e488a7e7fc5a01088ac0b9f9fa85f52c5bc902cacad5c26abda3d956ded0` |
| R1-D3 report | `55cfc4738c46d9aaf9887ad55e8302122dab0080630bb5bb528018fd3cf75f08` |
| aggregate closeout | `568e6b0ff7d5fe6f2e3048c83b662ab94cdbaacb32f4b93880ddc5a20e7738d0` |

The six report hashes are R1-A
`50368ee7f4664ea0adf968bcd9e261c7d9df24deffbad1cdfde15ee8ba751897`,
R1-B `74b028234b573213a39ee61ca9c222796bdd51d003058afb5893d4e773bfe97e`,
R1-C `72da8c4a9d356e54ab978baa76b220bcc2f9fc3267725d4b9eb39b17a18579f6`,
R1-D1 `90cb78d7a94b9e447f0ae6de4058ddc7558ef941f514333f1bc16b3c3e8c5490`,
R1-D2 `bba4f435387de2fdafccf96d6aea8e387cc45cdc5c84d2a47abd38ce0b3d3fbe`,
and R1-D3
`55cfc4738c46d9aaf9887ad55e8302122dab0080630bb5bb528018fd3cf75f08`.
Every report records the same executable and source identity.

All three lanes completed 16/16 games and 8/8 pairs with bins
`[0, 0, 8, 0, 0]`, zero missing moves, zero emergency fallbacks, and explicit
terminations. The A=A interval is `0 Elo [0, 0]`; the order-reversal difference
is `0 Elo`. Across 96 timed searches, p95/p99/maximum lateness was
`0.1000 / 0.4000 / 0.4000 ms`, mean engine elapsed-time difference was
`0.0104 ms`, and cold load was `583.2 ms`. The aggregate closeout records
`allReportsPassing=true`, `sourceIdentityExact=true`, and `r2Authorized=true`.
R2 is therefore ready and authorized as the next assignment; this closeout did
not begin R2.

#### R1 aggregate completion

R1 passes only after R1-A, R1-B, R1-C, R1-D1, R1-D2, and R1-D3 all have passing
machine-readable reports linked from one R1 closeout manifest. Rerun all six
gates from the clean strength-run source identity and publish their artifact
hashes. R2 remains unauthorized until that closeout explicitly records the
aggregate pass.

### R1 aggregate cost ceiling

Debug networks, at most 100,000 labels, and only harness-qualification null
games. No model-strength games.

### Failure route

Any exact-oracle failure blocks all later work. A synthetic target failure
routes to packing/features/sign/optimizer. A full-precision success followed by
serialized failure routes to quantization or serialization. Missing moves,
partial-value leakage, or node-accounting mismatches route to R1-D1. Repetition,
TT-history, qsearch-history, DFPN, or adjudication failures route to R1-D2.
Malformed pairs, deadline violations, unexplained A=A bias, or incomplete match
identity route to R1-D3. No historical match may be used as a substitute.

