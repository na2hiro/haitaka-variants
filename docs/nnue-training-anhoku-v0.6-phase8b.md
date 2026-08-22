# Anhoku NNUE v0.6 Phase 8B distributed data handoff

The Phase 8B 262k-target data configs are versioned in this checkout. Every
machine must use the same `strengthen` revision and a non-overlapping shard
range. The four-lane partition is:

| Machine | Shard |
| --- | --- |
| A | `1/4` |
| B | `2/4` |
| C | `3/4` |
| D | `4/4` |

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

Replace `1/4` with `2/4`, `3/4`, or `4/4` on the other machines. Do not use
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
