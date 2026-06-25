## WASM Search Strength Roadmap

### Summary

`haitaka_wasm` already has legal move generation, alpha-beta search, root DFPN
short-circuiting, incremental NNUE evaluation, a Phase 1 transposition table,
and a Phase 2 staged move picker.
Its search is still much simpler than modern Shogi engines: fixed-depth negamax,
no quiescence search, and no selective pruning beyond normal alpha-beta cutoffs.

The highest-return path is to keep improving the alpha-beta core before
attempting a large architectural rewrite. Phase 1 added the transposition table;
Phase 2 added stronger ordering infrastructure and later fixed two review-found
move-picker issues, but the measured build is still weaker than Phase 1 at short
movetime. The remaining regression is dominated by horizon/depth-parity behavior
and should be addressed with qsearch before selective pruning. Defer make/unmake
until the easier search wins have been measured, because board mutation rollback
is correctness-sensitive across all variants.

### Current Baseline

- `search_best_move` and `negamax` clone the board for each child and recurse
  with alpha-beta bounds.
- `MovePicker` still allocates per node, but it now partitions legal moves into
  staged buckets and sorts each bucket once instead of sorting the whole legal
  move list or rescanning all moves for every pick.
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
- `cargo test -p haitaka_wasm` passed: 63 tests.
- `cargo test -p haitaka_wasm --features annan` passed: 60 tests.
- `cargo test -p haitaka_wasm --features nekoneko` passed: 58 tests.
- `cargo bench -p haitaka_wasm --bench nnue -- --noplot` completed.

Measured strength and speed against the previous Phase 1 commit
`91e5120` (`Document wasm TT verification`). Phase 2 was A; Phase 1 was B.
All Elo numbers are movetime self-play, not fixed-depth self-play. Settings:
`--movetime-ms 20`, `--opening-random-plies 4`, seed `1`, 4 workers. Current
working tree after iterative `SearchOrdering` reuse was A; `91e5120` was B.

| Build | Self-play result | Score | Approx Elo | Depth-5 nodes | Depth-5 NPS |
|---|---:|---:|---:|---:|---:|
| standard | 52-148-0 / 200 | 26.0% | -181.7 | -81.7% | +24.5% |
| `--features annan` | 41-159-0 / 200 | 20.5% | -235.4 | -43.8% | +38.4% |
| `--features nekoneko` | 4-0-0 / 4 partial | 100.0% | +798.3 | +38.1% | +8.6% |

The NekoNeko self-play row is partial: the unpatched `91e5120` external engine
aborted on game 6, after 4 completed games, by rejecting a legal NekoNeko SFEN with
`failed to parse SFEN: The board representation is invalid`. That is the
triple-check validation bug fixed in Phase 2, so a full 200-game NekoNeko
comparison against the exact pre-PR revision is not valid without modifying the
baseline.

Fixed-depth `play` runs were used only for NPS and tree-size diagnostics because
external USI self-play currently reports `totalNodes=0` for child engines.

Artifacts from local measurement were written under
`/tmp/haitaka-pr21-ordering/`.

Phase 2 diagnosis:

- Done: standard and Annan lose heavily at `--movetime-ms 20` mostly
  because Phase 2 changes the completed-depth frontier, not because fixed-depth
  scores diverge. On the first 12 standard self-play openings, current and
  previous engines returned identical moves and scores at fixed depths 4 and 5.
  Under `go movetime 20`, however, 10 of the first 20 sampled standard openings
  diverged. Most divergences collapsed at `go movetime 50`.
- Representative standard opening:
  `lnsgk1snl/1r4gb1/pp1pppppp/2p6/9/9/PPPPPPPPP/1B2GK1R1/LNSG2SNL b - 5`.
  Fixed-depth search oscillates by parity: depth 4 chooses `7g7f` with score
  `-14`, depth 5 chooses `9g9f` with score `200`, depth 6 returns to `7g7f`
  with score `-14`, and depth 7 returns to `9g9f` with score `198`. Phase 1
  depth 5 takes about `41.6 ms` from a fresh search, while Phase 2 depth 5 takes
  about `10.6 ms`, so the 20 ms USI search commonly returns the odd-depth
  horizon move in Phase 2 while Phase 1 returns the even-depth move.
