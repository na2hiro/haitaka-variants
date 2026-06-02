## WASM USI Future Work

### Summary

The first WASM USI implementation intentionally keeps the protocol synchronous
and small. It is meant to establish a shared USI command surface for native CLI
and browser WASM engines, not to complete every engine-management feature.

### Deferred Items

- Async search and `stop`
  - v1 `go` returns only after search completes. Interruptible search needs a
    Worker-oriented control path and shared cancellation state.
- Ponder
  - Ponder requires background search, ponder-hit handling, and a clear
    scheduling policy. Those are separate from proving the USI command surface.
- Full time controls
  - v1 supports `go depth N` and `go movetime N`. Clock fields such as `btime`,
    `wtime`, `byoyomi`, and increments need real time-management policy.
- Multi-PV
  - v1 returns a single `bestmove`. Multi-PV needs search support and output
    format decisions before it should be exposed through USI.
- USI options
  - `setoption` is not implemented in v1. Options need stable names, defaults,
    validation, and package metadata.
- Web Worker harness
  - The WASM API is callable from JavaScript, but a production browser app
    should run engines inside Workers so synchronous search does not block UI.
- Browser self-play UI
  - The engine protocol comes first. Upload, pairing, progress display, and
    result persistence should be designed after Worker execution is stable.
- Native archive workflow
  - Native executable archives remain useful for developer benchmarking, but
    browser-portable WASM USI is the higher-priority runtime for public testing.
- Rating and report improvements
  - Confidence intervals, JSONL game logs, and richer summaries are deferred
    until the engine runtime and controller flow are stable.
- Cross-runtime rating policy
  - Native and WASM ratings should remain separate unless a later calibration
    plan proves they can be compared meaningfully.

### Decision Rule

Do not add deferred protocol features just because a parser recognizes their
USI keywords. Add each feature only when the search engine, runtime transport,
tests, and reporting behavior are all ready for it.
