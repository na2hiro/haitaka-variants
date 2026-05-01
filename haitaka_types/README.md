# haitaka_types

`haitaka_types` defines the shared core types used by the `haitaka-variants`
workspace.

It is primarily an internal support crate for `haitaka`, though it can still be
used directly by other crates in the workspace.

## What This Crate Provides

- colors, files, ranks, and squares
- pieces and promoted pieces
- USI move parsing and formatting
- bitboards and bitboard iteration
- slider move helpers
- feature-gated variant type support

Splitting these types into a separate crate lets `haitaka` run a build script
that generates slider move hash tables for magic bitboards without duplicating
type definitions.

## Features

- `std`
- `qugiy`
- `annan`

Use the same feature flags as the consuming crate:

```bash
cargo test -p haitaka_types
cargo test -p haitaka_types --features annan
```

## Related Crates

- `haitaka`: consumes these types for board representation and move generation.
- `haitaka_wasm`: exposes engine results to JavaScript.
- `haitaka_cli`: uses move/SFEN types in local tools.
