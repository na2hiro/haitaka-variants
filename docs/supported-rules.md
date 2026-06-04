# Supported Rules

- (default:) 本将棋
- `annan`: 安南将棋. When a piece B is placed behind a piece A and they're on the same side, A moves as if it's B.
- `anhoku`: 安北将棋. When a piece B is placed in front of a piece A and they're on the same side, A moves as if it's B.
- `antouzai`: 安東西将棋. Friendly pieces immediately left and right of A donate movement to A. If both adjacent donors exist, A can move as the union of both donor movement types.
- `taimen`: 対面将棋. When an enemy piece B is placed in front of a piece A, A moves as if it's B (and, symmetrically, B moves as if it's A). The donor is the opponent's piece directly ahead; the mutual swap follows automatically because each piece looks at the enemy facing it.
- `haimen`: 背面将棋. When an enemy piece B is placed behind a piece A, A moves as if it's B (and, symmetrically, B moves as if it's A). The donor is the opponent's piece directly behind.
- `neko`: ネコ将棋. Within each file, look at a maximal vertical run of contiguous **friendly** pieces. The 1st piece from the top swaps abilities with the 1st from the bottom, the 2nd with the 2nd, and so on. The middle piece of an odd-length run keeps its native movement.
- `nekoneko`: ネコネコ将棋. Same as `neko` but a run is any maximal vertical run of contiguous pieces **regardless of color** (only an empty square breaks a run), so a piece's partner may be an enemy.
- `yokoneko`: 横ネコ将棋. Same as `neko` but runs are **horizontal** (within a rank): the 1st piece from the left swaps abilities with the 1st from the right, and so on.
- `yokonekoneko`: 横ネコネコ将棋. Same as `nekoneko` but **horizontal**.

The variant feature flags are mutually exclusive compile-time engine modes. `annan` keeps its custom start position; the other variants currently use the standard shogi start position until variant-specific openings are documented.

`taimen` and `haimen` are "donor" variants like `annan`/`anhoku`, but the donating piece is an enemy rather than a friendly piece, and captures, promotion, and drops still use the physical moving piece.

The `neko` family is also a "donor" family — a piece adopts only its partner's movement *pattern* while moving in its own direction, and captures, promotion, and drops still use the physical piece — but the partner is found by **run reflection** (board-dependent) rather than a fixed adjacent square. Because removing, adding, or relocating any piece can re-segment a run and change another piece's effective movement, the `neko` move generator does not track pins and instead verifies king safety by replaying every candidate move. This is correctness-first; the resulting move generation is significantly slower than the fixed-offset variants and performance optimization is deferred.

`haitaka_learn` now covers NNUE data generation, training orchestration, export, and verification for all of the supported rule modes above. Use the matching Cargo feature for variant workflows.
