# Haitaka Variants

Haitaka Variants is an open engine workspace for building strong AI for shogi
variants.

Modern shogi AI is already stronger than top human players, but many variant
rules still have little or no serious engine support. Shogitter is a central
place where people play those variants. This repository is the engine side of
that effort: fast legal move generation, search, benchmarks, WASM delivery, and
NNUE training tools that can grow into stronger bots for Shogitter.

The first launch target is Annan shogi.

Current piece-influence rule modes:

- `annan`: a friendly piece behind the mover donates movement.
- `anhoku`: a friendly piece in front of the mover donates movement.
- `antouzai`: friendly pieces immediately left and/or right of the mover donate
  movement, combined as a union.

These variant rule features are compile-time options, so switching rule logic
does not add runtime overhead.

## Repository Overview

This repository is a Cargo workspace. The crates are intentionally split by
responsibility:

- [`haitaka`](haitaka/README.md): core board representation, SFEN parsing, legal
  move generation, perft examples, and DFPN mate search.
- [`haitaka_types`](haitaka_types/README.md): shared core types such as pieces,
  colors, squares, bitboards, moves, and slider helpers.
- [`haitaka_wasm`](haitaka_wasm/README.md): browser-facing `wasm-bindgen` layer
  exposing search, iterative deepening, perft, DFPN, and NNUE loading.
- [`haitaka_learn`](haitaka_learn/README.md): NNUE data generation, training
  orchestration, export, and verification.
- [`haitaka_cli`](haitaka_cli/README.md): local launch tools for play/debugging,
  engine self-play/rating checks, and Shogitter Engine Package v1 generation.

Supporting docs:

- [CONTRIBUTING.md](CONTRIBUTING.md): how different kinds of contributors can
  help.
- [PRELAUNCH.md](PRELAUNCH.md): launch-readiness checklist.
- [ROADMAP.md](ROADMAP.md): post-launch direction.
- [docs/benchmarks.md](docs/benchmarks.md): benchmark commands and reporting.
- [docs/shogitter-package.md](docs/shogitter-package.md): package layout for
  Shogitter.
- [docs/models.md](docs/models.md): model registry expectations.

## What Works Now

- Standard shogi legal move generation.
- Piece-influence variant support for Annan, Anhoku, and Antouzai.
- Perft, legal move generation, DFPN, and NNUE benchmark harnesses.
- DFPN mate search in the core engine.
- Browser-facing WASM search APIs.
- Local play/debug and self-play/rating commands.
- A `.tgz` Shogitter Engine Package v1 generator for `wasm-bindgen` engine
  artifacts.
- Local NNUE data generation, trainer orchestration, export, and verification.

## Quick Start

Run the launch-focused workspace tests:

```bash
cargo test --workspace
cargo test --workspace --features annan
```

Run additional variant-specific tests for crates that expose those feature
flags:

```bash
cargo test -p haitaka --features anhoku
cargo test -p haitaka_wasm --features anhoku
cargo test -p haitaka_learn --features anhoku

cargo test -p haitaka --features antouzai
cargo test -p haitaka_wasm --features antouzai
cargo test -p haitaka_learn --features antouzai
```

Ask the engine for one local move:

```bash
cargo run -p haitaka_cli -- play --human none --depth 3
cargo run -p haitaka_cli --features annan -- play --human none --depth 3
```

Run the core examples:

```bash
cargo run -p haitaka --release --example perft -- 4
cargo run -p haitaka --release --example dfpn -- "8k/6G2/7B1/9/9/9/9/9/K8 b R 1"
```

Run benchmark samples:

```bash
cargo bench -p haitaka --bench perft -- --noplot
cargo bench -p haitaka --bench dfpn -- --noplot
cargo bench -p haitaka_wasm --bench nnue -- --noplot
```

For more benchmark coverage, see [docs/benchmarks.md](docs/benchmarks.md).

## Build WASM For Shogitter

Install the WASM target and `wasm-pack`:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

Build the standard shogi WASM package:

```bash
wasm-pack build haitaka_wasm --target web --out-dir pkg --release
```

Build the Annan shogi WASM package:

```bash
wasm-pack build haitaka_wasm --target web --out-dir pkg --release --features annan
```

The output directory is `haitaka_wasm/pkg`.

## Make A Shogitter Engine Package v1

Create the standard package with one Cargo alias:

```bash
cargo pack
```

Create the Annan package with:

```bash
cargo pack-annan
```

These aliases run `wasm-pack build` and then `haitaka_cli package`. If you need
to debug the steps manually, build WASM first:

```bash
wasm-pack build haitaka_wasm --target web --out-dir pkg --release
```

Then create the `.tgz` package:

```bash
cargo run -p haitaka_cli --release -- package \
  --wasm-dir haitaka_wasm/pkg \
  --output target/haitaka-variants.tgz
```

The generated archive contains a root `shogitter-engine.json` manifest plus the
declared `engine/haitaka_wasm.js` and `engine/haitaka_wasm_bg.wasm` artifacts.
For package metadata details, see
[docs/shogitter-package.md](docs/shogitter-package.md).

## Feature Flags

### Core and Shared Types

- `std`: standard-library support in shared types.
- `qugiy`: alternative slider move implementation inherited from upstream.
- `annan`: friendly piece behind the mover donates movement.
- `anhoku`: friendly piece in front of the mover donates movement.
- `antouzai`: friendly pieces immediately left and/or right of the mover donate
  movement.

The variant rule features are mutually exclusive.

### `haitaka_wasm`

- `annan`
- `anhoku`
- `antouzai`

### `haitaka_learn`

- `annan`
- `anhoku`
- `antouzai`

### `haitaka_cli`

- `annan`

Use the same feature flag consistently across crates when working on a variant.

## NNUE Notes

- Standard shogi NNUE uses the same network layout as Fairy-Stockfish `HalfKAv2^`.
- `haitaka_wasm` can load external `.nnue` files and search with that evaluator.
  - You can find an example NNUE file for standard Shogi at [Fairy Stockfish's official site](https://fairy-stockfish.github.io/nnue/)
- `haitaka_learn` now supports standard, handicap, Annan, Anhoku, and Antouzai NNUE data generation / train / export / verify flows.
- Variant runs must use the matching feature build:
  - `--features annan`
  - `--features anhoku`
  - `--features antouzai`
- `haitaka_learn` now emits a concrete `rule_id` for built-in standard, handicap, Annan, Anhoku (`55`), and Antouzai (`95`) runs.
- `rules.rule_id` remains as an override when you need to match an external registry or when a custom handicap opening has no preset-based default.

For training details, see:

- [`haitaka_learn/README.md`](haitaka_learn/README.md)
- [`haitaka_learn.toml`](haitaka_learn.toml)

## Acknowledgments

This project still builds on the original `haitaka` design and on ideas/code
structure from `cozy-chess`.

Relevant references:

- [`tofutofu/haitaka`](https://github.com/tofutofu/haitaka)
- [`analog-hors/cozy-chess`](https://github.com/analog-hors/cozy-chess)
- Fairy-Stockfish NNUE tooling and model layout
- [`variant-nnue-pytorch`](https://github.com/fairy-stockfish/variant-nnue-pytorch)

## License

MIT. See [LICENSE](LICENSE).
