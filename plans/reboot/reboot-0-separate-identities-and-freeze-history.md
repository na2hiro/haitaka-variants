## Phase R0: separate identities and freeze history

### Question

Can a generation run be configured without any possibility that a training
warm start silently changes the trajectory or labels?

### Required changes

- Replace the shared `paths.bootstrap_nnue` meaning with three explicit types:

  - `training.initial_checkpoint`: `scratch`, full-precision checkpoint, or
    explicitly marked quantized-import diagnostic.
  - `generation.trajectory_evaluator`: evaluator kind, model hash if any, and
    trajectory-search parameters.
  - `generation.label_evaluator`: evaluator kind, model hash if any,
    label-search budget, target semantics, and score-transform version.

- Prefer separate generation and training config schemas. `generate-data` must
  not deserialize or inspect a training initialization field, and must reject a
  stray training-only field rather than silently ignoring it.
- Reject ambiguous legacy generation configs rather than guessing their intent.
- At the orchestration layer, add a regression test that varies a separate
  `TrainingConfig.initial_checkpoint` while holding one
  `CombinedDataGenerationConfig` fixed and proves byte-identical generated
  shards. The generator API itself must not accept a `TrainingConfig`.
- Use stage-specific manifests rather than one impossible all-purpose manifest:

  - transitional combined data generation, allowed only for historical import
    and R1-R3 debug/probe work: current generator executable/config, separately
    typed trajectory and label evaluators, seeds, and labeled output shards, but
    no training initialization;
  - training: trainer executable/config, input dataset-manifest hash,
    initialization/optimizer state, checkpoints, and export;
  - evaluation: model, harness, openings/suite, execution environment, and raw
    results;
  - composite registry record: immutable links among the stage manifests.

  R4 retires the transitional type for production data and adds distinct
  position-generation and labeling/dataset-assembly manifests with a lossless
  unlabeled-position artifact.

- Add an experiment registry. Each run records a hypothesis, changed variable,
  controls, cost ceiling, config hash, artifacts, outcome, and what the result
  does and does not prove. Every gate is machine-decisive: before results are
  opened, register its metric direction, baseline, minimum effect or
  noninferiority margin, uncertainty/decision rule, multiplicity handling, and
  cost ceiling.
- Mark historical datasets without deleting them:

  - Phase 8B: valid for duplicated-trajectory history; invalid for a
    diverse-256k claim.
  - Phase 8D/11: valid as a C/16 self-distillation and config-matched feature
    diagnostic in which no gain was established; invalid for a matched-example
    causal ablation or handcrafted-teacher claim.

- Mark historical match evidence without deleting it:

  - every in-process movetime match produced by the current no-move-loss path:
    non-decisional for strength, retention, or checkpoint selection;
  - Phase 8R equal-node match: diagnostic only because it contains 342 arbitrary
    fallback moves;
  - historical NPS: diagnostic only until reproduced at controlled single-game
    and declared parallel loads.

- Freeze the current C/16 network, current Phase 8D V1 candidate, handcrafted
  executable, match reports, and old plan by SHA-256.
- Freeze the target production execution specification before R5 width
  selection: shipped WASM interface and required complete search path, one
  concurrent game, supported device/host classes, clock policy, cold/warm state,
  and numerical ceilings for serialized bytes, peak memory, load time, and
  per-move latency. R1 repairs and qualifies the implementation; evaluation
  manifests later identify the actual artifact.

### Gate

R0 passes only when each stage manifest exposes exactly its own identities, the
composite registry links trajectory evaluator, label teacher, and initialization
without coupling them, a test demonstrates byte-identical generation after
changing only the separate training initialization, and historical
datasets/matches carry explicit claim-validity annotations. No new data run is
authorized before this passes.

### Implementation record (2026-09-01)

Implemented on branch `reboot`:

- Added strict, separate `CombinedDataGenerationConfig` and
  `TrainingWorkflowConfig` schemas. Generation commands reject training-only
  fields, legacy search fields, ambiguous label-on-sample behavior, and the
  identity-mismatch override.
- Added independently typed trajectory and label evaluators. They use separate
  teacher instances and search workspaces, so neither evaluator can inherit the
  training initialization.
- Added explicit `training.initial_checkpoint` variants for scratch,
  full-precision weights/resume state, and a hash-checked quantized-import
  diagnostic with an explicit transform version.
