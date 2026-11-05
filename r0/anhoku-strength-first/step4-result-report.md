# Anhoku strength-first Step 4 result

Step 4 completed as a valid development-only diagnostic on the MBP at `192.168.0.12`. It used 175.30M search nodes including retained failed-closed and regenerated work, below the frozen 180.80M ceiling. No GPU or paid compute was used.

The diagnostic selected the full quota of 128 parents: 64 disagreement cases and 64 controls, balanced 32 early, 64 middle, and 32 late. The restricted union contained 249 moves. All 249 child labels completed, including four exact terminal labels.

## Main result

| Evaluator | Mean restricted regret | Median | p95 |
| --- | ---: | ---: | ---: |
| C/16 | 0.038089 | 0.000000 | 0.056642 |
| Candidate width 256 | 0.046444 | 0.000000 | 0.103983 |
| Handcrafted | 0.043868 | 0.000000 | 0.091375 |

Candidate minus C/16 mean restricted regret was `+0.008355`, or about `+100.3` teacher score units. Its opening-group bootstrap interval was `[-0.014716, +0.031543]`, equivalent to about `[-176.6, +378.5]` score units. The interval includes zero, so this diagnostic does not establish that either evaluator is better on restricted teacher preference.

The useful finding is the interaction between score representation and search. The full-precision candidate had the best static calibration: Brier `0.012895`, correlation `0.548`, and MAE `2678.6`. Its integer export was weaker on all three measures and disagreed in sign on `21.6%` of nonterminal labeled moves. Full-precision and integer restricted static choices agreed on only `102/128` parents (`79.7%`).

Candidate search choices were stable across 2.5k, 10k, and 40k nodes on `75/127` eligible parents (`59.1%`). Full-precision static choice agreed with the candidate's 10k search choice on `94/128` parents (`73.4%`). Together with fitted teacher-on-model slopes near `2.0`, this supports testing score scaling and export/search interaction before another capacity increase.

On the 17 parents whose best restricted label was a mate, the candidate selected the exact best restricted move 14 times, versus 13 for C/16 and 12 for handcrafted. This small subset does not show a candidate mate-choice deficit. On 42 near-equal parents, candidate mean regret was `0.03256` versus `0.00879` for C/16.

The next bounded experiment should be CPU-only: compare the candidate's existing `1.0` score multiplier with a single `2.0` treatment derived from the development slope, applying the multiplier consistently to every evaluation-dependent search path. Recheck restricted regret, pruning sensitivity, and direct development games. New objective training or quantization-aware training would require separate GPU rental permission.

The detailed machine-readable analysis is in `out/anhoku-strength-first/step4-objective-search-diagnostic-20260907-v1/analysis/result.json`. This remains reused development evidence and does not authorize promotion or deployment.
