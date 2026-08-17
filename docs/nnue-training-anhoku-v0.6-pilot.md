# Anhoku NNUE v0.6 Local Data Pilot

- Date: 2026-08-17
- Host: Intel Core i7-8700, 6 cores / 12 threads, 31 GiB RAM
- Config: `haitaka_learn.anhoku-v0.6-pilot.toml`
- Purpose: validate the Phase 3 production data path and estimate 1M generation time

## Generation

The pilot used the production search and record contracts: depth-3 labels, depth-1
rollout, 180-ply limit, 64 positions per game, the Anhoku v1 suite, grouped splitting,
and deterministic bounded-memory shuffling. It generated 200 train and 20 validation
games with 12 configured jobs.

Measured `/usr/bin/time` result:

```text
wall_seconds=442.44
user_seconds=2927.74
system_seconds=1.24
max_rss_kib=216376
```

Train was sufficiently sharded to use the host in parallel:

- 200 games and 3,965 positions in 277.57 seconds;
- 43.2 games/minute;
- 14.28 positions/second;
- 19.83 positions/game.

Validation produced 495 positions from 20 games. Its 164.85-second split time is not
a production-throughput measurement: only two 10-game shards existed, so at most two
of the 12 workers were active.

Using the train throughput, 1M positions would take about 19.4 wall-clock hours on this
host. Scaling by games, the checked-in 50,000 + 5,000 game production config would take
about 21.2 hours and produce approximately 1.1M positions. A practical local planning
range is 16-27 hours because game length, opening mix, sustained clock rate, and host
load vary. The run consumed about 0.81 CPU-hours; linear scaling suggests roughly 200
CPU-hours for 55,000 games. Vast.ai time must be re-benchmarked on the rented CPU;
handcrafted label generation does not benefit directly from the training GPU.

## Artifacts

- train: 3,965 records, 285,480 bytes, SHA-256
  `9233b10541f26b63a1ad43bc6444aee09a9df67c72bf53fa6ac2df4eefebeb3d`
- validation: 495 records, 35,640 bytes, SHA-256
  `6fb51d25f51202793d62fbd86b87af74798ea934a3c38e2bd2406d50507dd79d`
- ignored output: `out/anhoku-v0.6-pilot/datasets/`

Both audits report zero pre-opening samples, unique split-qualified game IDs, and zero
train/validation opening-group overlap. Train used all ten assigned opening IDs and
validation used both assigned IDs.

## Dataset Gate

The Phase 7 gate does not pass:

| Check | Train result | Status |
| --- | ---: | --- |
| side-to-move share | 45.62% black / 54.38% white | pass |
| decisive win share | 84.67% | **fail** (>60%) |
| decisive loss share | 15.33% | pass |
| samples before opening | 0 | pass |
| opening-group overlap | 0 | pass |

Validation is more extreme: all 495 records have a relative win result and neither a
loss nor draw. This pilot is suitable for an end-to-end loader/trainer smoke test, but
not for measuring NNUE strength. Do not spend on the 1M generation or three-seed
training until the outcome imbalance is diagnosed and a representative pilot passes
the gate.

## Diagnosed Cause

The imbalance is present at game granularity, not just record weighting: 176 of the 200
train games stored only winner-relative samples, while 24 stored loser-relative samples.
The split was 96/3 for games whose first sampled ply was 8 and 80/21 for games whose
first sampled ply was 9, so merely randomizing the sampling parity did not remove it.

The generator currently skips rollout search on a sampled ply and then uses the deeper
label search's best move for self-play. With `sample_every_ply = 2`, one side therefore
selects moves at depth 3 while the other selects them at rollout depth 1. The sampled
side is made stronger by the act of sampling, directly coupling the final result label
to sample selection.

Before generating 1M positions, label search must be observational: every post-opening
self-play move should come from the same rollout policy, whether or not that position is
sampled. A new pilot must then pass the Phase 7 outcome gate. This correction will add a
depth-1 rollout search on sampled plies, so the timing measurement above is a slight
underestimate for the corrected generator and must be repeated after the fix.
