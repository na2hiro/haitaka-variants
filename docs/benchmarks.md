# Benchmarks

Benchmarks are part of the launch workflow. Use them before and after changes to
legal move generation, rule handling, search, DFPN, or NNUE evaluation.

## Pull Request Automation

The `Benchmarks` GitHub Actions workflow compares selected Criterion benchmarks
between a PR head and its base commit. It runs automatically only when core
engine, type, benchmark, Cargo, or benchmark-workflow files change. Maintainers
can also start it manually with `workflow_dispatch` for PRs or branches that need
performance validation.

The workflow is advisory on GitHub-hosted runners: benchmark command failures
fail the job, but detected slowdowns are reported in the workflow summary instead
of blocking the PR. Current thresholds are 5% for a warning and 15% for a
significant slowdown.

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
