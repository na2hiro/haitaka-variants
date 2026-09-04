## Phase R9: selection and promotion

### Harness qualification

Before testing a candidate:

- A-versus-identical-A paired intervals must lie wholly inside the frozen
  zero-bias equivalence margin and pass color/order reversal and deterministic
  replay checks. Here deterministic replay means identical schedule, identity,
  raw aggregation, and fixed-node decisions; timing-noisy fixed-time games are
  tested statistically, not required to be bit-identical. For distinct A/B
  order reversals, sign-transformed estimates must agree inside their frozen
  tolerance.
- Paired-game and pentanomial accounting must be verified from raw games. Every
  scheduled pair must be complete or deterministically retried, have a unique
  index, use the identical opening/start SFEN within the pair, and reverse
  colors and engine order. Missing, duplicate, or malformed pairs fail the run;
  they are never silently excluded. Recompute all bins from raw games.
- Development match openings and sealed promotion openings must be versioned,
  broad, independent of all generation sources, and non-overlapping.
- Match executable, model hashes, hardware, worker count, time source, search
  limits, adjudication, maximum plies, seed, and stopping rule are frozen.
- Resolved threads, CPU model, affinity, compiler flags, memory configuration,
  and concurrency level are included in the match identity; a report cannot be
  resumed or combined when any differ.
- The primary qualification and promotion condition is one concurrent game on
  a controlled, quiescent production-class host. Parallel matches are separate
  throughput studies and cannot replace or be pooled with this condition.
- The match must exercise the shipped WASM artifact through its production
  interface and complete search path, including root DFPN, timers, TT behavior,
  repetition history, and fallback policy. A native in-process proxy is allowed
  for development only after an end-to-end equivalence corpus verifies its
  search decisions, scores, completion metadata, and clock semantics; it never
  substitutes for the final production-interface match.
- A maximum-ply match outcome is reported distinctly; it is not silently
  converted for training use.
- Predeclare numeric model-size, native/WASM peak-memory, load-time, per-move
  p95/p99/maximum lateness, and evaluator-symmetry bounds. A nominal 100 ms
  match that violates them is invalid.

Development games may use either a fixed number of complete pairs, a validated
pair/pentanomial LLR with preregistered `H0`, `H1`, alpha, beta, minimum and
maximum pairs, or an anytime-valid confidence sequence. The existing unpaired
W/D/L SPRT and unvalidated approximate delta-normal paired interval are not
promotion statistics. Sequentially stopped runs report their declared decision
or anytime-valid bound, never an ordinary post-stop 95% interval.

### Promotion sequence

1. From development evidence alone, designate model A as the deployment
   candidate. Train model B as a replication of the same frozen recipe with
   independent data-generation and optimizer seeds; it is not an alternative
   chosen later by whichever sealed Elo is higher.
2. Run the small fixed-node tactical/search suite and equal-node paired matches
   against C/16 and handcrafted for both models on development evidence.
3. Run the target 100 ms/move development screen for both models under a valid
   decision mode.
4. Before opening either final suite, reproduce parity, combined throughput,
   latency, verifier, tactical, memory, load-time, clock, and complete
   production-search-path gates in the shipped WASM build and on a second
   supported host. Then preregister and freeze both final model hashes.
5. Open the final sealed static/search set once for both frozen models and apply
   its preregistered, multiplicity-adjusted gates. A failure rejects the model;
   it does not reopen the recipe.
6. Run model A through the production interface on final sealed promotion block
   A using a fixed, preregistered number of complete opening pairs with no
   interim inspection.
7. Run model B on disjoint final sealed promotion block B under the same frozen
   protocol. Neither result may change the recipe; a failed model requires a
   new candidate and a fresh future block.

### Final gate

For both independent models:

- the fixed-cap lower paired 95% confidence bound versus handcrafted at
  100 ms/move is above `0 Elo`, using a coverage-validated pair/pentanomial
  method that resamples or models the highest independent opening-source group
  and applies the preregistered multiplicity rule;
- no correctness, tactical, portability, memory, or runtime gate fails;
- the final sealed static/search gate passes without a post-unseal recipe
  change;
- zero searches return no move and zero searches use an unsearched emergency
  fallback;
- every scheduled pair is valid and complete, every game has an explicit legal
  termination reason, and all p95/p99/maximum time-overrun bounds pass for each
  evaluator;
- the decisive games exercised the actual shipped production artifact,
  interface, and full search path at one-game concurrency;
- no sealed evidence was used to alter the recipe.

Only then ship the preregistered model A and update the default recommendation.
Model B is replication evidence, not a second candidate eligible for
post-unseal selection.

Enforce this in code. Selection may nominate a candidate, but promotion must
fail closed unless the machine-readable result satisfies every required bound,
identity, replication, and zero-fallback condition. Merely running a
handcrafted benchmark is not a promotion gate.

