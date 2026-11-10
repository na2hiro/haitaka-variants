# Ordinary-movement small shogi

Each ruleset is a separate compile-time build. All five use one 8-byte `u64`
bitboard, dense file-major coordinates, compact TT moves and the same directional
ray backend. The standard 9x9 build retains its existing `u128` and Magic tables.

| Cargo feature / package ruleset | Shogitter ID | Board | Promotion ranks | Initial hands |
| --- | --- | --- | --- | --- |
| `minishogi` | 51 | 5x5 | 1 | none |
| `judkins` | 52 | 6x6 | 2 | none |
| `gorogoro` | 96 | 5x6 | 2 | none |
| `gorogoro-plus` | 97 | 5x6 | 2 | one knight and lance each |
| `minishogi-kingin` | 110 | 5x5 | 1 | one bishop and rook each |

The definitions follow `packages/core/src/db/rules/` in Shogitter. `RuleSpec`
selects the board geometry, start SFEN, promotion region and total piece inventory.
Captured pieces demote. Nifu, dead-piece drops and pawn-drop mate are prohibited;
pawns/lances on the last rank and knights on the last two ranks must promote.
Fourfold repetition draws; a perpetual checker loses. Supply the move history
with USI `position ... moves ...` for correct repetition adjudication.

These rules support native move generation, DFPN, handcrafted evaluation/search,
USI, direct WASM and Engine Package v1. Existing 9x9 NNUE/SFNN models are rejected.
The internal `small-board` feature is enabled by each ruleset; it cannot be used
alone or combined with movement-donor rules. Multiple small rules cannot be
compiled into one engine.

```sh
cargo run -p haitaka_cli --release --features judkins -- usi
cargo run -p haitaka --release --features gorogoro --example perft -- 3
cargo xtask package --ruleset gorogoro-plus
cargo test -p haitaka -p haitaka-types -p haitaka_wasm -p haitaka_cli --features judkins
```

The same commands accept the other features in the table. Packages are written
to `target/haitaka-variants-<ruleset>.tgz`, with the corresponding Shogitter rule
ID and no model. The engine package builder checks the compiled ruleset.

## Independent validation

Fairy-Stockfish's built-in `judkins` and `gorogoro` provide independent move
oracles. `scripts/reference-small-shogi.ini` adds the two initial-hand variants.
Each additional rule was compared over 128 deterministic positions. Fairy's
legal generator includes pawn-drop mate and instead scores the resulting mate
as a loss for the dropper; the comparator removes those checking pawn drops
when its independent reply generation finds zero legal replies. This happened
once in the tested gorogoro-plus sequence. Repetition scores are not compared:
Fairy's minishogi repetition convention differs from Shogitter.

```sh
cargo run -p haitaka --release --features gorogoro-plus \
  --example movegen_oracle -- 128 > target/gorogoro-plus-positions.tsv
python3 scripts/compare-minishogi.py --reference /path/to/stockfish \
  --variant gorogoroplus --variant-file scripts/reference-small-shogi.ini \
  --positions target/gorogoro-plus-positions.tsv
```

| Ruleset | Initial perft depth 3 (Fairy and Haitaka) |
| --- | ---: |
| minishogi | 2512 |
| judkins | 6183 |
| gorogoro | 4166 |
| gorogoro-plus | 44436 |
| minishogi-kingin | 46127 |

Native tests exercise inventory, promotion boundaries, knight/lance drops,
random SFEN roundtrips, generated-vs-individual legality, search and USI.
`scripts/verify-small-shogi.mjs` checks the actual generated WASM for all five
rules after generating `target/wasm-<ruleset>/haitaka_wasm.js` and its WASM with
wasm-pack/wasm-bindgen. It checks perft, search, USI move acceptance and model
rejection.

Kyoto, Kinkei Mai, Micro and Kyoto vs Minishogi use the separate
[reversible small-rule JSON backend](flip-small-shogi.md). Together these cover
all nine rules in Shogitter's five-five category. See [minishogi.md](minishogi.md)
for the larger-board geometry foundation.