- Representative Annan opening:
  `ln1gkg1nl/1rs3sb1/p1ppppp1p/1p5p1/9/1P5PR/P1PPPPP1P/1B2K4/LNSG1GSNL b - 5`.
  Depth 4 chooses `1f1e` with score `-650`; depth 5 chooses `9i9h` with score
  `444`; depth 6 returns to `1f1e` with score `-244`. Phase 1 depth 5 takes
  about `21.0 ms`, while Phase 2 depth 5 takes about `16.4 ms`, enough to flip
  many 20 ms searches from the even-depth result to the odd-depth result.
- Hash size sensitivity did not support the original collision hypothesis. With
  standard openings at 20 ms, the Phase 2 vs Phase 1 move differences persisted
  at 16 MB and 128 MB hash sizes. Tiny 1 MB hash made both engines noisier, but
  did not explain the main regression.
- The staged picker also resets killer/history ordering for each iterative
  deepening iteration, because `SearchOrdering` is created inside each fixed
  depth search. TT state carries across iterations, but killer/history learning
  does not, so the current Phase 2 strength impact is mostly the changed
  tactical/quiet ordering and the resulting depth-parity shift.
- Done: preserved and reused `SearchOrdering` across iterative-deepening
  iterations. Fixed-depth searches still get a fresh ordering table, but
  iterative search now keeps killer/history state across completed depths just
  like it already keeps TT state.
- Done: fixed promotion-only tactical scoring. Non-capturing promotions now use
  only the promotion delta as tactical gain, so major-piece promotions are not
  incorrectly classified as losing tactical moves.
- Done: fixed the review-found picker overhead issue. `MovePicker` now
  partitions legal moves into hash, tactical, killer, history, and losing
  tactical buckets during construction, sorts each bucket once with the existing
  deterministic tie-breaker, and consumes by index instead of rescanning the
  full move vector for every pick.
- Practical conclusion: do not tune Phase 2 by 20 ms Elo alone until qsearch is
  added or the time-control/depth-parity behavior is stabilized. The lower node
  count is real, but without qsearch it can expose worse odd-depth horizon moves.

Phase 2 follow-up before continuing beyond qsearch/selective search:

- Add qsearch next, before judging the staged picker by short-movetime Elo. The
  current evidence points to odd/even horizon instability as the main cause of
  the standard and Annan regression.
- Done: added a fixed-depth equivalence harness over representative openings.
  It compares the staged-picker/TT search score against a test-only reference
  alpha-beta search that does not use `MovePicker` or TT. The harness checks
  depths 4 and 5 for the standard/Annan-style builds so future ordering changes
  must preserve exact alpha-beta scores.
- Done: made completed-depth iterative tests deterministic under Annan debug CI
  by disabling DFPN for tests that specifically exercise iterative alpha-beta
  depth completion and by using `timeout_ms = 0` for no deadline.
- Keep a TT-disabled or TT-verification-strong comparison mode as a diagnostic
  tool, but deprioritize TT collision as the primary explanation for this
  regression because 16 MB and 128 MB hash runs showed the same 20 ms move
  divergences.
- Done: investigated the NekoNeko external self-play illegal moves seen during
  200-game runs. The root cause was not the move picker. External self-play sent
  `position sfen ...` to a child engine after a legal NekoNeko triple-check
  position, but `Board::from_sfen` rejected the SFEN because
  `checkers_and_pins_are_valid` still enforced fewer than three checkers for the
  Neko family. The child then searched its previous board and returned a move
  that was illegal for the driver's board. The validator now exempts the Neko
  family from the `< 3` checker assertion, and the external self-play client
  now fails immediately if a child reports an invalid `position` or `go` command.
- Reconsider tactical scoring after qsearch is in place. The current cheap
  capture/promotion gain is not SEE, but tuning it before qsearch risks fitting
  around the observed depth-parity artifact.

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
