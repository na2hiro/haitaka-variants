# Step 10 checkpoint-selection execution runbook

Prepared 2026-09-14 for a future delegated worker configured as **`gpt-5.6-luna`, reasoning effort `max`**. This document defines a gated prospective development experiment. **The design is approved; launch is unauthorized.** Reading this runbook, delegating it, completing review, or filling artifact bindings does not authorize opening generation, training, matches, or GPU rental. Obtain and record separate explicit user authorization for each applicable operation after its prerequisites are concrete and reviewable.

## 1. Assignment, authority, and stop conditions

Test whether a predeclared validation-loss selection rule capped at u1024 produces a stronger checkpoint than u2688 from the **same fresh training run**, against handcrafted at native MBP 1,000 ms/move. Complete only the phases authorized at handoff. A blocked preparation report is a valid handoff; do not interpret missing approval as permission to proceed.

Read these authoritative inputs in full before acting:

- [Step 10 contract](../r0/anhoku-strength-first/step10-checkpoint-selection-contract-v1.json): experiment parameters and boundaries.
- [Astra final review](../r0/anhoku-strength-first/step10-checkpoint-selection-review-v2.json): design approval and unresolved execution gates.
- [Main plan](anhoku-nnue-strength-first-plan.md): production timing policy and historical decision.
- [Prior diagnosis runbook](anhoku-nnue-strength-first-step9-diagnosis-runbook.md): preservation, sealing, and validation practices. Its 100 ms schedule and authorization do not apply to Step 10.
- Current `AGENTS.md`: machine preferences and explicit GPU-rental permission requirement.

The Step 10 contract controls this experiment's numeric parameters over generic historical plan instructions. Stop on an unresolved disagreement between authoritative records; do not silently repair the protocol. Record actual worker model/reasoning configuration. Do not claim Astra authored or reviewed an execution artifact unless an actual receipt identifies that artifact.

Never:

- Inspect, read, hash, summarize, copy, or pass to any tool the abandoned directory `out/anhoku-strength-first/step9-independent-confirmation-20260913-abandoned-1000ms-partial-unopened`. Avoid broad recursive inventories or hashes that would traverse it.
- Reopen, extend, or rescue the failed Step 9 confirmation; generate confirmation suites; change production evaluator defaults; deploy NNUE.
- Add seeds, checkpoints, strength screens, samples, or score-dependent stopping; select a checkpoint from match results.
- Replace frozen data, runtime, recipe, or missing artifacts with similarly named substitutes.

Handcrafted remains production. Every Step 10 result is exploratory development evidence. Native parity with WASM does not establish shipped-WASM strength or deployment qualification.

Stop immediately on an identity/hash mismatch, loader/trainer semantic mismatch, nonfinite loss, numerical failure, missing/invalid candidate or validation receipt, parity mismatch, opening overlap, invalid generation receipt, illegal move, fallback, missing result, malformed termination, or forecast exceeding a cap. Correctness failures cannot be auto-retried. Only documented owned-process interruptions have the bounded match recovery described below.

## 2. Immutable identities and a known provenance gate

Repository-relative paths below resolve from `/home/na2hiro/proj/haitaka-variants-anhoku` on the coordinator, or from a separately recorded isolated root on the execution host. Record full SHA-256 and byte count for every input and output in the manifest. A filename or commit alone does not establish artifact identity.

Observed input hashes at runbook preparation:

| File | SHA-256 |
| --- | --- |
| `r0/anhoku-strength-first/step10-checkpoint-selection-contract-v1.json` | `fa8ed0cbcbd573340cd43707a39fc7b17ba009ed53c54d42d2a8920d0c489649` |
| `r0/anhoku-strength-first/step10-checkpoint-selection-review-v2.json` | `73259d8d9896c5cabfef27fb4d01d95eafbe67870a522e47324967e426207bf4` |
| `plans/anhoku-nnue-strength-first-plan.md` | `f1bd5267a3afe2888fd0e513a2e7ec4acfa4fe9a64ca3172bc8650e7a39e537e` |
| `plans/anhoku-nnue-strength-first-step9-diagnosis-runbook.md` | `08c8075f4ce675dc2d010da6810be9ddf37ba00cee46420000ae9a5a335f81a0` |

**Known unresolved binding:** the final Astra receipt approves contract hash `e49936562e9af368637969c987045713391bd7dcc84e90c3f1b4ed29b77dc5be`, whereas the current contract bytes hash to `fa8ed0...`. The current contract also embeds that earlier reviewed hash. Do not assert these are identical or assume the difference is harmless metadata. Before launch, obtain the exact reviewed bytes and a verifiable diff/receipt reconciling them with the current contract, or obtain independent approval explicitly binding the current bytes. Preserve both records; do not rewrite the historical review. This runbook does not resolve that discrepancy or fill any authorization flag.

