# Anhoku independent-data confirmation result

The frozen 100 ms confirmation completed both 4,096-game schedules on the MacBook Pro, one game at a time. The joint criterion **failed**, so handcrafted remains the production evaluator and NNUE deployment is not authorized.

| Model | Games | W–L–D | Pair score | One-sided 95% lower bound | Decision |
| --- | ---: | ---: | ---: | ---: | --- |
| A, development-nominated u2,688 | 4,096 | 2,324–1,771–1 | 56.7505% | 55.5928% | Pass |
| B, independent-data u2,688 | 4,096 | 1,992–2,103–1 | 48.6450% | 47.4931% | Fail |

The preregistered conjunction required each model's one-sided 95% Student-t lower bound over 2,048 color-swapped opening-source pairs to exceed 50%. Model A passed strongly. Model B's point estimate itself was below 50%, so the conjunction failed decisively.

All 8,192 scheduled games were present. Artifact hashes, model and suite identities, colors, opening mapping, game indexes, commands, WDL totals, pair scores, and termination records passed the frozen analyzer. There were no fallbacks, maximum-ply games, interim result looks, or sample extensions. Results remained sealed until both complete schedules were available.

This was a separately frozen native MacBook Pro endpoint at 100 ms per move. It does not establish the plan's original 1,000 ms production claim, qualify the shipped WASM interface, or authorize deployment. The earlier partial 1,000 ms schedule remains unopened and excluded from analysis.

The validated result is preserved at `out/anhoku-strength-first/step9-independent-confirmation-100ms-20260913-v1/confirmation-result-v1.json` with SHA-256 `34f02d8d…`. End this candidate lane and retain handcrafted in production. Any future NNUE attempt should start from a new development hypothesis and prospective contract rather than extending or rerunning these confirmation suites.
