# Anhoku v0.6 Phase 8D-A: searched-stochastic trajectory repair

Status: implementation complete; the checked-in short smoke is not a
production freeze. Phase 8D-B remains unauthorized until the full launch-gate
telemetry is run and reviewed.

## Implemented contract

`haitaka_learn` now provides:

- `audit-data`, with deterministic counts for distinct 72-byte records and
  distinct packed boards (`bin` bytes `0..64`), duplicate multiplicity, and
  conflicting `(score, ply, result)` targets;
- separate `generation-semantic-v1` and `schedule-cardinality-v1` identities;
  semantic changes cannot be hidden by a shard-cardinality extension, while
  complete non-overlapping ranges can be reused when only the schedule grows;
- `searched-stochastic-rollout-v1`, which scores a canonical bounded legal-move
  set, retains a score window, and samples with a named SplitMix64 stream keyed
  by dataset, pair index, and ply;
- `trajectory-audit`, which performs no label searches and reports hashes,
  packed-board uniqueness, tranche yield, selected-vs-best gaps, game
  lengths/outcomes, pair symmetry, and summed rollout CPU time;
- `calibrate-labels`, which regenerates one matched base/swapped pair per suite
  ID and labels the same candidate roots at 50k, 100k, and 200k combined nodes;
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
feature family, and deterministic sharding. Its initial rollout candidate is
`margin=80`, `temperature=40`; these values are candidates for the launch gate,
not a strength-selected result.

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

- trajectory audit: 312 board occurrences, 312 distinct packed boards, 1.0
  uniqueness, 13/13 exact transformed move-sequence pairs, and a 4.64 mean
  selected-vs-best score gap; it intentionally blocks the 30-board/game final
  tranche threshold because a 12-ply smoke can yield only 12 boards/game;
- label calibration: 128 games, 126 candidate roots, zero paired-root
  mismatches, exact accounting, zero incomplete/terminal/mate labels at all
  three budgets, and 50k selected for the smoke's depth-2 label contract.

These smoke results validate the implementation and symmetry contract only;
they do not freeze the production rollout values or label budget.
