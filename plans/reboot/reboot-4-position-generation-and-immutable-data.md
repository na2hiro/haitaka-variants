## Phase R4: position generation and immutable data

### Question

Can the workflow produce broad, independent positions cheaply, then label each
unique board once with the frozen R3 teacher?

### Two-stage pipeline

1. **Generate positions.** This stage has no training initialization and no
   label search.
2. **Canonicalize, split, deduplicate, and label.** The fixed label teacher sees
   each retained unique board once. Labels are cached and immutable.

This separation allows a cheap trajectory policy and an expensive label teacher
without conflating their identities.

### Position-source mixture

Create a predeclared mixture of independent sources:

- true root MultiPV near-best sampling, or a deterministic mixture of searched
  near-best moves and uniformly sampled legal moves;
- diverse randomized openings and legal perturbations;
- handcrafted-versus-handcrafted games at more than one search budget;
- phase-balanced samples and terminal-adjacent samples;
- later, candidate-disagreement and blunder replay positions.

Remove the current lexicographic first-16 truncation. It is acceptable not to
search every legal move; use true MultiPV or uniform candidate sampling whose
inclusion probability is defined and audited.

### Corpus tiers

- `debug`: 64k positions for parity, overfit, and resume tests.
- `pilot`: 1M unique labeled boards for recipe and exposure curves.
- `broad`: nested 10M, 50M, and 100M prefixes under one frozen generator and
  split policy.
- `deep-core`: a smaller, more expensive subset relabeled at the deep-reference
  budget for calibration and later hard-example mixing.
- `static-clone`: cheap legal positions labeled by the exact handcrafted static
  evaluator, used only as an initialization/representation lane.

At 50,000 nodes per label, 100M labels require at least `5e12` search nodes
before rollout overhead. A scale plan that ignores this is not executable.
Measure whether broad cheaper labels plus a deep-core subset dominate uniformly
expensive labeling.

### Record and manifest requirements

For every board retain or derive:

- canonical board hash and color-swap canonical hash;
- source game, opening group, trajectory policy, ply, and phase strata;
- trajectory evaluator and label evaluator identities separately;
- label budget, completed/incomplete state, root move/PV where representable,
  raw score, target transform version, mate metadata, and label hash;
- result status: terminal win/loss/draw, repetition/adjudicated, or unknown/capped;
- active feature-row counts and output bucket.

A v2 record or loss sidecar may carry the richer metadata. The initial
teacher-only baseline may keep the current binary payload only if unknown
results are ignored and all provenance is losslessly available in the sidecar.

### Data gates

- No canonical or color-swapped board crosses training validation,
  development, any one-shot stage block, final sealed static/search data, or
  promotion-opening source groups.
- No opening/trajectory group crosses a grouped split.
- Duplicate weights are capped and effective sample size is reported. Do not use
  a rigid uniqueness percentage as a substitute for coverage at large scale.
- Report source, side, ply, phase, score, material, check/tactical, result-status,
  donor-state, bucket, and active-row distributions.
- Report feature rows never observed and rows below the declared minimum update
  count; distinguish unreachable rows from coverage failures.
- Capped games are `unknown`, never `draw`.
- The current roughly 35% mate-label rejection rate is explained and either
  repaired with bounded mate targets or explicitly balanced in the retained
  distribution.
- Regenerating any shard from its manifest produces the same board and label
  hashes.

### Cost ceiling

R4 stops after the 1M pilot and publishes actual throughput plus projected 10M,
50M, and 100M cost. A failed pilot does not authorize scaling or an automatic
rerun; only the separately bounded R4-F route below may authorize more work.
R5 must pass before larger generation.

### R4 implementation record (failed closed, 2026-09-03)

- Froze `r0/anhoku-reboot/r4-data-contract.json` before production generation
  (SHA-256 `81c9b6baec8bf6bafd01cf4370e432cfc4a4f60e78b417dce02096d57ebe1339`)
  and added the two-stage `r4-generate-positions`, `r4-assemble-positions`,
  `r4-label-shard`, `r4-verify-regeneration`, and `r4-finalize` workflow.