Freeze experiment ID `strength-first-step10-checkpoint-selection-20260914-v1`. The design basis commit is `1de1dbe`; record the actual implementation commit and relevant dirty-file hashes separately.

| Bound dependency | SHA-256 / identity |
| --- | --- |
| `r0/anhoku-strength-first/step9-diagnosis-report.json` | `2031bccd6be29043d065ad2f9c201c026267d5423b8642431a4d78fcdd62baab` |
| `r0/anhoku-strength-first/step9-model-b-training-experiment-v1.json` | `c838091b1a3163572b88bcd107d487ef4bb592fe4334c45c643800c7ee35d1fc` |
| `r0/anhoku-strength-first/step9-model-b-data-contract-v2.json` | `a6a949adf74c45a091ccea834a00e11df911bad87243f1868d0c2a41ab3ae6db` |
| Historical `scripts/strength_first_model_b_trainer.py` | `a54afc8cadbefa25096c59f3a7238c5b5a53ff367392792453708c9681e542d0` |
| Pinned external trainer revision | `61666d9e3653e4df9881b14c23f8fdcc4bf7779b` |
| External trainer `serialize.py` | `0b741a8e4e0c6759614fa70d069e51934a69cfa7e52792966b6e3eb47f79bce7` |
| `haitaka_learn/src/strength_first_model_b.rs` generator source | `30a474ecde43ae991bcbcb7816d5dc6233fdd6f8662bd3c506e373a75827e4af` |
| Preserved native runtime `out/anhoku-strength-first/step9-model-b-training-20260912-v1/runtime/haitaka_cli-u00000256` | `ad2e83f3921209c4006e3e23bdd105d951cd1919580560449cdff8fab2423009`, 11,372,128 bytes |

Resolve and record the external trainer checkout location; `serialize.py` is relative to that checkout, not necessarily this repository. The historical B wrapper hardcodes B's identity and schedule. It is provenance only and **must not launch Step 10**.

## 3. Workspace, hosts, and authorization ledger

First take read-only snapshots of coordinator HEAD, branch, `git status --short`, and relevant source hashes. Preserve all existing changes, including `AGENTS.md`, archives, unrelated scripts, and known historical MBP edits in `haitaka_learn/src/openings.rs` and `haitaka_learn/src/strength_first_model_b.rs`. Never reset, clean, overwrite historical outputs, or build over the frozen runtime. Use an isolated checkout/build directory with a recorded source basis and explicit copies of required modified sources. A clean checkout that omits required dirty source is not equivalent.

Designated match host: MBP, last endpoint `192.168.0.12`, `arm64`; historical checkout `/Users/na2hiro/anhoku-step9-model-b-c3109e8`, historical HEAD `2e8e45db3bd7bb473a52e63e5ae4be86ef10628c`. `AGENTS.md` also identifies the MBP as `192.168.0.2`. At future authorized remote preparation, verify hostname, architecture, checkout, runtime hash, and machine identity before use; address aliases are not permission to substitute a different host. Record the isolated absolute path. Prefer MBP for corpus scans, opening preparation, and parity; do not place heavy jobs on another person's active machine without establishing availability. Use no concurrent heavy work during fixed-time games, keep MBP plugged in, record thermal/session markers, and tie `caffeinate` only to the owned runner lifetime. Never terminate unrelated processes.

Use output root `out/anhoku-strength-first/step10-checkpoint-selection-20260914-v1/`, abbreviated `ROOT` below. Refuse an existing nonempty root for a fresh run. Resume only from its verified immutable manifest, authorization ledger, and preserved state. Do not erase a failed root to retry under the same identity.

Maintain a separate execution authorization ledger, preserving the design contract unchanged. For each grant record the exact user instruction/reference, timestamp, permitted phase, bound artifact hashes, host, budget, and limits. Required separate grants are `cpuOpeningGenerationAuthorized`, `trainingAuthorized`, `gpuRentalAuthorized`, and `cpuMatchAuthorized`. Initially all are false. User approval may explicitly cover several named phases, but one grant must not be inferred to cover another. Confirmation and deployment remain false permanently for this runbook. A quote is not rental approval. Recheck approval applicability after any artifact or budget change.

Recommended phase gates:

| Gate | Required evidence before advancing |
| --- | --- |
| Preparation | Design/review provenance reconciled; workspace preserved; authorized implementation scope established |
| Opening generation | Bound generator build and exclusion inputs; prepared deterministic validation gate; opening-generation user approval; budget available |
| Training | Frozen valid openings; bound Step 10 trainer/config; CPU fixtures pass; forecast fits; fresh quote and explicit training/rental approvals |
| Matches | Complete same-run training and selection; all candidate/control qualifications; frozen runner/analyzer/schedule; forecast fits; CPU-match user approval |
| Analysis | Both complete cells strictly validate and all eight accepted blocks remain hash-valid |

