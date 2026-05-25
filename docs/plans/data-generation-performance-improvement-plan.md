# Data Generation Performance Improvement Plan

## Goal

Reduce NNUE data-generation wall-clock time substantially without degrading label quality more than necessary for the current Haitaka training pipeline.

Current problem:

- Depth-4 generation can take multiple days.
- The generator currently spends teacher search not only on sampled positions, but also on most non-opening self-play plies.
- The fixed-depth teacher search is a simple alpha-beta/negamax path without the usual engine-side acceleration features like a transposition table.

## Current Behavior

The current pipeline does the following:

- Parallelizes across shard workers and shard lanes.
- Generates self-play games.
- Samples positions from those games.
- Uses teacher search for two different purposes:
  - produce scalar labels (`best_score`)
  - choose the next self-play move after the random opening phase

This is the main structural issue. At higher depth, the pipeline is paying for expensive teacher search on many positions that are never sampled into the final dataset.

## Main Bottlenecks

### 1. Teacher search is used for rollout policy

After `opening_random_plies`, self-play no longer uses a cheap move policy. It uses full teacher search to pick each move. This means generation cost grows roughly with:

- number of games
- average plies after the random opening
- teacher search cost at the selected depth

not just with the number of stored training samples.

### 2. Every sampled position is searched from scratch

The current teacher path performs a fresh fixed-depth search for each queried board. There is no cross-position cache or transposition reuse across searches.

### 3. Search implementation is intentionally simple

The current fixed-depth search is workable but missing several standard engine optimizations:

- no transposition table
- limited move ordering
- board cloning on each child expansion
- fresh move-vector allocation and sorting at each node

### 4. Depth is a blunt cost multiplier

Moving from depth 3 to depth 4 can increase node count sharply. If the pipeline applies that deeper search to both rollout and labeling, the total wall-clock cost rises quickly.

## Prioritized Improvements

## Phase 0: Low-risk operational wins

These should be done first because they are cheap and immediately measurable.

### 0.1 Run generation in release mode

If any long generation jobs are still using plain `cargo run`, switch all docs and scripts to:

```bash
cargo run -p haitaka_learn --release --features anhoku -- generate-data --config ...
```

Expected impact:

- potentially very large if current runs are debug builds

Risk:

- none

### 0.2 Always use all local cores

Prefer `--jobs 0` unless the machine becomes memory- or thermally-limited.

Expected impact:

- high on multi-core machines

Risk:

- low

### 0.3 Normalize multi-machine shard generation

The codebase already supports shard lanes and merge. Make this the standard path for large runs.

Expected impact:

- near-linear wall-clock reduction with additional machines, within practical limits

Risk:

- low

## Phase 1: Fix the biggest structural waste

This is the highest-value engineering change.

### 1.1 Decouple rollout from labeling

Change generation so that self-play move selection does not require full teacher search on nearly every non-opening ply.

Recommended design:

- Use a cheap rollout policy during game generation.
- Only run the expensive teacher on positions that will actually be written to the dataset.

Candidate cheap rollout policies:

- random legal move
- shallow search such as depth 1 or 2
- NNUE teacher at lower depth than labeling depth
- handcrafted teacher at shallow depth

Best initial version:

- rollout with shallow search
- label sampled positions with deeper search

Example:

- rollout depth: 1 or 2
- label depth: 4

Expected impact:

- very high
- likely the single largest reduction in generation time

Tradeoff:

- self-play game quality becomes weaker
- but the labels on retained positions can remain strong

Assessment:

This is usually a good trade for a pipeline whose objective is supervised position labeling rather than strong online play.

### 1.2 Add a relabel-only mode

Introduce a two-pass workflow:

1. Generate games and sampled positions cheaply.
2. Relabel stored positions with the expensive teacher later.

Benefits:

- expensive labeling becomes embarrassingly parallel
- easier to split across machines
- easier to retry interrupted work
- easier to compare different label depths on the same position set

Expected impact:

- very high for operational flexibility

Risk:

- moderate implementation cost

## Phase 2: Reduce teacher calls

These changes improve throughput even before search-engine optimization.

### 2.1 Sample less frequently

Tune:

- `sample_every_ply`
- `max_positions_per_game`

Expected impact:

- directly reduces expensive label count

