# Benchmarks

Benchmarks are part of the launch workflow. Use them before and after changes to
legal move generation, rule handling, search, DFPN, or NNUE evaluation.

## Legal Move Generation

```bash
cargo bench -p haitaka --bench legals -- --noplot
cargo bench -p haitaka --features annan --bench legals -- --noplot
```

Use this for changes to `Board::is_legal`, generated moves, drops, check
detection, pins, and Annan effective-piece behavior.

## Perft

```bash
cargo bench -p haitaka --bench perft -- --noplot
cargo bench -p haitaka --features annan --bench perft -- --noplot
```

Use this to catch broad move-generation regressions and speed changes.

## DFPN

```bash
cargo bench -p haitaka --bench dfpn -- --noplot
cargo bench -p haitaka --features annan --bench dfpn -- --noplot
```

Use this for mate-search changes.

## NNUE

```bash
cargo bench -p haitaka_wasm --bench nnue -- --noplot
```

Use this for inference and accumulator changes. Annan NNUE loading is currently
not enabled.

## Reporting Results

For performance PRs, include:

- command run
- CPU/OS
- default or `--features annan`
- before/after numbers
- whether the change affects correctness, speed, or both
