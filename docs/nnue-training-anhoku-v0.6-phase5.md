# Anhoku NNUE v0.6 Phase 5: Qsearch-PV Leaf Entries

Phase 5 of the [handcrafted-strength plan](../plans/anhoku-nnue-handcrafted-strength-plan.md)
adds an opt-in dataset position policy that stores the final traced search leaf
rather than the sampled root. The trainer record remains the existing 72-byte
format.

## Configuration And Record Semantics

Enable leaf records with:

```toml
[data]
position_policy = "qsearch-pv-leaf"
```

The default, `root-position`, is the legacy behavior. The leaf contract is
versioned as `qsearch-pv-v1`.

For a leaf record:

- the packed position is the final position on the selected alpha-beta PV after
  qsearch processes captures, promotions, required evasions, and the configured
  quiet-check allowance;
- the score is the static teacher evaluation of that leaf from its side to
  move, not the backed-up root score;
- the game result is oriented to the leaf side to move;
- the 72-byte `game_ply` field remains the sampling root ply;
- root-ply and leaf-distance ranges and the mean leaf distance are recorded in
  manifest and audit metadata.

The search APIs used by USI, WASM, and ordinary callers are unchanged. Native
learner-only entry points opt into a principal-leaf collector. The collector
tracks the chosen child alongside score updates and caches traces for
transposition-table cutoffs. Fixed-node traced and untraced searches retain the
same move, score, completed depth, and alpha-beta/qsearch counters.

## Filtering And Identity

Candidate leaves are excluded when:

- the traced leaf is terminal or lacks either king; or
- the backed-up search score is mate-saturated (`abs(score) >= 29000`).

Terminal and mate-score rejections have separate counters. A mate result may
legitimately have no ordinary static leaf; it is still rejected and counted
rather than causing generation to fall back to the root position.

Shard and final manifests identify `position_policy` and
`training_trace_version`. Resume and merge reject either mismatch. Missing
fields are accepted only as legacy `root-position` identity.

Additional metadata includes:

- candidate and stored position counts;
- terminal and mate-score rejection counts;
- root-ply minimum and maximum;
- leaf-distance minimum, maximum, and mean.

The deterministic audit report carries the same trace identity and counters.

## Correctness And Compatibility

Search fixtures cover:

- a tactical rook capture reaching a quiet leaf;
- a silver promotion retained in the leaf board;
- an in-check root reaching a legal quiet evasion leaf;
- repeated trace equality and static-evaluation equality at the leaf;
- traced versus untraced fixed-node search parity.

Dataset fixtures cover leaf-side result orientation when root and leaf sides
differ, byte-identical record packing, terminal/mate rejection counts,
root/leaf resume invalidation, trace-version merge rejection, and legacy shard
reuse.

A two-game depth-2 root-position fixture was generated from both the Phase 4
commit `68cd61f` and the Phase 5 implementation. Outputs were byte-identical:

| Split | SHA-256 |
|---|---|
| train | `d29f00603e30e1937da0c0a3bbfadc837a3d10ed6f7498c7505b2401a6312bb4` |
| validation | `19c05a6afcb707fccb4c3d8580ecf62aebf2b32345da6ae28fb289021aeb4422` |

## Release Smoke

The checked-in `haitaka_learn.anhoku-v0.6-phase5.smoke.toml` uses the same
50,000-node label budget, depth cap 64, rollout depth 1, suite, seeds, and four
jobs as the Phase 4 smoke. This isolates the position-policy change.

| Split | Candidates | Stored | Terminal rejected | Mate rejected | Leaf distance | Label nodes |
|---|---:|---:|---:|---:|---:|---:|
| train | 96 | 96 | 0 | 0 | 2–10, mean 4.531 | 4,800,000 |
| validation | 55 | 42 | 9 | 4 | 1–11, mean 5.000 | 2,750,000 |

Every candidate search consumed exactly 50,000 combined alpha-beta/qsearch
nodes. Train root plies ranged from 8–55; validation root plies ranged from
8–32. The audit reports reproduced all manifest trace fields and counters.

## Verification

Passed during implementation:

```text
cargo test -p haitaka_wasm --features anhoku
cargo test -p haitaka_learn --features anhoku
cargo check -p haitaka_wasm --features anhoku --target wasm32-unknown-unknown
RUSTFLAGS="-C target-feature=+simd128" \
  cargo check -p haitaka_wasm --features anhoku --target wasm32-unknown-unknown
cargo run --release -p haitaka_learn --features anhoku -- generate-data \
  --config haitaka_learn.anhoku-v0.6-phase5.smoke.toml --no-resume
cargo test --workspace --features anhoku
cargo fmt --all -- --check
git diff --check
```
