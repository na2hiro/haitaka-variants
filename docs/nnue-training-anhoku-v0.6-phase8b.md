# Anhoku NNUE v0.6 Phase 8B distributed data handoff

The Phase 8B 262k-target data configs are versioned in this checkout. Every
machine must use the same `strengthen` revision and a non-overlapping shard
range. The four-lane partition is:

| Machine | Shards |
| --- | --- |
| A (this host) | `1/4`, `2/4` |
| B | `3/4`, `4/4` |

On each machine:

```bash
git fetch origin strengthen
git switch strengthen
git pull --ff-only

git rev-parse HEAD
git status --short
```

Run the root and leaf lanes sequentially, using that machine's shard:

```bash
cargo generate haitaka_learn.anhoku-v0.6-phase8b-root-262k.data.toml --shard 1/4
cargo generate haitaka_learn.anhoku-v0.6-phase8b-leaf-262k.data.toml --shard 1/4
```

Replace `1/4` with the assigned shard(s) on the other machines. Do not use
the `.train.toml` files for generation and do not use
`--ignore-identity-mismatch`.

Copy each complete output directory back to one coordinator under distinct
names, preserving `datasets/shards`:

```bash
cargo merge haitaka_learn.anhoku-v0.6-phase8b-root-262k.data.toml \
  --input out/phase8b-root-262k-shard-1 \
  --input out/phase8b-root-262k-shard-2 \
  --input out/phase8b-root-262k-shard-3 \
  --input out/phase8b-root-262k-shard-4

cargo merge haitaka_learn.anhoku-v0.6-phase8b-leaf-262k.data.toml \
  --input out/phase8b-leaf-262k-shard-1 \
  --input out/phase8b-leaf-262k-shard-2 \
  --input out/phase8b-leaf-262k-shard-3 \
  --input out/phase8b-leaf-262k-shard-4
```

After merging, audit both lanes and run `check-matched` before creating the
GPU transfer bundles. The `.train.toml` configs are for the later single-seed
GPU run only.

## GPU training and fixed-control result

The two committed `.train.toml` runs were completed sequentially on the Vast
instance `ssh4.vast.ai` on 2026-08-24. The Haitaka checkout was
`719c3dd236952d918937e6c0365256efae31f735`, the trainer base was
`61666d9e3653e4df9881b14c23f8fdcc4bf7779b`, and both runs used initialization
seed `80`, `max_steps = 16`, and processed `262,144` training examples. The
train files contain 256,725 root records and 236,555 leaf records; trainer
epoch size must not be mistaken for dataset cardinality. A later packed-board
audit found only 8,641 distinct boards in the root file because deterministic
rollout repeated trajectories; the original report's use of “unique” was
incorrect. The GPU was an RTX 4070 SUPER with CUDA available. The Phase 8
three-seed matrix (`80`, `81`, `82`) was not run; this result must not be
described as a three-seed acceptance result.

The trainer's final progress loss was approximately `0.0820` for root and
`0.0643` for leaf. TensorBoard recorded loss on each lane's configured OOD-v2
validation file at every export checkpoint as follows:

| Lane | Step 4 | Step 8 | Step 12 | Step 16 |
| --- | ---: | ---: | ---: | ---: |
| Root | 0.069961 | 0.070478 | 0.069794 | 0.069632 |
| Leaf | 0.054149 | 0.054306 | 0.054157 | 0.053529 |

The fixed-anchor selector used the committed 4,096-game budget and was
budget-limited in both lanes. Step 16 was selected in each case:

| Lane | Selected NNUE SHA-256 | Fixed step-4 anchor | Paired Elo vs anchor (95% CI) |
| --- | --- | --- | ---: |
| Root | `12865f59f28f6e26feffcfae2e76c576f8eb31891148a8a9c167b8b50aac972c` | `adafe9b3a1906eb7ef1557e4157a186d2e7d58eb0703759a87eba1e78509d1a2` | +1.19 `[-3.94, +6.31]` |
| Leaf | `80ae84ce6aabc58036cef07905d801473f6f913b09d109fbc61dba55f8138d28` | `314236078ea711e3e3cd6caf8e819454186ae5b5691322a8b27a8bdaa3aa23b8` | +1.44 `[-3.61, +6.49]` |