This sequencing freezes model-blind openings before training. Do authorized design and implementation work to make the next approval request concrete; do not launch an experimental phase to resolve its own missing gate.

## 4. Execution artifacts to implement and bind

The following are **required future artifacts**, with proposed exact paths for this delegation. They are not asserted to exist. Every SHA-256 is **REQUIRED PRE-LAUNCH BINDING**, not a fabricated value. If absent, stop at the applicable gate and return the missing fields. Do not invoke hypothetical command interfaces until implemented, tested, and recorded.

| Artifact | Required path and purpose |
| --- | --- |
| Dedicated trainer entry point | `scripts/strength_first_step10_trainer.py`: implement Step 10 identity, seed, schedule, checkpoint retention, resume/export semantics |
| Resolved training configuration | `ROOT/training/resolved-config.json`: all actual settings, corpus and source identities; no unconsumed keys or silent library defaults |
| Trainer CPU fixtures | `scripts/strength_first_step10_trainer_fixture.py` and `ROOT/preflight/trainer-fixtures.json`: exercise the real trainer path |
| Corpus-family validator | `scripts/strength_first_step10_opening_gate.py`: use repository packed-board decoding and generator-compatible canonicalization, with its implementation dependencies bound |
| Generator build receipt | `ROOT/openings/generator-build.json`: exact `haitaka_learn` executable hash, source/lockfile/build/toolchain identities |
| Opening receipts | `ROOT/openings/generation.json`, `corpus-family-exclusion.json`, `independent-validation.json`, and `freeze.json` |
| Serial match runner | `scripts/strength_first_step10_runner.py`: exact schedule, sealed results, owned processes, budget/retry ledger |
| Validator/analyzer | `scripts/strength_first_step10_analysis.py`: strict block validation and fixed statistical implementation |
| Analysis fixtures | `ROOT/preflight/analysis-fixtures.json`: rejection, pairing, bootstrap, and decision-boundary tests |
| Forecast | `ROOT/preflight/forecast.json`: throughput, storage, cost, match time, and retry reserve |
| Execution binding | `ROOT/execution-bindings.json`: resolved paths, byte counts, SHA-256, commands, review and authorization receipts for every phase |

Create only artifacts covered by the future delegation's implementation scope. Record any path adaptation explicitly in the execution bindings; do not silently change the design contract. Freeze `ROOT/contract.json` as an exact copy of the reconciled contract. Bind this runbook's actual hash at delegation time. Use immutable phase receipts so new model/export hashes can be added without rewriting earlier prelaunch evidence. `ROOT/manifest.json` must bind all match inputs before any match starts; `ROOT/status.json` is mutable operational state, not the protocol authority.

Before launch, make each implemented phase's exact command argv, working directory, resolved environment, stdout/stderr destinations, tool versions, exit semantics, and receipt schema reviewable in `execution-bindings.json`. Include native and WASM artifacts, integer reference, parity fixture inputs, serializer, trainer dependency tree, and analysis RNG/quantile implementation. Null hashes are blocking, including generator binary, Step 10 trainer/config, match runner, and analyzer. Historical scripts may supply reusable logic but their historical authorization and settings do not transfer.

## 5. Frozen data and training semantics

Use one fresh scratch seed **`2026091414`**, distinct from A `2026091002` and B `2026091108`. No additional full run, warm start, historical weights, or seed selection is permitted. Keep the frozen Model B corpus unchanged:

| Artifact under `out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/` | Bytes | SHA-256 |
| --- | ---: | --- |
| `training.json` | 1,384 | `89f976b5fdfde9198fa9b671e8a6024758b1d638cceedecf9316371865cb0c55` |
| `training.bin` | 63,360,000 | `51ecff36f4aa69a98d52e1cac78a47a0e62b15c834d9f33902557c138055b121` |
| `training.jsonl` | 149,594,430 | `f213ae17df30478d12f9fcacdde057e85161cf40fd2a046cdb173cb1185a5fa0` |
| `validation.json` | 1,389 | `e72d1119554bc7bcfb2b56487217d5af6801e0892c7532197b539a6b330755bf` |
| `validation.bin` | 4,320,000 | `c128c7b58ab2b782badb4d504b6520a3dde8050820feb567250b127de6d96fee` |
| `validation.jsonl` | 10,123,281 | `4e576f0d4e0a0e80509f6ab695a71d1fe10754a231b76b7d20f30491050a4fb3` |

Require 880,000 training and 60,000 validation records, 72 bytes per record. Validate manifest/binary/sidecar consistency; do not regenerate or repartition. Transfer only explicit needed paths and verify hashes at both ends.

The resolved config must implement:

- `HalfKAv2^+DonorSingleEff`, widths L1/L2/L3 = 256/16/32, batch 16,384, one worker, noncyclic/unfiltered loader, zero random FEN skipping, finite final partial batch.
- Training order `pcg64dxsm-seed-epoch-record-permutation-v1`; validation `fixed-source-order-once-per-evaluation-v1`, exactly once over all 60,000 records per evaluation.
- Teacher-only `robust-clipped-huber-v1`; result loss disabled; lambda 1; score clip and normalization 12,000; normalized Huber delta 0.1; model-output-to-runtime score scale 600.
- Ranger: betas `[0.9, 0.999]`, epsilon `1e-7`, weight decay 0, lookahead alpha 0.5 and K 6, nSma threshold 5; parameter-group LR multipliers `[1,1,1,1,1,1,0.1,0.1]`; pinned model weight clipping before and after each update.
- Linear warmup for 64 optimizer updates, base LR 0.0015, cosine decay with minimum 0.00005 and **4,096-update horizon**. Execute through **u2688**, computing LR as if the horizon were still 4,096. Stopping at 2,688 is not a schedule rewrite.

Explicitly inherit and resolve every omitted optimizer/loader setting from the hash-bound historical B recipe; do not fall through to current library defaults. Reject mismatched or unused settings. Record actual examples, passes, epoch/data cursor, optimizer updates, LR, validation loss, finite/numerical checks, output/activation saturation, scale, and gradient/update diagnostics.

Validate every 128 updates through u2688. Retain immutable full-precision candidates at **128, 256, 384, 512, 640, 768, 896, 1024**, plus the fixed **2688** control, their validation receipts, and resumable state including optimizer, LR schedule, RNG, and cursor. Train to the control unconditionally unless a correctness or budget stop occurs. Loss improvement does not permit early termination, extra training, or an interim strength screen.

### CPU trainer fixtures before rental/training

Run small, bounded fixtures through the real Step 10 trainer in the future authorized preparation environment. Use tiny synthetic/fixture data, not a full CPU training run. Bind fixture inputs, argv, software and output hashes. Prove:

1. Config consumption, seed/experiment substitution, exact LR values at warmup and later boundaries under the 4,096 horizon, with execution stopping at 2,688.
2. Finite partial-batch handling, deterministic epoch permutations, fixed once-only validation order/count, teacher-only objective and score orientation.
3. Checkpoint dispatch for all eight candidates and explicit u2688, immutable retention, and selection using actual validation receipts. A scheduler unit fixture alone is insufficient; exercise real save/load/export with the actual trainer.
4. Interrupted versus uninterrupted execution in the same CPU environment reproduces model/optimizer/RNG/cursor and next updates exactly. Do not claim this establishes byte-identical CUDA recovery across hardware.
5. Export refers to the requested immutable checkpoint rather than `last`; missing/nonfinite losses, missing receipts, wrong counts, incomplete diagnostics, and invalid exports fail closed.
6. Tie selection uses the global minimum, including a case where sequential near-tie comparisons would choose incorrectly.

## 6. Fresh model-blind development openings

After opening-generation authorization and generator-build binding, run this exact generator shape from the isolated repository root. The subcommand belongs to **`haitaka_learn`**, not `haitaka_cli`. Freeze RNG `rand-std-rng-0.9-lockfile-v1`, seed `2026091415`, 128 positions, 16 random plies, prefix `step10-selection`.

```bash
target/release/haitaka_learn strength-first-generate-opening-suite \
  --output out/anhoku-strength-first/step10-checkpoint-selection-20260914-v1/openings/development.tsv \
  --report out/anhoku-strength-first/step10-checkpoint-selection-20260914-v1/openings/generation.json \
  --seed 2026091415 --positions 128 --random-plies 16 \
  --id-prefix step10-selection \
  --exclude-suite haitaka_learn/openings/anhoku-v1.tsv \
  --exclude-suite haitaka_learn/openings/anhoku-v2.tsv \
  --exclude-suite haitaka_learn/openings/anhoku-v3.tsv \
  --exclude-suite haitaka_learn/openings/anhoku-strength-confirmation-a-v1.tsv \
  --exclude-suite haitaka_learn/openings/anhoku-strength-confirmation-b-v1.tsv \
  --exclude-suite haitaka_learn/openings/anhoku-strength-confirmation-100ms-a-v1.tsv \
  --exclude-suite haitaka_learn/openings/anhoku-strength-confirmation-100ms-b-v1.tsv
```

Verify all seven prior-suite byte counts and SHA-256 against `openingProtocol.priorSuiteExclusions` before generation:

| Suite basename | Bytes | SHA-256 |
| --- | ---: | --- |
| `anhoku-v1.tsv` | 1,306 | `7150a2a5871c4d302b63ab99ea31abe086471fa38a213dc184f42ce5d05721a7` |
| `anhoku-v2.tsv` | 6,127 | `bc576bbe57c05b8b2112b416c1907845d38d5e087e8e3b71b44c19c4e1593307` |
| `anhoku-v3.tsv` | 6,148 | `1cc44568c8ee5a374d2a9ad0af2730c8564b8111aaab33da496c5c2a5fb4c92f` |
| `anhoku-strength-confirmation-a-v1.tsv` | 236,018 | `d58c3adfdb7ba96f0cf5e21e36c97fe48d1d3df337293b92b33665744f497f39` |
| `anhoku-strength-confirmation-b-v1.tsv` | 236,127 | `4824a0c63bae39d86148cc1bc4bd6471b4c8f1296a460ec5b150fba0cce54578` |
| `anhoku-strength-confirmation-100ms-a-v1.tsv` | 219,737 | `66fd0e8c365d85f965d02e214738a48bed23d7905f7bb3e2b45dfc3821c10e61` |
| `anhoku-strength-confirmation-100ms-b-v1.tsv` | 219,742 | `53f1cd689da3615317144ef958ba731986191fbb6847e6b3e6df3f79231377cf` |

The existing generator's prior-suite exclusions do **not** establish corpus-family exclusion. Before freezing, stream all five contract corpus binaries through the repository packed-board reader and use the same board-family definition as the generator's `canonical_family`, including 180-degree rotation/color swap. Never approximate this with textual SFEN comparison or a sampled audit. Check every generated opening against the complete family set.

Besides B training/validation above, the additional inputs under `out/anhoku-reboot-r4-replacement/datasets/` are:

| Artifact | Records / bytes | SHA-256 |
| --- | --- | --- |
| `training.bin` | 880,000 / 63,360,000 | `9ce80b25becf329f16135dd7c861374c0d51351e286b0f13396ffceb701fdc33` |
| `validation.bin` | 60,000 / 4,320,000 | `fcb2b62bc7974499adbc71ec241ec344d9fbe6b8a4b964bb4d19fd74ab20b81b` |
| `development.bin` | 60,000 / 4,320,000 | `21e7f9ca0cbd4b6b8256816f770f5dc6e66939aab7591cf3a3bc55072288cd00` |

Prove canonicalization with a board and its color swap, duplicate-family fixtures, and deliberately overlapping corpus/suite fixtures. Missing input, parse error, wrong count, family overlap, or duplicate family stops before freeze. Do not silently drop or replace overlapping rows, change seeds, or generate extra candidates until 128 pass. Such a recovery would require a reviewed deterministic protocol amendment and applicable authorization.

An independent validator must re-read the frozen candidate suite, verify 128 legal rows with unique IDs and canonical families, original row order, all seven exclusions and all five corpus-family exclusions, generation/build/config bindings, and output hashes. Produce these schemas from the contract:

- Generation: `haitaka-anhoku-strength-first-opening-generation-v1`.
- Corpus exclusion: `haitaka-anhoku-strength-first-opening-corpus-family-exclusion-v1`.
- Independent validation: `haitaka-anhoku-strength-first-step10-opening-validation-v1`.

Record actual independent validator/reviewer identity and implementation hash, never self-label an unchecked generation receipt independent. Freeze `development.tsv`, its SFEN-only equivalent, and four block files with explicit row-index/ID/SFEN mappings. Block b contains zero-based rows `[32*b,32*(b+1))`, original order. Both models use the identical mapping. Bind all receipts and files in `openings/freeze.json` before training. No model score, training outcome, or old diagnosis outcome may influence generation, filtering, ranking, or suite size.

## 7. Budget forecast, quote, approval, and training launch

Caps are cumulative: **one full training run; 2,688 optimizer updates; 4 GPU-hours; USD 5 rental cost; 8,589,934,592 bytes new storage; 24 CPU match wall-hours**. All training, opening-generation, and match attempts count toward applicable caps; no failure resets a counter. Match attempts are at most two per block. Reserve storage for immutable candidates, optimizer/resume state, exports, parity outputs, raw games, failed attempts, staging and transfer copies. Do not delete historical artifacts to fit.

Write the forecast with schema `haitaka-anhoku-strength-first-step10-throughput-storage-cost-forecast-v1`. Include measured throughput or justified historical transfer, training/validation/export/parity time, peak storage, 512-game MBP time, explicit retry reserve, assumptions and uncertainty. Historical 4,096-update training time **263.543865172 seconds** is only a nonbinding anchor. It excludes whatever operations its receipt did not measure. Use existing 1-second timing evidence or conservatively account for up to 10× comparable 100 ms search time; do not run unapproved games as a forecast probe. If the full schedule plus reserve does not fit, stop for a reviewed budget/design resolution; do not shorten it after launch.

Only during future authorized preparation obtain a fresh GPU quote: provider/offer identity, timestamp/expiry, hardware/VRAM, software compatibility, availability, hourly and storage/transfer charges, expected and maximum duration, expected and capped all-in cost, storage forecast, and cleanup/archival plan. No GPU quote or network request is performed by writing this runbook. A paid throughput probe is GPU work and needs approval too.

