# Step 9 post-confirmation diagnosis runbook

Created: 2026-09-14. Status: ready for delegation. Scope: bounded exploratory diagnosis after the failed Step 9 confirmation.

## Assignment and completion target

When delegated, execute this runbook as a CPU-only diagnosis of the failed Step 9 confirmation. Produce validated development comparisons and a report identifying which future hypothesis, if any, merits a new candidate lane. Stop after the deliverables in this document.

Do not train a model, rent a GPU, deploy NNUE, modify production evaluator defaults, reopen the failed confirmation decision, generate future confirmation suites, or continue into a new experiment. Handcrafted remains production. Every new match and analysis here is exploratory development evidence.

An independent Astra medium protocol review confirmed the proposed crossover and u768 diagnosis with these requirements:

- use the same deterministic 512-opening subset of each suite for every model;
- recompute the historical diagonal cells on those exact subsets instead of comparing new 512-pair results with the full 2,048-pair aggregates;
- audit existing artifacts before spending match time;
- treat checkpoint deterioration as a hypothesis rather than a conclusion;
- make no causal claim that two models can isolate corpus, initialization, training, or session effects;
- do not extend samples, add checkpoints, or change the schedule after seeing scores;
- allow an inconclusive diagnosis as a complete result.

## Immutable result and scope

The completed native MacBook Pro confirmation at 100 ms per move used 4,096 games and 2,048 color-swapped opening pairs for each model:

| Model | Suite | W–L–D | Pair score | One-sided 95% lower bound |
| --- | --- | ---: | ---: | ---: |
| A u2,688 | A | 2,324–1,771–1 | 56.7505% | 55.5928% |
| B u2,688 | B | 1,992–2,103–1 | 48.6450% | 47.4931% |

The conjunction failed because both models had to pass. All 8,192 games validated. This decision is final for that candidate lane.

Read and preserve these records:

- `plans/anhoku-nnue-strength-first-plan.md`
- `r0/anhoku-strength-first/step9-result-report.md`
- `r0/anhoku-strength-first/step9-result-report.json`
- `r0/anhoku-strength-first/step9-confirmation-100ms-contract-v1.json`
- `r0/anhoku-strength-first/step9-confirmation-100ms-execution-v1.json`
- `scripts/strength_first_step9_confirmation_100ms.py`
- `out/anhoku-strength-first/step9-independent-confirmation-100ms-20260913-v1/`

The last directory contains the validated result and the complete raw A-on-A and B-u2,688-on-B schedules. Verify its recorded hashes before using it.

The directory `out/anhoku-strength-first/step9-independent-confirmation-20260913-abandoned-1000ms-partial-unopened` remains unopened and excluded. Do not inspect, summarize, or use its outcomes.

The opened 100 ms suites and results are now development history. Nothing in this diagnosis establishes strength at 1,000 ms, shipped-WASM strength, or deployment qualification.

## Machines and preservation

Coordinator checkout:

```text
/home/na2hiro/proj/haitaka-variants-anhoku
```

Last verified MBP execution environment:

```text
endpoint: 192.168.0.12
checkout: /Users/na2hiro/anhoku-step9-model-b-c3109e8
confirmation checkout HEAD: 2e8e45db3bd7bb473a52e63e5ae4be86ef10628c
architecture: arm64
```

`AGENTS.md` also records `192.168.0.2` as the MBP address. Verify host identity with `hostname`, `uname -m`, and checkout inspection. Do not silently use a different computer if the address changes.

Run all CPU-intensive matches, corpus scans, parity work, and model evaluation on the MBP. Do not use the MBA because its owner is using it. Do not run another match or heavy audit concurrently with fixed-time games. Fixed-time games use one game at a time and one engine thread. Keep the MBP plugged in and use `caffeinate` only for the lifetime of the owned runner. Do not stop unrelated processes.

Inspect the coordinator and MBP working trees before editing. Preserve unrelated changes, including coordinator `AGENTS.md` and the known MBP changes to `haitaka_learn/src/openings.rs` and `haitaka_learn/src/strength_first_model_b.rs`. Do not reset or clean either checkout, delete historical artifacts, or build over the frozen runtime.

No GPU is needed. Any future GPU rental requires explicit user permission under `AGENTS.md`.

## Bound artifact identities