The automatic 1,024-game handcrafted context reports were:

| Lane | W-L-D | Paired Elo (95% CI) | Combined A+B aggregate NPS |
| --- | ---: | ---: | ---: |
| Root | 343-672-9 | -115.73 `[-137.00, -94.45]` | 26,079 |
| Leaf | 326-693-5 | -130.30 `[-151.55, -109.06]` | 26,673 |

The combined NPS column is not the NNUE-side rate. In the root match, NNUE
searched 39,744,932 main nodes at 17,272 NPS while handcrafted searched
80,353,284 at 34,875 NPS over nearly equal elapsed time. Leaf similarly ran at
17,472 NNUE NPS versus 35,856 handcrafted NPS. Phase 6's 5.60x figure was a
scalar-to-AVX2 fixed-position diagnostic, not evidence that NNUE had become
faster than handcrafted. Fixed-time Phase 8B therefore contains both a model
result and an approximately 2.02x root main-NPS deficit; it cannot determine
their separate Elo contributions without a literal equal-node match.

For the planned independent fixed-control comparison, each selected export
played 1,024 games against the preserved Phase 7 C/16 model
(`049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0`) using
the same Anhoku start SFEN, seed `7104`, four random opening plies, 20 worker
threads, 100 ms per move, and a 200-ply cap:

| Lane vs C/16 | W-L-D | Pair bins | Paired Elo (95% CI) | Aggregate NPS |
| --- | ---: | --- | ---: | ---: |
| Root | 515-504-5 | `33,1,438,2,38` | +3.73 `[-7.54, +15.01]` | 17,938 |
| Leaf | 511-506-7 | `36,2,434,1,39` | +1.70 `[-9.89, +13.28]` | 18,158 |

Both fixed-control intervals overlap zero and each other. The leaf policy has
the lower validation loss, but it did not produce a strength advantage in the
fixed-control or handcrafted point estimates. The operational Phase 8B choice
is therefore the root export, based on the slightly higher fixed-control point
estimate and the stronger handcrafted point estimate; this is a weak,
non-significant preference rather than a claim that root has established
superiority.

## Plan-based interpretation

The production manifests confirm that root and leaf used the same attempted
positions. Their candidate identity is
`7286a7705a04819991930e383e1ae145bafdddcf11ee3ba0353bc876fcfd2251`
for train and
`cc1d4e75b030eb85b9c7691ed5e22f596bf8f005edd7f4b71e0330ac604a97b8`
for OOD-v2. Data quality was:

| Lane/split | Candidates | Stored | Incomplete | Other leaf rejection | Black share | Larger decisive share |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Root train | 258,609 | 256,725 | 0.729% | — | 49.40% | loss 50.92% |
| Root OOD-v2 | 1,610 | 1,606 | 0.248% | — | 38.17% | win 64.99% |
| Leaf train | 258,609 | 236,555 | 0.729% | 14,738 terminal + 5,432 mate | 52.95% | loss 55.36% |
| Leaf OOD-v2 | 1,610 | 1,460 | 0.248% | 107 terminal + 39 mate | 47.95% | loss 52.40% |

