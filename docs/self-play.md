# Self-Play And Strength Checks

Use `haitaka_cli self-play` for local engine A/B checks. It can compare
in-process evaluators, raw USI subprocess engines, or native engine archives.

Run commands from the repository root.

## Quick In-Process Check

Use a fixed movetime budget for normal strength checks:

```bash
cargo run -p haitaka_cli --release -- self-play \
  --games 20 \
  --threads 0 \
  --movetime-ms 100
```

Compare an NNUE model against the handcrafted evaluator:

```bash
cargo run -p haitaka_cli --release -- self-play \
  --games 100 \
  --threads 0 \
  --movetime-ms 100 \
  --a-eval nnue \
  --nnue path/to/model.nnue \
  --opening-random-plies 4 \
  --seed 1
```

`--threads 0` uses available parallelism. The score and Elo estimate are
reported as `A - B`, so positive values mean engine A outscored engine B.
Prefer `--movetime-ms` for strength checks because fixed-depth matches can hide
improvements that make the engine search faster. When movetime is set, omitted
`--a-depth` and `--b-depth` mean uncapped iterative deepening; pass those flags
only when you want an explicit per-side movetime depth cap.

Use fixed depth as a debugging alternative when you need deterministic search
shape or a very cheap smoke test:

```bash
cargo run -p haitaka_cli --release -- self-play \
  --games 4 \
  --threads 1 \
  --a-depth 3 \
  --b-depth 2
```

## Native Engine Archives

Use native archives when comparing different code versions. The archive stores
the executable, optional NNUE model, build identity, git commit, dirty flag,
ruleset, target, and USI runtime metadata.

Build an engine binary and archive it:

```bash
cargo build -p haitaka_cli --release

cargo run -p haitaka_cli --release -- archive-engine \
  --output target/engines/haitaka-current.tgz \
  --binary target/release/haitaka_cli \
  --profile release \
  --target "$(rustc -vV | sed -n 's/^host: //p')"
```

Archive an engine with an NNUE model:

```bash
cargo run -p haitaka_cli --release -- archive-engine \
  --output target/engines/haitaka-nnue.tgz \
  --binary target/release/haitaka_cli \
  --nnue path/to/model.nnue \
  --profile release
```

Run an archived-engine match:

```bash
cargo run -p haitaka_cli --release -- self-play \
  --games 100 \
  --threads 4 \
  --a-engine-archive target/engines/haitaka-new.tgz \
  --b-engine-archive target/engines/haitaka-old.tgz \
  --movetime-ms 100 \
  --opening-random-plies 4 \
  --seed 1 \
  --report-dir target/self-play/new-vs-old
```

Native archives produced by `archive-engine` launch the archived executable as
`haitaka_cli usi`, adding `--eval nnue --nnue <extracted-model>` when the archive
contains NNUE. Extracted archive directories are cleaned after a successful
match.

## Raw External USI Engines

Use raw USI engine paths for quick local checks before creating archives:

```bash
cargo run -p haitaka_cli --release -- self-play \
  --games 20 \
  --threads 2 \
  --a-engine path/to/new/usi-engine \
  --b-engine path/to/old/usi-engine \
  --movetime-ms 100
```

Arguments can be passed per side. For a `haitaka_cli` binary, pass the `usi`
subcommand explicitly:

```bash
cargo run -p haitaka_cli --release -- self-play \
  --games 20 \
  --a-engine path/to/haitaka_cli \
  --a-engine-arg usi \
  --a-engine-arg --eval \
  --a-engine-arg nnue \
  --a-engine-arg --nnue \
  --a-engine-arg path/to/a.nnue \
  --b-engine path/to/haitaka_cli \
  --b-engine-arg usi \
  --movetime-ms 100
```

Do not pass both `--a-engine` and `--a-engine-archive` for the same side.

## Opening Control

Randomized openings:

```bash
cargo run -p haitaka_cli --release -- self-play \
  --games 100 \
  --movetime-ms 100 \
  --opening-random-plies 4 \
  --seed 1
```

The same generated opening is used for each consecutive color-swapped pair.

Opening suite file:

```text
# one SFEN per line; blank lines and comments are ignored
lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1
```

Run with the suite:

```bash
cargo run -p haitaka_cli --release -- self-play \
  --games 100 \
  --movetime-ms 100 \
  --openings openings.sfen \
  --opening-order sequential \
  --opening-random-plies 2 \
  --seed 1
```

`--opening-order sequential` cycles through the suite by pair index.
`--opening-order random` selects a deterministic random suite entry per pair
from `--seed`.

## Reports

Write both aggregate and per-game outputs:

```bash
cargo run -p haitaka_cli --release -- self-play \
  --games 100 \
  --movetime-ms 100 \
  --report-dir target/self-play/run-1
```

`--report-dir` creates two files:

- `self-play-report.json`
- `self-play-games.jsonl`

`self-play-report.json` includes:

- command settings
- package version
- git commit and dirty flag
- ruleset
- engine identity and launch metadata
- embedded native archive manifest metadata for archive engines
- score, approximate Elo, approximate 95% confidence interval, nodes, NPS, and
  warnings

`self-play-games.jsonl` writes one JSON object per completed game:

- game and pair index
- colors
- opening source and start SFEN
- played USI moves
- result and winner
- plies, nodes, elapsed time
- failure state, currently `null` because protocol failures stop the match

The Elo and confidence interval are intentionally approximate. Treat them as
local development signals, not publishable rating claims.

If the report directory already contains a saved report, `self-play` asks what
to do:

```text
1. Abort
2. Self-play more and merge result
3. Discard saved and override with new result
```

Merge mode appends new records to `self-play-games.jsonl`, continues game
indices from the existing summary, and rewrites `self-play-report.json` with the
combined aggregate. Unanswered non-interactive runs abort on an existing report
directory; use a fresh directory for automation or pipe an explicit choice.

If a run is interrupted with Ctrl+C, `self-play` stops starting new games,
waits for any in-flight games to finish, flushes `self-play-games.jsonl`, and
writes a partial `self-play-report.json`. The command exits with an interrupt
error, but the report directory can be selected again and merged from the saved
summary.

## Failure Policy

Self-play validates external `bestmove` responses against the local board.
Missing engine paths, startup timeout, search timeout, closed stdout, malformed
`bestmove`, and illegal `bestmove` fail the match clearly. They are not treated
as forfeits in the current workflow.

## Variants

Build and run with the matching feature for variant-specific checks:

```bash
cargo run -p haitaka_cli --release --features annan -- self-play \
  --games 100 \
  --threads 0 \
  --movetime-ms 100
```

Native archives record the active ruleset/features, but a match still depends
on the local `self-play` controller using the same board rules as the engines
being compared.
