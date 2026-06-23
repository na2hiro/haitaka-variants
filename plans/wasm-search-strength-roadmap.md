## WASM Search Strength Roadmap

### Summary

`haitaka_wasm` already has legal move generation, alpha-beta search, root DFPN
short-circuiting, and incremental NNUE evaluation. Its search is still much
simpler than modern Shogi engines: fixed-depth negamax, no transposition table,
basic move ordering, no quiescence search, and no selective pruning beyond normal
alpha-beta cutoffs.

The highest-return path is to improve the alpha-beta core before attempting a
large architectural rewrite. Add a transposition table and stronger ordering
first, then quiescence and selective search. Defer make/unmake until the easier
search wins have been measured, because board mutation rollback is
correctness-sensitive across all variants.

### Current Baseline

- `search_best_move` and `negamax` clone the board for each child and recurse
  with alpha-beta bounds.
- `legal_moves` allocates a fresh `Vec<Move>` per node and sorts the whole list.
- Move ordering currently prioritizes capture value, promotion, non-drop moves,
  and deterministic tie-breakers.
- Depth-zero nodes call `evaluate_or_mate` immediately, so volatile capture,
  promotion, and check positions can be evaluated before tactics settle.
- Iterative deepening reruns each depth without sharing search results between
  iterations.
- `Board::hash()` already exposes an incremental Zobrist key, so a
  transposition table does not require new board hashing infrastructure.
- NNUE evaluation already supports incremental position state. Preserve this path
  while changing search.

### Phase 1: Transposition Table

Add a bounded search-local or session-local transposition table.

Recommended entry fields:

- 64-bit position key or a verification fragment plus table index.
- remaining search depth.
- score.
- bound type: exact, lower, or upper.
- best move for hash-move ordering.
- generation or age for replacement.

Use the table in two ways:

- Cut off when an entry has sufficient depth and a compatible bound.
- Try the stored best move before generating or sorting the rest of the moves.

Start with a simple power-of-two table and depth-preferred replacement. Add
`tt_hits`, `tt_cutoffs`, `tt_stores`, and `tt_collisions` counters to the native
summary before tuning replacement policy.

Expected impact: high, especially depth 4 and above, and especially during
iterative deepening.

Risk: moderate. Mate-distance scores need conversion to and from table storage
using current ply so shorter mates remain preferred.

### Phase 2: Move Picker And Ordering

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

### Phase 3: Quiescence Search

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

### Phase 4: Selective Search

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

### Phase 5: Time Management And USI Surface

Strength in real play depends on time allocation, not only fixed-depth search.

Recommended additions:

- Parse and honor `btime`, `wtime`, `byoyomi`, `binc`, and `winc`.
- Add `stop` support for browser worker and native USI flows.
- Add `setoption name Hash value N`.
- Later: `Threads`, `MultiPV`, and ponder.

This overlaps with `plans/wasm-usi-future-work.md`; keep protocol features tied
to actual search/runtime support.

### Phase 6: NNUE And Data Quality

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

1. Add TT data structures and telemetry without enabling cutoffs.
2. Enable TT hash-move ordering.
3. Enable TT bound cutoffs.
4. Add killer and history heuristics.
5. Replace full move sorting with a staged move picker.
6. Add qsearch for captures/promotions and check evasions.
7. Add PVS and aspiration windows.
8. Add conservative LMR.
9. Evaluate null-move pruning and futility pruning separately.
10. Revisit make/unmake if clone overhead remains a top profiler item.

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