The corresponding dataset SHA-256 values are root train
`96c2281288ef6b49d7e53ef53de99b462202b3cf7fd812943b69c2b55c8bee48`,
root OOD-v2
`66c84ef3e28317fa7ae944630c6f93b3cdb73113c12478ec2b88ee9314204879`,
leaf train
`b65bb7be6bffa98e4a316d3e51ec68d4cc28e60f44ff7465afd26972292498f7`,
and leaf OOD-v2
`4bc532d47dbaa06b4a6c67dfe1791650489379d694c0b1b7a60af76a7afd36bc`.
Manifested teacher-search CPU time was 40.93 hours for root and 39.51 hours
for leaf, including validation. The two GPU trainer fits themselves took about
39 and 38 seconds; instance rental time and price were not preserved. The
ranking, handcrafted, and C/16 reports contain 14.94 aggregate engine-hours
across 12,288 games; this is summed engine elapsed time, not 20-worker wall
time.

The incomplete-label gate therefore passed. Train balance also passed. The
root OOD-v2 micro-distribution did not pass the old 45–55% side and 60%
decisive-outcome bounds, and only 9 of the 12 reserved opening IDs occurred in
the 40 validation games. Its loss is useful as a regression diagnostic, but
not yet a high-confidence OOD selector. The leaf loss is not numerically
comparable with root loss because leaf selection removes 20,170 additional
train candidates and changes both the positions and target distribution.

Steps 4, 8, and 12 exported to the same quantized NNUE in both lanes. Thus the
selector compared only two unique networks per lane, despite four checkpoint
events. The 4,096-game fixed-anchor comparisons precisely show that step 16 is
slightly better than the early quantized export, but they do not compare the
data policies with one another or establish an improvement over C/16. The
independent C/16 matches are the relevant Phase 8B gate: root and leaf both
have positive point estimates and lower confidence bounds narrowly above the
predeclared `-10 Elo` threshold, so either is non-inferior enough to continue;
neither has demonstrated superiority over C/16.

The handcrafted results remain decisive losses. Root's `-115.73 Elo` point
estimate is directionally better than C/16's prior `-128.4 Elo`, but the
direct root-versus-C/16 result is only `+3.73 Elo` and includes zero. The
project promotion condition is not close to satisfied, and scaling this
policy directly to 10M is not justified by Phase 8B.

Phase 8B also departed from the planned resource and evidence protocol. It
used 4,096 internal ranking games per lane and ran handcrafted context matches,
instead of the planned small C/16-first screens. No preserved tactical-suite
result was found, and the local artifact trees contain the selected `.nnue`
files but not the selected full-precision `.ckpt` files. These do not invalidate
the recorded matches, but they must be recorded as acceptance exceptions and
closed before a Phase 8C compute launch. A quantized `.nnue` must not be called
the full-precision continuation checkpoint.

## Decision and next experiment

Root was provisionally selected over leaf before the later diversity audit.
That historical choice remains reasonable as a root-over-leaf resource decision:
root has the better fixed-control and handcrafted point estimates, retains 8.5%
more accepted records, and avoids the opening-dependent leaf filtering that did
not buy Elo.

This remains a comparison of the models actually trained, but neither lane is
a valid 262k unique-position learning-curve point. The root export is a
repeated-trajectory historical control, not the scale control for repaired
Phase 8C.

Phase 8R later measured this root against handcrafted at equal combined nodes
and found `-36.78 Elo [-51.30, -22.26]`, proving an evaluation-quality deficit.
That diagnosis remains valid for this trained model and does not depend on the
incorrect dataset-cardinality description.

The subsequent Phase 8C GR2 audit showed that deterministic depth-1 rollout
replayed the same small trajectory set. The next experiment is therefore a
searched-stochastic trajectory repair followed by a genuinely unique 262k
root test. Only a successful repaired 262k model becomes the scale control for
fresh-seed 1M training. This Phase 8B root remains a direct historical strength
comparator, not the denominator of a unique-data learning curve.

The preserved local evidence is under:

- `out/anhoku-v0.6-phase8b-root-262k/artifacts/`
- `out/anhoku-v0.6-phase8b-leaf-262k/artifacts/`
- `out/anhoku-v0.6-phase8b-evaluation/root-vs-phase7-c16/`
- `out/anhoku-v0.6-phase8b-evaluation/leaf-vs-phase7-c16/`
