# Minishogi and compile-time board storage

Additional ordinary-movement small rules are documented in [small-shogi.md](small-shogi.md).

The `minishogi` Cargo feature supports Shogitter's 5五将棋, rule ID 51,
including legal moves, DFPN, handcrafted search, native USI, direct WASM,
and Shogitter Engine Package v1.

```sh
cargo run -p haitaka_cli --release --features minishogi -- usi
cargo run -p haitaka --release --features minishogi --example perft -- 5
cargo xtask package --ruleset minishogi
```

`cargo pack-minishogi` is an alias for the package command. It builds WASM and
writes `target/haitaka-variants-minishogi.tgz`, using profile `minishogi-default`,
`ruleId: 51`, SFEN positions, USI moves and no NNUE. One ruleset is compiled per
package; movement-donor features cannot be combined with minishogi. The native
package builder rejects a ruleset name that disagrees with the compiled feature.

## Rules and interoperability

- Board: 5 files by 5 ranks. Start: `rbsgk/4p/5/P4/KGSBR b - 1`.
- Promotion: the opponent's final rank. Entering or leaving it permits promotion;
  a pawn reaching the last rank must promote.
- Total native inventory: two each of pawn, silver, gold, bishop and rook, and
  two kings. Lances and knights are absent. Captured pieces demote.
- Nifu, dead pawn drops and pawn-drop mate are illegal.
- Fourfold repetition is a draw, except that a continuous checker loses.
  This follows Shogitter. Fairy-Stockfish instead loses ordinary minishogi
  repetition for the first player, so its repetition scores are not an oracle.
- Existing 9x9 NNUE and SFNN models are rejected by all loaders, including
  session loaders and package creation. SFNN's `mini` architecture is a 9x9
  network width profile, unrelated to minishogi.

Use `position ... moves ...` to provide repetition history: one SFEN cannot
describe preceding occurrences.

## Static representation

`RuleSpec` separates initial SFEN, promotion depth and inventory from
`BoardGeometry`. The feature selects `SelectedRule` at compile time. Generated
minishogi enums contain exactly 25 squares in file-major order,
`file * height + rank`. Its engine `BitBoard` is 8 bytes; standard shogi retains
its coordinate numbering, 16-byte `u128` bitboard, and Magic attack tables.
Code supporting both builds should use `BitBoardWord`, rather than assuming
that the public `.0` word is always `u128`.

The geometry foundation covers all 23 bounding rectangles in the inspected
Shogitter database, including rectangles used by other board games:

| Geometry | Square storage | Square index | TT coordinates |
| --- | --- | --- | --- |
| Up to 64 squares | `ScalarBitBoard<u64, BITS>` | `u8` | `u16` |
| 65–128 squares | `ScalarBitBoard<u128, BITS>` | `u8` | `u16` |
| 129–256 squares | `MultiBitBoard<WORDS, BITS>` | `u8` | `u32` |
| 257–1296 squares | `MultiBitBoard<WORDS, BITS>` | `u16` | `u32` |

For example, 12x12 needs three words; 36x36 uses `MultiBitBoard<21, 1296>`
(168 bytes). Explicit word and active-bit counts avoid unstable generic const
expressions. Constructors and operations preserve zero padding, including
complements and cross-word shifts. Linear bit shifts and geometric shifts are
separate; geometry also supports a playable-square mask for holes.

These geometry sets are distinct from the engine's small scalar bitboard.
Large geometries are a foundation, not an option to play a 36x36 game in the
current `Board`. Integrating a large rule still requires its coordinates,
physical pieces, capture/promotion semantics, and external notation. The TT
codec is selected through geometry, but single-step encodings do not represent
Chu shogi's multi-step moves. Loops and reflections need their own geometry.

Minishogi uses precomputed directional rays clipped at the first blocker.
Neither compiled ruleset introduces runtime ruleset dispatch in the hot path.
Future large-board backends should use fixed-capacity ray/occupancy operations
rather than exponentially expanding the 9x9 Magic tables.

## Verification

```sh
cargo test -p haitaka-types --features minishogi --lib
cargo test -p haitaka --features minishogi
cargo test -p haitaka_wasm --features minishogi --lib
cargo test -p haitaka_cli --features minishogi --test minishogi
cargo test -p xtask
```

Standard 9x9 documentation examples remain tested in the standard build.
Compact builds explicitly ignore those examples and test compact examples.

Compare complete legal-move sets with a separate Fairy-Stockfish binary:

```sh
cargo run -p haitaka --release --features minishogi \
  --example movegen_oracle -- 128 > /tmp/minishogi-positions.tsv
python3 scripts/compare-minishogi.py --reference /path/to/stockfish \
  --positions /tmp/minishogi-positions.tsv
```

Verify a built WASM module and package it:

```sh
wasm-pack build haitaka_wasm --target web --release --features minishogi \
  --out-dir ../target/wasm-minishogi
node scripts/verify-minishogi.mjs target/wasm-minishogi
cargo xtask package --ruleset minishogi --skip-wasm-build \
  --wasm-dir target/wasm-minishogi
```

Initial results on 2026-11-06, Intel i7-6700, Rust 1.98.1:

- Depth-5 initial-position perft: **533,203**, matching Fairy-Stockfish.
- All legal-move sets match Fairy-Stockfish across 128 deterministic positions.
- All 23 geometries match reference sets for logical operations and shifts;
  tests include 64/128-bit boundaries, padding, holes and the 1296th square.
- Native search/USI and Node plus Chrome 155 WASM checks pass.
- Standard native perft versus the pre-change baseline: startpos about +0.54%,
  endgame -1.34%, tsume -0.19%; no observed 2% regression. These local results
  do not estimate the search-tree cost of a large-board game.
- In Chrome, 40 alternating before/after measurements gave median paired
  perft changes of 0% for startpos and +0.67% for endgame, with identical node
  counts. The comparison loads separate WASM modules built from the pre-change
  Git snapshot and the implementation, using the same dependency lockfile.
- Native fixed-budget search (10,000 nodes, 20 alternating pairs): startpos
  -0.64%, endgame +0.01%. Best moves, completed values and searched node counts
  match. Reproduce with `scripts/compare-usi-nodes.py --before OLD_BINARY
  --after NEW_BINARY --nodes 10000 --samples 20`; both binaries must be standard
  builds with the same release profile and dependency lockfile.
