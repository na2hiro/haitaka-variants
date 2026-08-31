# Anhoku NNUE v0.7 Phase 11-A: DonorReceiverPairV2

## Decision

Phase 11-A passes. Phase 11-B is authorized, but no GPU training or strength
match was started in this assignment.

`DonorReceiverPairV2` replaces the V1 `DonorSingleEff` relation for the
experimental Anhoku family. Each influenced receiver contributes exactly one
sparse relation feature indexed by:

1. oriented receiver square;
2. donor color relative to the accumulator perspective;
3. receiver native piece type;
4. effective donor piece type.

The stable composite feature-set hash is `0xb38efce1`; the resulting runtime
network hash is `0xd0bd8e2b`. V1 remains the recommended/default Anhoku family
until the controlled Phase 11-B experiment passes.

## Implementation

The runtime recognizes both the historical V1 family and the Anhoku-only V2
family. Full refresh and incremental updates use the same donor enumeration and
the existing affected-square set. The trainer overlay contains the matching
16,200-row donor block and compiled C++ index anchors shared with the runtime
tests.

The `migrate-donor-receiver-pair-v2` command performs a byte-level quantized
migration. It preserves the base transformer, biases, PSQT values, and bucket
networks, and copies every V1 effective-type row into all ten receiver-native
slices. Configured V2 training performs this migration before importing a V1
bootstrap into the trainer, so the Phase 11-B lanes can start from functionally
identical C/16 evaluations.

The implementation config is
`haitaka_learn.anhoku-v0.7-phase11a.toml`. It permits V2 only for Anhoku and
does not change the V1 default.

## Equivalence and resource gates

The release-mode gate used the preserved C/16 network:

| Item | V1 | Migrated V2 |
| --- | ---: | ---: |
| Feature family | `HalfKAv2^+DonorSingleEff` | `HalfKAv2^+DonorReceiverPairV2` |
| Real features | 152,523 | 167,103 |
| Model bytes | 161,206,531 | 176,603,011 |
| SHA-256 | `049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0` | `5c67b70d97e89bef28029d06a69dab1da185e32ace1387569bf844f314df03fa` |
| Median full-refresh ns/position | 3,636.41 | 3,638.86 |

The real-feature increase is 9.56%, actual exported model growth is 9.55%, and
the measured representative inference regression is 0.07%. These pass the
respective 10% and 5% stop boundaries.

Equivalence passed with zero mismatches in:

- six fixed representative positions;
- 16 deterministic randomized games containing 754 positions/transitions;
- full-refresh versus incremental accumulators;
- V1 versus migrated-V2 scores and accumulators;
- six deterministic fixed-depth searches.

The migrated artifact was serialized, loaded as the V2 family, and rejected by
the migration path when the source was malformed or had the wrong family.

## Trainer/runtime parity and tactical suite

The trainer parity command temporarily installed the overlay, compiled the C++
data loader, restored the trainer checkout, and validated the Python block.
Trainer revision was
`61666d9e3653e4df9881b14c23f8fdcc4bf7779b`. Python reported the expected
16,200-row donor block and stable hash, while C++ and Rust shared index anchors
`[6520, 6601, 6682, 8140]`.

The veto-only suite is `scripts/phase11a-tactical-suite-v1.json`, SHA-256
`d0343f3583d16d996b5d3ef83eb5113a3cafebdfde0cff01d71d6ed09f41ab9d`.
It was frozen before V2 training and all six fixtures passed for both V1 and
migrated V2 with identical best moves and scores. It may veto Phase 11-B
candidates but must not select checkpoints or tune feature geometry.

Machine-readable evidence:

- `out/anhoku-v0.7-phase11a/artifacts/phase11a-gate.json`
- `out/anhoku-v0.7-phase11a/artifacts/trainer-feature-parity.json`
- `out/anhoku-v0.7-phase11a/artifacts/c16-donor-receiver-pair-v2.nnue`

## Reproduction

```sh
cargo test -p haitaka_wasm --features anhoku
cargo test -p haitaka_learn --features anhoku
cargo build --release -p haitaka_learn --features anhoku
target/release/haitaka_learn phase11a-gate \
  --input out/anhoku-v0.6-phase7.1-preserved/lane-c-step-16.nnue \
  --output-nnue out/anhoku-v0.7-phase11a/artifacts/c16-donor-receiver-pair-v2.nnue \
  --tactical-suite scripts/phase11a-tactical-suite-v1.json \
  --report out/anhoku-v0.7-phase11a/artifacts/phase11a-gate.json
target/debug/haitaka_learn verify-donor-receiver-pair-v2-trainer \
  --config haitaka_learn.anhoku-v0.7-phase11a.toml \
  --output out/anhoku-v0.7-phase11a/artifacts/trainer-feature-parity.json
```

The next assignment is Phase 11-B exactly as written in the execution plan.
