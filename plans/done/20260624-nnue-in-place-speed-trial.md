## NNUE In-Place Accumulator Speed Trial

### Summary

This trial tested whether Anhoku NNUE search speed could be improved by avoiding
per-child `NnuePositionState` copies and by removing unnecessary handcrafted
mobility work from NNUE leaf evaluation.

The result was negative. The attempted code was correct under tests, but matched
self-play showed a clear speed regression. The code should not be kept as-is.
The useful outcome is the measurement: reversible in-place accumulator updates
cost more than copying the accumulator in the current search architecture.

### Trialed Changes

- Search recursion carried `Option<&mut NnuePositionState>` instead of
  `Option<NnuePositionState>`.
- Each child applied NNUE feature deltas before recursion and unapplied them
  after recursion.
- King moves and terminal positions kept full-refresh fallback behavior.
- Donor-single delta update avoided `DonorFeatureBuffer` diffing and compared
  parent/child `Option<usize>` donor features directly.
- NNUE leaf evaluation used `board.generate_moves(|_| true)` for mate/no-move
  detection instead of counting full legal mobility.
- Criterion benchmark loading was extended with
  `HAITAKA_NNUE_BENCH_MODEL=/path/to/model.nnue`.

### Validation

Correctness checks passed:

```bash
cargo test -p haitaka_wasm
cargo test -p haitaka_wasm --features anhoku
git diff --check
```

Criterion with the Anhoku model confirmed incremental eval is much faster than
full refresh, but fixed-depth search was still much slower than handcrafted:

```bash
HAITAKA_NNUE_BENCH_MODEL=/Users/na2hiro/proj/shogitter/haitaka-anhoku-v0.4-epoch-018.nnue \
  cargo bench -p haitaka_wasm --features anhoku --bench nnue -- --noplot
```

Observed representative numbers:

- `nnue_eval/full_refresh`: about 17 us
- `nnue_eval/incremental_state`: about 3.34 us
- `nnue_search/startpos_d4_handcrafted`: about 15.3 ms
- `nnue_search/startpos_d4_incremental`: about 64.0 ms
- `nnue_search/startpos_d4_full_refresh`: about 127 ms

### Matched Self-Play Comparison

The valid comparison used:

- clean `HEAD` archive as the "without changes" baseline
- the same checked-in `Cargo.lock` copied into the baseline checkout
- current dirty tree as the "current changes" build
- same model path, seed, opening randomization, movetime, and thread count

Command:

```bash
cargo run -p haitaka_cli --release --features anhoku -- self-play \
  --games 20 \
  --threads 1 \
  --movetime-ms 100 \
  --a-eval nnue \
  --b-eval handcrafted \
  --nnue /Users/na2hiro/proj/shogitter/haitaka-anhoku-v0.4-epoch-018.nnue \
  --report-dir /tmp/haitaka-nnue-speed-... \
  --opening-random-plies 4 \
  --seed 1
```

Results:

| Metric | Without changes | Current changes | Diff |
|---|---:|---:|---:|
| Aggregate NPS | 266,598 | 156,998 | -41.11% |
| NNUE NPS | 214,875 | 114,606 | -46.66% |
| Handcrafted NPS | 321,961 | 201,241 | -37.50% |
| Total nodes | 19,973,238 | 10,956,886 | -45.14% |
| NNUE nodes | 8,322,782 | 4,084,543 | -50.92% |
| Handcrafted nodes | 11,650,456 | 6,872,343 | -41.01% |
| Avg plies | 41.9 | 38.8 | -7.40% |

### Important Caveat

An earlier baseline run from `/tmp` generated a fresh `Cargo.lock` because the
archive did not include the workspace lockfile. That produced misleading numbers.
For performance comparisons from a clean archive, always copy the original
`Cargo.lock` into the archive checkout before building.

### Conclusion

Do not replace accumulator copies with apply/unapply in this form.

The likely reason is simple: each searched edge now pays both apply and unapply
feature-delta costs, including donor-delta work, while the old path paid one
copy of a small fixed accumulator. In this codebase, that tradeoff is worse.

Useful lessons for future work:

- Keep the copy-based incremental state unless a benchmark proves a new approach
  wins.
- Optimize dense inference first; `evaluate_from_state` still costs about
  3.34 us per call.
- Consider SIMD/vectorized affine layers before more accumulator plumbing.
- Consider search-level improvements such as transposition tables, move
  ordering, or reduced leaf eval calls; the NNUE itself is not the only cost.
- If trying in-place state again, use an explicit stack of preallocated states
  or per-ply scratch buffers instead of reversible apply/unapply on every edge.