Present the concrete quote and hash-bound launch package to the user. **`AGENTS.md` requires explicit permission to rent Vast.ai.** Require both rental and training authorization; ceilings alone are not approval. If the offer expires or changes outside the approved scope, refresh the quote and obtain applicable approval before committing resources. Record owned resource IDs and lifecycle authority, without exposing credentials in receipts.

After approval, verify transferred inputs and trainer/config hashes on the GPU host, run only the bound Step 10 command, enforce stop/cost/storage counters, and retain immutable checkpoints plus validation history. Do not launch the historical B wrapper. Exact resumption of this run requires previously qualified state and an authorized, budget-fitting recovery path; it must not become a second seed/run or hide repeated updates/charges. Otherwise stop and report incomplete training. Retain and verify required model/resume artifacts on the coordinator before releasing an owned rental under the approved lifecycle plan.

## 8. Selection, exports, and exact qualification

Require all eight candidate checkpoints and complete finite validation receipts on the frozen 60,000-record validation set. Let `m` be the global minimum of their losses. Select the earliest update with `loss <= m + 1e-12`. Record every loss and hash, the global minimum, tie set, and selected update in `ROOT/training/selection.json`. Fix control at u2688 from the identical run/config/seed/corpus. Never substitute historical B u768/u2688 or a different run's checkpoint.

Qualification is fail-closed for **every candidate**, not only the selected candidate, and also for u2688. A missing checkpoint/receipt, nonfinite loss, incomplete diagnostic, export failure, or qualification failure stops before matches; never shrink the eight-candidate set. Freeze the validation-only selection receipt before any game runs.

Export the exact retained checkpoints with the pinned serializer. Bind checkpoint → full-precision evaluation → serialized network → independent integer reference → native/WASM receipt for each candidate/control. Use predeclared model-blind parity inputs, count, evaluator versions, terminal/orientation treatment, and metric definitions, recorded in the execution bindings before qualification. The contract does not specify a new numeric quantization-error ceiling or parity sample count: bind the applicable historical qualification procedure and resolve missing acceptance details before execution; do not invent a passed receipt or choose thresholds after viewing results.

Require **zero mismatches between integer reference, native, and WASM**, including applicable full/incremental/trace checks. Separately measure full-precision-to-integer quantization error (at least count, MAE and maximum error, score orientation and terminal handling). Full precision need not equal quantized integer scores exactly. Any full-precision numerical/export defect or exact integer/runtime disagreement blocks matches. Record exact WASM and reference hashes; the native match runtime must remain `ad2e83...`. Do not rebuild/substitute it if a flag or format fails.

## 9. Fixed native MBP match execution

Launch only after the CPU-match grant, complete qualifications, frozen manifest, host checks, fixture receipts and budget forecast. This is two cells of 128 pairs/256 games each, **512 games total**, eight serial blocks of 32 pairs/64 games. Candidate is engine A (`nnue`); opponent B is production handcrafted. Both receive 1,000 ms/move, one engine thread, one game process, sequential openings, zero random plies, maximum 200 plies, match seed **`2026091417`**. Color pairing uses `anhoku-rotate-180-and-swap-colors-v1` and must give the candidate both seats for each opening.

| Execution position | Cell | Block | Zero-based opening rows |
| ---: | --- | ---: | --- |
| 1 | `early-selected` | 0 | 0–31 |
| 2 | `u2688-control` | 0 | 0–31 |
| 3 | `early-selected` | 1 | 32–63 |
| 4 | `u2688-control` | 1 | 32–63 |
| 5 | `early-selected` | 2 | 64–95 |
| 6 | `u2688-control` | 2 | 64–95 |
| 7 | `early-selected` | 3 | 96–127 |
| 8 | `u2688-control` | 3 | 96–127 |

The runner resolves `RUNTIME`, `NETWORK`, `BLOCK_SFEN`, and a fresh `ATTEMPT_DIR` from verified manifest entries, not an unvalidated environment. Each block's exact argv is:

```bash
"$RUNTIME" self-play \
  --games 64 --threads 1 \
  --a-eval nnue --b-eval handcrafted --a-nnue "$NETWORK" \
  --movetime-ms 1000 \
  --openings "$BLOCK_SFEN" --opening-order sequential \
  --opening-random-plies 0 --seed 2026091417 \
  --max-plies 200 --report-dir "$ATTEMPT_DIR/native-raw"
```

`aDepth`, `bDepth`, and `nodesPerMove` are all **null** in the contract: pass no explicit depth cap or node cap. Both engines use the exact frozen production-search defaults under movetime. Record the runtime's resolved default semantics from source/help in preflight; null does not establish that the implementation has unlimited depth. Do not add depth/node flags or lower timing because a forecast or command fails.

