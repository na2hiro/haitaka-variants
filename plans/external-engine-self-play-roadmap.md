## External Engine Self-Play Roadmap

### Summary

The current `self-play` command can compare different evaluator settings,
NNUE files, and fixed search depths, but both sides still run inside the same
compiled engine version. That is useful for quick model and setting checks, but
it cannot answer the larger question: whether a new code version is stronger
than an older one.

Recommended sequence:

- [x] Phase 1: add a minimal USI subprocess mode
- [x] Phase 2: teach `self-play` to drive external engines
- [x] Phase 3: archive executable engine builds with identity metadata
- [x] Phase 4: improve rating reports and experiment tracking

Current status:

- [x] Native CLI USI mode exists.
- [x] Native `self-play` can launch external USI subprocess engines.
- [x] Shared WASM/CLI USI session support exists through `UsiSession`.
- [x] WASM exposes a `UsiEngine` API for USI command strings over JS/Worker
  transports.
- [x] Deferred WASM USI work is documented in
  `plans/wasm-usi-future-work.md`.
- [x] Native engine archive workflow is implemented for `.tgz` developer
  self-play archives.
- [x] Rating/report improvements are implemented with JSON summary reports,
  JSONL per-game logs, opening suites, reproducibility metadata, and approximate
  Elo confidence intervals.

This should start with the smallest reliable protocol surface. Full USI
ecosystem compatibility is not the first goal; controlled local comparison
between Haitaka builds is.

### Why This Order

- External-engine comparison needs a stable process protocol before packaging
  or rating automation can be trusted.
- A minimal USI mode lets the current engine run as a subprocess without first
  redesigning the existing in-process search API.
- `self-play` should prove it can handle process lifecycle, timeouts, crashes,
  and illegal moves before archived engine bundles become the default workflow.
- Archive metadata matters only after the subprocess interface is stable enough
  to reproduce games across versions.

### Phase 1 - Done

Add a minimal `haitaka_cli usi` mode.

Required command support:

- `usi`
- `isready`
- `usinewgame`
- `position startpos`
- `position sfen ...`
- `position ... moves ...`
- `go depth N`
- `go movetime N`
- `quit`

Initial output can be intentionally small:

- `id name Haitaka Variants`
- `id author ...` if a stable author string is already available
- `usiok`
- `readyok`
- `bestmove <move>`
- `bestmove resign` when no legal move exists

Implementation guidance:

- Reuse existing SFEN parsing and USI move parsing.
- Support fixed depth first, but include `movetime` because fixed-depth
  strength comparisons can hide search-speed improvements.
- Treat variant/ruleset identity as a build or manifest concern, not something
  inferred from USI traffic alone.
- Keep `ponder`, `stop`, hash options, and full time-control parsing out of
  scope for the first version unless they are needed by tests.

### Phase 2 - Done

Add external-engine support to `self-play`.

Suggested CLI shape:

```bash
haitaka self-play \
  --games 100 \
  --threads 4 \
  --a-engine path/to/old/haitaka \
  --b-engine path/to/new/haitaka \
  --go depth=3 \
  --opening-random-plies 4 \
  --seed 1
```

Behavior:

- If `--a-engine` or `--b-engine` is present, launch that side as a USI
  subprocess instead of using the in-process evaluator.
- Preserve color-swapped paired games and deterministic random openings.
- Validate each returned `bestmove` against the local `Board` before playing it.
- Record engine stderr or protocol logs enough to debug failures without
  flooding normal output.
- Treat process crash, timeout, malformed response, and illegal bestmove as
  match errors unless a later explicit policy chooses forfeits.

Out of scope for this phase:

- Swiss tournaments or many-engine pools
- SPRT
- persistent engine pools across unrelated commands
- remote engine execution

### Phase 3 - Done

Add an engine archive workflow for reproducible comparisons.

Suggested CLI shape:

```bash
haitaka archive-engine \
  --output target/engines/haitaka-<commit>.tar.zst \
  --binary target/release/haitaka \
  --nnue path/to/model.nnue
```

Archive contents should include:

- executable engine binary
- optional NNUE file
- manifest JSON

Manifest metadata should include:

- engine name and version
- git commit or revision
- dirty-worktree marker if available
- ruleset and compile-time feature identity
- build profile and target triple
- executable path inside the archive
- protocol name and version
- NNUE archive path and sha256 when present

`self-play` can later accept either raw engine paths or archive paths. Archive
support should extract to a temporary directory and launch the manifest-declared
binary.

### Phase 4 - Done

Improve rating reports after external matches are stable.

Useful additions:

- [x] per-engine identity summary in match output and report metadata
- [x] JSONL game log with opening source, start SFEN, moves, result, and failure
  state
- [x] aggregate JSON summary
- [x] confidence interval around the Elo estimate
- [x] separate depth-based and movetime-based report labels

Deferred statistical and product work is documented in
`plans/strength-measurement-future-work.md`.

### Test Plan

- [x] USI smoke test launches `haitaka usi`, sends `usi`, `isready`, `position`,
  and `go depth 1`, then verifies a legal `bestmove`.
- [x] `position ... moves ...` test verifies the engine searches from the expected
  post-move board.
- [x] Movetime support is covered by shared USI/session behavior and CLI search
  budget tests.
- [x] External self-play smoke test runs two local Haitaka subprocesses for a tiny
  match.
- [x] Failure tests cover missing engine path, startup timeout, search timeout,
  process exit, malformed `bestmove`, and illegal `bestmove`.
- [x] Archive test verifies the manifest records commit/ruleset/build/NNUE
  identity and that an archived engine can be launched for a smoke search.
- [x] WASM target check verifies the shared USI session and `UsiEngine` compile
  for `wasm32-unknown-unknown`.
- [x] Future-work docs test verifies intentionally omitted WASM USI work is
  named under `plans/`.

### Decision Rule

All four phases in this roadmap are complete. `usi`, external `self-play`,
archive support, movetime matches, JSONL/JSON reports, and confidence intervals
are available. Do not schedule Phase 1 or Phase 2 from this historical plan.

Deferred statistical and product extensions remain in
`plans/strength-measurement-future-work.md`. Current NNUE promotion work is in
`plans/anhoku-nnue-handcrafted-strength-plan.md`.
