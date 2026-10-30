## Phase R8: runtime and the fixed-time tax

### Question

How much equal-node strength is required to overcome NNUE's measured NPS cost,
and can inference recover that cost without changing scores?

### Required measurements

- Native and production WASM full-refresh and incremental latency by move type,
  including king moves, drops, promotions, captures, and donor changes.
- Alpha-beta NPS, qsearch NPS, and the versioned combined throughput
  `(alpha_beta_nodes + qnodes) / elapsed`, plus accumulator rebuild/update
  counts, cache behavior, and model bandwidth on the production position suite.
  Legacy main-search NPS alone is never used to derive the node handicap.
- A handcrafted-versus-handcrafted node-handicap curve using node ratios that
  match observed NNUE/handcrafted combined throughput. Publish both node
  components. This estimates the Elo tax of the runtime deficit without
  conflating evaluator quality.
- Equal-node and fixed-time results for the identical candidate network.
- Single-game production-like fixed-time performance as the primary runtime
  condition. Controlled parallel loads are separate throughput experiments;
  they never replace the target single-game result.

### Optimization rules

- Profile first and change one hotspot at a time.
- Preserve exact integer evaluator scores and, on a fixed-node search corpus,
  exact root scores, PVs, best moves, alpha-beta/qnode accounting, and declared
  trace hashes unless a search change is explicitly registered as a separate
  experiment.
- Do not retrain to validate a runtime-only change.
- Re-run accumulator differential tests and the full verifier after every
  optimization.

### Gate

Retain only changes that are score/search-contract exact, portable, pass the
frozen model-byte/peak-memory/load-time and p95/p99/max lateness ceilings, and
clear a preregistered minimum improvement in combined production throughput or
latency. The final fixed-time match, not a microbenchmark, decides whether the
tax is closed.

