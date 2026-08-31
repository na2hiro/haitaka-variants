# Anhoku NNUE v0.6 Phase 8R result

Status: Phase 8R-A implementation, Phase 8R-B calibration, and the 2,048-game
decision match are complete. The result is evaluation-quality-limited; Phase
8P is skipped for the current 262k model and the next branch is the Phase 8C
launch gate.

## Frozen implementation identity

The source-file hashes identify the dirty-worktree implementation that added
the node protocol; the base commit identifies the parent revision used by the
completed run.

| Item | Identity |
| --- | --- |
| Branch | `strengthen` |
| Base source commit | `719c3dd236952d918937e6c0365256efae31f735` |
| `haitaka_cli/src/main.rs` SHA-256 | `423d4fcbe5dd6be13a158e6c1c901d8a44b8d473c628d8cb8b74e1558131ae04` |
| `haitaka_wasm/src/lib.rs` SHA-256 | `6fe963f9528a6f96f8edfdcbf2bd71641cc329334aa395501305a8f2323eae65` |
| Node counting version | `alpha-beta-plus-qsearch-v1` |
| Node-search depth cap | 64 |
| Phase 8B root model | `out/anhoku-v0.6-phase8b-root-262k/artifacts/haitaka-anhoku-v0.6-phase8b-root-262k.nnue` |
| Root model SHA-256 | `12865f59f28f6e26feffcfae2e76c576f8eb31891148a8a9c167b8b50aac972c` |
| Decision executable SHA-256 | `1275f92d1d83ab2cb4219afeb2bd1db326fe99cae9952745a589a7af30542542` |
| Anhoku opening source SHA-256 | `bc576bbe57c05b8b2112b416c1907845d38d5e087e8e3b71b44c19c4e1593307` |

The opening-suite hash is recorded for source provenance. The 8R-B match
itself uses the Anhoku start SFEN plus deterministic random opening plies, not
the TSV opening suite.

Rebuild the release binary after any source change and record its hash in the
result report:

```bash
cargo build -p haitaka_cli --release --features anhoku
sha256sum target/release/haitaka_cli
sha256sum out/anhoku-v0.6-phase8b-root-262k/artifacts/haitaka-anhoku-v0.6-phase8b-root-262k.nnue
```

## Protocol contract

Both sides use `--nodes-per-move N`. The controller creates a fresh budget for
each move, and the budget counts alpha-beta and qsearch together. A completed
iterative-deepening result is retained if the next iteration is interrupted.
If depth 1 cannot complete, the engine records a fallback and plays the
deterministic lexicographically first legal move.

The machine-readable report must preserve, at per-side, per-game, and
aggregate levels:

- requested and consumed combined nodes;
- alpha-beta nodes and qnodes;
- completed depth, incomplete iterations, cap hits, and fallbacks;
- elapsed time, aggregate NPS, aggregate QNPS, and warnings.

For every node-budget record, verify:

```
consumedBudgetNodes == alphaBetaNodes + qnodes
consumedBudgetNodes <= requestedBudgetNodes
requestedBudgetNodes == nodesPerMove * searchedMoves
```

The self-play report command identity contains both `nodesPerMove` and
`nodeCountingVersion`. Resume/merge rejects a changed budget or counting
version. The current report schema versions are report v3 and game-record v2.

## Calibration

Use only the Phase 8B root export against handcrafted. Run each candidate in a
fresh report directory with the same fixed opening set:

- 32 color-swapped pairs, 64 games;
- Anhoku start SFEN:
  `lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1`;
- four deterministic random opening plies;
- calibration seed `8207`;
- one worker and a 200-ply cap;
- candidate budgets: 20,000, 50,000, and 100,000 nodes per move.

Example candidate invocation (repeat with each budget and report directory):

```bash
cargo run -p haitaka_cli --release --features anhoku -- self-play \
  --games 64 \
  --threads 1 \
  --a-eval nnue \
  --nnue out/anhoku-v0.6-phase8b-root-262k/artifacts/haitaka-anhoku-v0.6-phase8b-root-262k.nnue \
  --nodes-per-move 20000 \
  --sfen "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1" \
  --opening-random-plies 4 \
  --seed 8207 \
  --max-plies 200 \
  --report-dir out/anhoku-v0.6-phase8r/calibration-20k
```

Select the smallest candidate using telemetry only. Do not use wins, scores,
Elo, or other game outcomes. For each candidate, audit every JSONL record and
both `aBreakdown` and `bBreakdown`:

1. no protocol failure and every recorded move is legal;
2. exact combined-node accounting holds;
3. each evaluator's fallback count is at most 0.1% of its searched moves
   (the predeclared 99.9% depth-1 completion gate);
4. the requested-node totals are integral multiples of the candidate budget;
5. cap hits and incomplete iterations are reported consistently, with no
   protocol or accounting exception.

Record the three telemetry rows and the frozen budget in the eventual Phase
8R result section. If no candidate passes, stop and create a new reviewed
assignment; do not silently change the counting contract or budget list.

### Calibration result (telemetry only)

The three calibration jobs were run on 2026-08-25 with the same binary and
root model, one worker per job, seed `8207`, and 64 games per candidate. No
scores or game outcomes were used for this budget decision.

| Budget | Searched moves | NNUE depth-1 completion | Handcrafted depth-1 completion | Fallbacks | Exact accounting | Protocol failures | Aggregate elapsed |
| ---: | ---: | ---: | ---: | ---: | --- | ---: | ---: |
| 20,000 | 3,466 | 99.017% | 99.597% | 24 | pass | 0 | 320.0 s |
| 50,000 | 3,771 | 99.894% | 99.788% | 6 | pass | 0 | 807.9 s |
| 100,000 | 3,708 | 99.946% | 100.000% | 1 | pass | 0 | 1,656.5 s |

