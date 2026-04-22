# Contributing

Haitaka Variants needs help from players, rules experts, engine developers, ML
contributors, and Shogitter/web developers.

## Useful First Contributions

- Run `cargo run -p haitaka_cli -- play --human black --depth 3` and report
  illegal moves or obviously bad engine behavior.
- Run `cargo test -p haitaka --features annan` before and after changing rules.
- Add small SFEN positions that cover Annan edge cases.
- Run `cargo bench -p haitaka --bench legals -- --noplot` before and after
  movegen performance changes.
- Submit a training run report with hardware, config, dataset size, and verify
  results.

## Contribution Areas

- **Players**: try the local engine, collect bad moves, and share interesting
  positions.
- **Rules experts**: confirm variant rules, edge cases, illegal moves, and perft
  expectations.
- **Rust developers**: improve move generation, search, DFPN, WASM bindings, and
  CLI workflows.
- **ML/GPU contributors**: generate datasets, run NNUE training, export models,
  and verify results.
- **Web/Shogitter contributors**: consume generated engine packages and connect
  stronger bots to the site.

## Development Commands

```bash
cargo test
cargo test -p haitaka --features annan
cargo test -p haitaka_wasm --features annan
cargo test -p haitaka_learn --features annan
cargo test -p haitaka_cli --features annan
```

Local engine smoke tests:

```bash
cargo run -p haitaka_cli -- play --human none --depth 2
cargo run -p haitaka_cli -- self-play --games 2 --a-depth 2 --b-depth 1
cargo run -p haitaka_cli -- package --allow-missing-wasm
```

Benchmarks:

```bash
cargo bench -p haitaka --bench legals -- --noplot
cargo bench -p haitaka --bench perft -- --noplot
cargo bench -p haitaka --bench dfpn -- --noplot
```

## Pull Requests

- Keep rule changes small and include SFEN-based regression tests when possible.
- Include benchmark numbers for move generation or search performance changes.
- Say whether the change was tested with `--features annan`.
- Do not include generated training outputs or large model files directly in PRs.
