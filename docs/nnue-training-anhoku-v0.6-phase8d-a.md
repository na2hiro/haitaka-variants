# Anhoku v0.6 Phase 8D-A: searched-stochastic trajectory repair

Status: complete. The original Phase 8D-A calibration blocked as expected;
Phase 8D-A.2 then passed its v3 full-trajectory, sampled cross-host, and
adaptive-label calibration gates at commit `6da4ce7`. Phase 8D-B data
generation completed, but its initial unique-board readiness gate blocked.
Phase 8D-B.1 completed its schedule-only recovery and passed the final dataset
gate. Seed-80 training is the next action.

## Implemented contract

`haitaka_learn` now provides:

- `audit-data`, with deterministic counts for distinct 72-byte records and
  distinct packed boards (`bin` bytes `0..64`), duplicate multiplicity, and
  conflicting `(score, ply, result)` targets;
- separate `generation-semantic-v2` and `schedule-readiness-v1` identities;
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
- `root-position-adaptive-retry-v1`, which replaces rejected incomplete,
  accounting-invalid, terminal, missing-King, or mate roots at the next normal
  sampling ply without consuming the accepted quota;
- `calibrate-labels`, which symmetry-couples base/swapped retry attempts,
  targets one accepted root per game, tests 50k first, and escalates only for
  incomplete-search or accounting failures;
- packed-board minimum enforcement during full generation, merge, and training
  readiness. The old `minimum_train_positions` spelling remains readable as a
  deprecated packed-board floor.

`uniform-rollout-v1` remains parseable for historical manifests and configs,
but the Phase 8D commands reject it and the new Phase 8D config does not use it.
There is no arbitrary-opening or unsearched-uniform fallback.

## Reproduction

The completed v2 evidence remains reproducible with
`haitaka_learn.anhoku-v0.6-phase8d-a.toml`. The recovery config is
`haitaka_learn.anhoku-v0.6-phase8d-a2.toml`. It fixes the v3 suite, 52 train and
12 OOD-v2 IDs, zero random opening plies, C/16 bootstrap, the existing donor
feature family, deterministic sharding, and two pairs per opening for a
256-game audit. The first 128 games establish complete opening coverage; the
second cycle detects repeat-opening trajectory collapse. Its initial rollout
candidate is `margin=80`, `temperature=40`; these values are candidates for the
launch gate, not a strength-selected result.

Run the label-free gate first:

```bash
cargo run --release -p haitaka_learn --features anhoku -- trajectory-audit \
  --config haitaka_learn.anhoku-v0.6-phase8d-a2.toml \
  --jobs 0 \
  --output out/anhoku-v0.6-phase8d-a2/trajectory-audit-jobs0.json
```

Then, only after the rollout values are frozen from legality/symmetry/cost,
move-quality, and diversity telemetry:

```bash
cargo run --release -p haitaka_learn --features anhoku -- calibrate-labels \
  --config haitaka_learn.anhoku-v0.6-phase8d-a2.toml
```

The adaptive report requires 128 accepted roots, zero exhausted games or bad
stored labels, exact accounting and paired symmetry, and mean attempts per
accepted root <=1.25 overall and <=1.50 in either split. Side/outcome retry
bias remains telemetry, not a gate, because every rejected requested slot must
now be deterministically replaced.

## Phase 8D-A.1 legality recovery

The first production-shaped `jobs=1` audit stopped before report publication at
train game 62. The failure was not a color-swap parser defect: the source board
was already invalid at pair 31, ply 117. The preceding legal position and the
accepted move were:

```text
2sg1k3/4g2b1/l2Kp1s1+P/pP2PPpp1/2+r6/P6P1/+n3+bgP2/2+p5+l/3+s3+n1 b Prgs2n2l5p 129
5d5c
```

`5d5c` captured the White Pawn donor on 5c. That restored the White Gold on 5b
to native Gold movement and created an attack on the Black King on 6c, but the
not-in-check same-side-donor fast path had accepted the move without replaying
post-move king safety.

The recovery keeps the batched fast path. Only moves that capture an active
opposing donor are split out and replayed before `is_legal` or move generation
accepts them. The same correctness rule is shared by Anhoku, Annan, and
Antouzai; non-captures and captures of non-donors do not pay the replay cost.
The exact game-62 position is a core move-generation regression and a dynamic
SFEN color-swap round-trip regression. Color-swap errors now retain source and
transformed SFEN context.

