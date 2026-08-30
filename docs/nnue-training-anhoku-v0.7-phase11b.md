# Anhoku NNUE v0.7 Phase 11-B result

## Decision

**NOT RETAINED; stop after seed 80.** DonorReceiverPairV2 did not establish a
fixed-time gain over the matched DonorSingleEff control. At the predeclared
4,096-game cap, V2's paired 95% interval still included zero. The plan therefore
forbids seed 81, a fresh handcrafted diagnostic, and promotion or scaling of
this feature.

Both fixed step-16 lanes, full-precision checkpoints, exported networks, logs,
per-game match records, and machine-readable closeout were preserved locally
before Vast instance `49301592` was destroyed.

## Training and identities

The run used Haitaka `d96c330cd8b3896b812e7d84d928610b40cc8a38`,
Phase 11-A implementation `c26e4fd`, and trainer
`61666d9e3653e4df9881b14c23f8fdcc4bf7779b`. The reviewed trainer patch was
applied with source SHA-256
`79603cc66250e335ba242477137366f0aa8a2e530ffa36f3abfb582fafaf802f`;
the resulting `model.py`/`train.py` diff hash was
`87f5a9a446bb929854dbf01b38db16980e4faee73a2f86044ae725f98ee0bc4b`.

Both lanes used seed 80, the identical audited 279,627-record train file and
3,218-record OOD-v2 validation file, C/16-equivalent initialization, batch
16,384, lambda 0.8, LR 0.00015, and exactly 16 optimizer steps.

| Lane | Feature family | NNUE bytes | NNUE SHA-256 | checkpoint SHA-256 |
| --- | --- | ---: | --- | --- |
| V1 | `HalfKAv2^+DonorSingleEff` | 161,206,541 | `f7111caf885db66e528c56f23ffe9446609daf1f9a1b3a13cc1c2043b1a66632` | `442e2030620b21a6f3fdf2add33eae6039f1fd865d466f20a8c3ffe0e0360a39` |
| V2 | `HalfKAv2^+DonorReceiverPairV2` | 176,603,039 | `7e94100c24c495265fed01c06c4f9359f44aa52182c8481b46bf936f63c63a31` | `9d7997027791298b2d4de0a3e61acc571c48ec4c1895c222f0dc2fe292fc373b` |

## Non-strength gates

Both runtime verifiers passed all 14 positions and their depth-2 search smoke.
The frozen six-position tactical suite passed for both trained networks with
all expected moves. Median fixed-position full-refresh latency was 3,499.94 ns
for V1 and 3,524.52 ns for V2, a 0.70% regression against the 5% limit.

The configured OOD-v2 validation scalar (`id_val_loss`) ended at
`0.0741922334` for V1 and `0.0743985325` for V2. Both improved from their first
recorded value; V2 ended 0.278% above V1. No numerical OOD veto threshold had
been frozen, so this remains a reported diagnostic and was not used to rescue
the failed strength result.

## Fixed-time strength gate

V2 was engine A and V1 engine B throughout. Both used the same Anhoku binary,
100 ms/move, four deterministic random opening plies, paired colors, 200-ply
cap, and 20 workers. The initial stream used seed 1180. Because its interval
included zero, the single authorized extension used non-overlapping seed 12180.

| Batch | V2-V1-draw | Pair bins | Paired Elo (95% CI) |
| --- | ---: | --- | ---: |
| Initial 1,024 | 495-528-1 | `[55, 0, 418, 1, 38]` | `-11.20 [-24.03, +1.63]` |
| Extension 3,072 | 1517-1548-7 | `[143, 1, 1263, 2, 127]` | `-3.51 [-10.80, +3.79]` |
| Cumulative 4,096 | 2012-2076-8 | `[198, 1, 1681, 3, 165]` | `-5.43 [-11.77, +0.91]` |

Cumulative main-search NPS was 16,220.85 for V2 and 16,397.87 for V1. V2's
1.08% regression passes the 5% boundary. There were no runtime warnings,
fallbacks, or node-budget cap hits. Strength alone fails: the cumulative lower
95% bound is not above zero at the maximum sample.

## Rental and retained artifacts

The on-demand RTX 3090 cost `$0.161111/hour`. The final pre-destroy snapshot
recorded 8,643.26 seconds (2.401 hours), projecting `$0.3868`; the account
credit change was about `$0.385`. The instance had no persistent volume and is
confirmed absent from `vastai show instances` after destruction.

- Training archive: `target/pretrain-bundles/anhoku-v0.7-phase11b-seed80-results.tgz`, SHA-256 `7a9c87571dc465f03e9146717b54a190d24dd3a2d0bbf5106418e49e4f43f3ba`.
- Remote gate archive: `target/pretrain-bundles/anhoku-v0.7-phase11b-seed80-gate-results.tgz`, SHA-256 `c3194be8e21ebb321ae725b1580747e8b69764e70d808801dbc8ff2ef420f8f2`.
- Final local closeout archive: `target/pretrain-bundles/anhoku-v0.7-phase11b-seed80-closeout.tgz`, SHA-256 `547754e0ae0bce54fa1ea0296db4bc09c1a250d7de5ab5ff22106c6270298281`.
- Machine decision: `out/anhoku-v0.7-phase11b-seed80-v2/artifacts/phase11b-gate/seed80/phase11b-gate-result.json`.
- Cumulative report: `out/anhoku-v0.7-phase11b-seed80-v2/artifacts/phase11b-gate/seed80/cumulative-4096.json`.
- Tactical/latency report: `out/anhoku-v0.7-phase11b-seed80-v2/artifacts/phase11b-gate/seed80/trained-tactical-latency-gate.json`.

The representation implementation remains available as experiment history,
but V1 remains the recommended Anhoku family. A new representation hypothesis
requires a new written plan boundary.
