# Anhoku v0.5 / v0.5.1 corrected NNUE selection

This records the corrected Vast.ai run completed on 2026-08-16. The run was
started from Haitaka commit `18e60f95306ada22ac974ff42668e79a5494ab8a` and
trainer commit `2388a9bb7bf7004eee3954ee72ff4d407a1bc1bd`.

## Instance and elapsed-time reference

The Vast.ai offer was captured at 2026-08-16 11:50 JST after the runs had
finished. These values are useful as a baseline when comparing future offers:

| Item | Value |
| --- | --- |
| Instance / host / machine | `47768870` / `209938` / `56948` |
| Verification / reliability | Verified / 99.74% |
| Price | $0.070/hour, no savings plan |
| GPU | 1x GeForce RTX 3060, 12 GB VRAM, 12.0 TFLOPS |
| CUDA / DLPerf | Max CUDA 13.1, DLPerf 12.2, 174.4 DLPerf/$/hour |
| GPU memory bandwidth | 314.7 GB/s |
| CPU | Xeon E5-2686 v4, 36 cores / 36 CPUs |
| RAM | 64.1 GB total; about 16 GB in use in the snapshot |
| Disk | 80 GB NVMe, advertised 2358 MB/s |
| PCIe | 3.0 x16, advertised 10.5 GB/s |
| Network | advertised 727.2 Mbps up / 843.4 Mbps down |
| Image | `vastai/pytorch_cuda-13.1.2-auto/jupyter` |
| Persistent volume | None |

All times below are JST and reconstructed from the preserved Lightning,
self-play, verification, and disk-monitor timestamps. They are approximate at
phase boundaries but sufficient for offer and cost planning.

| Work | Start | Finish | Elapsed |
| --- | --- | --- | ---: |
| Original v0.5.1 train/select using the former selector | Aug 15 20:43 | Aug 16 00:48 | about 4 h 05 m |
| Corrected v0.5.1 re-ranking and benchmark | Aug 16 06:45 | Aug 16 07:14 | about 29 m |
| v0.5 fresh GPU training, 60 epochs | Aug 16 07:14 | Aug 16 07:59 | about 44 m |
| v0.5 anchored ranking and benchmark | Aug 16 08:01 | Aug 16 09:42 | about 1 h 41 m |
| Entire corrected supervised sequence | Aug 16 06:45 | Aug 16 09:42 | about 2 h 57 m |
| Final 3.28 GB archive download to the local machine | Aug 16 10:31 | Aug 16 10:50 | 18 m 54 s |

At the listed hourly rate, the corrected three-hour sequence cost roughly
$0.21. The UI showed about 20 hours of instance age at capture, or roughly
$1.40 at the flat rate; that larger total includes setup, transfers, diagnosis,
code changes, tests, and the superseded run. The archive transfer averaged only
2.75 MB/s despite the much higher network figure advertised for the offer, so
transfer time should be budgeted from observed end-to-end throughput rather
than the offer number alone.

The corrected v0.5 GPU phase took less than one third of its complete run. The
CPU-bound 32,768-game ranking was the longer phase and used 34 self-play
threads. For this workflow, CPU capacity and per-core performance materially
affect total cost; choosing a faster GPU alone may save little. The 80 GB disk
was adequate with `storage_saver = true`: the final audit showed 43 GB used and
38 GB available even while the 3.28 GB handoff archive remained on the
instance. A similar 80 GB offer remains a reasonable minimum, while extra disk
provides margin for non-storage-saver runs or multiple retained checkpoint
sets.

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