- Changed the public generation and training command paths to accept only their
  corresponding typed config. `pipeline` now requires separate generation and
  training config files. The ambiguous legacy `train-select` path is disabled.
- Added combined-generation, training, and evaluation manifest schemas plus a
  composite experiment registry with preregistered gates and immutable stage
  manifest links. Generation and training publish their stage manifests.
- Added historical claim-validity annotations for Phase 8B, Phase 8D/11,
  affected movetime matches, Phase 8R, and NPS evidence. Frozen R0 artifacts
  include C/16, the Phase 8D V1 candidate, the handcrafted executable,
  representative match reports, and the superseded plan, all checked by
  SHA-256.
- Added example strict generation and training configs and documented the new
  commands in `haitaka_learn/README.md`.
- Added an orchestration regression proving byte-identical train and validation
  shards when only a separate training initialization changes. Added strict
  schema, cross-stage-field, registry, and unresolved-production-policy tests.

CPU-only verification completed:

- `cargo check -p haitaka_learn --features anhoku`
- `cargo test -p haitaka_learn --features anhoku`
- focused R0 manifest/registry/production-policy tests
- focused byte-identical generation regression
- `cargo fmt --all -- --check` and `git diff --check`

R0 closeout completed after audit:

- Froze the production path as Shogitter Engine Package v1 using
  `wasm-bindgen` and `usi-wasm-v1` in one dedicated Web Worker. Positions are
  supplied through `position startpos|sfen ... moves ...`, preserving replay
  history, and searched with `go movetime 100` at one-game concurrency.
- Froze desktop Chromium/Firefox/Safari hosts with WebAssembly, ES modules,
  Workers, `performance.now`, and at least 1 GiB available to the Worker;
  supported CPU classes are x86_64 and arm64 with WASM SIMD128.
- Froze cold load as fresh Worker/module/NNUE instantiation and warm games as a
  persistent Worker/model with `usinewgame` clearing the TT before each game.
  The deadline uses `instant 0.1.13`'s WASM monotonic clock and is polled every
  256 alpha-beta or qsearch nodes.
- Froze ceilings of 192 MiB serialized model bytes, 512 MiB peak Worker memory,
  5,000 ms cold load, and 125 ms maximum per move at the 100 ms target. These
  are preregistered acceptance bounds; R1 must measure and qualify them, not
  revise them after results are opened.
- Made the experiment registry non-vacuous: every preregistration requires
  exactly one outcome and at least one hash-checked stage manifest. The R0
  outcome links a verification manifest that independently records the
  trajectory evaluator, label evaluator, both initialization identities, the
  exact strict generation config, test source, assertions, command, and result.
- Bound the preregistration to
  `haitaka_learn.anhoku-reboot-r0.generation.toml` by path, bytes, and SHA-256;
  the gate reparses it as a strict generation config and recomputes its source
  hash.
- Removed the strict `merge-data` CLI override and made its public API reject an
  identity-mismatch override before reading or writing merge artifacts.
- Added a machine-checked contamination report. The gate recomputes counts from
  hash-frozen raw JSONL using `incompleteIterations == plies + 1`: Phase 8D
  candidate-versus-handcrafted 330/1,024, C/16 retention 381/1,024, Phase 8B
  retention 1,771/4,096, Phase 8D anchored checkpoint ranking 1,795/4,096, and
  Phase 11 V2-versus-V1 1,822/4,096. It also recomputes Phase 8R's 342 fallback
  moves in 42/2,048 games.
- Added strict machine-readable R1 specifications for the deterministic parity
  corpus, generated sentinel network, and interruption-safe search contract in
  `r0/anhoku-reboot/r1-fixture-specifications.json`. These specify inputs,
  transitions, independent oracles, activation/export artifacts, interruption
  cases, result fields, and fail-closed pass rules without implementing R1.
- `target/debug/haitaka_learn r0-gate --bundle r0/anhoku-reboot` passed on
  2026-09-01 after independently validating the registry, frozen artifacts,
  raw contamination evidence, R1 fixture specifications, and production
  execution contract.

No GPU rental, strength match, or production data generation was performed
during R0. R1 correctness work is now authorized; R2 and production corpus
generation remain unauthorized until their own gates pass.

### Cost ceiling

CPU tests only. No rented GPU, strength match, or production data generation.

### What passing proves

It proves experiment attribution is possible. It does not prove that any
teacher or model is strong.

