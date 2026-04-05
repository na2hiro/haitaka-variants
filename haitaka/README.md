# haitaka

`haitaka` is the core engine crate inside the `haitaka-variants` workspace.

It started from upstream `tofutofu/haitaka`, but this fork now uses the crate as the base for variant support, mate solving, WASM integration, and NNUE work.

## What This Crate Does

- board representation for 9x9 shogi
- SFEN parsing and formatting
- legal move generation
- perft tooling
- Zobrist hashing
- DFPN mate search
- optional Annan shogi support via a feature flag

This crate is the rules/movegen core used by:

- `haitaka_wasm`
- `haitaka_learn`

## Features

- `std`
- `qugiy`
- `annan`

## Examples

Perft:

```bash
cargo run -p haitaka --release --example perft -- 4
```

DFPN:

```bash
cargo run -p haitaka --release --example dfpn -- "8k/6G2/7B1/9/9/9/9/9/K8 b R 1"
```

## Testing

```bash
cargo test -p haitaka
cargo test -p haitaka --features annan
```

## How This Fork Differs From Upstream

Compared to upstream `main`, the core crate in this workspace now includes:

- Annan shogi move generation and legality handling
- DFPN mate solving (`Board::dfpn`)
- additional Annan-specific movegen and mate-search regressions
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
