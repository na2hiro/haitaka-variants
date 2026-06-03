## Strength Measurement Future Work

The first reporting phase intentionally keeps Haitaka's self-play measurement
local, synchronous, and easy to inspect. The items below are deferred so the v1
scope stays focused on reproducible developer comparisons.

### SPRT And Sequential Testing

Deferred because early local matches need transparent score, Elo, and confidence
summary output before they need accept/reject automation. Add SPRT once match
logs and opening-suite behavior are stable enough for larger samples.

### STC/LTC Experiment Presets

Deferred because current self-play supports depth and movetime budgets, but does
not yet define shared short-time-control or long-time-control presets. Add named
presets after enough local benchmark runs identify useful defaults.

### Distributed Workers

Deferred because this phase targets one developer machine. A Fishtest-like
worker/server model needs authentication, task leases, result validation, and
resumability, which are separate product concerns.

### PGN Or CSA Output

Deferred because SFEN plus USI move JSONL is simpler for the current engine and
variant support. Add PGN or CSA export if downstream analysis tools need it.

### Engine Crash Forfeits

Deferred because the current policy treats protocol errors, crashes, malformed
`bestmove`, and illegal moves as match errors. Forfeit policy should be explicit
once tournament-style runs become a real use case.

### Multi-Engine Tournaments

Deferred because Phase 4 compares engine A against engine B. Round-robin,
gauntlet, and Swiss-style tournament scheduling should build on the same per-game
record schema later.

### Cross-Runtime Rating Pools

Deferred because native and WASM runtimes have different performance profiles.
Keep their ratings separate until a calibration policy is defined.

### Browser Rating Webapp

Deferred because browser self-play should use the USI-over-WASM worker transport
planned separately. The JSON report schema here can inform that UI, but this
phase does not implement upload, worker orchestration, or local browser ratings.

### Resumable Matches

Deferred because JSONL logs make completed games visible, but the command does
not yet resume from partial output. Add resume semantics only after the record
schema has survived real match usage.
