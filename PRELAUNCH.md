# Pre-Launch Checklist

These are launch readiness items, not post-launch roadmap promises. They should
work before a public announcement.

## Required

- Local play/debug command:
  - `cargo run -p haitaka_cli -- play --human black --depth 3`
  - `cargo run -p haitaka_cli --features annan -- play --human none --depth 3`
- Engine self-play/rating command:
  - `cargo run -p haitaka_cli --release -- self-play --games 4 --a-depth 3 --b-depth 2`
- Legal move benchmark instructions:
  - [docs/benchmarks.md](docs/benchmarks.md)
- Shogitter package generator:
  - `cargo run -p haitaka_cli -- package --wasm-dir haitaka_wasm/pkg`
- CI covers default and Annan builds.
- Public docs avoid machine-specific local paths.
- Issue templates exist for rules bugs, performance work, variant requests, and
  training reports.
- A ready-to-create launch issue list exists in [docs/launch-issues.md](docs/launch-issues.md).

## Announcement Bar

At launch, the repo should be able to honestly say:

- You can run a local shogi-variant engine.
- You can play against it or ask it for a move.
- You can run self-play comparisons.
- You can benchmark legal move generation.
- You can generate a Shogitter package artifact.
- You can start producing NNUE datasets and training reports.