| Input | Path | SHA-256 |
| --- | --- | --- |
| A u2,688 | `out/anhoku-strength-first/step8-compact-seed-replication-20260910-v1/exports/selected-u00002688.nnue` | `6c4f31ad5441aabe5b0e3feeefa411cc7b5bcee194ff896b2662f26c5069a8fa` |
| B u2,688 | `out/anhoku-strength-first/step9-model-b-training-20260912-v1/exports/candidates/candidate-u00002688.nnue` | `0ecde2640520e5b2566527c4098decbb1050f729d8999188435c6d8b258fe221` |
| B u768 | `out/anhoku-strength-first/step9-model-b-training-20260912-v1/exports/selected-u00000768.nnue` | `ba4cff98b0a94607ba92367888606d37fc5621a41c3b861558c013eb7117e6ef` |
| Suite A | `haitaka_learn/openings/anhoku-strength-confirmation-100ms-a-v1.tsv` | `66fd0e8c365d85f965d02e214738a48bed23d7905f7bb3e2b45dfc3821c10e61` |
| Suite B | `haitaka_learn/openings/anhoku-strength-confirmation-100ms-b-v1.tsv` | `53f1cd689da3615317144ef958ba731986191fbb6847e6b3e6df3f79231377cf` |
| MBP runtime | `target/release/haitaka_cli` | `ad2e83f3921209c4006e3e23bdd105d951cd1919580560449cdff8fab2423009` |
| Preserved runtime copy | `out/anhoku-strength-first/step9-model-b-training-20260912-v1/runtime/haitaka_cli-u00000256` | `ad2e83f3921209c4006e3e23bdd105d951cd1919580560449cdff8fab2423009` |

Verify every hash before games. Filenames do not establish identity. B u768 already has a passed parity receipt at:

```text
out/anhoku-strength-first/step9-model-b-training-20260912-v1/final-u00000768/parity-receipt-u00000768.json
```

Require it to bind the exact u768 network and frozen runtime hashes above. Do not substitute another export or checkpoint.

## Output layout and preflight

Use this fresh output root:

```text
out/anhoku-strength-first/step9-post-confirmation-diagnosis-20260914-v1/
```

Refuse a nonempty root unless resuming its hash-verified manifest. Never overwrite an attempt. Create:

```text
manifest.json
status.json
schedules/a-selected.tsv
schedules/a-selected.sfen
schedules/a-selection.json
schedules/b-selected.tsv
schedules/b-selected.sfen
schedules/b-selection.json
audit/
matches/<cell>/block-<0..3>/attempt-<n>/
analysis/
report.md
```

`manifest.json` must bind scope, input hashes, original evidence, host and runtime identities, selection algorithm and outputs, exact block order, seeds, limits, runner, analyzer, and tool versions. Save and hash the manifest, runner, and analyzer before launching any match. Keep the historical evidence immutable; reference or copy it without rewriting the original files.

Implement a separate diagnostic analyzer by reusing the frozen confirmation analyzer's validation principles. Do not edit the frozen analyzer or relax its requirements. Before launching, use lightweight synthetic fixtures to prove rejection of:

- a missing or duplicate game or pair;
- wrong network, suite, SFEN, color order, seed, or command settings;
- malformed or inconsistent WDL/result data;
- a non-null failure state or any aggregate/per-engine fallback.

## Phase 1: bounded existing-artifact audit

Complete this audit before starting match computation:

1. Read the Step 8 and Step 9 experiment JSONs, their completed `run.json` files, validation histories, selection histories, export records, and parity records. Do not treat stale preflight authorization/status fields as completed-run evidence.
2. Compare the actual feature family, width, objective, output scale, optimizer, schedule, update count, data ordering/exposure, corpus manifests, exporter, runtime, and seed. List every match and difference. Model A's training seed is `2026091002`; Model B's is `2026091108`. Do not assume corpus and seed are the only differences without verifying them.
3. Read `r0/anhoku-strength-first/step9-model-b-data-contract-v2.json` and the corpus manifests referenced by the resolved configurations. Prefer existing audit summaries.
4. If summaries are insufficient, make one streaming MBP pass over each relevant train/validation sidecar. Expensive record parsing must use a deterministic sample capped at 10,000 records per split. Record the sample rule and unavailable fields.
5. Compare available phase/material, side-to-move, check/tactical, donor-state, label quantiles/sign/clipping, duplicate/source-group, and exclusion summaries. Never guess binary fields; locate and use repository readers.
6. Compare existing loss, saturation, output-scale, and update-magnitude diagnostics for B u768 and B u2,688.
7. Cross-corpus static validation is optional only when existing tooling supports the exact objective and score orientation. Cap it at 2,048 deterministically selected positions for each model/corpus combination and label it exploratory.

If the audit finds a concrete identity, export, parity, orientation, recipe, or data-integrity defect, stop before matches and report it. A real correctness defect is more actionable than another match. Otherwise continue to the frozen diagnostic schedule.