- Generated 16,000 games in 160 immutable shards on nubu, the current Ubuntu
  host, MBA, and MBP using an exact 8/16/56/80 shard allocation. The resulting
  1,590,513 candidate records contained 69,129 exact duplicates and 6,500
  color-swap duplicates, leaving 1,514,884 unique color-swap families.
- Assembled exactly 1,000,000 unique pilot positions with frozen
  880,000/60,000/60,000 training/validation/development and
  150,000/500,000/300,000/50,000 early/middle/late/terminal-adjacent quotas.
  Opening-group overlap and cross-split color-swap overlap were both zero;
  duplicate weight was one. A 64,000-position exact-static clone and a
  4,096-position deep-reference core were also produced.
- Labeled the pilot with the R3-frozen handcrafted 10,000-node teacher. Host
  shares including deep-core were 50,256/100,512/351,280/502,048 positions,
  preserving the requested approximate 1:2:7:10 nubu/Ubuntu/MBA/MBP ratio.
  All shards used label source revision
  `d1e31faae753a226e67b01a35035ca5548ee3cd7bb165dd556a4d7a35488c38b`.
- The pilot corpus exposed 82,799 searches (8.2799%) that exhausted the
  node budget before completing depth one. They are recorded with null raw and
  bounded scores and `unknown/incomplete` mate state; no partial-root value or
  draw-like zero entered a trainer payload. This leaves 806,359 training,
  55,736 validation, and 55,106 development numeric labels. Deep-core retained
  4,086 of 4,096 labels after ten analogous incomplete searches.
- The gate report records `mateLabels=219,028`, but the current reporter counts
  all 82,799 `unknown/incomplete` rows as mate labels because their mate class
  is not `none`. The raw labels contain 136,229 actual bounded win/loss mate
  labels. It also computes feature-row coverage over all 1,000,000 attempts,
  whereas trainer payloads contain only completed labels, and calls
  `deep-core-complete` true from 4,096 attempts even though only 4,086 finite
  deep-core records were written. R4-F must correct these telemetry and gate
  semantics and recompute retained-record coverage; none of the reporting
  defects supplied a target for an incomplete row.
- For the 1M pilot tier, measured throughput was 14.2664 attempts per
  worker-second, 19.4707 core-hours, and `$3.8941` at the frozen
  `$0.20`/core-hour model. The 4,096 deep-core attempts add 1.2322 core-hours
  and `$0.2464`, making total pilot-plus-deep-core search 20.7029 core-hours and
  `$4.1406`. The projected costs of `$38.94` for 10M, `$194.71` for 50M, and
  `$389.41` for 100M are per attempted pilot-tier position. At the observed
  91.7201% finite-label yield, the corresponding mechanical estimates per
  10M/50M/100M finite trainer records would be
  `$42.46`/`$212.28`/`$424.57`; these are diagnostics, not authorization to
  scale. Zero node-accounting failures were observed.
- Byte-identical position regeneration passed for Ubuntu/x86_64 shard 8 and
  MBP/arm64 shard 80. The implemented proof does not replay their labels and
  therefore does not yet satisfy the prose requirement to reproduce both board
  and label hashes. The R4 gate report is
  `out/anhoku-reboot-r4/r4-gate-report.json` (SHA-256
  `2fa26d4629c2ddf3fa1f9f61632f3d29558929eb67ea736ae4957598b3d04fc7`):
  sixteen of seventeen named gates report true and only
  `finite-labels-complete` is false. Therefore R5 is not authorized. No
  10M/50M/100M broad tier, training run, or strength game was started.

### R4-F finite-label remediation and next authorized assignment (required, 2026-09-03)

