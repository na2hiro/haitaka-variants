# Anhoku NNUE v0.6 corrected-data baseline

This report records the Phase 7 experiment completed on 2026-08-20. It isolates
the corrected post-opening sampling and uniform-rollout data policy while
retaining the compatible v0.5.1 architecture and training hyperparameters.

## Technical summary

The dataset gate passed and all three initialization seeds trained, exported 60
checkpoints, completed paired checkpoint selection, and passed the 14-position
NNUE verification suite. The result does **not** support advancing this control
unchanged: every run selected epoch 0, validation loss worsened after epoch 0,
and the selected models lost about 230 Elo to handcrafted and 91 Elo to the
corrected v0.5.1 baseline at 100 ms. Median handcrafted Elo was -233.81; the
three-seed sample standard deviation was only 3.01 Elo, so the regression is
repeatable rather than an isolated seed failure.

The production merge contains 1,845,886 train and 265,571 validation positions.
This is larger than the phase's approximate 1M label because corrected uniform
rollout increased the yield per game; the configured 50,000 + 5,000 games were
kept unchanged. The experiment therefore measures the requested v0.6 dataset,
but the document name retains the Phase 7 `1m-data` contract.

Phase 8 is **not approved** from this result. Before another full training run,
the epoch-0 optimum and the large gap to v0.5.1/handcrafted should be diagnosed
with a smaller controlled experiment.

## Data passed the production gate

The final dataset was assembled from the existing local lanes plus
`anhoku-v0.6-mac.tgz` and `anhoku-v0.6-78.tgz`. Together they provided a complete
set of 2,500 train shards and 250 validation shards. The `-78` lane had a null
engine revision while the other lanes recorded
`3841b5e5ef82836bcc2362b1b1469ca5bf798ff8`; all other data/config identities
matched, so the merge used the explicit identity-mismatch override without
rewriting any source manifest.

| Gate | Train | Validation | Result |
| --- | ---: | ---: | --- |
| Positions | 1,845,886 | 265,571 | recorded |
| Side to move | 49.96% black / 50.04% white | 50.42% black / 49.58% white | pass |
| Relative outcome among decisive labels | 50.46% win / 49.54% loss | 49.78% win / 50.22% loss | pass |
| Draw share | 26.44% | 0.00% | informational |
| Samples before opening | 0 | 0 | pass |
| Unique game IDs | 50,000 / 50,000 | 5,000 / 5,000 | pass |
| Train/validation opening-group overlap | 0 | 0 | pass |

The validation split has no draws, but both decisive orientations remain near
50/50 and all specified gates pass. This distribution difference is retained as
a limitation because it may contribute to the rising validation loss.

### Dataset identity and hashes

| Artifact | SHA-256 |
| --- | --- |
| Base v0.6 TOML / config identity | `e4e879a255a1ad4d20b665001b7c9e434c7d089ce2813de0ad0cdf3e311f553c` |
| Anhoku v1 opening suite | `7150a2a5871c4d302b63ab99ea31abe086471fa38a213dc184f42ce5d05721a7` |
| Merged train binary | `e101d179c96115f65fa886c1b0cf13b38a25ac815930c1f9217d531edf1db8e4` |
| Merged validation binary | `405d8a72a29cdf1e67539c9ac75440774475054c60c4c43c7a04600437fe9021` |

The recorded policy identity is `per-game-random-v1` sampling,
`uniform-rollout-v1` play, `opening-group-hash-v1` split with seed 76, and
bounded chunk shuffle with seed 77. Sampling seed is 75. No sample occurs in the
opening phase and teacher-move storage is intentionally unavailable/zero.

## Training and selection completed for all seeds

Training used `HalfKAv2^+DonorSingleEff`, batch size 16,384, lambda 0.8,
random-FEN skipping 3, epoch size 1,000,000, validation size 100,000, and 60
epochs. The three runs passed explicit trainer seeds 1, 2, and 3; each log records
`Global seed set to N`. The Haitaka source snapshot was
`3841b5e5ef82836bcc2362b1b1469ca5bf798ff8`; the trainer revision was
`2388a9bb7bf7004eee3954ee72ff4d407a1bc1bd`.

The Vast container used an RTX 3060 12 GiB with driver 595.71.05. The working
environment required Python 3.12, PyTorch 2.13.0+cu132, NumPy 1.26.4,
CuPy 13.6.0 for CUDA 13, and setuptools below 81 for the legacy Lightning 1.9.5
dependency path.

