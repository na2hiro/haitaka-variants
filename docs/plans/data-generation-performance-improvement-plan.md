# Data Generation Performance Improvement Plan

Status: historical plan, superseded on 2026-08-17 by
[Anhoku NNUE Handcrafted-Strength Execution Plan](../../plans/anhoku-nnue-handcrafted-strength-plan.md).

## Completion Update (2026-08-17)

The main structural performance change proposed here is complete:

- `data.rollout_search_depth` now keeps non-labeling self-play search cheap;
- `data.search_depth` independently controls sampled-position labels;
- both budgets are recorded in manifests and checked by resume/merge;
- release/all-core generation, search telemetry, TT, move ordering, and qsearch
  support have also moved beyond the baseline described below.

The remaining text is retained as historical rationale. In particular, the
“current behavior,” bottleneck list, execution order, and recommendation below
must not be treated as the current repository state. Increasing uniformly
random opening plies is no longer recommended for production data; the active
plan instead removes opening-phase samples and introduces stronger opening
sources, grouped validation, deterministic shuffling, fixed-node labels, and
qsearch-leaf examples.

## Historical Goal

Reduce NNUE data-generation wall-clock time substantially without degrading
label quality more than necessary for the pipeline at the time.

Original problem:

- Depth-4 generation could take multiple days.
- The generator spent label-depth teacher search on sampled positions and most
  non-opening self-play plies.
- The fixed-depth teacher path lacked engine-side acceleration such as a
  transposition table.

## Original Behavior

The pipeline originally did the following:

- Parallelizes across shard workers and shard lanes.
- Generates self-play games.
- Samples positions from those games.
- Uses teacher search for two different purposes:
  - produce scalar labels (`best_score`)
  - choose the next self-play move after the random opening phase

This was the main structural issue: the pipeline paid for label-depth search on
many positions that were never stored.

## Main Bottlenecks

### 1. Teacher Search Was Used For The Rollout Policy

After `opening_random_plies`, self-play used the full label-depth teacher to
pick each move. Generation cost therefore grew with:

- number of games
- average plies after the random opening
- teacher search cost at the selected depth

not just with the number of stored training samples.

### 2. Every sampled position is searched from scratch

The teacher path performed a fresh fixed-depth search for each queried board,
without cross-position cache or transposition reuse across searches.

### 3. Search implementation is intentionally simple

The fixed-depth search at the time lacked several standard optimizations:

- no transposition table
- limited move ordering
- board cloning on each child expansion
- fresh move-vector allocation and sorting at each node

### 4. Depth is a blunt cost multiplier

Moving from depth 3 to depth 4 can increase node count sharply. If the pipeline applies that deeper search to both rollout and labeling, the total wall-clock cost rises quickly.

## Prioritized Improvements

## Phase 0: Low-risk operational wins

These should be done first because they are cheap and immediately measurable.

### [done] 0.1 Run generation in release mode

If any long generation jobs are still using plain `cargo run`, switch all docs and scripts to the
xtask wrapper, which reads the config ruleset, adds the matching Cargo feature, and always uses a
release build:

```bash
cargo generate haitaka_learn.anhoku-v0.5.1.toml
```

Expected impact:

- potentially very large if current runs are debug builds

Risk:

- none

### [done] 0.2 Always use all local cores

Use the default all-core worker setting unless the machine becomes memory- or thermally-limited.

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

### [done] 1.1 Decouple rollout from labeling

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

### [superseded] 2.2 Increase `opening_random_plies`

This was a historical throughput workaround. Do not use it for new production
datasets: uniformly random opening moves and samples from that phase are now
considered a data-quality risk. Follow the active strength plan's opening and
sampling policy instead.

Historical rationale: while rollout and labeling shared one expensive budget,
pushing more plies into the random phase reduced teacher calls.

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

The hypothesis was that more labels at moderate depth might outperform fewer
uniform depth-4 labels.

## Phase 3: Improve Search Engine Performance (Historical Baseline)

Once structural waste is reduced, optimize the fixed-depth teacher itself.

### 3.1 Add a transposition table

The fixed-depth search was expected to gain most from a TT.

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

The original move ordering was basic. Proposed additions were:

- TT move ordering
- killer moves
- history heuristic
- better tactical prioritization

Expected impact:

- medium to high

Risk:

- low to moderate

### 3.3 Reduce board-copy overhead

The search cloned the board for child exploration. The proposal was to consider
make/unmake if the architecture could support it safely.

Expected impact:

- medium

Risk:

- moderate to high because this touches correctness-sensitive move logic

### 3.4 Reduce per-node allocation

The search built a fresh `Vec<Move>` and sorted it at each node.

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

## Historical Recommended Execution Order (Superseded)

1. Confirm all large runs use `--release`, the default all-core worker setting, and multi-machine shard splits where available. Phase 0.1 and 0.2 are complete; 0.3 remains open.
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

- update runbooks to require `--release` for serious generation jobs [done]
- add profiling counters for teacher-call counts and search states
- add a config split between rollout depth and label depth [done]

### Medium term

- implement relabel-only pipeline stage
- implement transposition table in fixed-depth search
- add stronger move ordering

### Long term

- consider make/unmake search path
- consider selective DFPN tactical labeling
- consider dataset curriculum or mixed-depth labeling by position class

## Historical Recommendation (Completed/Superseded)

The first engineering target should be:

- stop using the expensive teacher for both rollout and labeling

The second target should be:

- add a transposition table to the fixed-depth search

Those two changes are the most likely to convert a multi-day depth-4 run into something operationally manageable without distorting the ML pipeline too much.

Rollout/label decoupling and TT support are now implemented. Use the active
Anhoku strength plan for the current implementation order.
