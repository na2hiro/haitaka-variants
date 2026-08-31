# Anhoku NNUE v0.7 Phase 11-C learnability and quantization audit

## Decision

**`EXPRESSED_NOT_RETAINED`; authorize Phase 12-A only.** DonorReceiverPairV2
did learn receiver-native/effective-donor interactions, and a small fraction
survived export quantization into runtime-visible PSQT differences. Removing
only that interaction changes evaluations on the frozen train, OOD-v2, and
Phase 11-B replay corpora and changes deterministic depth-2 search results.
The feature therefore reached runtime behavior, but it already failed the
completed 4,096-game Phase 11-B strength protocol. It remains retired as a
strength hypothesis.

This was a local CPU-only audit. It took no optimizer step, generated or
relabeled no position, ran no self-play or strength game, changed no network
candidate, and used no Vast instance. The collapsed network is an audit-only
counterfactual and must never enter checkpoint selection or play.

## Frozen inputs and reproducibility

The audit ran from source descendant `ac757f83286b99927359cd86a1c5ef3dc29166a3`
of the required Phase 11-B implementation. It verified every frozen identity
before analysis, including trainer revision
`61666d9e3653e4df9881b14c23f8fdcc4bf7779b`, the reviewed and applied trainer
patches, both step-16 checkpoints and NNUEs, both corpora, the tactical suite,
and both Phase 11-B archives. It also verified the two replay reports as
separate 1,024/3,072-game batches with seeds 1180/12180, V2 as engine A, V1 as
engine B, and the frozen 100 ms/opening protocol.

The checked-in Python helper used explicit
`torch.load(..., weights_only=False)` in the isolated trainer environment. Its
stable binary intermediate has SHA-256
`49eac109b8ea8c9f434c6d0ca086b5b325db7a3ad8f09e70e7ebdc700b18eedd`.
Rust then proved that quantizing all 16,200 checkpoint rows reproduces every
exported transformer and PSQT integer. This establishes checkpoint/runtime row
index parity rather than assuming it.

The audit-only collapsed V2 network is 176,603,039 bytes with SHA-256
`4687f49e2b8d914c941dab99cecd4d3e1623f5f7bc6bd75aef227a415fbacb13`.
For each fixed `(oriented receiver square, relative donor color, effective
donor type)` group, it takes the float32 arithmetic mean across the ten native
slices and applies the production serializer's round-to-even scales once
(`127` for transformer weights and `9600` for PSQT). Only relation-row bytes
are replaced; all other V2 parameters remain byte-identical. The result reloads
as `HalfKAv2^+DonorReceiverPairV2`.

## Coverage

Every one of the 279,627 train records and 3,218 OOD-v2 records was scanned for
both accumulator perspectives. Distinct-board counts reproduced the frozen
276,949 train and 3,215 OOD-v2 identities.

| Measure | Train | OOD-v2 |
| --- | ---: | ---: |
| Relation activations, record-weighted | 4,372,488 | 53,470 |
| Observed relation rows | 11,368 | 2,024 |
| Coverage of all 16,200 rows | 70.17% | 12.49% |
| Coverage of 14,256 structurally reachable rows | 79.74% | 14.20% |

There are 1,944 structurally unreachable rows: front-edge receiver rows plus
the impossible same-color king-receiver/king-donor conjunction, with their
overlap counted once. Train left 2,888 reachable rows unseen. OOD-v2 contributed
34 rows not seen in train, while train contributed 9,378 rows not seen in OOD.

Coverage scarcity exists but is not the deciding classification. Only 0.229%
of train activations landed in rows with fewer than 8 train occurrences and
1.702% landed in rows with fewer than 32. Across train plus OOD-v2 those rates
were 0.235% and 1.705%. The machine report retains fixed-bin histograms and
grouped record/distinct-board coverage for square, color, native type, and
effective type.

## Learned separation and quantization

Exactly 1,386 of 1,620 collapsible groups had train coverage; the same 1,386
groups developed nonzero full-precision slice separation, while all 234
zero-coverage groups remained exactly tied. Across transformer and PSQT
dimensions, 28,444,268 nonzero full-precision pairwise slice differences were
measured.

Quantization erased almost all of them, but not all: 626 pairwise differences
survived, a ratio of `0.00002201` (0.00220%). The survivors cover 60 PSQT
dimensions in 50 groups. No transformer-weight dimension retained a slice
difference. This is a prominent low-survival warning, but the written route is
binary: any surviving quantized difference is sufficient evidence that the
interaction was expressed at runtime.

The JSON contains per-group transformer and PSQT dispersion in their separate
float/integer domains, including zero/nonzero counts, L1, L2/RMS, maximum and
percentiles. It also contains unweighted and train-occurrence-weighted summaries
while retaining all 234 zero-coverage groups. Raw float and integer magnitudes
are never compared as if they shared units.

## Evaluation and search sensitivity

Original-minus-collapsed V2 deltas were always at most one native evaluation
point, but were nonzero on every required corpus:

| Corpus | Distinct positions | Nonzero score deltas | Zero-delta rate | Absolute mean / RMS / max |
| --- | ---: | ---: | ---: | ---: |
| Train | 276,949 | 14,481 | 94.771% | 0.0523 / 0.2287 / 1 |
| OOD-v2 | 3,215 | 128 | 96.019% | 0.0398 / 0.1995 / 1 |
| Phase 11-B replay | 108,973 | 10,605 | 90.268% | 0.0973 / 0.3120 / 1 |

The report separately includes V1-minus-V2 signed and absolute distributions
for the same three splits. Both Phase 11-B JSONL batches reconstructed cleanly
to exactly 4,096 games; malformed/illegal moves, duplicate batch identities,
or incomplete game indices would have stopped the audit.

The 1,024 search positions were the distinct replay positions with the smallest
SHA-256 of canonical packed-board bytes. Selection SHA-256 is
`f5d754390a7753901732b003800e4efb13b4c7ed7481b967aa0c616c8e601dae`.
At deterministic depth 2, original and collapsed V2 differed on 3 best moves
and 84 best scores. This is sensitivity evidence only, not Elo or checkpoint
selection. Both networks passed all six unchanged tactical fixtures.

## Artifacts and route

- Machine audit: `out/anhoku-v0.7-phase11c/artifacts/phase11c-audit.json`,
  SHA-256 `728c517764c28adaeed35baf3a755f873871b70ea27f4f7c6dcde61345cd80f3`
- Audit-only collapsed network: `out/anhoku-v0.7-phase11c/artifacts/collapsed-v2-audit-only.nnue`
- Stable search selection: `out/anhoku-v0.7-phase11c/artifacts/replay-selection-1024.json`,
  SHA-256 `a7114442486177ed16d05885a47486787c01c777a183f5568f8de55a8b3da641`
- Stable full-precision intermediate: `out/anhoku-v0.7-phase11c/artifacts/v2-full-precision-relation-v1.bin`

The exact route is **Phase 12-A, evaluation-error attribution launch gate**.
That route is authorized but was not executed here. Phase 12-B, another feature,
new training, new data, and any new strength or handcrafted match remain
unauthorized.