| Seed | Selected checkpoint | Selected SHA-256 | Ranking games/status | Validation loss epoch 0 / final |
| ---: | --- | --- | --- | ---: |
| 1 | epoch 0, step 62 | `3d63acb3cceb409cbb9d3502c5424d3bdd36e4df5371a06775ac57ba8e3d041a` | 23,424 / decisive | 0.082754 / 0.099487 |
| 2 | epoch 0, step 62 | `0e601eece921ffd337e700fdac88baff52461cd1c845d717ec2a78a3f05179da` | 32,768 / budget-limited | 0.080585 / 0.092077 |
| 3 | epoch 0, step 62 | `a99eea4878f34dc8e7992b8bcbb5d9f6623862b8db427a5333aa60b3f1d4714b` | 28,288 / decisive | 0.081634 / 0.095357 |

All 60 checkpoint exports per seed were retained as NNUE candidates. The
selector's displayed `+0.0 Elo` is not a handcrafted result: epoch 0 is the
fixed anchor, so its self-rating is defined as zero. In seeds 1 and 3 every
challenger's upper confidence bound fell below the anchor; seed 2 exhausted the
32,768-game budget. For all three seeds the minimum validation loss also occurred
at epoch 0, independently supporting the selection result.

Each exported winner passed the 14-position verification suite, including the
configured search smoke test.

## The corrected-data models remain far below handcrafted

The automatic benchmark used 1,024 games (512 color-swapped pairs), 20 threads,
100 ms per move, four random opening plies, opening seed 9,000,001, and a
200-ply cap. A is the selected v0.6 NNUE and B is handcrafted; negative Elo is a
v0.6 loss.

| Seed | A wins / B wins / draws | Paired Elo | 95% CI |
| ---: | ---: | ---: | ---: |
| 1 | 203 / 804 / 17 | -233.81 | [-260.66, -206.95] |
| 2 | 212 / 804 / 8 | -229.18 | [-254.70, -203.66] |
| 3 | 203 / 806 / 15 | -234.84 | [-260.15, -209.53] |
| **Median** | — | **-233.81** | — |

The mean is -232.61 Elo, the sample standard deviation is 3.01 Elo, and the
full seed range is only 5.66 Elo. Every interval is far below zero. The small
between-seed spread makes the negative result robust to initialization seed.
With only three seeds, the exact-value table is more legible and less suggestive
of a distribution shape than a chart, so no chart is used.

## The corrected-data models also regress against v0.5.1

The formal baseline is the corrected v0.5.1 re-selection at epoch 6,
SHA-256 `1c6ffefb34fe53137d33c3ccd5668dc507c4b11e4841cf6c6670167a4d26380f`.
The original v0.5.1 export with SHA `e3c187...` was detected during setup and
excluded from final comparisons. All final matches reuse the same color-swapped
opening pairs, random opening seed 9,000,001, and four random opening plies.

### 100 ms paired strength

| Seed | Games | A wins / B wins / draws | Paired Elo | 95% CI | A/B NPS |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1,024 | 375 / 644 / 5 | -93.46 | [-114.28, -72.64] | 26,612 / 24,194 |
| 2 | 1,024 | 374 / 635 / 15 | -90.55 | [-111.45, -69.65] | 26,518 / 24,630 |
| 3 | 1,024 | 378 / 632 / 14 | -88.02 | [-109.53, -66.50] | 25,888 / 23,922 |
| **Median** | — | — | **-90.55** | — | — |

The direct v0.5.1 mean is -90.68 Elo, sample standard deviation is 2.72 Elo,
and seed range is 5.44 Elo. All confidence intervals exclude zero. v0.6 searches
about 5% to 10% more main-search nodes per second in these matches, but that
speed difference does not compensate for its weaker evaluation.

### Fixed-depth evaluation-quality diagnostic

The current self-play interface has no per-side node-limit option. Adding one
would mix a code change into an experiment-only phase, so the required
equal-node diagnostic is approximated by equal fixed depth 3, matching the
teacher-label depth. These 32-game diagnostics are not used as promotion Elo;
the reports preserve actual nodes and NPS so the search-work mismatch remains
visible. Each seed/opponent result contains 16 color-swapped pairs.

| Opponent | Seed | Games | A wins / B wins / draws | Paired Elo | 95% CI | A/B nodes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| v0.5.1 | 1 | 32 | 6 / 25 / 1 | -237.45 | [-366.43, -108.47] | 5,955,013 / 9,130,433 |
| v0.5.1 | 2 | 32 | 10 / 21 / 1 | -124.50 | [-250.20, +1.20] | 7,391,439 / 10,032,184 |
| v0.5.1 | 3 | 32 | 14 / 18 / 0 | -43.66 | [-167.97, +80.65] | 6,279,748 / 7,483,909 |
| handcrafted | 1 | 32 | 7 / 25 / 0 | -221.14 | [-377.82, -64.45] | 7,757,843 / 21,045,891 |
| handcrafted | 2 | 32 | 8 / 23 / 1 | -176.66 | [-311.56, -41.76] | 7,426,035 / 15,981,387 |
| handcrafted | 3 | 32 | 8 / 23 / 1 | -176.66 | [-311.56, -41.76] | 7,156,224 / 14,073,944 |