The R4 v1 contract, position corpus, label attempts, datasets, and failed gate
report are immutable failed history. They may be used as diagnostic/tuning
evidence, but the 917,201-row filtered payload is not a passing 1M pilot and is
not eligible for R5. Do not overwrite the report, reinterpret the failed gate,
or substitute a partial score, artificial draw, or static value for an
incomplete search.

The failure is not missing completely at random. A read-only audit of all pilot
labels found incomplete rates of 0.0193%/2.8982%/18.664%/24.574% for
early/middle/late/terminal-adjacent positions and 9.4091% versus 0.2727% for
tactical versus non-tactical positions. Source rates ranged from 4.7878% to
12.9014%, while arm64 and x86_64 rates were 8.2782% and 8.2893%. The incomplete
records had exact node accounting, depth zero, a partial iteration, and almost
all of their 10,000 nodes in qsearch. Consequently, silently accepting the
filtered payload or backfilling without a prospective bias gate would change
the intended data mixture.

The next worker must implement only the following bounded R4-F assignment, in
order:

1. **Publish the failure analysis.** Bind a machine-readable
   `out/anhoku-reboot-r4f/incomplete-analysis.json` to the original R4 contract,
   all 160 position-shard manifests and data hashes, assembly report, label
   manifests and data hashes, dataset manifests, and gate report. Report
   attempted, completed, and incomplete counts and rates by split,
   source/opening group, phase/ply, side, material, check/tactical state, result
   status, donor state, output bucket, active-row strata, host, and architecture.
   Include completed-depth, alpha-beta-node, qsearch-node, and total-node
   distributions; compare attempted with retained distributions; correct mate
   accounting; distinguish attempted from finite deep-core counts; and
   recompute feature-row coverage using only finite trainer records.
2. **Preregister before any new search.** Freeze
   `r0/anhoku-reboot/r4f-label-completeness-contract.json`. Existing R4 labels,
   including all known failures, are tuning evidence only. Select a
   hash-determined confirmation set from previously unlabeled members of the
   frozen 1,514,884-family pool, without consulting labels or sealed R5/R9
   evidence. Before selection, publish and bind a canonical unused-pool
   manifest/hash derived from all frozen position shards; the assembly count
   alone is not a pool identity. Freeze the candidate fixed budgets and any
   label-specific bounded qsearch policy, reference budget, sample and strata,
   RNG seeds, completion and target-quality metrics, missingness and
   retained-distribution limits, feature-coverage limits, selection rule, and
   board-policy-evaluation/node/core-hour/dollar ceilings. Across all replay,
   candidate, and reference lanes, the probe may use at most 50,000 total
   board-policy search evaluations and no more than 50,000 distinct previously
   unlabeled boards. A policy is not selected merely because it finishes more
   searches; it must also pass the frozen deep-reference, determinism,
   node-accounting, distribution, and cost boundaries.
3. **Run the bounded probe, close out R4-F, and stop.** Publish hash-frozen raw
   results plus `out/anhoku-reboot-r4f/r4f-gate-report.json`, bound to the
   contract and analysis. Reproduce the current 10,000-node results on the
   declared replay subset, independently recompute every metric, and select
   exactly one route by the frozen rule. Use a versioned exact schema and the
   complete named-gate set `original-evidence-bound`, `analysis-recomputed`,
   `contract-frozen-before-search`, `fresh-confirmation-isolated`,
   `replay-deterministic`, `node-accounting-exact`, `target-quality`,
   `missingness-bias`, `retained-coverage`, `cost-ceiling`, `single-route`,
   `route-rule-recomputed`, and `no-downstream-work`; reject a missing, extra,
   or non-Boolean gate. The required `selectedRoute` enum is exactly one of
   `r1-d1`, `r1-d2`, `r3`, `replacement-r4`, or `stop`.

   Set `closeoutValid=true` if and only if
   `original-evidence-bound`, `analysis-recomputed`,
   `contract-frozen-before-search`, `fresh-confirmation-isolated`,
   `cost-ceiling`, `single-route`, `route-rule-recomputed`, and
   `no-downstream-work` are all true. Set `routeAuthorized=true` if and only if
   `closeoutValid=true` and the selected route exactly matches independent
   recomputation of the frozen rule. A report with `closeoutValid=false` has no
   authoritative route, even if `selectedRoute` is populated. Set top-level
   `passed=true` if and only if all named gates are true, and set
   `replacementR4Authorized=true` if and only if `passed=true`,
   `routeAuthorized=true`, and `selectedRoute=replacement-r4`.

   That direct route requires the complete R3 label-policy identity to remain
   unchanged: teacher/source identity, 10,000-node pilot budget, 250,000-node
   deep-reference/deep-core budget, search/qsearch and node-counting semantics,
   target semantics, score transform, and loss. The report never records
   `r5Authorized=true`.

