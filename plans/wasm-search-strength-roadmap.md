## WASM Search Strength Roadmap

### Summary

`haitaka_wasm` already has legal move generation, alpha-beta search, root DFPN
short-circuiting, incremental NNUE evaluation, a Phase 1 transposition table,
and a Phase 2 staged move picker.
Its search is still much simpler than modern Shogi engines: fixed-depth negamax,
no quiescence search, and no selective pruning beyond normal alpha-beta cutoffs.

The highest-return path is to keep improving the alpha-beta core before
attempting a large architectural rewrite. Phase 1 added the transposition table;
Phase 2 added stronger ordering infrastructure, but the first measured build is
weaker at short movetime and needs diagnosis before adding qsearch or selective
search. Defer
make/unmake until the easier search wins have been measured, because board
mutation rollback is correctness-sensitive across all variants.

### Current Baseline

- `search_best_move` and `negamax` clone the board for each child and recurse
  with alpha-beta bounds.
- `MovePicker` still allocates a fresh `Vec<Move>` per node, but it now consumes
  generated moves by staged selection instead of sorting the whole list.
- Move ordering currently prioritizes TT/hash moves, winning/equal tactical
  moves, killer moves, history-ranked quiet moves, and losing tactical moves.
- Depth-zero nodes call `evaluate_or_mate` immediately, so volatile capture,
  promotion, and check positions can be evaluated before tactics settle.
- Iterative deepening now reuses one transposition table across completed depth
  iterations.
- `Board::hash()` already exposes an incremental Zobrist key, so a
  transposition table does not require new board hashing infrastructure.
- NNUE evaluation already supports incremental position state. Preserve this path
  while changing search.

### Phase 1: Transposition Table - Done

Phase 1 has been implemented in `haitaka_wasm`.

Implemented:

- Added a compact rshogi/YaneuraOu-style clustered TT in
  `haitaka_wasm/src/tt.rs`: 32-byte aligned clusters, 3 compact 10-byte entries,
  16-bit key fragments, packed move storage, depth, generation, bound, score,
  and eval fields.
- Added replacement policy with generation aging, depth preference, collision
  reporting, and `hashfull` telemetry.
- Added mate-score conversion with ply adjustment before TT store/load.
- Added legal hash-move ordering. Packed moves are never trusted directly; they
  are unpacked and only used if present in the generated legal move list.
- Added sufficient-depth TT cutoffs for exact/lower/upper bounds.
- Added TT telemetry: `tt_probes`, `tt_hits`, `tt_cutoffs`, `tt_stores`,
  `tt_collisions`, and `tt_hashfull`.
- Reused one TT across iterative-deepening iterations and one session-local TT
  across `UsiSession` searches.
- Added WASM APIs `set_hash_size_mb(size_mb)` and `clear_hash()`.
- Added USI `option name Hash type spin default 16 min 1 max 1024` and
  `setoption name Hash value N`.
- Kept Phase 1 single-threaded; no atomics or lock-free TT races were added.

Verification completed:

- `cargo test -p haitaka_wasm` passed: 57 tests.
- TT packing tests cover representative board moves, drops for hand pieces, and
  invalid packed moves.
- TT entry behavior tests cover exact-bound overwrite, same-key empty-move
  preservation, different-key collision reporting, and generation aging.
- Search tests cover TT stats exposure, tiny 1 MB hash search, and iterative
  deepening TT reuse.
- Existing handcrafted and NNUE legal-best-move tests still pass.
- Movetime self-play was used for Elo, not fixed-depth self-play.

Measured strength and speed against clean `main`:

| Build | Movetime self-play | Score | Approx Elo | 95% CI |
|---|---:|---:|---:|---:|
| standard | 118-79-3 | 59.75% | +68.6 | +20.5 .. +119.5 |
| `--features annan` | 101-97-2 | 51.0% | +6.9 | -41.4 .. +55.6 |
| `--features nekoneko` | 85-86-29 | 49.75% | -1.7 | -50.2 .. +46.7 |

Fixed-depth `play` runs were used only for NPS and tree-size diagnostics because
external USI self-play currently reports `totalNodes=0` for child engines.

| Build | Depth | Nodes diff | NPS diff | Time diff |
|---|---:|---:|---:|---:|
| standard | 5 | -30.6% | -2.9% | -28.5% |
| `--features annan` | 5 | -19.5% | +1.4% | -20.5% |
| `--features nekoneko` | 5 | -27.1% | +8.0% | -32.5% |

Phase 1 verification gaps to keep as future work:

- Add an explicit TT-disabled runtime/config path so fixed-depth equality tests
  can compare TT-enabled and TT-disabled searches inside the same binary.
