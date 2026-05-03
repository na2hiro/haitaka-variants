# Haitaka Variants

Haitaka Variantsは、変則将棋AIやその機械学習に関するリポジトリです！これは`haitaka`というモダンで高速な本将棋の合法手生成エンジンを以下の点で拡張したものです。

- 現在は、安南、安北、安東西といった役割変化系のルールに対応しています。 ([supported-rules.md](./docs/supported-rules.md))
  - bitboardを使用した高速な合法手生成ができ、手元のM4 Mac上におけるperftの計算では約1億NPS出ます。
  - ルールごとの拡張に当たってRustのfeature機能を使っているため、使用されないルール個別のコードはコンパイル時に取り除かれるため、拡張によるオーバーヘッドが少なくなっています。
- 簡単な反復深化alphabeta探索による思考
  - WebAssembly (WASM)にコンパイルしてブラウザ上で動かした場合、数十万NPS (ノード毎秒)読め、これだけで５秒以内に絞っても深さ５とか６まで読めるので安南初心者からすると結構強いAIになっています。
  - 現状、手書きの非常にナイーブな（合法手の数と駒得のみを用いた）評価関数となっています。
- NNUEモデルの学習と評価
  - CUDAを用いてNNUEモデル(Fairy Stockfish互換)の学習を行うパイプラインを用意してあります。
  - 上記alphabeta思考の際にNNUEモデルによる評価を行うことができます。
- [将棋ったー](https://shogitter.com)上でのAI対局
  - 「将棋ったーAI」を対局相手として選択し、対局できるようになっています。
  - 個人的なビルドやNNUEモデルなどはマイページよりエンジン登録をすることにより、自分で対局相手として選択することができるようになります。
  - （今後は、学習したNNUEモデルを将棋ったー上で共有し、他のプレイヤーが対戦できるような環境を整える予定です）

変則将棋AIに興味がある、盛り上げたいという方は、[CONTRIBUTING.md](./CONTRIBUTING.md)もご覧ください。

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
- [ROADMAP.md](ROADMAP.md): post-launch direction.
- [docs/benchmarks.md](docs/benchmarks.md): benchmark commands and reporting.
- [docs/models.md](docs/models.md): model registry expectations.
- [docs/shogitter-package.md](docs/shogitter-package.md): package layout for
  Shogitter.

## What Works Now

- Standard shogi legal move generation.
- Piece-influence variant support for Annan, Anhoku, and Antouzai.
- Perft, legal move generation, DFPN, and NNUE benchmark harnesses.
- DFPN mate search in the core engine.
- Browser-facing WASM search APIs.
- Local play/debug and self-play/rating commands.
- A `.tgz` Shogitter Engine Package v1 generator for `wasm-bindgen` engine
  artifacts.
- Local NNUE data generation, trainer orchestration, export, and verification. (not tested)

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

## Build WASM

You can build WebAssembly and run the engine in the browser. There's also a command for generating a package for Shogitter. 

See [haitaka_wasm/README.md](haitaka_wasm/README.md).

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
- [`docs/anhoku-nnue-training.md`](docs/anhoku-nnue-training.md)

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