Only an R4-F report with `closeoutValid=true` and `routeAuthorized=true`
determines a separate assignment. Otherwise repair or rerun R4-F without
starting any routed work. A valid closeout routes as follows:

- A search-semantic, determinism, interruption, or accounting defect invalidates
  the affected evidence and routes to R1-D1, or to R1-D2 when qsearch/history
  correctness is implicated. R1 must pass again before returning through R3.
- Any code or semantic change to teacher search or qsearch first requalifies the
  affected R1-D1/R1-D2 gates. A required change to label budget, teacher search,
  qsearch policy, target semantics, or score transform then routes to R3. Pass
  a new preregistered teacher decision gate before replacement R4 labeling;
  R4-F itself cannot authorize that labeling.
- Only if the unchanged R3 teacher passes fresh R4-F confirmation may the
  R4-F report authorize a replacement R4 contract with deterministic,
  label-value-blind rejection and replenishment from the frozen unused
  candidate pool. That contract must predeclare the ordering,
  split/phase/source quotas, attempt ceiling, maximum incomplete rate, and
  retained-versus-attempted bias bounds.
- If no affordable route passes, remain stopped in R3/R4-F rather than weakening
  a gate.

Any subsequently authorized replacement R4 assignment must produce new
evidence rather than bless the failed run: write a versioned contract, registry
entry, output directory, manifests, datasets, regeneration proofs, cost
forecast, and gate report. The replacement contract and report must bind and
fully validate the original failed R4 evidence, the authorizing R4-F contract
and report, and every intervening replacement R1/R3 report. The direct route is
valid only when R4-F selected `replacement-r4`; an R1/R3 route instead requires
the affected replacement R1 gates and a new passing R3 report with
`r4Authorized=true`. Do not trust only a top-level authorization Boolean or
accept a missing/mismatched link.

Existing completed labels may be imported only when the replacement manifest
binds their original manifest/data hashes and the teacher, budget, target, and
label-policy identity is byte-for-byte unchanged; do not rewrite them as if the
replacement contract produced them. New labels on the direct unchanged-teacher
route must use the archived original labeling source/executables on their
declared architectures, or pass a preregistered bit-exact label-replay
equivalence gate for a replacement executable. A matching evaluator name is
not sufficient. Otherwise relabel under the newly R3-qualified identity. Never
relabel only the known failures at a larger budget while retaining the old
10,000-node labels, because that creates a difficulty-conditioned mixed
teacher.

The replacement schema must keep an immutable ledger of every attempt separate
from its accepted finite cohort. It must contain exactly
880,000/60,000/60,000 finite training/validation/development records and 4,096
finite deep-core records, preserve grouped and color-swap split isolation and
the declared mixtures, report every incomplete attempt with null targets,
recompute coverage on retained records, and state costs both per attempt and
per finite record. Use separate, honestly named gates for accepted finite-cohort
completeness, finite deep-core completeness, and the preregistered maximum
incomplete-attempt rate; never reuse the failed v1 `finite-labels-complete` name
with weaker semantics. Regeneration evidence must reproduce both position and
label hashes for imported and newly produced paths under the declared
cross-architecture scope, not only regenerate positions.