Because this is equal-depth rather than a hard node cap, it is a search-shape
diagnostic and not a literal completion of the equal-node method. The 100 ms
paired results remain the decision-quality strength evidence. The depth-3
diagnostic is directionally negative in all six cells. Its v0.5.1 uncertainty is
wide for seeds 2 and 3, but all three handcrafted point estimates are at least
176 Elo below zero. The node totals also show why these rows cannot be relabeled
as equal-node results: handcrafted searches 2.0x to 2.7x as many main-search
nodes, while v0.5.1 searches 1.2x to 1.5x as many.

## Failures were diagnosed and preserved

The first launch attempts failed before GPU training and remain visible in the
supervisor log:

1. The container lacked Rust/Cargo; the documented minimal rustup installation
   supplied Cargo 1.97.1.
2. The first generated seed TOMLs were malformed; they were regenerated from
   the verified base config.
3. Lightning 1.9.5 failed with NumPy 2 because `np.Inf` was removed; NumPy was
   pinned to 1.26.4.
4. The trainer requirements selected a CUDA 11 CuPy wheel; it was replaced by
   `cupy-cuda13x==13.6.0` and a real CUDA kernel plus trainer launch verified the
   fix.
5. The legacy Lightning namespace path required `pkg_resources`; setuptools was
   pinned below 81.

Two comparison-launch corrections are also preserved but excluded from the
formal tables. The first comparison used the original v0.5.1 export
(`e3c187...`) before the corrected re-selection was identified; it was stopped
and rerun against `1c6ffe...`. Seed 2/3 timed matches were then briefly launched
in parallel with fixed-depth work; their partial JSONL files are labeled
`contended-excluded`, and the formal 100 ms matches were rerun sequentially on
an otherwise idle CPU.

The successful CUDA 13 sequence then completed all three seeds with zero train
and verify exit codes. A checkpoint cleanup watcher removed only superseded
Lightning `.ckpt` files after their NNUE exports were recorded, preventing the
80 GB instance disk from filling. Candidate NNUE exports, selection manifests,
match reports, TensorBoard events, final models, and the latest resume checkpoint
were retained.

## Interpretation and Phase 8 decision

The corrected dataset itself satisfies the planned structural and distribution
gates, and the result is highly consistent across seeds. The failure is in model
quality after fitting: validation loss is best immediately after the first epoch,
later checkpoints are weaker than the fixed anchor, and even the selected epoch-0
models lose heavily to handcrafted. This experiment does not identify whether
the primary cause is the no-draw validation distribution, score/mate weighting,
the much larger-than-1M effective train set, or the changed position policy.

Phase 8 must not inherit this training recipe as an approved control. A smaller
controlled follow-up should first:

1. reproduce epoch-0 selection on a compact subset while retaining all epoch
   metrics;
2. compare validation slices with and without draws and by opening group;
3. inspect mate-saturated labels and loss weighting;
4. add a true node-limit self-play mode in a separate code change, then rerun
   the evaluation-quality diagnostic; and
5. require a v0.5.1 parity gate before spending on Phase 8 root/leaf training.

## Open questions for the next controlled run

- Does a validation slice with the train split's draw rate restore alignment
  between validation loss and paired strength?
- Are mate-saturated depth-3 labels or their effective loss weight responsible
  for the rapid post-epoch-0 degradation?
- Does holding the train corpus near one million positions change the optimum,
  or is the regression tied to corrected position semantics rather than corpus
  size?

## Acceptance review

| Phase 7 criterion | Result |
| --- | --- |
| Three initialization seeds complete or diagnosed | pass: all three complete; pre-training failures preserved |
| Best checkpoint selected per seed | pass: all three select epoch 0 |
| Median handcrafted Elo and seed variance reported | pass: -233.81 median, 3.01 Elo sample SD |
| Dataset distribution, opening, and split gates | pass |
| v0.5.1 and handcrafted 100 ms paired comparisons | pass: 1,024 games per seed/opponent, all paired |
| Equal-node diagnostic | qualified: fixed-depth-3 substitute because the released CLI has no node limit |
| No experiment code changes | pass: only this result documentation is tracked |

The Phase 7 experiment is closed as a negative result. It does not approve
Phase 8 and does not promote any v0.6 model as the default.
