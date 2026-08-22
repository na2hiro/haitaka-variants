# Anhoku NNUE Phase 7.1 Early-Training Diagnostic Plan

- Status: ready for the next experiment agent
- Created: 2026-08-21
- Parent plan: [Anhoku NNUE Handcrafted-Strength Execution Plan](anhoku-nnue-handcrafted-strength-plan.md)
- Parent result: [Anhoku NNUE v0.6 corrected-data baseline](../docs/nnue-training-anhoku-v0.6-1m-data.md)
- Detailed diagnosis: `out/anhoku-v0.6/diagnostics/epoch0-analysis/report.html`
- Required worktree: `/home/na2hiro/proj/haitaka-variants-anhoku-phase7`
- Required branch: `strengthen-phase-3`
- Starting Haitaka commit: `1c251f5001da1edf07034185ba1a2202a93ccc4f`

## Agent Assignment

Execute one short, controlled Phase 7.1 experiment that identifies why Anhoku
v0.6 peaks at its first saved checkpoint. Do not start Phase 8 and do not make
an architecture change in this assignment.

The previous Vast instance was destroyed. Its old SSH endpoint is invalid and
must not be reused. All inputs needed for Phase 7.1 are preserved locally, but
a new GPU host must be provisioned for training.

Use only the worktree and branch listed above. The user's current
`strengthen` branch is separate work and must remain untouched.

## Decision To Make

Determine which explanation best fits the early regression:

1. the original initial learning rate is too large for the v0.6 distribution;
2. fresh initialization is the problem, while the corrected v0.5.1 weights can
   be fine-tuned safely;
3. the `lambda = 0.8` score/result mixture contributes materially to the
   regression; or
4. the v0.6 depth-1 rollout state distribution and depth-3 labels are harmful
   even with a low learning rate and a strong warm start.

End with one explicit recommendation:

- approve one recipe for a Phase 8 control only after the parity gate passes;
- require a stronger-rollout data pilot before Phase 8; or
- record the result as inconclusive and name the smallest additional test.

## Why Phase 7.1 Exists

Phase 7 saved one checkpoint per nominal one-million-position epoch. With batch
size 16,384, epoch 0 was actually saved after 62 optimizer steps and 1,015,808
accepted positions. It was not an untrained model.

All three Phase 7 seeds had their minimum validation loss at epoch 0. Every one
of the 177 later checkpoint Elo point estimates was at or below the epoch-0
anchor. The selected v0.6 models then lost a median 90.55 Elo to corrected
v0.5.1 and 233.81 Elo to handcrafted at 100 ms. The selector therefore did not
merely choose the wrong checkpoint; later training reduced measured strength.

The current validation set is also an unsuitable sole early-stopping set. It
contains only two held-out opening IDs, has a 0% draw rate, a 10.3% mate-score
rate, and mean absolute score 4,446. The train set contains ten opening IDs,
has a 26.4% draw rate, a 6.9% mate-score rate, and mean absolute score 3,232.
Keep this validation set as a legacy out-of-distribution diagnostic, not as the
primary selector.

The v0.6 train records also have only 58.0% agreement between the sign of the
depth-3 score and the final decisive result. This does not by itself prove bad
labels, but it makes the rollout/target and lambda hypotheses worth separating.

## Preserved Inputs

Do not regenerate the Phase 7 train data before this diagnostic.

| Input | Local path | Required identity |
| --- | --- | --- |
| Complete v0.6 merged train | `out/anhoku-v0.6/datasets/train.bin` | SHA-256 `e101d179c96115f65fa886c1b0cf13b38a25ac815930c1f9217d531edf1db8e4` |
| Legacy OOD validation | `out/anhoku-v0.6/datasets/validation.bin` | SHA-256 `405d8a72a29cdf1e67539c9ac75440774475054c60c4c43c7a04600437fe9021` |
| Complete shard archive | `out/anhoku-v0.6/input-archives/anhoku-v0.6-mac.tgz` | verify and record before use |
| Additional shard archive | `out/anhoku-v0.6/input-archives/anhoku-v0.6-78.tgz` | verify and record before use |
| Corrected v0.5.1 anchor | `out/anhoku-v0.6/baselines/v0.5.1/haitaka-anhoku-v0.5.1.reselected.nnue` | SHA-256 `1c6ffefb34fe53137d33c3ccd5668dc507c4b11e4841cf6c6670167a4d26380f` |
| Phase 7 configs | `out/anhoku-v0.6/training-configs/` | preserve unchanged |
| Phase 7 checkpoint rankings | `out/anhoku-v0.6-seed{1,2,3}/artifacts/selection/ranking.json` | reference only |

