## External Engine Self-Play Roadmap

### Summary

The current `self-play` command can compare different evaluator settings,
NNUE files, and fixed search depths, but both sides still run inside the same
compiled engine version. That is useful for quick model and setting checks, but
it cannot answer the larger question: whether a new code version is stronger
than an older one.

Recommended sequence:

- Phase 1: add a minimal USI subprocess mode
- Phase 2: teach `self-play` to drive external engines
- Phase 3: archive executable engine builds with identity metadata
- Phase 4: improve rating reports and experiment tracking

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

### Phase 1

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

### Phase 2

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

### Phase 3

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

### Phase 4

Improve rating reports after external matches are stable.

Useful additions:

- per-engine identity summary in match output
- JSONL game log with opening seed, start SFEN, moves, result, and failure state
- aggregate CSV or JSON summary
- confidence interval around the Elo estimate
- separate depth-based and movetime-based report labels

Do not over-invest in rating math before the subprocess and archive workflows
are reliable. A simple Elo-style estimate is enough for early local iteration.

### Test Plan

- USI smoke test launches `haitaka usi`, sends `usi`, `isready`, `position`,
  and `go depth 1`, then verifies a legal `bestmove`.
- `position ... moves ...` test verifies the engine searches from the expected
  post-move board.
- Movetime smoke test verifies `go movetime N` returns a legal move without
  hanging.
- External self-play smoke test runs two local Haitaka subprocesses for a tiny
  match.
- Failure tests cover missing engine path, startup timeout, search timeout,
  process exit, malformed `bestmove`, and illegal `bestmove`.
- Archive test verifies the manifest records commit/ruleset/build/NNUE identity
  and that an archived engine can be launched for a smoke search.

### Decision Rule

If only one item can be scheduled next, implement Phase 1. A minimal `usi` mode
is the foundation for every later step and gives immediate feedback on whether
Haitaka can be driven reliably as an external engine.

After Phase 1, prefer Phase 2 over archive polish. The archive format should be
shaped by the needs of a working external `self-play` driver, not guessed in
advance.
