# Anhoku v0.5 / v0.5.1 corrected NNUE selection

This records the corrected Vast.ai run completed on 2026-08-16. The run was
started from Haitaka commit `18e60f95306ada22ac974ff42668e79a5494ab8a` and
trainer commit `2388a9bb7bf7004eee3954ee72ff4d407a1bc1bd`.

## Why the models were reselected

The former selector made the first exported checkpoint the incumbent and ran a
separate SPRT for each later checkpoint. A candidate that reached the game cap
without crossing an SPRT boundary was recorded as inconclusive and could not
replace the incumbent, even when its point estimate was clearly stronger. This
made epoch 0 appear to win both runs.

The corrected selector uses the first unique NNUE only as a fixed zero-Elo
anchor. It imports or generates complete color-swapped pairs, records the five
pentanomial pair-score bins, and ranks every unique NNUE by paired Elo. After an
initial screen, additional batches are allocated using
`rating + 1.5 * standard_error`. Training and selection no longer compete for
CPU: checkpoints are exported while the trainer runs, and self-play starts
after the trainer exits.

## Corrected results

### v0.5.1 recovery

- Config: `haitaka_learn.anhoku-v0.5.1.toml`
- Imported candidates: 60 unique NNUEs (61 stored aliases)
- Fresh ranking games: 8,192
- Selected checkpoint: `epoch=6-step=434.ckpt`
- NNUE SHA-256: `1c6ffefb34fe53137d33c3ccd5668dc507c4b11e4841cf6c6670167a4d26380f`
- Fixed-anchor result: +61.13 Elo, 95% CI [+49.02, +73.24]
- Handcrafted benchmark: -110.08 Elo, 95% CI [-132.29, -87.87]
- Output: `out/anhoku-v0.5.1/artifacts/haitaka-anhoku-v0.5.1.reselected.nnue`

### v0.5 rerun

- Config: `haitaka_learn.anhoku-v0.5.toml`
- Training: epoch 0 through 59 from a fresh initialization
- Ranked candidates: 60 unique NNUEs
- Fresh ranking games: 32,768
- Selected checkpoint: `epoch=3-step=248.ckpt`
- NNUE SHA-256: `3514402ef07205eb3a848128f1eb5486b92cfdfc2c5baee83b0ac5d3876bd3bd`
- Fixed-anchor result: +67.84 Elo, 95% CI [+56.62, +79.05]
- Handcrafted benchmark: -178.40 Elo, 95% CI [-201.57, -155.24]
- Output: `out/anhoku-v0.5-rerun/artifacts/haitaka-anhoku-v0.5.rerun.nnue`

Both rankings ended as `budget-limited`: the highest point estimate was
exported, but its lower confidence bound did not exceed every other candidate's
upper bound. Both exported NNUEs passed the 14-position verification suite. The
handcrafted matches are report-only and never override NNUE checkpoint choice.

## Preserved evidence

`out/` is intentionally git-ignored. Before destroying the Vast instance, the
following were copied locally and hash-checked:

- original and corrected final NNUEs;
- `ranking.json`, export metadata, verification, and handcrafted reports;
- all v0.5.1 legacy candidates and 960 legacy match batches;
- corrected ranking matches and v0.5 rerun candidates;
- Lightning/TensorBoard logs and training logs;
- the original v0.5 and v0.5.1 datasets.

The reusable workflow and recovery command are documented in
`docs/vast-ai-nnue-training.md`. Vast supervisor definitions used for this run
are under `scripts/vast/`.