The 20,000 and 50,000 candidates fail the 99.9% gate. The 100,000 candidate
is the smallest passing budget and is therefore frozen for the decision match.
For every candidate, requested nodes equaled consumed nodes and consumed nodes
equaled alpha-beta nodes plus qnodes in every audited game and side breakdown.
The calibration reports are under
`out/anhoku-v0.6-phase8r/calibration-{20k,50k,100k}/`.

## Decision match

After freezing the smallest passing calibration budget, the calibration
openings were discarded and a fresh predeclared opening stream was run:

- exactly 2,048 games, forming 1,024 color-swapped pairs;
- Anhoku start SFEN and four random opening plies;
- decision seed `8208`;
- depth cap 64 supplied by node mode;
- 200-ply cap;
- no concurrent generation, training, or unrelated CPU match load;
- root NNUE versus handcrafted only.

The frozen budget is `100000` nodes per move:

```bash
cargo run -p haitaka_cli --release --features anhoku -- self-play \
  --games 2048 \
  --threads 20 \
  --a-eval nnue \
  --nnue out/anhoku-v0.6-phase8b-root-262k/artifacts/haitaka-anhoku-v0.6-phase8b-root-262k.nnue \
  --nodes-per-move 100000 \
  --sfen "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1" \
  --opening-random-plies 4 \
  --seed 8208 \
  --max-plies 200 \
  --report-dir out/anhoku-v0.6-phase8r/decision
```

The match was run without inspecting outcome aggregates during generation. The
final report, complete game JSONL, command identity, executable hash, model
hash, pentanomial bins, telemetry, and warnings are preserved under
`out/anhoku-v0.6-phase8r/decision/`.

### Decision result (2026-08-25)

The match completed with exit code 0. The report contains 2,048 unique game
records and 1,024 complete color-swapped pairs. All `failureState` values and
warnings are empty; every record's move count matches its ply count. Exact
combined-node accounting and both side breakdowns pass for every game.

| Metric | Result |
| --- | ---: |
| NNUE A wins / handcrafted B wins / draws | 904 / 1,120 / 24 |
| NNUE score rate | 44.7266% |
| Paired Elo (A - B) | -36.78 |
| Paired 95% CI | [-51.30, -22.26] Elo |
| Pair count | 1,024 |
| Pentanomial bins (report order) | [289, 13, 529, 11, 182] |
| Average game length | 55.85 plies |
| Requested / consumed combined nodes | 11,437,400,000 / 11,409,617,792 |
| Alpha-beta nodes / qnodes | 2,881,991,152 / 8,527,626,640 |
| Fallbacks (NNUE / handcrafted) | 182 / 160 |
| Protocol failures / warnings | 0 / 0 |
| Aggregate search elapsed | 191,465.2 s |
| Aggregate NPS / QNPS | 15,052 / 44,539 |

Two 200-ply draws consumed less than their nominal requested budget; this is
within the recorded `consumed <= requested` contract and produced no failure
or accounting mismatch. The observed wall time was approximately 2 h 41 min
on this 12-logical-CPU machine with `--threads 20`.

### Fallback sensitivity audit

The decision distribution produced 342 fallbacks in 114,374 searched moves
(`0.299%`). This exceeds the calibration gate of at most 0.1%, even though the
100k calibration sample passed. The fallbacks affected 42 games in 41 opening
pairs; 278 of the 342 were concentrated in two 200-ply draws. This is a
distribution-shift caveat in the decision protocol and must not be hidden by
the zero-warning report.

As a post-hoc sensitivity analysis, removing every pair containing any
fallback leaves 983 complete pairs. Their pentanomial bins are
`[279, 10, 507, 10, 177]`, NNUE score rate is 44.8118%, and paired Elo is
`-36.18` with paired 95% CI `[-51.05, -21.31]`. This analysis does not replace
the predeclared 1,024-pair result, but its upper bound is also below zero, so
fallback contamination does not explain the classification.

### Interpretation and next branch

The paired upper 95% bound is below zero, so the result is
**evaluation-quality-limited** under the plan's classification rule. Equal-node
NNUE does not beat handcrafted; this diagnostic cannot promote the model.
Phase 8P is therefore skipped for this 262k root model and Phase 8C is next.

This does not show that runtime is solved. In the completed 100 ms Phase 8B
match the same root model was `-115.73 Elo` behind, while NNUE main-search NPS
was 17,272 against handcrafted's 34,875. In this equal-node run NNUE also used
more aggregate search time than handcrafted. Results from different protocols
cannot be subtracted as an exact runtime Elo penalty, but together they show a
proved evaluation-quality deficit plus a material secondary runtime deficit.

Phase 8C must first test whether root-only scaling to 1M closes the quality
gap. After selecting the reproducible 1M winner without handcrafted outcomes,
run both the planned 100 ms handcrafted diagnostic and one fresh fixed
100k-combined-node diagnostic. If the latter establishes `-10 Elo`
non-inferiority while the former still loses, Phase 8P becomes the next phase.
If equal-node quality is still significantly below handcrafted, continue only
along a data-policy or data-scale branch supported by the Phase 8C scale test.
Phase 8C generation and training have not been launched.

## Implementation verification

The focused implementation tests were rerun after result review:

- `cargo test -p haitaka_cli --features anhoku`: 51 unit and 5 integration
  tests passed;
- `cargo test -p haitaka_wasm --features anhoku`: 81 tests passed;
- no test failed.

The completed result was classified using the plan's predeclared boundaries:

- runtime-dominant if the NNUE lower 95% bound is above 0 Elo;
- evaluation-quality-limited if the NNUE upper 95% bound is below 0 Elo;
- mixed/inconclusive if the interval crosses zero.

No model is promoted from the equal-node diagnostic.