The `mac` archive appears to contain the complete shard set. Before relying on
it, accept only paths matching `shard-[0-9]{6}.{bin,json}`, reject duplicate
numeric shard IDs, ignore AppleDouble/xattr entries, and verify exactly 2,500
train shard pairs and 250 validation shard pairs. If that gate fails, reconcile
the two archives by shard ID and manifest identity; do not silently prefer one
duplicate.

All reused shards must match the Phase 7 config identity
`e4e879a255a1ad4d20b665001b7c9e434c7d089ce2813de0ad0cdf3e311f553c`
apart from the already documented null engine revision in the `-78` lane. Any
other mismatch stops the experiment.

## Experimental Matrix

Use trainer seed 1 for all four lanes so only the named treatment changes.
Use the same feature set, batch size, data order, validation files, checkpoint
steps, and evaluation openings in every lane.

| Lane | Initialization | Initial LR | Lambda | Question answered |
| --- | --- | ---: | ---: | --- |
| A | fresh | 0.0015 | 0.8 | Reproduce the Phase 7 early trajectory at fine granularity |
| B | fresh | 0.0003 | 0.8 | Test whether the original LR is too large |
| C | corrected v0.5.1 warm start | 0.00015 | 0.8 | Test whether v0.6 gradients destroy known strength |
| D | corrected v0.5.1 warm start | 0.00015 | 1.0 | Remove the final-result term and test target conflict |

For lambda, retain the trainer's existing convention: `1.0` means pure search
score and `0.0` means pure final result. Lane D therefore removes the 20%
final-result component used by lambda 0.8.

## Invariants

The following values must not differ between lanes unless the matrix says so:

- feature set: `HalfKAv2^+DonorSingleEff`;
- train data and deterministic record order;
- ID and OOD validation data;
- batch size: 16,384;
- random FEN skipping: 3;
- trainer seed: 1;
- no resume from a prior lane;
- one GPU and one lane at a time;
- validation size: 100,000 nominal positions;
- fixed checkpoint and validation steps;
- 100 ms paired-match settings and opening pairs;
- no architecture, quantization, SIMD, or search change.

Do not lower the batch size merely to obtain exactly 25,000 positions between
checkpoints. That would confound checkpoint granularity with optimization.

## Checkpoint Granularity And Training Budget

Preserve the Phase 7 nominal epoch size of 1,000,000 positions, but stop within
the first epoch:

- `max_steps = 16`;
- run validation every two optimizer steps;
- save a checkpoint every two optimizer steps;
- expected saved steps: 2, 4, 6, 8, 10, 12, 14, and 16;
- each interval is 32,768 accepted positions at batch size 16,384;
- the final checkpoint is approximately 262,144 accepted positions.

This is the nearest clean implementation of the proposed 25k/250k pilot while
preserving the Phase 7 batch size. Record nominal and actual accepted positions
for every checkpoint. Do not label step 2 as exactly 25k.

Because the run stops before the end of the original one-million-position
epoch, the epoch-level StepLR scheduler must not decay the LR during this pilot.
Assert the actual optimizer LR at every checkpoint. An accidental scheduler
step every 32,768 positions invalidates the lane.

For warm-start lanes, the unmodified corrected v0.5.1 NNUE is the step-0
strength anchor. Do not overwrite it or count import/conversion as training.

## Required Preflight Implementation

The Phase 7 trainer hard-coded LR 0.0015 and saved only at epoch boundaries.
Before renting a long-running host or launching all lanes, make the smallest
reproducible trainer change that provides:

