# Anhoku strength-first Step 3 width-512 result

Generated 2026-09-07T18:17:45+09:00. Status: **complete valid development evidence, with a training-state archive limitation**. This report makes no confirmation, promotion, or deployment claim.

The width-512 scratch run completed all 4,096 optimizer updates using the same seed, data, objective, optimizer, schedule, batch size, and 66,753,664-example exposure as width 256. Validation loss reached its minimum of `0.012386211649576822` at update 1,920 and ended at `0.012424714914957682`. The minimum was 0.92% below the width-256 minimum of `0.01250062993367513`.

The three frozen scheduled screens used 64 color-swapped pairs at 10,000 nodes per move. All games had zero missing moves, illegal moves, emergency fallbacks, and unsearched moves. The single draw at update 256 was explicitly retained as a maximum-ply-capped draw.

| Update | W-D-L | Score | Opening-group 95% interval | Gate |
| ---: | ---: | ---: | ---: | --- |
| 256 | 51-1-76 | 40.23% | 32.56%–47.91% | Continue |
| 1,024 | 81-0-47 | 63.28% | 55.18%–71.38% | Continue |
| 2,048 | 80-0-48 | 62.50% | 53.32%–71.68% | Continue |

The update-4,096 screen was exploratory and unscheduled. It finished 75-0-53, scoring 58.59%; its opening-group interval, recomputed from the retained pair results, is 49.27%–67.92%.

Final selection compared the two frozen roles over all 128 pairs at 10,000 nodes per move. Update 1,920 had the lowest validation loss, while update 1,024 had the highest scheduled-screen score.

| Candidate | Role | W-D-L | Score | Opening-group 95% interval |
| ---: | --- | ---: | ---: | ---: |
| 1,920 | Lowest validation loss | 156-0-100 | 60.94% | 53.74%–68.14% |
| 1,024 | Highest scheduled score | 162-0-94 | 63.28% | 55.18%–71.38% |

Neither candidate had a fixed-time result before selection, so the frozen ranking used equal-node score and selected update 1,024. The selection history has SHA-256 `25445f921f1b9f4dea60dc845fc78c777883680416ca902c2f79bacfb03a7c20`.

The selected-only 128-pair screen at 100 ms per move finished 155-0-101, scoring 60.55% with a grouped interval of 52.16%–68.93%. Its raw-match receipt has SHA-256 `0332fee92664e4ad91e90b471039dc41fcf4264e8c3a699a253e67676c7ace32`; all 256 games passed the move-integrity checks.

The retained selected network is `exports/selected-u00001024.nnue`, 161,206,528 bytes, SHA-256 `208ad04d9a1d56083aae79d8162d6f83e49a4c742192a0fd01b5c7bdc46603fd`. It is byte-identical to the validated preselection u1,024 candidate. The full-precision-to-integer difference was measured separately. Integer-reference-to-native, integer-reference-to-WASM, and native-to-WASM checks covered 1,024 positions with zero mismatches; native full-refresh and incremental evaluations also matched. Quantization error was mean absolute delta 101.50, p95 620.61, p99 815.72, and maximum 1,035.08 runtime score units.

## Matched width comparison

Both selected models used the same ordered 64 opening groups, so the report pairs each width-512 group score with the corresponding width-256 group score. Negative differences favor width 256.

| Budget | Width 256 | Width 512 | Width 512 − width 256 | Paired 95% interval |
| --- | ---: | ---: | ---: | ---: |
| 10,000 nodes/move | 69.53% (178-0-78) | 63.28% (162-0-94) | −6.25 points | −17.71 to +5.21 points |
| 100 ms/move | 67.19% (172-0-84) | 60.55% (155-0-101) | −6.64 points | −17.15 to +3.87 points |

Width 512 lowered the best offline loss, but its selected model had weaker point estimates in both playing tests. Both paired intervals include zero. The result supplies no evidence that added width improves playing strength, while the uncertainty is too wide to establish that width 512 is intrinsically worse. The fixed-time tests ran on different rental instances, so the retained evidence does not support a capacity-only latency claim.

This loss-strength disagreement routes to the plan’s Step 4 objective/score/search diagnosis before spending on more capacity or more data. Width-256 update 1,920 remains the development nominee. Handcrafted remains the production recommendation because every game set here is reused development evidence.

## Integrity and rental closeout

The local archive retains all five candidate NNUEs, the selected NNUE, receipts, metrics, raw games, parity records, and runtime binaries. Hashes, grouped statistics, game counts, and move-integrity counters were rechecked locally. The selected-export command could only complete after the selection graph validator passed on the remote checkout.

The archive does not retain the full-precision `.pt` checkpoints, the final optimizer state, or redundant top-level scheduled NNUE copies. Those were excluded while transferring the 8.3 GB remote directory under its shutdown deadline; the retained archive is 1,190,408,465 bytes across 382 files. The selected inference model and every evaluated candidate remain reproducible as runtime inputs, but training cannot be resumed and the selection graph cannot be replayed locally because its validator requires the missing checkpoints.

Vast.ai instance `50127135` was destroyed and its final lookup returned no instance. The September 7–8 billing lookup reported $1.387 total: $1.324 GPU, $0.053 storage, $0.006 download, and $0.004 upload. This stayed within the approved $5 envelope.
