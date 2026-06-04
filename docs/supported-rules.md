# Supported Rules

- (default:) 本将棋
- `annan`: 安南将棋. When a piece B is placed behind a piece A and they're on the same side, A moves as if it's B.
- `anhoku`: 安北将棋. When a piece B is placed in front of a piece A and they're on the same side, A moves as if it's B.
- `antouzai`: 安東西将棋. Friendly pieces immediately left and right of A donate movement to A. If both adjacent donors exist, A can move as the union of both donor movement types.
- `taimen`: 対面将棋. When an enemy piece B is placed in front of a piece A, A moves as if it's B (and, symmetrically, B moves as if it's A). The donor is the opponent's piece directly ahead; the mutual swap follows automatically because each piece looks at the enemy facing it.
- `haimen`: 背面将棋. When an enemy piece B is placed behind a piece A, A moves as if it's B (and, symmetrically, B moves as if it's A). The donor is the opponent's piece directly behind.

The variant feature flags are mutually exclusive compile-time engine modes. `annan` keeps its custom start position; `anhoku`, `antouzai`, `taimen`, and `haimen` currently use the standard shogi start position until variant-specific openings are documented.

`taimen` and `haimen` are "donor" variants like `annan`/`anhoku`, but the donating piece is an enemy rather than a friendly piece, and captures, promotion, and drops still use the physical moving piece.

`haitaka_learn` now covers NNUE data generation, training orchestration, export, and verification for all of the supported rule modes above. Use the matching Cargo feature for variant workflows.