R5 becomes authorized only when the new report validates the complete
authorization chain above, has every named replacement gate true, records
`passed=true` and `r5Authorized=true`, and identifies the sole eligible 1M
dataset manifest. Stop after that replacement closeout; R5 is a separate
assignment.

Before execution, this R4-F assignment authorized only analysis,
implementation/tests needed for its reporter and gate, and the bounded probe
above. It did not itself authorize a replacement 1M labeling run, the
10M/50M/100M broad tier, R5 training, GPU rental, feature or architecture
changes, or any strength game.

### R4-F and replacement R4 implementation record (complete, 2026-09-04)

R4-F was executed fail-closed. The bound analysis is
`out/anhoku-reboot-r4f/incomplete-analysis.json` (SHA-256
`e72f5a8011560d0f744b486ee33f2d4c7d5b7af458cf59c54d8a3c167a307be3`),
and the canonical unused-pool manifest is
`out/anhoku-reboot-r4f/unused-pool-manifest.json` (SHA-256
`3729938a89aaf6b98f7ce10a77ae5bb433c0c8c969aecfe8af01e8a6518e0e81`).
The R4-F report (SHA-256
`3fb6b1bc5644243200e833c3c59831b97415f8ef644044b2620be3f3b5731e7c`)
had a valid closeout and authoritative `r3` route, but remained failed overall
because the direct-route retained-coverage gate was false. It did not authorize
R5 or replacement labeling by itself.

The separately preregistered R3 requalification then passed all ten gates with
the original teacher, 10,000/250,000-node budgets, search semantics, target,
transform, and loss unchanged. Its report is
`out/anhoku-reboot-r4f-r3/r3-gate-report.json` (SHA-256
`31152a5a3e6a002dee1c6b034721350a0d5b825ba7b47476ea6e14e5bdb9e053`)
and records `r4Authorized=true`.

Replacement R4 was subsequently executed under the frozen v8 contract
`r0/anhoku-reboot/r4-replacement-contract.json` (SHA-256
`bbd9a0924702114f8cf542f559d7affc99870e7d4b10f7783fc3f529fa28356b`).
Versions 1 through 7 remain preserved in separately named failed-evidence
directories. The v8 coverage comparison uses the finite-trainer baseline
published by R4-F rather than the original attempted-position telemetry: the
replacement reduced never-observed reachable rows from 36,935 to 36,065 and
below-minimum reachable rows from 19,764 to 17,397, while donor-row measures
did not regress (236 never-observed and 6 below-minimum in both).

The replacement attempted 131,261 new pilot labels, of which 28,713 were
incomplete, and 16 new deep labels. Across imported and new pilot attempts the
incomplete rate was 111,512/1,131,261 = 9.8573%, within the frozen 15% ceiling;
the conditional new-attempt rate is separately reported as 21.8747%. The
immutable ledger contains 1,135,373 attempts. The accepted finite cohort has
exactly 880,000 training, 60,000 validation, and 60,000 development records,
plus 4,096 finite deep-core records. Search accounting records 1,318,250,000
nodes, 3.672 core-hours, and $0.7344 estimated cost, with 10M/50M/100M
forecasts of $71.61/$358.07/$716.15.

The final report is
`out/anhoku-reboot-r4-replacement/r4-gate-report.json` (SHA-256
`2b3b56a7f8ffc0a5d45902667b1b7299aa01f80e5692b0c7dcb2aa8399c64359`).
All 23 exact named gates are true; it records `passed=true`,
`r5Authorized=true`, and no training, strength game, or broad generation. The
sole R5-eligible dataset manifest is
`out/anhoku-reboot-r4-replacement/datasets/pilot-1m.json` (SHA-256
`1f52e6301c84931bec1ed43939f685da051d3647bb11478ebd53c71311025c5b`).
This closes R4-F and replacement R4. R5 is now unblocked but remains a separate
assignment.

