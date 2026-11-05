# F1 loss ablation report

Completed 2026-09-28. Experiment: `strength-first-f1-loss-ablation-20260925-v1`.

## Decision

The preregistered development screen **did not advance**. T beat fresh control C on the matched development screen, but did not reach the required 0.52 score against incumbent I. The conditional T–I holdout was not run. I remains the experimental incumbent; handcrafted remains the production evaluator. No deployment or follow-on experiment is authorized by this result.

The F1 analysis contains 512 accepted development games: 128 color-swapped pairs for each matchup. All 16 blocks were accepted on their first attempt; there were no scored-block retries or capped games. The screen uses both ordinary and conservative scores, with a 0.52 threshold against each opponent.

| Matchup | Pairs / games | W–D–L from T's perspective | Caps | Ordinary score | Conservative score | Conservative pair histogram `{0, .25, .5, .75, 1}` | Part telemetry wall time |
| --- | ---: | ---: | ---: | ---: | ---: | --- | ---: |
| T–C | 128 / 256 | 136–0–120 | 0 | 0.53125 | 0.53125 | 23 / 0 / 74 / 0 / 31 | 9,969.816 s (2.769 h) |
| T–I | 128 / 256 | 116–0–140 | 0 | 0.453125 | 0.453125 | 35 / 0 / 70 / 0 / 23 | 9,610.943 s (2.670 h) |

T–C cleared both thresholds. T–I fell below both, so the overall screen failed. With zero caps, ordinary and conservative scores coincide. This is a development screen, not a significance test; it does not show equivalence or establish that the loss recipe is generally ineffective.

## Training and frozen candidates

C used clipped-score Huber loss. T used the specified sigmoid-space teacher loss with training-only `tau = 1412.6912797248515`, calculated as the median nonzero absolute training target divided by `ln(3)`. This is distribution-scaled teacher regression, not empirical outcome calibration.

Both arms completed 1,024 updates and processed 16,691,968 examples. The common validation-Huber selector chose update 768 for both arms, before games. Selected validation Huber was 0.0010225237369537355 for C and 0.0012131194750467937 for T. T's selected export is [u768](../../out/anhoku-strength-first/f1-loss-ablation-20260925-v1/training/arms/T/exports/u000768.nnue); C's is [u768](../../out/anhoku-strength-first/f1-loss-ablation-20260925-v1/training/arms/C/exports/u000768.nnue). I is the frozen Step 10 u768 artifact.

CPU qualification and CUDA qualification for C and T passed. MBP runtime qualification passed for I, C and T: 128 native/WASM parity rows per model with zero mismatches, 12 terminal/history queries per model, eight timed model searches per model, and eight fixture games. Fixture games were not included in scored results.

## Analysis and package integrity

Before opening development metrics, review found that the packaged offline analyzer compared provisional browser `move-limit-reached` labels directly with canonical replay results. The replay helper treats that browser label as provisional: replay can return `maximum-ply-capped` or adjudicate a true terminal at ply 200. The correction retained the independent replayed-ply equality check and used the already frozen canonical replay helper for score and genuine-cap treatment. It changed no raw games, accepted receipts, score rule, scientific threshold or runtime package.

The [offline analysis repair receipt](../../out/anhoku-strength-first/f1-loss-ablation-20260925-v1/amendments/offline-analysis-repair-01/repair.json) preserves both source versions, hashes and test results. The repaired analyzer reproduced the immutable development analysis. The original package-bound analyzer was restored byte-for-byte; the frozen package verifies at SHA-256 `93967440d92158ae2315322294120c53e437dd5480fa5618832661c70cec38cc` with 2,628 files.

## Resource use and closeout

The development blocks record 19,611.027 s (5.448 h) of MBP wall time. Including 74.871 s of retained setup/freeze activity and 77.020 s of qualification telemetry gives 5.490 h of measured MBP activity against the 32-hour grant. The setup ledger excludes earlier staging commands for which no timing receipt exists; no time was imputed for that gap.

GPU training and CUDA qualification occurred on Vast.ai instance 52952789. The required archive was verified, the instance was destroyed, and the post-destroy provider listing was empty. The final provider invoice was not available before destruction; the last monitor sample estimated USD 1.6848 at 2.974 billed hours, so that is not asserted as a final charge. The separately approved additional five-hour GPU reserve was unused after the training and CUDA work completed.

The official [closeout](../../out/anhoku-strength-first/f1-loss-ablation-20260925-v1/closeout.json) records category `screen-did-not-advance`, `deploymentAuthorized=false`, and zero jobs/searches dispatched by closeout. It passed twice with identical output. No holdout analysis or holdout games were created.

## Evidence

- [Development analysis](../../out/anhoku-strength-first/f1-loss-ablation-20260925-v1/analysis/development.json)
- [Accepted development blocks and raw/replayed games](../../out/anhoku-strength-first/f1-loss-ablation-20260925-v1/games/development/)
- [Runtime qualification](../../out/anhoku-strength-first/f1-loss-ablation-20260925-v1/runtime/qualification.json)
- [Instance disposition](../../out/anhoku-strength-first/f1-loss-ablation-20260925-v1/rental/instance-disposition-final.json)
- [Frozen package manifest](../../out/anhoku-strength-first/f1-loss-ablation-20260925-v1/package.json)