- an explicit initial-learning-rate argument, defaulting to 0.0015;
- step-based checkpointing with a configured interval of two optimizer steps;
- validation every two optimizer steps;
- `max_steps = 16` termination;
- LR logging at each validation/checkpoint;
- loading a bootstrap `.nnue` through the existing trusted conversion path;
- offline loss evaluation of an arbitrary checkpoint against both ID and OOD
  validation binaries.

Defaults for existing configs must remain unchanged. The trainer change should
be committed in the custom trainer repository, or preserved as a reviewed patch
plus exact base revision if a commit is impossible. Record both Haitaka and
trainer revisions in every lane manifest.

The external trainer used by Phase 7 recorded revision
`2388a9bb7bf7004eee3954ee72ff4d407a1bc1bd`. Reproduce from that revision if it
is available. If it is unavailable, document the resolved base commit and
diff it against the Phase 7 behavior before running the matrix.

Preflight tests:

1. CLI parsing preserves the old LR and checkpoint behavior when new flags are
   omitted.
2. A two-step smoke run saves a valid checkpoint at step 2.
3. The saved checkpoint exports to NNUE and passes the 14-position verifier.
4. A warm-start smoke run records the corrected v0.5.1 hash and applies the
   requested LR rather than a serialized optimizer LR.
5. Offline ID/OOD evaluation is deterministic for one fixed checkpoint.
6. TensorBoard or equivalent logs include train loss, ID validation loss,
   OOD validation loss, optimizer step, accepted positions, and LR.

Do not begin lanes B-D until lane A step 2 has passed these checks.

## Build An In-Distribution Validation Set

The primary validation set must come from the Phase 7 train-opening domain,
while remaining disjoint from the training records used by all four lanes.

Build it from whole Phase 7 train shards, not by cutting records out of the
already shuffled final `train.bin`:

1. Extract and validate the complete train shard set from the preserved archive.
2. Treat one shard as an indivisible group. Each shard contains 20 games, or
   ten color-swapped game pairs.
3. Select exactly 250 shard IDs with a checked-in deterministic hash rule and a
   new fixed split seed. This is the ID-validation set.
4. Use the remaining 2,250 shards for the Phase 7.1 train file.
5. Deterministically shuffle each assembled record file with a new recorded
   shuffle seed and the existing bounded-memory policy.
6. Verify zero shard/game-pair overlap and include all ten Phase 7 train opening
   IDs in both files.
7. Emit manifests, audits, hashes, selected shard IDs, and the assembly script.

The exact split and shuffle seeds may be chosen once by the agent, but must be
written into the plan result and never changed after looking at model metrics.

ID-validation gates before training:

- draw-rate difference from train is at most 5 percentage points;
- mate-score-rate relative difference is at most 20%;
- mean-absolute-score relative difference is at most 20%;
- decisive score/result-agreement difference is at most 5 percentage points;
- all ten train opening IDs are represented;
- no held-out shard contributes records to the Phase 7.1 train file;
- record counts are multiples of the 72-byte ABI and hashes are stable over two
  independent assemblies.

If a gate fails, change only the deterministic stratification algorithm, state
the reason, and regenerate before any model is trained. Never tune the split
after observing lane results.

## OOD Validation Policy

Retain the existing Phase 7 validation binary as `legacy-ood-validation.bin`.
It contains opening IDs 010 and 011 and is useful for direct comparison with
Phase 7, but it must not select a checkpoint or tune LR.

Report its loss for every checkpoint as a secondary metric. Explicitly label it
as a two-opening OOD diagnostic.

Before Phase 8, expand the reviewed Anhoku opening suite to at least 64 opening
IDs and reserve at least 12 IDs for an OOD-v2 validation set. That suite work is
not required to launch the four optimizer lanes and must not silently change
their data. Phase 7.1 may prepare OOD-v2 after the lane result, but legacy OOD
performance alone cannot approve Phase 8.

## Run Procedure

### 1. Local preparation

