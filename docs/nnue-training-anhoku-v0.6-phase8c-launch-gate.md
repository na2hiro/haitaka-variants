# Anhoku NNUE v0.6 Phase 8C launch gate

Status: **PASS — pre-generation gate only** (2026-08-25). Production 1M data
generation, multi-machine shard work, training, and Phase 8C strength matches
were not started in this assignment.

The machine-readable closeout is
[`phase8c-launch-gate.json`](../out/anhoku-v0.6-phase8c-launch-gate/phase8c-launch-gate.json).
The runner is [`scripts/phase8_launch_gate.py`](../scripts/phase8_launch_gate.py)
and never invokes `generate-data` or `merge-data`.

## Gate decision

| Gate item | Result | Evidence |
| --- | --- | --- |
| Phase 8B closeout and reuse decision | Pass | [Phase 8B result](nnue-training-anhoku-v0.6-phase8b.md); completed matches reused; rental time and price remain unrecovered |
| C/16 and selected exports preserved and hashed | Pass | Artifact table below |
| Selected step-16 checkpoints recovered or loss recorded | Pass | Both local `.ckpt` files are present and hashed below |
| Versioned verifier | Pass | 14 verification fixtures and legal depth-2 search smoke passed for C/16, root, and leaf |
| Versioned tactical suite | Pass | Six Anhoku fixtures passed for all three models; suite SHA-256 is `e5d6764ea2361c0d8219c5dfe4b56420085e542d1c900e4f265fa0721eb18449` |
| Stratified OOD-v2 contract | Pass | 12 reserved IDs, 16 color-swapped pairs per ID, 384 validation games, excluded from train |
| Accepted-record target | Pass | Config requires and generation/merge enforce at least `1,048,576` accepted train records |
| Phase 8C experiment identity | Pass | Frozen data/training config SHA-256 is `e4471c3ff1a14f113dfd0bb78cfcf0c5268d819cd1292aff1a967a2339fea58a` |

The source checkout used for the gate was `719c3dd236952d918937e6c0365256efae31f735`
with a dirty worktree because the Phase 8R and launch-gate changes are still
uncommitted. Before any machine starts generation, freeze these changes in one
source revision and record that revision on every machine.

## Immutable Phase 8B closeout

The recorded Phase 8B audit table, dataset identities, CPU-hours, unique
counts, candidate identities, protocol exceptions, and completed match results
remain the closeout. The actual Phase 8B dataset hashes were rechecked:

| Artifact | SHA-256 |
| --- | --- |
| C/16 model | `049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0` |
| Phase 8B root train | `96c2281288ef6b49d7e53ef53de99b462202b3cf7fd812943b69c2b55c8bee48` |
| Phase 8B root validation | `66c84ef3e28317fa7ae944630c6f93b3cdb73113c12478ec2b88ee9314204879` |
| Phase 8B leaf train | `b65bb7be6bffa98e4a316d3e51ec68d4cc28e60f44ff7465afd26972292498f7` |
| Phase 8B leaf validation | `4bc532d47dbaa06b4a6c67dfe1791650489379d694c0b1b7a60af76a7afd36bc` |

The selected model and checkpoint artifacts are:

| Artifact | Path | SHA-256 |
| --- | --- | --- |
| C/16 | `out/anhoku-v0.6-phase7.1-preserved/lane-c-step-16.nnue` | `049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0` |
| Phase 8B root export | `out/anhoku-v0.6-phase8b-root-262k/artifacts/haitaka-anhoku-v0.6-phase8b-root-262k.nnue` | `12865f59f28f6e26feffcfae2e76c576f8eb31891148a8a9c167b8b50aac972c` |
| Phase 8B leaf export | `out/anhoku-v0.6-phase8b-leaf-262k/artifacts/haitaka-anhoku-v0.6-phase8b-leaf-262k.nnue` | `80ae84ce6aabc58036cef07905d801473f6f913b09d109fbc61dba55f8138d28` |
| Phase 8B root step 16 | `out/anhoku-v0.6-phase8b-root-262k/logs/lightning_logs/version_0/checkpoints/epoch=0-step=16.ckpt` | `49b9f67495c9a4b207a2c6de37c7dc2617ba7782c53825dc52d6edf3eefe471f` |
| Phase 8B leaf step 16 | `out/anhoku-v0.6-phase8b-leaf-262k/logs/lightning_logs/version_0/checkpoints/epoch=0-step=16.ckpt` | `32eccbc8d4de92658beaf5969cb6543cf3ea72e7e05bf1b337dc7f47ccbe0dfe` |

