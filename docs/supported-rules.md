# Supported Rules

- (default:) 本将棋
- `minishogi`: 5五将棋 (Shogitter rule 51), with a compact 64-bit board, a one-rank promotion zone, and Shogitter-compatible repetition. See [the rule and storage documentation](minishogi.md).
- `annan`: 安南将棋. When a piece B is placed behind a piece A and they're on the same side, A moves as if it's B.
- `anhoku`: 安北将棋. When a piece B is placed in front of a piece A and they're on the same side, A moves as if it's B.
- `antouzai`: 安東西将棋. Friendly pieces immediately left and right of A donate movement to A. If both adjacent donors exist, A can move as the union of both donor movement types.
- `anki`: 安騎将棋. Friendly pieces on chess-knight squares donate movement. If at least one donor exists, donor movement replaces native movement.
- `tenjiku`: 天竺将棋. Same behind-friendly donor geometry as Annan, but the moving piece keeps its native movement in addition to the donor movement.
- `taimen`: 対面将棋. When an enemy piece B is placed in front of a piece A, A moves as if it's B (and, symmetrically, B moves as if it's A). The donor is the opponent's piece directly ahead; the mutual swap follows automatically because each piece looks at the enemy facing it.
- `haimen`: 背面将棋. When an enemy piece B is placed behind a piece A, A moves as if it's B (and, symmetrically, B moves as if it's A). The donor is the opponent's piece directly behind.
- `neko`: ネコ将棋. Within each file, look at a maximal vertical run of contiguous **friendly** pieces. The 1st piece from the top swaps abilities with the 1st from the bottom, the 2nd with the 2nd, and so on. The middle piece of an odd-length run keeps its native movement.
- `nekoneko`: ネコネコ将棋. Same as `neko` but a run is any maximal vertical run of contiguous pieces **regardless of color** (only an empty square breaks a run), so a piece's partner may be an enemy.
- `yokoneko`: 横ネコ将棋. Same as `neko` but runs are **horizontal** (within a rank): the 1st piece from the left swaps abilities with the 1st from the right, and so on.
- `yokonekoneko`: 横ネコネコ将棋. Same as `nekoneko` but **horizontal**.
- `tenkyo`: 点鏡将棋. A piece moves as the piece on the point-symmetric square around 5,5, regardless of color. The center square reflects to itself.

The variant feature flags are mutually exclusive compile-time engine modes. `minishogi` uses its 5x5 start position, `annan` keeps its custom start position, and the other variants currently use the standard shogi start position until variant-specific openings are documented.

`taimen` and `haimen` are "donor" variants like `annan`/`anhoku`, but the donating piece is an enemy rather than a friendly piece, and captures, promotion, and drops still use the physical moving piece.

The `neko` family is also a "donor" family — a piece adopts only its partner's movement *pattern* while moving in its own direction, and captures, promotion, and drops still use the physical piece — but the partner is found by **run reflection** (board-dependent) rather than a fixed adjacent square. Because removing, adding, or relocating any piece can re-segment a run and change another piece's effective movement, the `neko` move generator does not track pins and instead verifies king safety by replaying every candidate move. This is correctness-first; the resulting move generation is significantly slower than the fixed-offset variants and performance optimization is deferred.

`tenkyo`, `tenjiku`, and `anki` also use correctness-first replay filtering for legal move generation because a move can change donor relationships outside ordinary pin and interposition masks.

`haitaka_learn` covers NNUE data generation, training orchestration, export, and verification for the 9x9 rule modes above. Use the matching Cargo feature for variant workflows. Minishogi currently uses handcrafted evaluation; dedicated NNUE training is a later step.

## Historical shogi

The compile-time `heian-8x8`, `heian-9x8` and `heian-9x9` modes support Shogitter rules 12, 13 and 14. They discard captures, promote to canonical golds and draw with only kings remaining. See [Heian rules and verification](heian.md) for usage and [the roadmap](../plans/historical-shogi/README.md) for later stages.

The `shoshogi` (43) and `chushogi` (48) modes support multiple royal pieces and completed lion turns through explicit JSON APIs. See [Sho](shoshogi.md) and [Chu](chushogi.md).

The `torishogi` (16), `washogi` (50), `heian-dai` (15) and `daishogi` (49) modes use JSON v3 with rule-local piece IDs. Tori includes reserves and drops; Heian Dai and Dai preserve ambiguous promotion origins. All use handcrafted evaluation. See [adopted rules, API and verification](classical-stage4.md).

The `tenjiku-dai` mode supports Tenjiku Daishogi (59), with ordered freezing/burning effects, up to three hops per turn and JSON v4. It is separate from the 9x9 `tenjiku` mode (56). See [adopted rules and verification](tenjiku-dai.md).

The `dai-dai` mode supports Dai Dai Shogi (60) on a 17x17 board, with mandatory capture promotions, atomic hook rays, three-hop komainu turns and JSON v4. See [adopted rules and verification](dai-dai.md).

The `maka-dai-dai` mode supports Maka Dai Dai Shogi (61) on a 19x19 board, with capture-dependent conversions, ambiguous gold origins, emperor movement and JSON v4. See [adopted rules and verification](maka-dai-dai.md).

The `taishogi` mode supports Tai Shogi (63) on a 25x25 board, with the royal set ix/cg/uc, Maka-style conversions, gold/elephant promotion origins and JSON v4. See [adopted rules and verification](taishogi.md).

The `taikyoku` mode supports Taikyoku Shogi (64) on a 36x36 board, with four-hop Bunju turns, transit captures, finite jumps, 11-rank optional promotions and JSON v5. This completes all 14 historical shogi rules. See [adopted rules and verification](taikyoku.md).
