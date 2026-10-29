# Anhoku v0.6 Phase 1: Dataset Contract And Baseline Audit

Phase 1 of the [handcrafted-strength plan](../plans/anhoku-nnue-handcrafted-strength-plan.md)
adds deterministic dataset auditing and makes sampling and teacher-move semantics part
of dataset identity. It deliberately does not add the Phase 2 opening suite.

## Preserved v0.5.1 Audit

The ignored machine-readable report is stored at
`out/anhoku-v0.5.1/datasets/train.audit.json`. It was produced from the preserved
training binary and manifest with `haitaka_learn.anhoku-v0.5.1.toml` supplying fields
that predate the expanded manifest.

- bytes: 94,837,176 (`1,317,183` records of 72 bytes)
- SHA-256: `80a497077c07fc1651fcf543a1ca6002112db9249d79499cabee7eacbc8d674d`
- ply parity: 1,317,183 even, 0 odd
- side to move: 1,317,183 Black, 0 White
- outcomes relative to side to move: 1,155,997 wins (87.76%), 110,562 losses
  (8.39%), and 50,624 draws (3.84%)
- teacher moves: 0 nonzero, 1,317,183 zero
- samples before `opening_random_plies = 16`: 199,800
- score range: -29,998 to 29,999; mean 2,775.3163; absolute mean 3,531.3279
- mate-like scores (`abs(score) >= 29000`): 105,734; clamped i16 scores: 0

This reproduces the baseline bias described by the execution plan and additionally
quantifies the opening contamination.

## v0.6 Contract

The new [v0.6 config](../haitaka_learn.anhoku-v0.6.toml) uses
`sampling_policy = "per-game-random-v1"`. Each game derives a phase in
`0..sample_every_ply` from its existing deterministic game seed, without consuming
the opening-move RNG stream. Sampling starts no earlier than
`max(sample_start_ply, opening_random_plies)`.

Shard and final manifests record `sampling_phase`, `sample_after_opening`, and
`teacher_move_encoding`. Strict resume and merge reject a mismatched contract. The
existing `--ignore-identity-mismatch` option is the explicit compatibility override.

The 72-byte ABI is unchanged. Its 16-bit move slot remains zero and is declared
`teacher_move_encoding = "unavailable"`. Config validation rejects teacher-move
consumers, and training refuses legacy/unspecified dataset manifests so zero cannot be
silently treated as a real teacher move.