Both selected full-precision checkpoints were recovered locally. The
quantized `.nnue` exports and `.ckpt` files are recorded as separate artifacts;
neither export is being described as a full-precision checkpoint. Phase 8B
trainer rental time and cost were not preserved, so they remain explicitly
unknown and are not estimated here.

## Verifier and tactical evidence

The existing 14-position `haitaka_learn verify` contract passed for all three
models, including a legal depth-2 search smoke move (`6g6f`). The new
versioned tactical suite contains six Anhoku positions covering a capture,
promotions, a checking drop, check evasion, and a complex donor-rule threat.
The C/16 expected moves were stable for both Phase 8B exports at their
fixture-declared depths. No loss value was used as a selector or as a veto.

The raw verifier reports and gate JSON are preserved under
`out/anhoku-v0.6-phase8c-launch-gate/verifier/` and
`out/anhoku-v0.6-phase8c-launch-gate/`.

## Frozen stratified OOD-v2 contract

The data config is
[`haitaka_learn.anhoku-v0.6-phase8c-root-1m.data.toml`](../haitaka_learn.anhoku-v0.6-phase8c-root-1m.data.toml).
It changes the validation schedule from the Phase 8B hash sampling to
`equal-color-swapped-pairs-v1`:

- reserved IDs: `anhoku-v2-053` through `anhoku-v2-064`;
- exactly 16 pairs per ID, with one base and one rotated/color-swapped game per
  pair;
- 192 pairs / 384 games total;
- validation records never enter the production train split;
- later audit must report all 12 per-opening losses and the unweighted
  macro-average, plus aggregate counts and incomplete-label rejection rate.

The schedule is implemented in the opening selector and included in shard and
final manifest identity. A changed schedule or pair count is rejected by
resume and merge identity checks.

## Frozen root-only 1M config

The config pins the following identities:

| Setting | Value |
| --- | --- |
| Root games / validation games | `16,600` / `384` |
| Attempted roots per train game | `64` |
| Minimum accepted train records | `1,048,576` |
| Teacher | `50,000` combined alpha-beta + qsearch nodes, depth cap `64`, `alpha-beta-plus-qsearch-v1` |
| Position policy | `root-position` |
| Rollout | `uniform-rollout-v1`, depth `1` |
| Warm start | C/16 SHA-256 above |
| Feature family | `HalfKAv2^+DonorSingleEff` |
| Learning rate / lambda | `0.00015` / `0.8` |
| Training epoch size | `1,048,576` |
| Checkpoint / validation cadence | steps `16, 32, 48, 64` (approximately 262k, 524k, 786k, 1M examples) |
| Data shards | 10 games per shard; resumable |

The accepted-record minimum is a generation contract, not merely a trainer
`epoch_size`. A complete local generation and a merged dataset both fail before
training when the train manifest contains fewer than the minimum.

## Predeclared Phase 8C experiment

The three fresh training seeds are `80`, `81`, and `82`. Each starts from the
immutable C/16 bootstrap and uses the same data config, feature family,
learning rate, lambda, batch size, epoch size, and checkpoint schedule. Seed 80
in the old Phase 8B 262k pilot is not one of these 1M replications.

For all later strength work, record the exact current executable hash, model
hash, config hash, and opening-suite hash before starting. The opening suite is
`haitaka_learn/openings/anhoku-v2.tsv` with SHA-256
`bc576bbe57c05b8b2112b416c1907845d38d5e087e8e3b71b44c19c4e1593307`.

