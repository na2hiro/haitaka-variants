# Supported Rules

- (default:) 本将棋
- `annan`: 安南将棋. When a piece B is placed behind a piece A and they're on the same side, A moves as if it's B.
- `anhoku`: 安北将棋. When a piece B is placed in front of a piece A and they're on the same side, A moves as if it's B.
- `antouzai`: 安東西将棋. Friendly pieces immediately left and right of A donate movement to A. If both adjacent donors exist, A can move as the union of both donor movement types.

The variant feature flags are mutually exclusive compile-time engine modes. `annan` keeps its custom start position; `anhoku` and `antouzai` currently use the standard shogi start position until variant-specific openings are documented.