Use `ROOT/matches/<cell>/block-<0..3>/attempt-<1..2>/`. Store argv, bindings, host/session/thermal markers, start/end times, process exits, owned runner/game PIDs, native raw data, strict validation, accepted artifact hashes, and budget counters. `status.json` records active block, accepted completed blocks/games, attempts, timing, and remaining owned processes. Resume must preserve accepted completion counts and execution order, not initialize an empty ledger.

The unattended runner validates each block automatically. Require 64 unique ordered game indexes, 32 complete opening pairs, exact IDs/SFEN/color mapping, network/runtime/suite/seed/command bindings, legal move/result/termination records, consistent winner/WDL/plies, null failure state, and zero aggregate/per-engine fallbacks. Recompute WDL and pair scores from raw games and compare aggregates. Preserve maximum-ply status and all draws; never discard an inconvenient termination or silently reinterpret malformed output.

Before matches, bounded synthetic fixtures must reject missing/duplicate games or pairs, wrong network/suite/SFEN/color/seed/settings, malformed WDL or termination, illegal/fallback/failed games, and inconsistent aggregate counts. Test partial-attempt exclusion, immutable accepted-block resume, sealing, retry caps, and identical-pair mapping across cells.

**Seal all outcome-bearing data until both complete cells validate.** The automatic validator may read raw data internally but expose only pass/fail, progress counts, active block, sanitized failure category and timing. Redirect score-bearing stdout/stderr into sealed attempt artifacts. Neither worker nor coordinator may tail raw logs, inspect WDL, summarize pair scores, or open game records during execution. A validation error must not leak outcomes into progress messages. No adaptive selection, schedule change, optional screen, or extension.

### Interruption and retry

For a documented owned-process interruption, send Ctrl+C/SIGINT once, wait briefly, then SIGTERM only that owned process if necessary. Record cause and signals. Preserve the incomplete attempt, exclude it entirely, and rerun the whole 64-game block in a new attempt directory only if budgets and approvals remain valid. Never splice partial output or overwrite an attempt. Reverify hashes/validation for every accepted completed block before resume.

At most **two attempts per block**, with every attempt charged against time/storage/cost. A second failure from the same cause stops the experiment. Correctness failures—including missing results from an apparently completed block—stop immediately without automatic retry. An interrupted partial block is recoverable only when interruption is established, not inferred to excuse malformed results. Do not alter validation, runtime, settings, or sample to finish. On budget stop preserve sealed evidence and report `budget-stopped`, not convergence or strength failure.

## 10. Analysis and interpretation

Unseal only after all eight accepted blocks and both complete cells validate, with 256 games/128 pairs per model and matching opening IDs. Recheck hashes and full schedule consistency. If execution stops early, do not unseal partial outcomes to fill a report; mark missing primary results unavailable.

For opening i and model M, score each game from M's perspective: win 1, draw 0.5, loss 0. Define `pairScore[M,i] = (gameScore1 + gameScore2) / 2`. Define `d[i] = pairScore[early-selected,i] - pairScore[u2688-control,i]`. The **one primary contrast** is the mean of the 128 `d[i]` values. This compares each model against handcrafted on identical openings; it is not a direct head-to-head match.

Use contract bootstrap `paired-opening-score-difference-splitmix64-v1`, **10,000 replicates**, seed **`2026091416`**, paired opening-level resampling with replacement, two-sided percentile 95% descriptive unadjusted interval. Keep both models' pair scores together; never resample 512 individual games independently. Bind exact SplitMix64/index-sampling and percentile conventions in the analyzer before games, with deterministic fixture vectors and boundary tests. If historical conventions cannot be resolved, stop for an explicit implementation binding rather than silently substitute a library RNG.

Decision, in score units (0.02 = two percentage points):

- `success`: interval lower endpoint **> +0.02**.
- `failure` / material inferiority: interval upper endpoint **< −0.02**.
- `inconclusive`: every other interval, including equality at either boundary and failure to demonstrate improvement.

The null is that the early-selected checkpoint's advantage is at most +0.02. Failure to reject that null is not evidence of inferiority. Report the point difference and both endpoints in score units and percentage points. Secondary WDL/score versus handcrafted, candidate losses, exposure/LR, parity, timing, maximum-ply games and draws are descriptive and cannot change selection or stopping.

Interpret the interval as opening uncertainty conditional on this one fresh run, frozen suite, and reused B corpus/validation set. It excludes seed, corpus, selection-rule and session/thermal variability. Exposure and schedule position remain coupled. A favorable result supports this prospective selection rule under these conditions; it does not prove overtraining, establish u768 as universal, rescue confirmation, or qualify deployment. An inconclusive result completes the bounded experiment.

## 11. Reporting, closeout, and handoff

Retain at least:

