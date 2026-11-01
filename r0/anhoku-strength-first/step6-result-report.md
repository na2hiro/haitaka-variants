# Anhoku: corrected NNUE runtime and 1-second browser baseline

The CPU audit found and corrected an extra six-bit shift in feature-transformer activation. The serialized weights already use activation scale 127; hidden affine layers still require their six-bit shift. Model bytes and the 1× output multiplier are unchanged. No GPU or deployment was used.

On 8,277 nonterminal positions, full-precision MAE fell from 550.3 to 44.8 score units. Independent integer reference, native full/incremental/trace evaluation, and current WASM agreed exactly on 8,278 positions. Full-precision static move agreement improved from 71/127 to 119/127 eligible parents.

The first 64-pair browser attempt completed play but Node 14 failed while formatting optional memory telemetry, before the raw games were saved. The v2 harness writes qualified games atomically before telemetry and pins Node 26.8.1. The complete schedule was replayed; results below use only that replay.

## Game results

| Comparison | Time per move | Games | W–L–D | Candidate score | Opening-group 95% interval |
| --- | ---: | ---: | ---: | ---: | ---: |
| Native corrected vs legacy | 1000 ms | 64 | 27–37–0 | 42.2% | 31.2%–53.1% |
| Browser corrected vs handcrafted (primary) | 1000 ms | 128 | 67–61–0 | 52.3% | 43.8%–60.9% |
| Browser corrected vs handcrafted (secondary) | 100 ms | 32 | 17–15–0 | 53.1% | 34.4%–71.9% |

On the 16 openings common to both browser schedules, the paired 1,000 ms minus 100 ms score difference was -6.2 percentage points, with a 95% interval [-34.4, +21.9]. Both opponents receive the stated budget, so this is a change in relative score against handcrafted, not a direct long-time versus short-time match.

The native result does not demonstrate that the correction improves game strength. It is an eight-worker arm64 development match against the legacy runtime. Browser results use one concurrent game on Ubuntu and compare against handcrafted; these environments and opponents are not pooled. All openings are reused development evidence, not independent confirmation.

## Browser timing and resources

- primary, 1000 ms: p95 elapsed 1000.1 ms, p99 1000.1 ms, maximum 1003.2 ms; maximum overshoot 3.2 ms. Capped games: 0.
  Measured peak Chrome process-group RSS: 1849.7 MiB.
- secondary, 100 ms: p95 elapsed 100.1 ms, p99 100.1 ms, maximum 103.4 ms; maximum overshoot 3.4 ms. Capped games: 0.
  Measured peak Chrome process-group RSS: 1838.8 MiB.

No missing, unsearched, or emergency-fallback moves were accepted. Per-search timing excludes model/module load time, which is retained separately in the machine-readable report. A second supported device and numeric product acceptance limits are still needed before production rollout.

## Teacher diagnosis and residual export error

All 271 restricted child labels completed. Mean restricted regret was 0.035458 for legacy and 0.012169 for corrected. Corrected minus legacy was -0.023289, with a 95% opening-cluster interval [-0.054589, +0.000993]. The interval includes zero. Three handcrafted roots were incomplete and are disclosed; the legacy/corrected comparison covers all 128 parents.

The diagnostic used 68.78M search nodes, below its 129.28M ceiling. Static enumeration of all legal children did not perform exhaustive child search. Residual MAE with only FT quantization was 39.2; no affine weights were clipped. FT quantization is a possible next ablation, not an automatic training or GPU decision.

## Validation and interpretation

- Runtime regression suite: 93 passed, one explicit-model test initially ignored; the latter was then run with the actual model and passed. Real-model scalar/SIMD and width-512 activation regression checks also passed.
- Historical Python integer oracles shared the runtime shift defect. They are corrected for future use; archived outputs remain measurements of the legacy implementation.
- The original long legacy browser launch was deferred before game search when the defect was verified. The current campaign froze the correction and qualified it before starting games.
- Parent canonical boards exclude all 128 previously labeled parents. Source games and openings are development history; exact training-position non-overlap is not asserted.
- The first browser result was lost to a reporter-only Node 14 incompatibility after 64/64 pairs. The v2 replay is the sole browser sample used here.
- No model was retrained, no paid compute was rented, and nothing was deployed.

## Next decision

Keep the serializer-consistent correction in the working runtime and use the measured 1-second baseline for future development. Do not promote the NNUE merely because export fidelity improved. Before new training, re-screen at most two retained checkpoints under the corrected runtime at 1 second, because prior checkpoint selection used the legacy implementation. Only then choose a single further ablation if the game result and residual-error evidence support it. Final promotion still requires independent opening groups, replication, target-device checks, and the plan’s confirmation criterion.

Reproduction and raw evidence: `r0/anhoku-strength-first/step6-browser-export-v2.json`; `scripts/strength_first_export_analyze.py`, `scripts/strength_first_export_wasm_audit.mjs`, `scripts/strength_first_export_remote.py`, `scripts/strength_first_browser_baseline.py`, and `scripts/strength_first_browser_export_report.py`; `out/anhoku-strength-first/step6-browser-export-20260908-v1/` and `out/anhoku-strength-first/step6-browser-export-retry-20260910-v2/`.
