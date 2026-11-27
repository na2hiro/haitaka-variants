# Model Registry

## Adopted models (standard NNUE per rule)

These small king-free nets are the standard NNUE for their rules. They are
trained CPU-only by self-play; the training record is
[nnue-small-net-results.md](nnue-small-net-results.md).

All of them use the L1=256 transformer, so engines that load them must be built
with the rule feature plus `nnue-l1-256`, for example
`cargo build -p haitaka_cli --release --features annan,nnue-l1-256`. Each
file's header carries a per-rule feature hash, so a net only loads into the
engine of its own rule.

| Rule | Version | Model | Training run | Feature set | Bytes | SHA-256 | vs handcrafted (100 ms/move) |
|---|---|---|---|---|---:|---|---:|
| Anhoku | v1.8 | `anhoku-v1.8` | kf-all8 | KingFreeA+DonorSingleEff | 2,014,965 | `c7fcbebc9d87336f0b30fba2777ca84884132e215c2430f1f0b4bab6597f7e27` | +244 (1 s/move, sealed openings) |
| Annan | v1.4 | `annan-v1.4` | kf4 | KingFreeA+DonorSingleEff | 2,014,954 | `eb52ca73d01faf4fa480590e7ca9be00b6e6eba7074e96bbb297bad4e505d404` | +482 |
| Haimen | v1.4 | `haimen-v1.4` | kf4 | KingFreeA+DonorSingleEff | 2,014,955 | `3a854e9dd5e3d425e46b07246df1e2afeffb6494d7245200dbac4f2788dc71d7` | +476 |
| Taimen | v1.4 | `taimen-v1.4` | kf4 | KingFreeA+DonorSingleEff | 2,014,955 | `2f48a54cf6a224e5065f79d89c59d40cfa10488321604a856bae3bf4170359d8` | +454 |
| Antouzai | v1.4 | `antouzai-v1.4` | kf4 | KingFreeA+DonorPairSlots | 2,896,237 | `b23ac1c5453833610eba54a27751957d040d5f0e70a69048c87a731129409966` | +283 |
| Neko | v1.4 | `neko-v1.4` | kf4 | KingFreeA+DonorSingleEff | 2,014,953 | `7e459348583b930016e6e86a6e9dc3ee774c666cce1813891d0c070317f2099e` | +270 |
| Yokoneko | v1.4 | `yokoneko-v1.4` | kf4 | KingFreeA+DonorSingleEff | 2,014,957 | `590eff3ee18cd26923213890b1e04b1a387231ceab66e7ad3c52be4123915642` | +332 |
| Tenjiku | v1.4 | `tenjiku-v1.4` | kf4 | KingFreeA+DonorSingleEff | 2,014,956 | `e80faf92282bb79694d3a1480e04c0945aecc6c5476b8674e35e6eb73b35a59a` | +446 |

### Versions

Each adopted net has a version `v<format>.<generation>[.<retrain>]`, shown in
Shogitter as "Haitaka NNUE v1.4" and carried in the model id (`annan-v1.4`).

- **format**: the network's shape. `1` is KingFreeA+Donor with L1=256 and 8
  layer stacks. It goes up only when the engine build must change (new input
  features or layer sizes), so nets with the same format load into the same
  engine build of their rule.
- **generation**: the self-play generation the net was trained through (the
  training run `kf4` is generation 4). It goes up each time the training data
  is regenerated with the previous net as the teacher.
- **retrain** (optional): a retrain on the same data, for example with other
  settings or a fix, as in `v1.4.1`.

Anhoku's adopted net was trained on eight generations of data (`kf-all8`), so it
is v1.8.

The weights are not committed to this repository. They are served by
[Shogitter](https://shogitter.com) as `/engines/haitaka/<rule>/<model>-<sha256 prefix>/nnue.bin` (for example `annan-v1.4-eb52ca73d01f`),
next to a `provenance.json` that records the training data and the evaluation.
License: MIT, as for the rest of this repository.

Earlier large HalfKAv2-family nets (`docs/nnue-training-anhoku-v0.*`) are not
adopted; they only edged out the handcrafted evaluation.

## Sharing a model

For a receipt-gated workflow that trains, exports, checks native/WASM parity,
and evaluates a candidate before independent confirmation, see
[Strength-first NNUE workflow](nnue-strength-first-workflow.md).

When you've built a model, please reach out to me on [the official Discord server](https://discord.com/invite/CYRWPQMGXu), create an issue or any other way!

When sharing a model, I'd like to know:

- variant name
- NNUE architecture (in case you changed training code)
- training config
- dataset size
- hardware
- training duration
- validation result
- `haitaka_learn verify` output
- license (MIT is assumed, unless specified)
- download location (do not commit to the repository!)