```text
ROOT/contract.json
ROOT/execution-bindings.json
ROOT/manifest.json
ROOT/status.json
ROOT/authorization/                  # grants and phase receipts
ROOT/preflight/                      # fixtures, provenance, forecast
ROOT/openings/                       # suite, mappings, build/exclusion/validation/freeze receipts
ROOT/training/                       # config, run, metrics, validation/selection history, resume state
ROOT/checkpoints/                    # all eight candidates and u2688
ROOT/exports/                        # corresponding immutable exports
ROOT/parity/                         # exact parity and separate quantization receipts
ROOT/matches/<cell>/block-<b>/attempt-<n>/
ROOT/analysis.json
ROOT/report.md
ROOT/artifact-index.json             # relative paths, bytes, SHA-256
```

Hash every required retained artifact; keep a separate checksum for the final artifact index rather than making it self-referential. Verify exact transfer hashes on coordinator and execution host. Preserve raw attempts, accepted-block mapping, stop/retry history, actual resource spending, and model-to-checkpoint provenance. Do not recursively inventory forbidden historical paths.

The complete report includes the question, scope, reconciled approval identity, actual authorizations and execution bindings, host/software, corpus/opening provenance, all candidate losses and selection, same-run control proof, qualification and quantization results, two-cell WDL/pair scores, primary interval and decision, timing/capped games, budgets/retries, limitations and one bounded next recommendation. State `deploymentAuthorized: false`, `confirmationAuthorized: false`, `handcraftedRemainsProduction: true`, and failed Step 9 confirmation unchanged.

For a blocked/early-stopped run, report exact phase, missing approval or binding, defect, completed non-outcome evidence, unavailable cells/primary decision, preserved attempt paths, spending, and remaining owned processes/resources. Do not manufacture a result table or label correctness/budget failure a statistical loss.

Stop owned runners/caffeinate when appropriate, verify no owned game remains, archive required weights/resume state locally, and close only owned paid resources within the approved lifecycle scope. Record disposition and recoverability of anything removed; preserve historical artifacts. Validate the report and hash index with the bound analyzer, run `git diff --check`, and inspect the final changed-file list against the authorized scope. A future request may authorize coordinator report publication/plan links; **this runbook does not require or authorize a commit, push, plan edit, or deployment**. Stop after handing back results or the precise blocked gate.

Return these explicit handoff fields to the delegating agent:

| Field | Required content |
| --- | --- |
| `worker` | Actual model, reasoning effort, worker ID |
| `status` / `lastCompletedGate` | Preparation-only, blocked, complete, correctness-stopped, interruption-stopped, or budget-stopped; exact phase |
| `provenance` | Contract/review/runbook hashes; reconciliation receipt and implementation revision/dirty bindings |
| `authorizations` | Each phase's grant/reference, scope, caps; missing grants |
| `bindings` | Trainer/config, generator/build, opening receipts, runner/analyzer, native/WASM/reference and all missing hashes |
| `training` | Seed, corpus hashes, actual updates/examples, LR horizon, candidate loss/checkpoint/export table, selected update, same-run control proof |
| `openings` | Frozen suite hash, 128-row mapping, all seven suite/five corpus exclusions and independent validation |
| `qualification` | Candidate/control parity receipts and separate quantization metrics; defects |
| `matches` | Host, runtime/settings, eight-block ledger, accepted games/pairs, attempts, seal/unseal state |
| `result` | Primary contrast/interval/decision only if fully qualified; otherwise unavailable with reason |
| `resources` | Actual GPU-hours/cost/storage/CPU match time, retry reserve used, owned process/resource disposition |
| `deliverables` | Report, analysis, manifest, artifact-index paths and hashes; transfer verification |
| `workspace` | Exact files changed, unrelated changes preserved, commit status |
| `nextAction` | One bounded recommendation or exact unresolved gate/approval request; no automatic continuation |

## Final checklist

- [ ] Actual Luna max configuration recorded; full contract/review read; reviewed/current contract hash discrepancy reconciled.
- [ ] Isolated workspace and dirty source preserved; immutable inputs and every required execution artifact hash verified.
- [ ] Separate phase approvals recorded; fresh quote and explicit GPU rental/training approval before paid work.
- [ ] Real CPU trainer fixtures pass; one fresh seed, frozen B corpus, 4,096 LR horizon, execution through u2688.
- [ ] Model-blind 128-opening suite frozen with seven prior-suite and five complete canonical corpus-family exclusions plus independent receipt.
- [ ] All eight early candidates and same-run u2688 retained/qualified; global-minimum validation-only selection frozen.
- [ ] Integer-reference/native/WASM zero-mismatch parity passes; full-precision quantization error reported separately.
- [ ] Bound 1,000 ms MBP commands, seed, exact eight-block schedule, sealing, strict validator, retry and cumulative budgets enforced.
- [ ] Both cells complete before disclosure; one paired bootstrap contrast and strict ±0.02 thresholds applied unchanged.
- [ ] Report/index/transfer hashes verified; owned resources accounted for; explicit handoff returned.
- [ ] Abandoned partial unopened; confirmation closed; handcrafted production unchanged; no deployment or unauthorized continuation.