The repaired game 62 completed all 180 plies with per-ply source-SFEN
validation enabled and wrote a diagnostic JSON. That one-game report is
expected to fail full coverage gates and is not launch evidence. Resume the
phase only after applying the adjacent-King validation repair below. Any run
from before both legality fixes is invalid and must not be compared or merged.

The next local `jobs=0` audit reached train game 102, pair 51, ply 114 before
the color-swap canonicalizer rejected the position after `4d4e`. This move
legally leaves the Kings adjacent: the opposing King receives Pawn movement
from its Anhoku donor and does not attack the moved King. Move generation used
the effective movement and accepted the move correctly, but the generic SFEN
validator still imposed standard Shogi's unconditional adjacent-King ban.

Influence-variant SFEN validation now relies on the existing variant-aware
effective-attack check instead of geometric King adjacency. Standard Shogi
retains the unconditional ban. Millisecond-scale regressions cover legality,
move generation, SFEN reparsing, and color-swap round-trip on the exact game-102
position; full trajectory replay is left to the production audit.

The full repaired local `jobs=0` audit completed at commit `7dac0ee`: all 256
games and 128 transformed pairs completed, 34,210 of 34,492 packed boards were
distinct (99.18%), and the final post-coverage tranche produced 111.53 new
boards/game. A 16-game `jobs=1` nubu sample contained eight exact transformed
pairs and matched the corresponding local trajectory hashes, source identity,
and policy. Both the production trajectory gate and sampled reproducibility
gate passed. The rollout parameters are fixed during label recovery.

## Production label-calibration result

The report at `out/anhoku-v0.6-phase8d-a/artifacts/label-calibration.json` has
matching commit, config, C/16 teacher, suite, and rollout provenance. It is safe
to use for the stop decision but does not select a production budget.

- 128 games produced 126 candidate roots. `anhoku-v2-048` produced none: both
  orientations ended after one ply, before the fixed ply-8 sampling origin.
- At 50k, 100k, and 200k, train and OOD-v2 each had zero incomplete labels,
  zero terminal labels, exact node accounting, and zero side rejection delta.
- Every budget reported six train and two OOD-v2 mate labels, with identical
  aggregate rejection counts. Train's outcome rejection-rate delta was 0.2222
  and OOD-v2's was 0.20, both above the frozen 0.05 gate.

This is candidate-eligibility failure, not evidence that 200k is too small.
Additional nodes did not change the aggregate rejection counts. No budget is
selected and the original calibration must not be overridden.

## Phase 8D-A.2 adaptive-label contract

Implementation and production-gate status: complete.
`anhoku-v3` preserves 63 positions and replaces `048` with generator pair
index 52, chosen before any label or strength inspection. The parser now also
rejects duplicate color-swap orbits. Policy version, attempt cap, rejection
counts by reason/side/outcome/opening/root ply, exhaustion, and attempts per
accepted position are recorded in semantic identity or manifests as
appropriate. Regression tests cover mate retry, exhaustion logic, transformed
root matching, and jobs/shard determinism.

1. Version the suite as `anhoku-v3`, replacing only the unusable train opening
   `anhoku-v2-048`. Choose its replacement without label, loss, or strength
   results; require legal unique base/swapped positions and eight matched
   frozen-rollout candidate plies from 8 through 22. Preserve all other
   positions and the 12-ID OOD-v2 boundary.
2. Add `root-position-adaptive-retry-v1`. Incomplete, terminal, missing-King,
   or mate candidates are rejected and counted, then the next normal sampling
   ply is tried without consuming the accepted quota. Calibration targets one
   accepted root per game and permits eight attempts. Retry parameters and
   counters are semantic identity and manifest fields.
3. Require base/swapped calibration roots to accept or retry together at the
   same transformed ply. Test mate retry, exhaustion, symmetry, and jobs/shard
   determinism.
4. Because the suite hash changes, rerun the full trajectory gate and a bounded
   cross-host sample containing the replacement opening plus OOD-v2 evidence.
5. Recalibrate at 50k first. Escalate to 100k and then 200k only for incomplete
   search or accounting failure, never to cure mate/terminal roots. A pass
   requires 128 accepted roots, every opening represented, zero inadmissible
   stored labels or exhausted games, exact accounting and symmetry, no missing
   requested slots by side/outcome, mean attempts/accept <=1.25 overall and
   <=1.50 per split. Raw retry bias remains reported telemetry.
6. If this contract fails at 200k or exceeds its retry-cost bounds, stop for a
   position-policy/opening-source review. Do not generate Phase 8D-B data.

## Phase 8D-A.2 production evidence