- Confirm branch `strengthen-phase-3` and starting commit `1c251f5`.
- Preserve a clean diff before implementation.
- Verify all input hashes and archive identities.
- Build the ID train/validation files and pass every data gate.
- Create four reviewed TOMLs under a Phase 7.1-specific name.
- Give every lane a separate output directory; never reuse Phase 7 logs.
- Bundle code, configs, train/ID/OOD data, the v0.5.1 anchor, and hashes for the
  new host.

Suggested output layout:

```text
out/anhoku-v0.6-phase7.1/
  datasets/
    train.bin
    id-validation.bin
    legacy-ood-validation.bin
    *.json
    selected-id-validation-shards.txt
  lane-a/
  lane-b/
  lane-c/
  lane-d/
  matches/
  artifacts/
```

Suggested config names:

```text
haitaka_learn.anhoku-v0.6-phase7.1-a.toml
haitaka_learn.anhoku-v0.6-phase7.1-b.toml
haitaka_learn.anhoku-v0.6-phase7.1-c.toml
haitaka_learn.anhoku-v0.6-phase7.1-d.toml
```

### 2. New host preflight

Use a host with one CUDA GPU with at least 12 GiB VRAM, at least 16 CPU threads,
and at least 100 GiB free disk. Follow
[`docs/vast-ai-nnue-training.md`](../docs/vast-ai-nnue-training.md), including
the known Python 3.12, NumPy 1.26.4, CUDA-compatible CuPy, Lightning 1.9.5, and
setuptools-below-81 constraints from Phase 7.

Before full execution:

- record GPU, driver, CPU, disk, Python, PyTorch, CUDA, CuPy, Cargo, Haitaka,
  and trainer revisions;
- run a real CUDA kernel;
- verify the train/ID/OOD and anchor hashes after transfer;
- run lane A through step 2, export it, and pass NNUE verification;
- confirm checkpoint cleanup cannot delete an unexported or unhashed model.

### 3. Train sequentially

Run A, B, C, and D sequentially on the same idle GPU. Do not overlap CPU
self-play with GPU training. For every saved step:

- preserve checkpoint path and SHA-256 until export succeeds;
- export an NNUE immediately and record its SHA-256;
- run the 14-position verifier;
- record train loss, ID loss, legacy OOD loss, LR, optimizer step, nominal
  positions, and actual accepted positions;
- preserve failures and reruns with explicit exclusion labels.

After export and hash verification, large superseded `.ckpt` files may be
deleted to protect disk space. Keep all NNUE exports and checkpoint-to-export
mappings.

## Strength Screening

Validation loss alone does not decide this phase. Compare exported checkpoints
against the fixed corrected v0.5.1 NNUE at 100 ms.

Use the same color-swapped opening pairs and one fixed opening seed for all
lanes and checkpoints. Preserve per-game JSONL and pentanomial pair counts.

1. Screen every exported checkpoint with at least 32 color-swapped pairs
   (64 games) against the fixed v0.5.1 anchor.
2. Within each lane, extend the two best point estimates that also have
   competitive ID loss to at least 256 paired games.
3. Extend the overall best candidate sequentially until one of these occurs:
   - its 95% CI lower bound is greater than -10 Elo;
   - its 95% CI upper bound is below -10 Elo; or
   - 4,096 games have completed.
4. Run a 1,024-game handcrafted comparison only if the v0.5.1 non-inferiority
   gate passes.

The 64-game screen ranks candidates but cannot establish parity. Do not report
its winner as a promoted model.

Do not use the existing per-lane epoch-0 anchored selector as the final Phase
7.1 decision. The required anchor is the unchanged corrected v0.5.1 NNUE, not
the first checkpoint from each lane.

## Interpretation Rules

Use these rules before seeing the results:

- If B moves the loss/strength peak later and clearly outperforms A, the
  original LR is a material contributor.
- If C loses strength immediately while ID loss improves, the v0.6 data
  gradient is harmful even from a known-strong initialization.
- If D is materially more stable or stronger than C, the final-result component
  contributes to the regression.