- Add larger self-play runs, ideally 1,000+ games per ruleset and at more than
  one time control, because Annan and NekoNeko 200-game confidence intervals
  crossed zero.
- Teach external USI self-play to parse child-engine `info` nodes/nps so
  movetime Elo reports can include nodes, NPS, and TT telemetry.
- Add a reusable benchmark command or script that records the fixed-depth NPS
  and movetime Elo artifacts outside `/tmp`.
- Add a tactical fixture suite with exact expected scores or moves where TT
  enabled/disabled equality can be tested safely.

### Phase 2: Move Picker And Ordering - Implemented, Needs Follow-Up

Phase 2 has been implemented in `haitaka_wasm`, but the first measured version
is not a strength improvement at short movetime.

Implemented:

- Added `haitaka_wasm/src/movepick.rs` with a staged move picker.
- Replaced full per-node move sorting with staged selection:
  1. TT/hash move.
  2. Winning captures and promotions.
  3. Equal captures and promotions.
  4. Killer moves.
  5. Quiet moves ranked by history heuristic.
  6. Losing captures.
- Added two killer slots per ply and a side-aware history table.
- Added distinct history keys for drops and board moves:
  `(side, dropped piece, to square)` for drops and
  `(side, from square, to square, promotion)` for board moves.
- Added ordering telemetry:
  `beta_cutoffs`, `first_move_cutoffs`, `hash_move_tries`,
  `hash_move_cutoffs`, `killer_move_tries`, `killer_move_cutoffs`,
  `history_move_tries`, and `history_move_cutoffs`.
- Exposed ordering telemetry through native summaries, WASM getters, and
  iterative-search JS iteration objects.
- Kept Phase 2 exact: no qsearch, LMR, futility pruning, null-move pruning, PVS,
  or broad checking-move generation was added.

Verification completed:

- `cargo check -p haitaka_wasm` passed.
- `cargo test -p haitaka_wasm` passed: 61 tests.
- `cargo test -p haitaka_wasm --features annan` passed: 58 tests.
- `cargo test -p haitaka_wasm --features nekoneko` passed: 57 tests.
- `cargo bench -p haitaka_wasm --bench nnue -- --noplot` completed.

Measured strength and speed against the previous Phase 1 commit
`91e5120` (`Document wasm TT verification`). Phase 2 was A; Phase 1 was B.
All Elo numbers are movetime self-play, not fixed-depth self-play. Settings:
`--movetime-ms 20`, `--opening-random-plies 4`, 4 workers. Standard and Annan
used 200 games with seed 1. NekoNeko 200-game attempts with seeds 1 and 2 both
aborted because an external child engine returned an illegal move, so the row
below is an 80-game seed-2 sample and is not directly comparable.

| Build | Games | Result A-B-D | Score | Approx Elo | 95% CI |
|---|---:|---:|---:|---:|---:|
| standard | 200 | 34-166-0 | 17.0% | -275.5 | -349.5 .. -217.8 |
| `--features annan` | 200 | 24-176-0 | 12.0% | -346.1 | -436.5 .. -281.6 |
| `--features nekoneko` | 80 | 49-24-7 | 65.625% | +112.3 | +36.4 .. +200.6 |

Fixed-depth `play` runs were used only for NPS and tree-size diagnostics because
external USI self-play currently reports `totalNodes=0` for child engines.

| Build | Depth | Nodes diff | NPS diff | Time diff |
|---|---:|---:|---:|---:|
| standard | 5 | -81.8% | -61.0% | -53.5% |
| `--features annan` | 5 | -43.8% | -43.6% | -0.3% |
| `--features nekoneko` | 5 | +38.1% | +2.6% | +34.5% |

Artifacts from local measurement were written under
`/tmp/haitaka-phase2-results/`.

Phase 2 follow-up before continuing to qsearch/selective search:

- Diagnose why standard and Annan lose heavily despite searching far fewer nodes.
  The top suspicion is that the new ordering changes which shallow or
  collision-prone TT entries dominate short-movetime search.
- Add a TT-disabled or TT-verification-strong comparison mode so move-ordering
  changes can be tested for fixed-depth score equality without TT collision
  effects.
- Investigate the NekoNeko external self-play illegal moves seen during 200-game
  runs: seed 1 aborted on `P*1b`, seed 2 aborted on `3b2c`.
- Reconsider tactical scoring before tuning: the current cheap
  capture/promotion gain is not SEE and may be too disruptive for Shogi and
  variant positions.

### Not Done Yet: Phase 3 - Quiescence Search

Replace direct depth-zero evaluation with a capped tactical search.

Initial scope:

- Use static evaluation as stand-pat when not in check.
- If in check, search all legal evasions and disallow stand-pat.
- Search captures and promotions.
- Consider checking moves only with a small qsearch depth/check budget.
- Order qsearch moves with captures/promotions first.
- Add simple delta pruning only after correctness tests exist.

