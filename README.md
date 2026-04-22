# haitaka-variants

`haitaka-variants` is an engine workspace for different shogi variants, forked from [`tofutofu/haitaka`](https://github.com/tofutofu/haitaka), which supports fast 9x9 shogi move generation.

- Feature flags for different shogi variants
  - `annan` for 安南
  - `anhoku` for 安北
  - `antouzai` for 安東西
  - Since these flags are compile-time options, there's zero overhead for switching logics for other variants
- a DFPN mate solver in the core engine
  - *Note: this is implemented with simple instructions to a coding agent. Needs more verification*
- a browser-facing WASM search layer
  - Simple alphabeta search with iterative deepening + DFPN prepass
  - When `.nnue` model is loaded, evaluate with Fairy-Stockfish-compatible NNUE instead
- a local NNUE data/training/export/verify pipeline

This repository is meant to be an engine workspace, not just a single move-generation crate.

## Workspace

- `haitaka`
  - core board representation, SFEN parsing, legal move generation, perft, and DFPN
- `haitaka_types`
  - shared types and feature-gated variant pieces/ranks support
- `haitaka_wasm`
  - `wasm-bindgen` wrapper exposing search, iterative deepening, perft, DFPN, and NNUE loading
- `haitaka_learn`
  - CLI for NNUE data generation, trainer orchestration, export, and verification

## Main Changes From Upstream `tofutofu/haitaka`

At a high level, the `wasm` branch adds:

- piece-influence variant support via mutually exclusive feature flags
  - `annan`: friendly piece behind the mover donates movement
  - `anhoku`: friendly piece in front of the mover donates movement
  - `antouzai`: friendly pieces immediately left and/or right of the mover donate movement; both sides combine as a union
  - move generation
  - legality checking
  - check detection
  - validation / zobrist support
- a standalone DFPN mate solver in the core crate
  - `Board::dfpn(...)`
  - `haitaka/examples/dfpn.rs`
  - `haitaka/benches/dfpn.rs`
- a new `haitaka_wasm` crate
  - fixed-depth search `search()`
  - engine-owned iterative deepening with timeout `search_iterative_deepening()`
    - root DFPN prepass for iterative search
  - `perft()` and direct `dfpn()` exports
  - Fairy-Stockfish-compatible NNUE loading `load_nnue()` and evaluation
  - incremental NNUE accumulator updates and native benches
- a new `haitaka_learn` crate
  - generate training data from Haitaka positions/search
  - call upstream [`variant-nnue-pytorch`](https://github.com/fairy-stockfish/variant-nnue-pytorch)
  - export `.nnue`
  - verify the exported net inside Haitaka
- supporting workspace changes in `Cargo.toml`, examples, benches, and shared types

The current `main..wasm` commit history is centered around:

- Annan support
- WASM build/search support
- search speed work
- NNUE support
- incremental NNUE updates
- `haitaka_learn`
- DFPN
- iterative deepening + DFPN integration
- stricter handling of illegal pawn-drop mate (`uchi-fuzume`)

## Quick Start

### Core engine

```bash
cargo test -p haitaka
cargo test -p haitaka --features annan
cargo test -p haitaka --features anhoku
cargo test -p haitaka --features antouzai
```

### WASM layer

```bash
cargo test -p haitaka_wasm
cargo test -p haitaka_wasm --features annan
cargo test -p haitaka_wasm --features anhoku
cargo test -p haitaka_wasm --features antouzai
```

### Perft and DFPN examples

```bash
cargo run -p haitaka --release --example perft -- 4
cargo run -p haitaka --release --example dfpn -- "8k/6G2/7B1/9/9/9/9/9/K8 b R 1"
```

### Benches

```bash
cargo bench -p haitaka --bench perft -- --noplot
cargo bench -p haitaka --bench dfpn -- --noplot
cargo bench -p haitaka_wasm --bench nnue -- --noplot
```

## Feature Flags

### `haitaka`

- `std`
- `qugiy` ([ref. about qugiy algorithm](https://yaneuraou.yaneu.com/2021/12/03/qugiys-jumpy-effect-code-complete-guide/))
- `annan`
- `anhoku`
- `antouzai`

The variant rule features are mutually exclusive.

### `haitaka_wasm`

- `annan`
- `anhoku`
- `antouzai`

### `haitaka_learn`

- `annan`
- `anhoku`
- `antouzai`

## NNUE Notes

- Standard shogi NNUE uses the same network layout as Fairy-Stockfish `HalfKAv2^`.
- `haitaka_wasm` can load external `.nnue` files and search with that evaluator.
  - You can find an example NNUE file for standard Shogi at [Fairy Stockfish's official site](https://fairy-stockfish.github.io/nnue/)
- Annan currently has a documented NNUE data-generation workflow in `haitaka_learn`.
- Anhoku and Antouzai are currently core-engine/search feature modes only; add NNUE rule IDs and data workflow entries before using them for training runs.

For training details, see:

- `haitaka_learn/README.md`
- `haitaka_learn.toml`

## Acknowledgments

This project still builds on the original `haitaka` design and on ideas/code structure from `cozy-chess`.

Relevant references:

- `tofutofu/haitaka`
- `analog-hors/cozy-chess`
- Fairy-Stockfish NNUE tooling and model layout
- `variant-nnue-pytorch`

## License

MIT. See `LICENSE`.
