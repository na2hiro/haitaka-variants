# Anhoku NNUE v0.6 Phase 4: Fixed-Node Label Budget

Phase 4 of the [handcrafted-strength plan](../plans/anhoku-nnue-handcrafted-strength-plan.md)
adds deterministic fixed-node teacher labels while preserving the existing
depth-only dataset path and the independent shallow rollout search.

## Search Contract

New datasets may select:

```toml
[data]
label_search_nodes = 50000
label_search_max_depth = 64
rollout_search_depth = 1
```

`label_search_nodes` is mutually exclusive with legacy `search_depth`, and a
node budget requires an explicit positive depth cap. The node-counting contract
is versioned as `alpha-beta-plus-qsearch-v1`:

- an alpha-beta node is counted on entry to the root or `negamax` search;
- a qsearch node is counted on entry to qsearch;
- terminal child positions evaluated inline retain the pre-existing search
  counter semantics and do not add a separate node;
- one shared counter covers every iterative-deepening attempt;
- an atomic admission check stops at the exact budget, so reported overshoot is
  zero even if the search is parallelized later.

The result is the move and score from the last fully completed depth. Reaching
the depth cap can finish below the node budget. If the budget is exhausted
before depth 1 completes, generation fails with an actionable error instead of
writing a partial label. Search itself is currently single-threaded; generation
jobs have separate workspaces and budgets.

Depth-only mode remains the default when none of the label-budget keys is
present, preserving the historical default depth of 2.

## Dataset Integration

Label and rollout searches use separate entry points. A sampled position uses
the configured depth or node label budget, while every self-play move under
`uniform-rollout-v1` continues to use only `rollout_search_depth`.

Shard and final manifests now identify:

- `label_search_budget`, `label_search_nodes`, and
  `label_search_max_depth`;
- `node_counting_version`;
- alpha-beta and qsearch counters for both label and rollout searches;
- combined label nodes and average nodes per label;
- label, rollout, and summed teacher-search elapsed seconds.

Resume and merge validate all four fixed-node identity fields. Missing new
fields remain accepted only for legacy depth-budget shards. The historical
`search_depth` field remains populated for depth datasets and uses `0` as a
non-ambiguous sentinel for node-budget datasets.

`generation_cpu_seconds` is the sum of elapsed teacher-search time across
worker jobs. It can exceed split wall time when jobs overlap; `elapsed_seconds`
continues to report split wall time.

## Compatibility And Determinism

A two-game depth-2 fixture was generated from both `strengthen-phase-3`
(`3841b5e`) and the Phase 4 implementation. The binary outputs were
byte-identical:

| Split | SHA-256 |
|---|---|
| train | `d29f00603e30e1937da0c0a3bbfadc837a3d10ed6f7498c7505b2401a6312bb4` |
| validation | `19c05a6afcb707fccb4c3d8580ecf62aebf2b32345da6ae28fb289021aeb4422` |

Repeated 5,000-node start-position searches returned the same move, score,
completed depth, alpha-beta count, qsearch count, and exact total. Dataset tests
also demonstrate exact 5,000-node aggregate labels, depth-1 rollout accounting,
identity-triggered regeneration, merge rejection, and legacy-manifest reuse.

## Release Smoke

The checked-in
`haitaka_learn.anhoku-v0.6-phase4.smoke.toml` run used 50,000 nodes per label,
a depth cap of 64, rollout depth 1, four generation jobs, and the Anhoku v1
opening suite. A first 5,000-node attempt correctly stopped when one suite
position could not complete depth 1; this is why the bounded smoke uses 50,000.
Production budgets should be calibrated from a representative pilot rather than
assuming 5,000 is sufficient for every tactical position.

| Split | Positions | Label nodes | Nodes/label | Label search seconds | Rollout search seconds | Split wall seconds |
|---|---:|---:|---:|---:|---:|---:|
| train | 96 | 4,800,000 | 50,000 | 26.848 | 1.399 | 16.927 |
| validation | 55 | 2,750,000 | 50,000 | 10.705 | 0.737 | 6.765 |

The train split reported 1,090,734 alpha-beta and 3,709,266 qsearch label
nodes. Validation reported 657,598 alpha-beta and 2,092,402 qsearch label
nodes. All 151 labels spent the exact configured budget; rollout counters were
recorded separately.

## Verification

Passed during implementation:

```text
cargo test -p haitaka_wasm --features anhoku
cargo test -p haitaka_learn --features anhoku
cargo check -p haitaka_wasm --features anhoku --target wasm32-unknown-unknown
RUSTFLAGS="-C target-feature=+simd128" \
  cargo check -p haitaka_wasm --features anhoku --target wasm32-unknown-unknown
cargo run --release -p haitaka_learn --features anhoku -- generate-data \
  --config haitaka_learn.anhoku-v0.6-phase4.smoke.toml --no-resume
cargo test --workspace --features anhoku
cargo fmt --all -- --check
git diff --check
```