- If C and D both degrade similarly, especially when D uses pure score targets,
  prioritize the depth-1 rollout/state-distribution hypothesis over lambda.
- If C remains near its step-0 anchor while fresh lanes remain weak, prefer
  warm-start fine-tuning and treat initialization/optimization as the main
  issue.
- If ID loss and paired strength disagree, paired strength is the decision
  metric; investigate the loss calibration rather than selecting by ID loss.
- Legacy OOD loss may describe generalization behavior but cannot override the
  fixed-anchor strength result.

Do not claim that the v0.5.1 score/result agreement of 80.4% is a clean target.
It is partly inflated by the known legacy sampled-move coupling bias.

## Phase 8 Gate

Phase 8 remains blocked unless one Phase 7.1 candidate satisfies all of the
following:

- ID train/validation distribution gates passed before training;
- at least one checkpoint improves ID loss without a tactical or verification
  regression;
- its 100 ms paired-strength 95% CI lower bound against corrected v0.5.1 is
  greater than -10 Elo;
- the result is not dependent on the two-opening legacy OOD selector;
- all lane changes and non-teacher invariants are verified by config and hashes;
- an OOD-v2 suite plan with at least 64 IDs and at least 12 held-out IDs is
  ready before the Phase 8 result is interpreted.

If no lane passes, do not add more seeds to the same recipe. Generate a small
new pilot with depth-2 or fixed-node rollout, keep depth-3 observational labels
initially, and repeat the strongest warm-start lane before spending on Phase 8.

## Required Result Artifacts

Write `docs/nnue-training-anhoku-v0.6-phase7.1.md` containing:

- answer-first conclusion and Phase 8 decision;
- full four-lane config matrix and revisions;
- train, ID-validation, and legacy-OOD audits and hashes;
- checkpoint table with step, actual positions, LR, train loss, ID loss, OOD
  loss, NNUE hash, and verifier status;
- validation-loss curves for all lanes;
- every fixed-anchor screen and extension result, not only winners;
- paired Elo, 95% CI, W/L/D, pentanomial counts, games, nodes, NPS, and timing;
- failed launches, exclusions, reruns, and resource usage;
- explicit evaluation of each hypothesis and the next smallest experiment.

Also preserve under `out/anhoku-v0.6-phase7.1/`:

- full TOMLs and expanded trainer argv;
- source and trainer patches/commits;
- environment manifest;
- shard-selection list and assembly script;
- dataset binaries, manifests, audits, and SHA-256 files;
- TensorBoard event files;
- all NNUE exports and checkpoint mappings;
- verification JSON;
- per-game match JSONL and aggregate reports;
- a compact transfer archive for local recovery.

## Acceptance Checklist

- [ ] Work ran on `strengthen-phase-3`, not `strengthen`.
- [ ] Old destroyed Vast endpoint was not treated as available state.
- [ ] Archive/shard identity and counts were verified.
- [ ] Deterministic ID split passed all distribution and leakage gates.
- [ ] Legacy OOD was retained but excluded from checkpoint selection.
- [ ] LR and step-checkpoint support preserved old defaults.
- [ ] Lane A step-2 smoke passed before B-D launched.
- [ ] A-D completed sequentially with seed 1 and all invariants checked.
- [ ] Steps 2-16 were exported, hashed, evaluated, and verified per lane.
- [ ] Every checkpoint received a fixed-v0.5.1 strength screen.
- [ ] The best candidate was extended to a CI decision or the game cap.
- [ ] Handcrafted was tested only after v0.5.1 parity passed.
- [ ] The result document evaluates all four hypotheses.
- [ ] Phase 8 was explicitly approved or blocked against the written gate.
- [ ] `cargo fmt`, focused tests, relevant trainer smoke tests, and
  `git diff --check` passed for tracked changes.

## Out Of Scope

- Phase 8 fixed-node/qsearch-leaf generation;
- another three-seed or 60-epoch run;
- 10M data generation;
- NNUE feature or layer changes;
- SIMD/runtime optimization;
- default-model promotion;
- selecting a model solely from validation loss;
- silently changing openings, search, batch size, or data policy between lanes.
