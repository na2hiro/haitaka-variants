# haitaka_wasm

`haitaka_wasm` is the browser-facing engine layer for Haitaka Variants.

It wraps the core `haitaka` board and search code with `wasm-bindgen` so web
clients can run search, iterative deepening, perft, DFPN, and NNUE loading.

## What This Crate Exposes

- `search(sfen, depth)`
- `search_iterative_deepening(sfen, max_depth, timeout_ms)`
- `perft(sfen, depth)`
- `dfpn(sfen, max_nodes, max_time_ms, tt_megabytes, max_pv_moves)`
- `load_nnue(bytes)`
- `load_sfnn(nn_bytes, progress_bytes, arch)`
- `UsiEngine`
  - `new()`
  - `send(line)`
  - `load_nnue(bytes)`
  - `load_sfnn(nn_bytes, progress_bytes, arch)`

The native Rust side also exposes hidden helper functions used by `haitaka_cli`
and tests.

`UsiEngine` speaks USI command strings through JavaScript calls. It is intended
for browser and Worker transports where stdin/stdout are not available. The
initial API is synchronous: `send("go ...")` returns after search completes.

## TuyouraOu11 micro

`load_sfnn(nn_bytes, progress_bytes, arch)` installs the micro evaluator in the
browser runtime; `UsiEngine.load_sfnn` installs it only in that engine instance.
Both return `SFNN_ka2_2048_8_64_progress8` after validating the two files and clear
their own search TT on success. Failed loads preserve the previous evaluator/TT.
The existing `load_nnue` API still handles HalfKAv2 models.

Native `sfnn::SfnnModel` exposes full-refresh and incremental evaluation. See
[format, provenance, build and verification steps](../docs/tuyouraou11-sfnn.md).
The optional `sfnn-diagnostics` feature is for verification only; omit it from
production packages. Micro is accepted only by standard-shogi builds.

## Build For Browser Use

Install prerequisites:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

Build standard shogi:

```bash
wasm-pack build haitaka_wasm --target web --out-dir pkg --release
```

Build Annan shogi:

```bash
wasm-pack build haitaka_wasm --target web --out-dir pkg --release --features annan
```

Build an NNUE runtime with the compact 256-value transformer width:

```bash
wasm-pack build haitaka_wasm --target web --out-dir pkg --release \
  --features anhoku,nnue-l1-256
```

`nnue-l1-256` changes only the compile-time transformer width (L1) from 512
to 256. Feature geometry, hidden-layer geometry, and the NNUE file version stay
unchanged. The width-derived network and section hashes are intentionally
different, so the runtime only accepts models serialized with the matching L1.
The selected values are available to native consumers as
`NNUE_TRANSFORMER_WIDTH`, `NNUE_TRANSFORMER_OUTPUT_WIDTH`, and
`NNUE_OUTPUT_LAYER_HASH`.

The generated package is written to:

```text
haitaka_wasm/pkg
```

That directory is the expected input for `haitaka_cli package`, which copies
the manifest-declared JS and WASM artifacts into a Shogitter Engine Package v1
archive.

## Build For Shogitter Engine Package v1

From the repository root, run the workspace `xtask` helper directly with:

```bash
cargo xtask package
```

For Annan:

```bash
cargo xtask package --ruleset annan
```

The repository also defines shorter package aliases:

```bash
cargo pack
```

For Annan:

```bash
cargo pack-annan
```

Additional variant aliases follow the same pattern, such as
`cargo pack-taimen` and `cargo pack-nekoneko`.

For manual debugging, run the steps directly:

```bash
wasm-pack build haitaka_wasm --target web --out-dir pkg --release
cargo run -p haitaka_cli --release -- package \
  --wasm-dir haitaka_wasm/pkg \
  --output target/haitaka-variants.tgz
```

For Annan:

```bash
wasm-pack build haitaka_wasm --target web --out-dir pkg --release --features annan
cargo run -p haitaka_cli --release --features annan -- package \
  --wasm-dir haitaka_wasm/pkg \
  --ruleset annan \
  --rule-id 26 \
  --output target/haitaka-variants-annan.tgz
```

The generated archive contains a root `shogitter-engine.json` manifest plus the
declared `engine/haitaka_wasm.js` and `engine/haitaka_wasm_bg.wasm` artifacts.
For package metadata details, see
[docs/shogitter-package.md](docs/shogitter-package.md).

## Tests And Benchmarks

```bash
cargo test -p haitaka_wasm
cargo test -p haitaka_wasm --features annan
cargo test -p haitaka_wasm --features anhoku,nnue-l1-256
cargo bench -p haitaka_wasm --bench nnue -- --noplot
```

NNUE affine inference selects AVX2 or NEON once when a model is loaded and
otherwise keeps the scalar kernel. WASM uses scalar inference by default; enable
SIMD128 explicitly for compatible deployment targets:

```bash
RUSTFLAGS="-C target-feature=+simd128" \
  wasm-pack build haitaka_wasm --target web --out-dir pkg --release
```

The NNUE Criterion benchmark always measures synthetic dense-layer shapes. Set
`HAITAKA_NNUE_BENCH_MODEL` to include full-refresh, incremental-state, and
search measurements using a real model:

```bash
HAITAKA_NNUE_BENCH_MODEL=/path/to/model.nnue \
  cargo bench -p haitaka_wasm --features anhoku --bench nnue -- --noplot
```

## Notes

- NNUE loading is currently intended for standard shogi.
- Annan search uses Annan-specific rule logic from `haitaka`.
- Use matching feature flags when building WASM and generating packages.