Expected impact: high for tactical stability. This should reduce horizon-effect
blunders where the engine stops immediately after an unstable capture or threat.

Risk: moderate. Unbounded qsearch can explode in Shogi because drops and checks
create many forcing continuations. Add node and ply caps from the first version.

### Not Done Yet: Phase 4 - Selective Search

Add these only after TT, move picker, and qsearch are stable:

- Principal variation search.
- Aspiration windows around the previous iteration's root score.
- Late move reductions for quiet, non-checking, non-tactical late moves.
- Null-move pruning with conservative restrictions.
- Futility pruning near leaves.
- Razoring or reverse futility after qsearch is reliable.
- Check extensions or singular extensions only after strong TT instrumentation.

Null-move pruning needs extra caution in Shogi and variants. Drops, zugzwang-like
positions, and variant movement rules can make null-move assumptions less safe.
Gate it behind depth, material, in-check, previous-null, and variant-specific
conditions, and verify by self-play and tactical suites.

Expected impact: high after the prerequisites, but easy to make weaker if added
without measurement.

Risk: moderate to high. Every pruning rule should be added behind focused tests
and SPRT-style self-play comparisons.

### Not Done Yet: Phase 5 - Time Management And USI Surface

Strength in real play depends on time allocation, not only fixed-depth search.

Already covered by Phase 1:

- Add `setoption name Hash value N`.

Remaining additions:

- Parse and honor `btime`, `wtime`, `byoyomi`, `binc`, and `winc`.
- Add `stop` support for browser worker and native USI flows.
- Later: `Threads`, `MultiPV`, and ponder.

This overlaps with `plans/wasm-usi-future-work.md`; keep protocol features tied
to actual search/runtime support.

### Not Done Yet: Phase 6 - NNUE And Data Quality

The NNUE runtime already follows the right broad design: sparse features,
incremental update, integer inference, and variant-specific feature hashes.
Search improvements will make generated labels and self-play stronger, but NNUE
quality still needs its own loop.

Recommended work:

- Measure handcrafted vs NNUE strength at equal movetime, not only equal depth.
- Generate quieter training positions by using qsearch leaf positions or
  filtering positions with unresolved tactics.
- Keep variant-specific datasets separate unless a transfer experiment proves
  mixed data helps.
- Add tactical and mate-heavy validation sets so better average eval does not
  hide worse forcing-line play.
- Track NPS, completed depth, node count, win rate, and confidence intervals for
  each model/search change.

### Measurement Plan

Each phase should include:

- Perft and move legality regressions.
- Fixed-position search snapshots for tactical and quiet positions.
- NPS and node-count benchmarks at depths 3, 4, and 5.
- Self-play at fixed movetime, not only fixed depth.
- A/B comparison against the previous engine with identical NNUE model and time
  controls.

Useful counters:

- nodes, qnodes, leaves.
- beta cutoffs and first-move cutoffs.
- TT hit/store/cutoff/collision counts.
- hash-move tried and hash-move cutoff counts.
- killer/history cutoff counts.
- qsearch max ply and qsearch timeout/cap hits.

### Suggested Implementation Order

1. Done: add TT data structures and telemetry.
2. Done: enable TT hash-move ordering.
3. Done: enable TT bound cutoffs.
4. Done: add killer and history heuristics.
5. Done: replace full move sorting with a staged move picker.
6. Not done: add qsearch for captures/promotions and check evasions.
7. Not done: add PVS and aspiration windows.
8. Not done: add conservative LMR.
9. Not done: evaluate null-move pruning and futility pruning separately.
10. Not done: revisit make/unmake if clone overhead remains a top profiler item.

### References

- Transposition tables:
  https://www.chessprogramming.org/Transposition_Table
- Move ordering:
  https://www.chessprogramming.org/Move_Ordering
- Quiescence search:
  https://www.chessprogramming.org/Quiescence_Search
- Late move reductions:
  https://www.chessprogramming.org/Late_Move_Reductions
- Stockfish search implementation:
  https://github.com/official-stockfish/Stockfish/blob/master/src/search.cpp
- Stockfish move picker:
  https://github.com/official-stockfish/Stockfish/blob/master/src/movepick.cpp
- YaneuraOu search implementation:
  https://github.com/yaneurao/YaneuraOu/blob/master/source/engine/yaneuraou-engine/yaneuraou-search.cpp
- NNUE overview from Stockfish nnue-pytorch:
  https://github.com/official-stockfish/nnue-pytorch/blob/master/docs/nnue.md
- Proper NNUE dataset study:
  https://arxiv.org/abs/2412.17948