## Phase 2: deterministic matched opening subsets

For each full suite independently:

1. Verify its full-file hash from the identity table.
2. Parse every non-comment row strictly as one nonempty ID, a tab, and one nonempty SFEN.
3. Require exactly 2,048 unique rows and unique IDs.
4. For each row, compute SHA-256 over this exact UTF-8 byte sequence, including the two newline characters:

   ```text
   step9-diagnosis-v1\n<FULL_SUITE_SHA256>\n<OPENING_ID>
   ```

5. Rank by the 64-character lowercase digest, breaking a hypothetical tie by opening ID.
6. Select the 512 lowest-ranked rows, then restore their original suite order.
7. Record original suite index, ID, SFEN, rank digest, selected index, and block index in `schedules/<suite>-selection.json`.
8. Write the selected TSV and SFEN files and bind their hashes in `manifest.json`.

The same selected opening IDs must be used for every model on a given suite. Divide each selected suite in restored order into four fixed consecutive blocks of 128 pairs. The analyzer must recover historical A-u2,688-on-A and B-u2,688-on-B pair scores for these exact selected original-suite indexes. Never compare a 512-pair cell with the full 2,048-pair mean as though they were matched.

## Phase 3: fixed match schedule

Run these four new cells, totaling 4,096 new games:

| Cell ID | Model | Suite | Pairs | Games |
| --- | --- | --- | ---: | ---: |
| `a2688-b` | A u2,688 | selected B | 512 | 1,024 |
| `b2688-a` | B u2,688 | selected A | 512 | 1,024 |
| `b768-a` | B u768 | selected A | 512 | 1,024 |
| `b768-b` | B u768 | selected B | 512 | 1,024 |

Each cell has four 128-pair, 256-game resumable blocks. Freeze and use this interleaved execution order to distribute session drift:

| Execution position | Cell | Block |
| ---: | --- | ---: |
| 1 | `a2688-b` | 0 |
| 2 | `b2688-a` | 0 |
| 3 | `b768-b` | 0 |
| 4 | `b768-a` | 0 |
| 5 | `b2688-a` | 1 |
| 6 | `b768-b` | 1 |
| 7 | `b768-a` | 1 |
| 8 | `a2688-b` | 1 |
| 9 | `b768-b` | 2 |
| 10 | `b768-a` | 2 |
| 11 | `a2688-b` | 2 |
| 12 | `b2688-a` | 2 |
| 13 | `b768-a` | 3 |
| 14 | `a2688-b` | 3 |
| 15 | `b2688-a` | 3 |
| 16 | `b768-b` | 3 |

Suite A uses seed `2026091301`; suite B uses seed `2026091302`. With sequential openings and zero random plies, the seed should not affect the positions, but keep it bound to match the historical commands.

Do not add checkpoints, 1,000 ms matches, sample extensions, or score-dependent stopping. Do not inspect scores until all four fixed cells complete; progress counts, failures, and elapsed time may be inspected.

Verify CLI syntax on the MBP with:

```bash
target/release/haitaka_cli self-play --help
```

Also inspect the preserved runner at:

```text
out/anhoku-strength-first/step9-model-b-training-20260912-v1/mbp-confirmation-100ms.sh
```

Each block must resolve to this command shape:

```bash
"$RUNTIME" self-play \
  --games 256 --threads 1 \
  --a-eval nnue --b-eval handcrafted --a-nnue "$NETWORK" \
  --movetime-ms 100 \
  --openings "$BLOCK_SFEN" --opening-order sequential \
  --opening-random-plies 0 --seed "$SUITE_SEED" \
  --max-plies 200 --report-dir "$ATTEMPT_DIR/native-raw"
```

Do not change the runtime or settings because a flag fails. Diagnose the identity or syntax mismatch first. Estimate remaining time from completed valid blocks and observed wall time; do not promise a fixed ETA.

## Execution, interruption, and resume

Use one unattended serial runner that stops on a process or validation failure. Record the owned runner/game PIDs and active block in `status.json`. Tie `caffeinate` to the runner lifetime. The delegating agent need not poll while games run.

After every block, run the validator automatically and validate:

- the exact command, runtime, network, suite, block, and seed bindings;
- 256 unique ordered game indexes and 128 complete pairs;
- alternating candidate colors within every pair;
- exact selected SFEN and original-suite mapping;
- legal result vocabulary, consistent winner and result, moves/plies, and termination data;
- null failure state and zero aggregate/per-engine fallbacks;
- recomputed WDL and score equal the raw report.

