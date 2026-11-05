# Anhoku strength-first Step 3 result

Generated 2026-09-07T12:36:39+09:00. Status: **complete valid development evidence**. This report makes no confirmation, promotion, or deployment claim.

The width-256 scratch run completed all 4,096 optimizer updates, consuming 66,753,664 examples, or 75.856 equivalent passes over the 880,000-record training set. Validation loss reached its minimum of `0.01250062993367513` at update 1,920 and rose slightly to `0.012580316670735676` by update 4,096.

The three frozen scheduled screens used 64 color-swapped pairs at 10,000 nodes per move. All games completed with zero missing moves, illegal moves, emergency fallbacks, or unsearched moves.

| Update | W-L-D | Score | Opening-group 95% interval | Gate |
| ---: | ---: | ---: | ---: | --- |
| 256 | 52-76-0 | 40.63% | 32.34%–48.91% | Continue |
| 1,024 | 71-57-0 | 55.47% | 46.53%–64.40% | Continue |
| 2,048 | 87-41-0 | 67.97% | 59.53%–76.40% | Continue |

The update-4,096 run was an exploratory unscheduled screen and is excluded from the formal scheduled-screen history. It scored 73-55-0, or 57.03%, over 64 pairs. Together with the higher validation loss, this suggests that continuing the same compact run past the selected region did not help.

Final selection compared the two roles fixed in advance on all 128 pairs at 10,000 nodes per move. Update 1,920 was the lowest-validation-loss checkpoint; update 2,048 was the highest scheduled-screen checkpoint.

| Candidate | Role | W-L-D | Score | Opening-group 95% interval |
| ---: | --- | ---: | ---: | ---: |
| 1,920 | Lowest validation loss | 178-78-0 | 69.53% | 61.95%–77.12% |
| 2,048 | Highest scheduled score | 174-82-0 | 67.97% | 59.53%–76.40% |

Neither candidate had a fixed-time result before selection, so the frozen ranking used equal-node score first. It selected update 1,920. The selection graph validates with SHA-256 `ef807855d7aae34eb9d22562924cb82a55424ae69560a211ba393f365e562e3f`.

The selected-only 128-pair screen at 100 ms per move finished 172-84-0, scoring 67.19% with a grouped 95% interval of 59.49%–74.89%. Its full raw-match receipt validates with SHA-256 `52621b57116c7bf9e3da7bf9ca62bee921c9851844887ebad8a05092e4a24362`; all 256 games passed the same move-integrity checks.

The retained selected network is `exports/selected-u00001920.nnue`, 83,048,702 bytes, SHA-256 `97015a2437adb8889203ceea9198b65cac75c43520061c87aa19be2c81a2d896`. It is byte-identical to the preselection u1,920 candidate that passed the full-precision → integer-reference → native/WASM check on 1,024 positions: zero mismatches in every integer/runtime comparison, with native full refresh equal to incremental evaluation. Quantization was measured separately: mean absolute score delta 155.29, p95 656.74, p99 683.67, and maximum 1,080.13 runtime score units.

The complete receipt validators passed for the selection graph and the final fixed-time graph. The selected-export receipt snapshot contains 74 files, the retained training and match processes are no longer running, and Vast.ai instance 50101026 is confirmed `actual_status=exited` and `intended_status=stopped`. The billing lookup observed a $1.213 instance charge for the September 6–8 window, within the approved $5 envelope.

This is strong development evidence for the compact recipe, but the openings were reused for screening, checkpoint choice, and the post-selection screen. The lower interval above 50% therefore does not satisfy the independent confirmation protocol. Handcrafted remains the production recommendation, and no deployment is authorized.

The next bounded experiment should be the matched width-512 capacity comparator with the same data, objective, exposure, and seed. The compact model improved materially, while loss bottomed and the exploratory late screen regressed; changing width alone tests whether capacity is now limiting. That follow-up has not been authorized or started.
