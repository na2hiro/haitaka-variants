# Anhoku Step 5: score scale and thinking time

CPU development experiment on the MacBook Pro at 192.168.0.12. All 384 games completed.

Retain the original 1× evaluation scale. The 2× treatment did not demonstrate a benefit: it scored 30–34 against 1× at both budgets, had lower observed scores against handcrafted, and did not improve restricted teacher regret. Use 1 second as the primary budget for the next strength evaluation, retaining 100 ms as a secondary check. Production settings are unchanged.

The next improvement investigation should focus on the previously observed integer-export versus full-precision disagreement, with the candidate and 1× scale fixed. This experiment provides no reason to spend GPU time on another capacity increase.

| Matchup (score for first engine) | Budget | W–L–D | Score | Opening-group 95% interval |
| --- | ---: | ---: | ---: | ---: |
| scale2-vs-scale1 | 100ms | 30–34–0 | 46.9% | 35.9% to 57.8% |
| scale2-vs-scale1 | 1000ms | 30–34–0 | 46.9% | 39.1% to 56.2% |
| scale1-vs-handcrafted | 100ms | 43–21–0 | 67.2% | 56.2% to 76.6% |
| scale1-vs-handcrafted | 1000ms | 42–22–0 | 65.6% | 53.1% to 76.6% |
| scale2-vs-handcrafted | 100ms | 39–25–0 | 60.9% | 50.0% to 71.9% |
| scale2-vs-handcrafted | 1000ms | 37–27–0 | 57.8% | 46.9% to 68.8% |

Each cell uses the same 32 development openings, one game with each engine color, 200-ply cap, and eight concurrent games. Both opponents receive the stated time budget. Intervals resample opening pairs; these are exploratory measurements on reused openings.

Doubling the score changed 29/128 root choices. Expanded-union restricted regret was 0.046451 at scale 1 and 0.053678 at scale 2. Scale 2 minus scale 1 was +0.007228, with a 95% opening-cluster interval [-0.001368, +0.022101]. Lower regret is better; this does not establish an improvement from scaling.

The default scale reproduced all 128 prior root results. All 8,290 child static-score checks passed. Both root probes completed at every parent, and all 24 additional child labels completed. Diagnostic compute was 8.56M nodes; existing 249 teacher labels were reused.

The multiplier is applied after final integer NNUE score division, consistently through full refresh, accumulator evaluation, and diagnostic traces. Scale 2 clamps static values below the mate band. Terminal values and default scale 1 behavior are preserved.

## Time-budget differences

- scale2-vs-scale1: score at 1 second minus score at 100 ms = +0.0 percentage points; paired opening interval [-14.1, +14.1].
- scale1-vs-handcrafted: score at 1 second minus score at 100 ms = -1.6 percentage points; paired opening interval [-17.2, +14.1].
- scale2-vs-handcrafted: score at 1 second minus score at 100 ms = -3.1 percentage points; paired opening interval [-18.8, +12.5].

## Search and pruning response

- Scale 1, fixed 10k-node roots: 72.0% qsearch nodes; mean completed depth 2.54.
- Scale 2, fixed 10k-node roots: 68.4% qsearch nodes; mean completed depth 2.63.
- Direct 100 ms games: qsearch cap hits per qnode were 0.0023 for scale 2 and 0.0125 for scale 1. These counters show a search response to scaling, but the game trajectories differ and the rates alone are not a strength measure.
- Direct 1000 ms games: qsearch cap hits per qnode were 0.0019 for scale 2 and 0.0116 for scale 1. These counters show a search response to scaling, but the game trajectories differ and the rates alone are not a strength measure.

## Browser timing smoke

A separate single-worker Ubuntu Chrome check used the archived Step 1 width-256 WASM runtime and the selected candidate at default scale. Four paired openings were stopped after two plies (16 searches per budget). These capped games provide timing evidence only.

- 100 ms budget: p95 and maximum elapsed 103.1 ms; maximum deadline overshoot 3.1 ms; no missing, unsearched, or emergency-fallback moves.
- 1000 ms budget: p95 and maximum elapsed 1002.8 ms; maximum deadline overshoot 2.8 ms; no missing, unsearched, or emergency-fallback moves.

This supports 1 second as a practical timing target for further testing, but does not validate all production devices. Browser model/module load time is separate from these per-search measurements.

## Limits and validation

- 32 reused development openings; exploratory intervals, no multiplicity correction.
- Native arm64 with eight concurrent single-thread games; not browser latency or target-device validation.
- Both opponents receive the same time budget. Relative score changes do not directly measure 1-second versus 100-ms strength for the same model.
- Root regret is restricted to the expanded union and handcrafted teacher, not an objective game-theoretic label.
- Production model and timing remain unchanged.
- Real-model regression test passed on arm64; release CLI and diagnostic builds passed; source diff whitespace check passed.
- The initial regression-test invocation used a relative model path that Cargo resolved from the crate directory. Re-running with the absolute path passed.
- The first game launch rejected the stable-ID TSV format before game search. The same openings were converted to bare SFEN; completed diagnostics were reused, and the original contract and controller log were retained.

Artifacts: `out/anhoku-strength-first/step5-scale-time-20260908-v1/analysis/result.json`, raw paired games, root probes, new child labels, command manifests, binary hashes, and the frozen `r0/anhoku-strength-first/step5-scale-time-v1.json`.
