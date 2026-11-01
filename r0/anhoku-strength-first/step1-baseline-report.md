# Anhoku strength-first Step 1 baseline

Generated 2026-09-06T21:23:11+09:00. Status: **valid development evidence**. This report does not make a promotion claim.

The corrected native runs used the same 128 color-swapped pairs (64 opening IDs repeated twice) at 10,000 combined alpha-beta plus qsearch nodes per move. Every retained run has zero emergency fallbacks and zero missing moves.

| Candidate | W-L-D | Score | Opening-group 95% interval |
| --- | ---: | ---: | ---: |
| C/16 | 102-150-4 | 40.63% | 32.72%–48.53% |
| Current seed 17 | 112-142-2 | 44.14% | 35.19%–53.09% |
| Compact width 256 | 122-134-0 | 47.66% | 38.92%–56.40% |

The browser runs used one Chrome module worker and the matching SIMD WASM build for 64 color-swapped pairs at 100 ms per move. Both independent audits passed with zero fallback, missing, or unsearched moves.

| Candidate | W-L-D | Score | Opening-pair 95% interval | Model load | Cold load | Peak process-group RSS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| C/16 | 61-67-0 | 47.66% | 38.92%–56.40% | 333.5 ms | 1054.5 ms | 2.17 GB |
| Compact width 256 | 58-70-0 | 45.31% | 38.03%–52.60% | 179.9 ms | 656.2 ms | 1.94 GB |

The fixed-time score difference is unresolved. Compact remains the Step 3 family because it was strongest in the equal-node baseline and materially smaller to load and run. The next experiment starts from scratch and tests the new schedule; no current network is established stronger than handcrafted.

Earlier native directories without `valid-v3` are excluded because they contain emergency fallbacks (C/16: 18; current seed 17: 2; compact: 2).
