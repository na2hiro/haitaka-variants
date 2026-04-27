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

The native Rust side also exposes hidden helper functions used by `haitaka_cli`
and tests.

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

The generated package is written to:

```text
haitaka_wasm/pkg
```

That directory is the expected input for `haitaka_cli package`, which copies
the manifest-declared JS and WASM artifacts into a Shogitter Engine Package v1
archive.

## Build For Shogitter Engine Package v1

From the repository root:

```bash
cargo pack
```

For Annan:

```bash
cargo pack-annan
```

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

See [`../docs/shogitter-package.md`](../docs/shogitter-package.md) for the
archive layout.

## Tests And Benchmarks

```bash
cargo test -p haitaka_wasm
cargo test -p haitaka_wasm --features annan
cargo bench -p haitaka_wasm --bench nnue -- --noplot
```

## Notes

- NNUE loading is currently intended for standard shogi.
- Annan search uses Annan-specific rule logic from `haitaka`.
- Use matching feature flags when building WASM and generating packages.