Predeclared match streams use the Anhoku start SFEN, four deterministic random
opening plies, and no concurrent generation or training:

| Use | Games | Seed | Budget |
| --- | ---: | ---: | --- |
| Per-seed C/16 screen | 64 paired games per checkpoint | `8300` | 100 ms/move |
| Best checkpoint per seed extension | 1,024 paired games | `8301` | 100 ms/move |
| Direct overall 1M vs Phase 8B root scale test | sequentially 1,024 to at most 4,096 paired games | `8302` | 100 ms/move |
| Fresh fixed-time handcrafted diagnostic | exactly 2,048 paired games | `8303` | 100 ms/move |
| Fresh equal-node handcrafted diagnostic | exactly 2,048 paired games | `8304` | 100,000 combined nodes/move |

The direct scale test succeeds when its lower 95% bound is above `0 Elo` and
fails when its upper 95% bound is below `+5 Elo`; otherwise it is capped
inconclusive at 4,096 games. The per-seed reproducibility screen requires a
positive median paired Elo, at least two seeds with lower 95% bounds above
`-10 Elo`, and no tactical, verifier, OOD-v2, or fixed-time NPS regression.
Only one checkpoint per seed may continue past the 64-game screen. The two
2,048-game handcrafted diagnostics are run only after those gates pass, and
their outcomes are not used to select the 1M winner.

The equal-node diagnostic preserves the Phase 8R counting version and full
telemetry. If either evaluator's fallback rate exceeds `0.1%`, retain the full
predeclared result as primary and add the clean-pair sensitivity analysis.

Resource ceilings are:

- at most four local CPU generation lanes and `240` aggregate generation
  CPU-hours;
- no GPU rental or GPU training before the merged train and OOD-v2 manifests
  pass their audits;
- at most `4` GPU-hours for the three 1M fits after the data gate;
- at most `160` aggregate engine-hours for the predeclared screens and matches.

Reaching a ceiling stops the relevant assignment and records a capped or
inconclusive result; it does not authorize an unplanned extension.

## Multi-machine generation handoff (not run here)

After the source revision and config hash are frozen, use four machines with
the following contiguous shard ranges. The config has 1,660 train shards and
39 validation shards; the same selector range applies to both splits.

| Machine | `--shard-index` | `--shard-index-end` | Train shards | Validation shards |
| --- | ---: | ---: | ---: | ---: |
| A | 0 | 414 | 0–414 | 0–8 |
| B | 415 | 829 | 415–829 | 9–18 |
| C | 830 | 1244 | 830–1244 | 19–28 |
| D | 1245 | 1659 | 1245–1659 | 29–38 |

On each machine, first record the frozen source commit, clean/dirty status,
config SHA-256, opening-suite SHA-256, and C/16 SHA-256. Then run the following
command with that machine's range:

```bash
cargo run --release -p haitaka_learn --features anhoku -- generate-data \
  --config haitaka_learn.anhoku-v0.6-phase8c-root-1m.data.toml \
  --shard-index <START> --shard-index-end <END> --shard-count 4
```

Do not use `--ignore-identity-mismatch`. Copy each complete output directory
back to one coordinator under a distinct machine name, preserving every
`datasets/shards` file. The coordinator then merges only after all expected
ranges are present:

```bash
cargo run --release -p haitaka_learn --features anhoku -- merge-data \
  --config haitaka_learn.anhoku-v0.6-phase8c-root-1m.data.toml \
  --input out/anhoku-v0.6-phase8c-root-1m-machine-a \
  --input out/anhoku-v0.6-phase8c-root-1m-machine-b \
  --input out/anhoku-v0.6-phase8c-root-1m-machine-c \
  --input out/anhoku-v0.6-phase8c-root-1m-machine-d
```

The coordinator must audit the merged train and validation binaries and
manifests, verify exactly 12 OOD IDs with exactly 16 pairs each, verify the
accepted train count and incomplete-label rate, run `check-matched`-style
identity checks without an override, and retain all per-machine shards until
those hashes and counts are recorded. This is the next assignment; it was not
started by the launch-gate agent.
