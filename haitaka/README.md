# haitaka

`haitaka` is the core engine crate inside the `haitaka-variants` workspace.

It started from upstream `tofutofu/haitaka`, but this fork now uses the crate
as the base for variant support, mate solving, WASM integration, and NNUE work.

## What This Crate Does

- board representation for 9x9 shogi
- SFEN parsing and formatting
- legal move generation
- perft tooling
- Zobrist hashing
- DFPN mate search
- optional piece-influence variants via mutually exclusive feature flags

This crate is the rules/movegen core used by:

- `haitaka_wasm`
- `haitaka_learn`
- `haitaka_cli`

## Features

- `std`
- `qugiy`
- `annan`
- `anhoku`
- `antouzai`

Variant features are compile-time modes:

- `annan`: a friendly piece behind the mover donates movement.
- `anhoku`: a friendly piece in front of the mover donates movement.
- `antouzai`: friendly pieces immediately left and/or right of the mover donate movement; if both exist, movement is the union of both donor piece types.

Use `--features annan` when you want Annan shogi rules:

```bash
cargo test -p haitaka --features annan
cargo run -p haitaka --release --features annan --example perft -- 4
```

## Examples And Tools

Perft:

```bash
cargo run -p haitaka --release --example perft -- 4
cargo run -p haitaka --release --example perft -- 4 "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"
```

DFPN:

```bash
cargo run -p haitaka --release --example dfpn -- "8k/6G2/7B1/9/9/9/9/9/K8 b R 1"
```

DFPN corpus:

```bash
cargo run -p haitaka --release --example dfpn_corpus
```

Magic search helper:

```bash
cargo run -p haitaka --release --example find_magics
```

## Testing

```bash
cargo test -p haitaka
cargo test -p haitaka --features annan
cargo test -p haitaka --features anhoku
cargo test -p haitaka --features antouzai
```

## Benchmarks

```bash
cargo bench -p haitaka --bench legals -- --noplot
cargo bench -p haitaka --bench perft -- --noplot
cargo bench -p haitaka --bench dfpn -- --noplot
cargo bench -p haitaka --features annan --bench legals -- --noplot
```

See also [`../docs/benchmarks.md`](../docs/benchmarks.md).

## How This Fork Differs From Upstream

Compared to upstream `main`, the core crate in this workspace now includes:

- piece-influence variant move generation and legality handling
- DFPN mate solving (`Board::dfpn`)
- additional variant-specific movegen and mate-search regressions
- supporting board/validation/type changes needed by the WASM and NNUE layers

For the full workspace-level summary, see the repository root `README.md`.

## Related Crates In This Workspace

- `haitaka_types`
- `haitaka_wasm`
- `haitaka_learn`

## Acknowledgments

This crate still builds on the original `haitaka` project and on ideas/code structure from `cozy-chess`.

## License

MIT. See `../LICENSE`.