The local `jobs=0` full audit completed 256/256 games. It covered all 52 train
and 12 OOD IDs with two pairs each, matched all 128/128 transformed move
sequences, produced 34,732 distinct boards from 35,014 occurrences (99.19%),
and yielded 114.09 new boards/game in the final post-coverage tranche. The
replacement `anhoku-v3-048` produced complete matched trajectories in both
cycles, with game lengths 180 and 83 plies.

The bounded nubu `jobs=1` sample selected lanes 11--12 of 26: 24 games and 12
pairs covering `anhoku-v3-045` through `-052`, including replacement `-048`,
plus OOD IDs `-061` through `-064`. All 12 transformed pairs and every sampled
trajectory hash matched the local run exactly; source identity and rollout
policy also matched. Its own full-audit decision is intentionally false because
a 24-game sample cannot meet the 64-ID/two-cycle coverage gate; the dedicated
cross-host comparison passed.

Adaptive calibration then passed at the first budget, 50,000 nodes. All 64 IDs
provided eight matched candidate attempts. It accepted exactly 128 roots with
zero exhausted games, zero incomplete/terminal/accounting-invalid labels, zero
bad stored labels, and exact node accounting. Six train and two OOD mate
attempts were replaced at the next paired sampling ply. Attempts per accepted
root were 1.0577 train, 1.0833 OOD, and 1.0625 overall, below the respective
1.50/1.25 gates. Therefore 100k and 200k were not run and 50k is frozen for
Phase 8D-B.

Evidence:

- `out/anhoku-v0.6-phase8d-a2/trajectory-audit-jobs0-6da4ce7.json`
- `out/anhoku-v0.6-phase8d-a2/trajectory-audit-cross-host-comparison-6da4ce7.json`
- `out/anhoku-v0.6-phase8d-a2/artifacts/label-calibration.json`

## Phase 8D-B result and Phase 8D-B.1 handoff

The frozen next config is
`haitaka_learn.anhoku-v0.6-phase8d-b-root-262k.toml`. It retains the selected
50k budget, C/16, v3 suite, searched-stochastic policy, feature family,
sampling, LR/lambda, and seed 80. It requests 6,200 train games and 96 balanced
OOD games, permits 64 accepted roots and 72 attempts/game, and enforces at
least 262,144 distinct packed train boards. The label-free yield projection and
record ceilings are recorded inline in the config and in the main plan.

The initial distributed run covered all 6,200 train and 96 validation games at
the frozen identity. Strict merge produced 210,882 accepted train records and
209,282 distinct packed boards (99.24% unique), then correctly stopped below
the 262,144 minimum before publishing the final validation manifest. The
detailed audit is
`out/anhoku-v0.6-phase8d-b-root-262k/artifacts/phase8d-b-initial-train-audit.json`.

Phase 8D-B.1 extends only the schedule to 8,200 train games under
`haitaka_learn.anhoku-v0.6-phase8d-b1-root-262k-extension.toml`. Generate the
exact 2,000-game tail as lanes 31--40 of 41 while keeping engine revision
`5c23b02`:

```bash
cargo generate-data \
  haitaka_learn.anhoku-v0.6-phase8d-b1-root-262k-extension.toml \
  --jobs 0 --shard 32-41/41
```

For distributed generation, freeze one source/config/suite/teacher identity on
all machines and add `--shard-index <LANE> --shard-index-end <LANE>
--shard-count <COUNT>`. Never use `--ignore-identity-mismatch`; merge only
complete, contiguous, non-overlapping lanes. Before training, require the
packed-board floor, at least 95% uniqueness, exact label accounting, zero bad
stored labels, and review retry/exhaustion and balance telemetry.

The distributed extension completed at engine revision `5c23b02`. Strict merge
accepted all 820 train shards and 10 validation shards, publishing 279,627
train and 3,218 validation records. The final train audit found 276,949
distinct packed boards (99.04% unique), 14,805 above the 262,144 floor. It also
confirmed exact candidate accounting, zero stored mate/clamped labels, balanced
sides, and zero train/validation opening overlap. The implementation has no
separate `READY.json`; successful publication of both merged manifests plus
the passing audits is the readiness contract, and the trainer independently
rechecks the distinct-board floor. Final evidence:

- `out/anhoku-v0.6-phase8d-b-root-262k/artifacts/phase8d-b1-final-train-audit.json`
- `out/anhoku-v0.6-phase8d-b-root-262k/artifacts/phase8d-b1-final-validation-audit.json`

Run the authorized seed-80 training with `haitaka-variant-nnue-pytorch`
revision `61666d9e3653e4df9881b14c23f8fdcc4bf7779b`, matching Phase 8B.

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
