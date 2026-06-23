## WASM Search Strength Roadmap

### Summary

`haitaka_wasm` already has legal move generation, alpha-beta search, root DFPN
short-circuiting, incremental NNUE evaluation, and a Phase 1 transposition table.
Its search is still much simpler than modern Shogi engines: fixed-depth negamax,
basic move ordering, no quiescence search, and no selective pruning beyond normal
alpha-beta cutoffs.

The highest-return path is to keep improving the alpha-beta core before
attempting a large architectural rewrite. Phase 1 added the transposition table;
next add stronger ordering, then quiescence and selective search. Defer
make/unmake until the easier search wins have been measured, because board
mutation rollback is correctness-sensitive across all variants.

### Current Baseline

- `search_best_move` and `negamax` clone the board for each child and recurse
  with alpha-beta bounds.
- `legal_moves` allocates a fresh `Vec<Move>` per node and sorts the whole list.
- Move ordering currently prioritizes capture value, promotion, non-drop moves,
  and deterministic tie-breakers.
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

### Not Done Yet: Phase 2 - Move Picker And Ordering

Replace full per-node sorting with a staged move picker.

Recommended order:

1. PV move or hash move from the previous iteration / TT.
2. Winning captures and promotions.
3. Equal captures and promotions.
4. Killer moves, with mate killers if useful.
5. Quiet moves ranked by history heuristic.
6. Losing captures.

Shogi-specific additions:

- Treat checking moves as tactically important, but avoid generating all checks
  at every quiet node until benchmarks show it pays off.
- Drops need their own history key, likely `(side, dropped piece, to square)`.
- Board moves can use `(side, from square, to square, promotion flag)` or a
  compact move encoding if one exists.
- Captures should eventually move from victim-only ordering to MVV-LVA or SEE
  when attacker identity is cheap enough.

Expected impact: medium to high. This also makes later LMR and futility pruning
safer because "late move" will mean something.

Risk: low to moderate. The main risk is destabilizing deterministic tests if
tie-breaks change; update tests to assert legality or clear tactical outcomes
instead of incidental best moves where appropriate.

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
4. Not done: add killer and history heuristics.
5. Not done: replace full move sorting with a staged move picker.
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
