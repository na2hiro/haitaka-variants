# Anhoku NNUE v0.6 Phase 6: Vectorized Inference

Phase 6 of the [handcrafted-strength plan](../plans/anhoku-nnue-handcrafted-strength-plan.md)
adds architecture-specific integer affine kernels without changing NNUE files,
network arithmetic, accumulator updates, or search behavior.

## Implementation

- The scalar affine kernel remains the portability path and correctness oracle.
- Generic x86-64 binaries select AVX2 once when each model layer is loaded.
- AArch64 binaries select NEON after runtime feature detection.
- `wasm32` builds select SIMD128 only when compiled with
  `-C target-feature=+simd128`; ordinary WASM builds retain scalar inference.
- AVX2, NEON, and SIMD128 widen `u8` activations and `i8` weights before
  multiplication. This remains exact over the full input domain and avoids
  saturating packed multiply-add intermediates.
- The copy-based incremental accumulator is unchanged.

The Criterion suite now covers the three real network shapes independently and
can force scalar inference for matched full-refresh and incremental comparisons.
Set `HAITAKA_NNUE_BENCH_MODEL` to the model under test.

## Correctness Evidence

The randomized kernel tests cover lengths around every vector boundary, the
full `u8`/`i8` boundary values, padding, and these donor-network shapes:

| Layer | Active inputs | Padded inputs | Outputs |
|---|---:|---:|---:|
| hidden 1 | 1,024 | 1,024 | 16 |
| hidden 2 | 16 | 32 | 32 |
| output | 32 | 32 | 1 |

The real-model test used:

- model: `out/anhoku-v0.5/artifacts/haitaka-anhoku-v0.5.nnue`;
- SHA-256: `e00ecf1cb85c08b6115709d5bbd8bb0e3f860e99af0960c271c594abdbe7975d`;
- feature family: `HalfKAv2^+DonorSingleEff` under `--features anhoku`.

All eight buckets and all three layers matched scalar output. Start position
and six-piece-handicap evaluations matched, and fixed-depth start-position
search returned identical moves and scores. AVX2 tests ran on the reference
host. Both scalar and SIMD128 WASM kernels ran under Node and matched. The NEON
path cross-compiles successfully; its target-generic parity tests remain to be
run on AArch64 hardware before an ARM release.

## Performance

Reference host:

- Intel Core i7-8700, x86-64 AVX2;
- Rust 1.94.1;
- release profile with fat LTO and one codegen unit;
- baseline commit `3841b5e5ef82836bcc2362b1b1469ca5bf798ff8`;
- identical `Cargo.lock` and real donor model in both builds.

Criterion medians from the five-position evaluation batch:

| Benchmark | Scalar baseline | AVX2 | Speedup |
|---|---:|---:|---:|
| full refresh | 46.19 us | 30.66 us | 1.51x |
| incremental state | 20.73 us | 4.36 us | 4.75x |

The first dense layer improved from approximately `3.92 us` to `0.618 us`
(`6.34x`). The smaller second and output layers also improved, though their
absolute contribution is much lower.

A three-run median start-position diagnostic with a 100 ms budget, incremental
evaluation, no DFPN, and depth cap 64 recorded:

| Backend | Median NPS |
|---|---:|
| scalar | 30,996 |
| AVX2 | 173,694 |
| speedup | **5.60x** |

These are diagnostic NPS values, not strength results. Phase 8 remains
responsible for the paired fixed-time comparison.

## Verification

Passed:

```text
cargo test --workspace --features anhoku
HAITAKA_NNUE_TEST_MODEL=/absolute/path/to/haitaka-anhoku-v0.5.nnue \
  cargo test --release -p haitaka_wasm --features anhoku \
  real_donor_model_affine_layers_and_evaluation_match_scalar -- --nocapture
HAITAKA_NNUE_TEST_MODEL=/absolute/path/to/haitaka-anhoku-v0.5.nnue \
  cargo test --release -p haitaka_wasm --features anhoku \
  phase6_100ms_nps_diagnostic -- --nocapture
cargo check -p haitaka_wasm --features anhoku --target aarch64-unknown-linux-gnu
wasm-pack test --node haitaka_wasm --features anhoku
RUSTFLAGS="-C target-feature=+simd128" \
  wasm-pack test --node haitaka_wasm --features anhoku
cargo fmt --all -- --check
git diff --check
```

Acceptance status:

- scalar/AVX2 and scalar/SIMD128 bit-exactness: passed;
- real donor model, evaluation, and fixed-depth search parity: passed;
- dense, full-refresh, and incremental Criterion coverage: passed;
- native incremental evaluation improvement of at least 1.5x: passed (`4.75x`);
- 100 ms NNUE NPS improvement of at least 1.5x: passed (`5.60x`);
- non-SIMD WASM scalar fallback: passed;
- NEON implementation and cross-compilation: passed; runtime parity awaits an
  AArch64 host.
