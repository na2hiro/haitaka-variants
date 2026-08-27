# Anhoku v0.6 Phase 8D-A: searched-stochastic trajectory repair

Status: implementation complete; the checked-in short smoke is not a
production freeze. Phase 8D-B remains unauthorized until the full launch-gate
telemetry is run and reviewed.

## Implemented contract

`haitaka_learn` now provides:

- `audit-data`, with deterministic counts for distinct 72-byte records and
  distinct packed boards (`bin` bytes `0..64`), duplicate multiplicity, and
  conflicting `(score, ply, result)` targets;
- separate `generation-semantic-v1` and `schedule-readiness-v1` identities;
  semantic changes cannot be hidden by a shard-cardinality extension, while
  complete non-overlapping ranges can be reused when only the schedule or
  packed-board readiness floor grows;
- `searched-stochastic-rollout-v1`, which scores a canonical bounded legal-move
  set that always contains the canonical root search's best move, retains a
  score window, and samples with a named SplitMix64 stream keyed by dataset,
  pair index, and ply; `splitmix64-v1` is the only accepted RNG version;
- `trajectory-audit`, which performs no label searches and reports hashes,
  two complete cycles across all 52 train and 12 OOD openings, packed-board
  uniqueness, post-initial-coverage tranche yield, legal/scored/truncated
  candidate counts, selected-vs-best gaps, game lengths/outcomes, pair
  symmetry, and summed rollout CPU time;
- `calibrate-labels`, which regenerates one matched base/swapped pair per suite
  ID, requires every ID to produce a non-empty matched root set, and labels the
  same candidate roots at 50k, 100k, and 200k combined nodes;
- packed-board minimum enforcement during full generation, merge, and training
  readiness. The old `minimum_train_positions` spelling remains readable as a
  deprecated packed-board floor.

`uniform-rollout-v1` remains parseable for historical manifests and configs,
but the Phase 8D commands reject it and the new Phase 8D config does not use it.
There is no arbitrary-opening or unsearched-uniform fallback.

## Reproduction

The production-shaped pilot config is
`haitaka_learn.anhoku-v0.6-phase8d-a.toml`. It fixes the v2 suite, 52 train and
12 OOD-v2 IDs, zero random opening plies, C/16 bootstrap, the existing donor
feature family, deterministic sharding, and two pairs per opening for a
256-game audit. The first 128 games establish complete opening coverage; the
second cycle detects repeat-opening trajectory collapse. Its initial rollout
candidate is `margin=80`, `temperature=40`; these values are candidates for the
launch gate, not a strength-selected result.

Run the label-free gate first:

```bash
cargo run --release -p haitaka_learn --features anhoku -- trajectory-audit \
  --config haitaka_learn.anhoku-v0.6-phase8d-a.toml
```

Then, only after the rollout values are frozen from legality/symmetry/cost,
move-quality, and diversity telemetry:

```bash
cargo run --release -p haitaka_learn --features anhoku -- calibrate-labels \
  --config haitaka_learn.anhoku-v0.6-phase8d-a.toml
```

The calibration report selects the smallest budget only when both train and
OOD-v2 satisfy the 1% incomplete-label, exact-accounting, zero-terminal/mate,
and side/outcome rejection-rate delta <= 0.05 gates. If none passes, its
decision is `blocked` and requires an explicitly written adaptive-retry
contract.

## Smoke evidence

The bounded smoke config uses 26 games, 12 plies, and one candidate root per
game so the implementation can be checked quickly. Its reports are written
under `out/anhoku-v0.6-phase8d-a-smoke/` when the commands above are adapted to
that config.

- trajectory audit, remeasured after root-best candidate inclusion: 312 board
  occurrences, 312 distinct packed boards, 1.0 uniqueness, 13/13 exact
  transformed move-sequence pairs, 8,246 legal moves, 2,466 scored candidates,
  5,780 truncated candidates, a 5.51 mean selected-vs-best score gap, and 0.729
  summed rollout CPU seconds in the remeasurement run;
- the previous label-calibration smoke predates root-best candidate inclusion
  and is no longer evidence. Its 126-root result would also be blocked because
  two opening IDs had no matched root.

The short trajectory smoke omits 51 train IDs, gives every visited ID only one
pair, and has no tranche after the initial 128-game coverage boundary. It is
therefore intentionally blocked by both the two-cycle coverage and repeat-yield
gates. These smoke results do not freeze the production rollout values or label
budget.