Until all four cells finish, the validator and runner may expose only pass/fail state, completed counts, active block, and timing. They must not print, tail, or summarize WDL, scores, pair scores, or outcome-bearing game records. Store those values in sealed block artifacts for the final analysis.

Record maximum-ply games and preserve all draws. An unexpected termination format is an error to diagnose, not a result to discard.

On interruption, send Ctrl+C/SIGINT once to the owned current game process and wait briefly. If ignored, record that fact and use SIGTERM only for that owned process. Preserve the incomplete attempt and exclude it entirely. Rerun the whole 128-pair block under a new attempt directory; never splice partial files. Resume only completed blocks whose hashes and validation still pass.

Repeated failure from the same cause blocks further execution. Report it rather than weakening validation or changing the schedule.

## Phase 4: analysis

For each opening, aggregate its two color-swapped games into candidate pair score: win `1`, draw `0.5`, loss `0`. Report the six model/suite cells below, marking the two historical subsets and their earlier execution session:

| Model | Suite A | Suite B |
| --- | --- | --- |
| A u2,688 | historical selected 512-pair subset | new `a2688-b` |
| B u2,688 | new `b2688-a` | historical selected 512-pair subset |
| B u768 | new `b768-a` | new `b768-b` |

Compute these paired contrasts on identical suite/opening IDs:

- A u2,688 minus B u2,688 on suite A;
- A u2,688 minus B u2,688 on suite B;
- the suite-balanced average of those two contrasts, weighting suites equally;
- B u768 minus B u2,688 on suite A;
- B u768 minus B u2,688 on suite B;
- the suite-balanced average of those two contrasts, weighting suites equally.

Use opening pairs as the independent units, never individual games. Report two-sided 95% descriptive intervals from a paired opening-level bootstrap with a fixed manifest-bound seed and 10,000 replicates. Preserve every compared model's scores together when resampling an opening. Resample suites separately for an equal-weight suite-balanced average. Label every interval exploratory and unadjusted for multiple comparisons.

The two historical diagonals ran in an earlier session. Contrasts containing them retain session and thermal confounding. The crossovers may reveal model-by-suite interaction, but two trained models cannot causally separate corpus, initialization, recipe, or session effects. State these limits prominently.

## Interpretation and stopping rules

| Observed pattern | Supported next hypothesis | Still unresolved |
| --- | --- | --- |
| A exceeds B u2,688 on both suites | B-model generalization weakness | Corpus, seed, recipe, and session attribution |
| Rankings change materially by suite | Model-by-opening-distribution interaction | Sampling noise and historical-session effects |
| B u768 exceeds B u2,688 consistently | Earlier checkpoint may generalize better | Overtraining, checkpoint-selection variance, and other changes |
| B u768 improves on only one suite | Checkpoint-by-suite interaction | Broad checkpoint superiority |
| Contrasts remain wide or inconsistent | Diagnosis inconclusive | More evidence needs a separately reviewed budget |
| Audit finds a concrete correctness defect | Fix that defect in a future lane | Strength after correction |

Stop after the bounded audit and four fixed cells. A favorable u768 result cannot rescue Step 9 or authorize promotion. Do not automatically propose more of the same data. An inconclusive result is a valid completion outcome.

## Required deliverables

Retrieve completed artifacts to the coordinator and verify transfer hashes. Add:

```text
r0/anhoku-strength-first/step9-diagnosis-report.json
r0/anhoku-strength-first/step9-diagnosis-report.md
```

The reports must reference the immutable diagnostic manifest and include:

- actual scope, runtime, host, hashes, exclusions, retries, and validation status;
- the full six-cell table and all matched contrasts with uncertainty;
- configuration, corpus, and model-audit observations;
- supported hypotheses and unresolved alternatives;
- one bounded next proposal, or a recommendation to stop NNUE work;
- explicit `deploymentAuthorized: false` and the unchanged failed-confirmation decision;
- completion status and any remaining owned processes.

If the initial audit finds a correctness defect, or execution cannot complete under the fixed protocol, write an early-stop report instead. Include all completed and validated non-outcome evidence, the exact defect or blocker, preserved attempt paths, and every unrun or unusable cell marked `unavailable`. Do not fabricate a complete six-cell table or inspect partial match outcomes merely to fill it.

Link the completed diagnosis report from `plans/anhoku-nnue-strength-first-plan.md`. Run relevant lightweight analyzer fixtures, machine-readable report validation, and `git diff --check`. Inspect the staged changes and commit only diagnosis-related files. Do not include unrelated workspace changes.

Stop after reporting. A fresh candidate lane requires its own prospective plan after this diagnosis is reviewed.
