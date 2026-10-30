# Anhoku corrected-runtime checkpoint rescreen

This CPU-only development rescreen compares the two compact checkpoints selected in advance: update 1,920 for lowest validation loss and update 2,048 for highest scheduled game score. Both use the serializer-consistent runtime, the same 64 opening groups, 1,000 ms per move, one browser worker, and handcrafted as the opponent.

| Checkpoint | Games | W–L–D | Conservative score | Opening-group 95% interval |
| --- | ---: | ---: | ---: | ---: |
| u1920 | 128 | 67–61–0 | 52.3% | 43.8%–60.9% |
| u2048 | 128 | 76–52–0 | 59.4% | 51.6%–67.2% |

The opening-paired u2048 minus u1920 conservative score difference is +7.0 percentage points, with a 95% interval [-3.1, +17.2].

The frozen development rule selects **u2048**: u2048 has the higher conservative 1-second score on the frozen common openings.

Update 2,048 passed corrected full-precision/integer/native/WASM qualification on 1,024 positions with zero mismatches. Its corrected export quantization MAE is 18.44 score units.

This is checkpoint selection on reused development openings. Handcrafted remains the production recommendation; no promotion, deployment, training, or GPU use is authorized by this result.
