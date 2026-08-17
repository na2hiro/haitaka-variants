# Anhoku v0.6 Phase 2: Versioned Opening Suite

Phase 2 of the [handcrafted-strength plan](../plans/anhoku-nnue-handcrafted-strength-plan.md)
replaces production uniform-random openings with a fixed, auditable suite. It does not
add searched-stochastic openings, shuffle records, or start training.

## Suite v1

The checked-in source is
[`haitaka_learn/openings/anhoku-v1.tsv`](../haitaka_learn/openings/anhoku-v1.tsv).
It contains 12 stable IDs and has SHA-256
`7150a2a5871c4d302b63ab99ea31abe086471fa38a213dc184f42ce5d05721a7`.

Candidates were generated deterministically at 16 legal Anhoku plies with seed
`20260817`. Positions containing an early capture in hand or an early promotion were
removed during review. This is intentionally a small first suite: its purpose is to
make opening distribution reproducible and inspectable before the Phase 7 learning
curve, not to approximate searched MultiPV play.

`validate-openings` checks every base and transformed position with the active Anhoku
move generator. It requires both kings, at least one legal move, unique IDs, unique
canonical positions, and a reversible color transformation.

## Pair And Identity Contract

For games `2n` and `2n+1`, suite selection uses the same deterministic pair seed and
opening ID. Game `2n` uses the base SFEN. Game `2n+1` uses
`anhoku-rotate180-color-swap-v1`, which rotates the board, exchanges piece and hand
colors, and changes side to move.

Shard and final manifests record:

- `opening_policy`
- `opening_suite_id`
- `opening_suite_sha256`
- `opening_transformation`
- the selected `opening_ids`
- per-game pair index, opening ID, color role, and selected SFEN

Strict resume and merge reject any opening-identity mismatch. The existing
`--ignore-identity-mismatch` flag remains the only explicit compatibility override.
The production [Anhoku v0.6 config](../haitaka_learn.anhoku-v0.6.toml) selects the
suite and sets `opening_random_plies = 0`; `uniform-random` remains available only as
an explicitly named compatibility/smoke-test policy.

## Generation Smoke Result

The checked-in [`haitaka_learn.anhoku-v0.6.smoke.toml`](../haitaka_learn.anhoku-v0.6.smoke.toml)
keeps the production suite, seed, feature family, label depth 3, rollout depth 1, and
sampling contracts while limiting the run to four train and four validation games at
60 plies.

Two clean runs produced byte-identical datasets:

- train: 79 positions, 5,688 bytes, SHA-256
  `48479910b6c954102aa277704bf89ad02cb50f7d1d8f098fec70d88871667914`
- validation: 62 positions, SHA-256
  `5296ec5301d0c28d68a8fea664599ae5b63f1929890ab15a87fde0b386859eba`

The train audit contained both ply parities (55 even and 24 odd), zero samples before
the configured opening end, zero nonzero teacher moves, and the expected suite SHA and
transformation identity. All four game records formed two adjacent base/swapped pairs
with matching opening IDs. The ignored machine-readable artifacts are under
`out/anhoku-v0.6-smoke/datasets/`.