Tradeoff:

- fewer samples

Recommendation:

- benchmark dataset quality against position count rather than assuming denser is better

### 2.2 Increase `opening_random_plies`

If the current rollout remains teacher-driven after the opening, pushing more plies into the cheap/random phase reduces search calls.

Expected impact:

- moderate

Tradeoff:

- noisier self-play trajectories

### 2.3 Use mixed-depth labeling

Instead of using one expensive depth for every sample:

- label most positions at depth 2 or 3
- label a smaller, selected subset at depth 4

Selection strategies:

- positions with checks available
- tactical or high-volatility positions
- endgame positions
- a fixed random fraction

Expected impact:

- high

Tradeoff:

- non-uniform label strength

Assessment:

For current training scale, more labeled positions at moderate depth may outperform fewer labels at uniform depth 4.

## Phase 3: Improve search engine performance

Once structural waste is reduced, optimize the fixed-depth teacher itself.

### 3.1 Add a transposition table

The current fixed-depth search should gain the most from a TT.

Expected impact:

- high, especially at depth 4 and above

Why:

- self-play search repeatedly reaches transposed positions
- alpha-beta benefits heavily from memoized bounds and best moves

Recommended scope:

- store depth
- store score or bound type
- store best move for move ordering

### 3.2 Improve move ordering

Current move ordering is basic. Add:

- TT move ordering
- killer moves
- history heuristic
- better tactical prioritization

Expected impact:

- medium to high

Risk:

- low to moderate

### 3.3 Reduce board-copy overhead

The search currently clones the board for child exploration. Replace this with a make/unmake path if the engine architecture can support it cleanly.

Expected impact:

- medium

Risk:

- moderate to high because this touches correctness-sensitive move logic

### 3.4 Reduce per-node allocation

The search currently builds a fresh `Vec<Move>` and sorts it at each node.

Possible improvements:

- stack-based move buffers
- reusable scratch buffers
- partial ordering instead of full sort

Expected impact:

- medium

Risk:

- moderate

## Phase 4: Selective tactical acceleration

### 4.1 Use DFPN selectively, not universally

DFPN should not become the default teacher for all positions. It is specialized for forced-mate proof/disproof.

However, it may still help as a selective fast-path:

- root positions with checking moves
- near-terminal tactical positions
- relabel pass for mate-critical samples

Expected impact:

- low to medium overall
- high on the specific tactical subset

Assessment:

This is a targeted optimization, not the main solution to multi-day generation time.

## Recommended Execution Order

1. Confirm all large runs use `--release`, `--jobs 0`, and multi-machine shard splits where available.
2. Measure the current fraction of runtime spent on:
   - rollout teacher calls
   - sampled-position labeling
   - file IO and shard assembly
3. Implement rollout/label decoupling.
4. Add relabel-only mode.
5. Benchmark mixed-depth datasets against uniform-depth datasets.
6. Add a transposition table to the fixed-depth search.
7. Add stronger move ordering.
8. Evaluate whether selective DFPN improves the tactical subset enough to justify maintenance cost.

## Suggested Benchmark Matrix

Compare at least these configurations:

- Baseline: rollout depth 4, label depth 4
- Cheap rollout: rollout depth 1, label depth 4
- Mixed depth: rollout depth 1, label depth 3
- Hybrid dataset: 80% label depth 3, 20% label depth 4
- Search-optimized baseline after TT

Track:

- wall-clock time
- positions per second
- teacher searches per generated game
- average search states per label
- final dataset size
- training loss behavior
- downstream playing strength or validation metrics

## Concrete Deliverables

### Short term

- update runbooks to require `--release` for serious generation jobs
- add profiling counters for teacher-call counts and search states
- add a config split between rollout depth and label depth

### Medium term

- implement relabel-only pipeline stage
- implement transposition table in fixed-depth search
- add stronger move ordering

### Long term

- consider make/unmake search path
- consider selective DFPN tactical labeling
- consider dataset curriculum or mixed-depth labeling by position class

## Recommendation

The first engineering target should be:

- stop using the expensive teacher for both rollout and labeling

The second target should be:

- add a transposition table to the fixed-depth search

Those two changes are the most likely to convert a multi-day depth-4 run into something operationally manageable without distorting the ML pipeline too much.
