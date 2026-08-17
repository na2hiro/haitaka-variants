# Anhoku NNUE v0.6 Phase 3: Deterministic Shuffle And Grouped Validation

Phase 3 of the [handcrafted-strength plan](../plans/anhoku-nnue-handcrafted-strength-plan.md)
changes dataset ordering and leakage control only. It does not change the 72-byte
training ABI, opening generation, search labels, or trainer behavior.

## Split Contract

The production and smoke configs select `opening-group-hash-v1`. Before generating a
game, Haitaka hashes every stable opening ID with `split_seed`, ranks the IDs, and
assigns a validation share. Both splits receive at least one ID; suites with four or
more IDs reserve at least two validation groups. The remaining IDs belong to train.

Game selection is restricted to the assigned IDs. Consequently all repetitions of an
opening, including both members of an Anhoku base/color-swapped pair, remain in one
split. Per-game metadata now has a split-qualified `game_id`. Final and shard manifests
store the policy/version, seed, and complete train/validation ID lists. Resume and
merge compare all of this identity before accepting a shard.

The historical `independent-legacy` policy remains available and is written explicitly
in pre-v0.6 checked-in configs.

## Shuffle Contract And Memory Bound

`bounded-chunk-v1` streams the raw game-order shard records into temporary chunks of
`shuffle_chunk_records` records. Each chunk is shuffled by a PRNG derived from the
shuffle seed, split name, and chunk index. A seeded affine permutation with a step
coprime to the chunk count determines chunk output order without retaining an array of
chunk paths. The final output is therefore deterministic across job counts and merge
locations, while differing from game-order concatenation.

Only one record chunk and two fixed 64 KiB I/O buffers coexist. Excluding allocator and
short path metadata, the enforced algorithmic heap bound is:

```text
shuffle_chunk_records * 72 + 131072 bytes
```

The default 65,536-record chunk uses 4,849,664 bytes (about 4.63 MiB). Validation caps
the setting at 1,000,000 records, whose bound is 72,131,072 bytes (about 68.79 MiB).
The final manifest records the configured chunk size and calculated bound.

## Audit And Compatibility

Dataset audit JSON now includes split/shuffle identity and a `groups` object containing
game counts, unique qualified game IDs, assigned opening-group counts, and the exact
train/validation opening-ID intersection. A grouped v0.6 audit must report zero overlap.

Focused tests cover byte-identical repeated generation, output order differing from raw
shard order, disjoint game/opening IDs, the calculated memory bound, audit overlap, and
resume/merge rejection after split or shuffle identity mutation.

## Generation Smoke Result

Two clean release-mode runs of `haitaka_learn.anhoku-v0.6.smoke.toml` produced the same
files:

- train: 83 records, SHA-256
  `2907c007118f14116e8a8e14f59581d5b0b9c86d89ce2aa065e7b5ddf9e044c3`
- validation: 77 records, SHA-256
  `870afada2c51c317baaaa7baa32b05646016a901a30ff85e802208db0a624dc7`

The equal-size smoke split assigned six of the 12 suite IDs to each side. Its audit
reported four unique qualified game IDs, six assigned train groups, six assigned
validation groups, and zero opening-group overlap. With 16-record chunks, the manifest
reported a 132,224-byte shuffle buffer bound, and no temporary chunk directory remained
after assembly.
