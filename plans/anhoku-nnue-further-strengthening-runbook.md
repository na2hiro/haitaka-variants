# Anhoku NNUE further-strengthening runbook

Prepared 2026-09-25. Parent: [future work item 1](anhoku-nnue-strength-first-plan.md#1-further-strengthening-ideas). This is a new, bounded R&D workstream, not a continuation of Step 18 and not a Shogitter release gate.

**Status: completed 2026-09-28 — screen did not advance.** The original plan was executed under the user's explicit authorization, including the approved GPU work and added time budget. The [F1 report](../r0/anhoku-strength-first/f1-loss-ablation-report.md), [development analysis](../out/anhoku-strength-first/f1-loss-ablation-20260925-v1/analysis/development.json), and [closeout](../out/anhoku-strength-first/f1-loss-ablation-20260925-v1/closeout.json) record the result. The development gate failed against I, so no holdout was run. I remains the experimental incumbent; this runbook authorizes no further experiment, deployment, commit or push.

## 1. Outcome and limits of the experiment

Run one paired loss ablation, named **F1**:

- **I — incumbent:** immutable Step 10 u768, selected by the completed Step 18 campaign.
- **C — fresh control:** clipped-score Huber training on the incumbent's frozen corpus.
- **T — treatment:** identical fresh initialization, corpus, record order, architecture, optimizer, exposure and checkpoint selector; change only the training objective to the precisely specified sigmoid-space loss below.

Two scratch training arms, one initialization seed, one temperature rule, eight eligible checkpoints per arm, one development screen, and at most one independent T-versus-I holdout. No temperature grid, learning-rate search, additional seed, longer run, alternate checkpoint after games, or automatic next experiment.

The development T-versus-C comparison addresses the loss hypothesis for this matched run. The independent T-versus-I test addresses whether this particular artifact improves on the selected model. Neither proves a generally better training recipe; I is not a matched training control. A negative or inconclusive result is a completed experiment, not permission to keep searching.

Step 18 selected I and supported its superiority over handcrafted at one second: 614W/0D/410L on 512 independent color-swapped pairs, conservative score 59.9609375%, fixed-bet lower bound 0.5466365671327367. That remains valid historical evidence under its frozen conditions, not evidence that T works, and not deployment approval. See the [completed report](../r0/anhoku-strength-first/step18-model-strength-closeout-report.md). Handcrafted remains production until a separate integration/release decision.

## 2. Immutable starting inputs

All paths are relative to this repository. Reverify bytes and SHA-256 before use and after transfer. Missing or changed inputs block launch; do not silently substitute a newer model, rebuild the retained match engine, or refresh expected hashes.

| Input | Path | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| I | `out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/inputs/model/step10-u768.nnue` | 83048703 | `ddf42d5ca2e7a2a380bb13b8c4de8b3b7ee6807766d614cdb22757eead7f142b` |
| Retained match WASM | `out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/package/web-pkg/haitaka_wasm_bg.wasm` | 18279053 | `8677f7c83f48186b8132a534185044402d25cc9f3d071363912506ca0203dfc0` |
| Retained JS glue | `out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/package/web-pkg/haitaka_wasm.js` | 36924 | `d8eaf8ad3bcf842588f4fc2d3a83d6cb7f2d29637a5097396db3b0877036a724` |
| Train binary | `out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/training.bin` | 63360000 | `51ecff36f4aa69a98d52e1cac78a47a0e62b15c834d9f33902557c138055b121` |
| Validation binary | `out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/validation.bin` | 4320000 | `c128c7b58ab2b782badb4d504b6520a3dde8050820feb567250b127de6d96fee` |
| Train sidecar | `out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/training.jsonl` | 149594430 | `f213ae17df30478d12f9fcacdde057e85161cf40fd2a046cdb173cb1185a5fa0` |
| Validation sidecar | `out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/validation.jsonl` | 10123281 | `4e576f0d4e0a0e80509f6ab695a71d1fe10754a231b76b7d20f30491050a4fb3` |

Bind the corresponding `training.json` / `validation.json` manifests and the incumbent recipe at `out/anhoku-strength-first/step10-checkpoint-selection-20260914-v1/training/resolved-config.json`. Verify 880,000 training and 60,000 validation records, 72-byte entries, legal variant positions, binary/sidecar agreement, teacher provenance and side-to-move score orientation. Reverify canonical train/validation family separation. There is no new corpus generation or relabeling in F1. Preserve the existing data distribution, including its weaknesses; report them instead of silently filtering it.

Create a new root, `out/anhoku-strength-first/f1-loss-ablation-20260925-v1`, with experiment ID `strength-first-f1-loss-ablation-20260925-v1`. Historical roots are read-only inputs. Hash the complete F1 implementation/config/dependency inventory before execution; use append-only attempt directories and atomic publication. All receipts carry actual producer, UTC time, paths/bytes/full hashes, upstream bindings, resource consumption and truthful authorization/execution flags.

## 3. Freeze the scientific contract before training

### 3.1 Loss and scale

Let `r` be the raw floating-point network output, `p = 600*r` its runtime-score prediction, `y = clamp(teacher_score, -12000, 12000)`, and `S = 12000`. Both arms ignore outcome/result fields. Do not clamp the prediction or change runtime output scale.

Control C uses the existing per-record loss with `e = (p-y)/S`:

```text
H(p,y) = 0.5*e^2                     when |e| <= 0.1
         0.1*(|e| - 0.05)            otherwise
```

For T, calculate exactly one temperature on CPU, using only the frozen training labels, before initialization/training:

1. Decode all training scores with the production loader's orientation/conversion; cross-check against sidecars.
2. Sort the nonzero values of `abs(y)`. Use their median (arithmetic mean of the two central values for an even count), `M`.
3. Set `tau = M / ln(3)`, using float64. Persist the counts, quantiles, `M`, full-precision `tau`, algorithm and data hashes in `calibration.json`. Empty/nonfinite/zero input to this rule blocks the experiment; do not choose a fallback after inspection.
4. Freeze that value for all training and validation. No validation, development or holdout result may fit or retune it.

The treatment is the record-mean of:

```text
P(p,y) = 8*(tau/S)^2 * (sigmoid(p/tau) - sigmoid(y/tau))^2
```

Use a numerically stable sigmoid and the same production precision policy as C; finite-check losses, outputs and gradients. The prefactor matches H's local quadratic coefficient around `p=y=0`; it does not make gradients equal elsewhere or guarantee equal effective optimization. Record saturation and gradient diagnostics rather than changing LR to compensate.

This is **distribution-scaled probability-space teacher regression**, not empirically calibrated Anhoku win probabilities. The median nonzero target magnitude maps to 0.75 by construction. Choosing that mapping and prefactor is an F1 experimental design choice, not an established optimum. A genuine outcome-calibration experiment needs verified completed outcomes and a separate contract. Do not import chess constants such as 361/400/410 into this experiment.

The [official NNUE technical documentation](https://raw.githubusercontent.com/official-stockfish/nnue-pytorch/master/docs/nnue.md) discusses sigmoid-space objectives and evaluation-to-outcome calibration; it supplies motivation, not this variant's scale or evidence of a gain. The [Step 17 review](../r0/anhoku-strength-first/step17-review-and-next-step.md#3-untested-hypotheses-and-established-practice) records the broader hypotheses and limitations.

### 3.2 Shared recipe and checkpoint rule

| Component | Both C and T |
| --- | --- |
| Initialization | Fresh scratch, seed `2026092501`; identical initial model/optimizer/RNG state; no incumbent weights or warm start |
| Features / layers | HalfKAv2^+DonorSingleEff, 256/16/32; corrected retained FT behavior; output scale 1× |
| Data | Exact binaries/sidecars in section 2; no filtering, skipping, augmentation change or result mixing |
| Loader | 16,384 batch; one worker; finite noncyclic epochs; final partial batch retained; PCG64DXSM seed/epoch permutations from the qualified core |
| Optimizer | Ranger; betas .9/.999; epsilon 1e-7; weight decay 0; lookahead alpha .5, K=6; nSma threshold 5; gradient centralization disabled |
| LR groups / projection | FT, l1-factor/l1/l2 weights and biases 1×; output weights/biases .1×; bound clipping before and after each update |
| LR schedule | Base .0015, warmup 64 updates, cosine minimum .00005, horizon 4096 |
| Stop | Exactly 1024 updates per arm, regardless of validation ranking |
| Validation | All 60,000 records in fixed source order; initial and each 128 updates; record-weighted mean, including the final partial batch |
| Eligible checkpoints | 128, 256, 384, 512, 640, 768, 896, 1024 |
| Selection metric | **The same Huber H above for both arms**, even though T trains on P |
| Selector | Compute global minimum once; choose earliest eligible update with H <= minimum + 1e-12; no game evidence |

Keep initialization, parameter groups, dtype, feature mapping, clipping bounds and schedule formula bound to actual qualified implementation, not library defaults. Train arms sequentially on the same approved environment, C then T, resetting all state. Save initial state identities and prove equality across arms. Record batch index digests, examples seen, epoch/cursor and LR; require their full update sequences to match. Losses/gradients/weights after update 1 are expected to differ.

Stopping at 1024 removes the historical u2688 late-control tail, which cannot affect earlier checkpoints. It does **not** shorten the cosine horizon. This is an explicit prospective exposure choice shared by both arms, not a byte-for-byte replay of Step 10. No u2688 control is required. Evaluate both H and P as diagnostics for both arms, but never choose checkpoints by whichever metric looks favorable. The fixed H selector preserves the historical selection criterion; F1 does not test a sigmoid-trained/sigmoid-selected policy.

Retain eight immutable portable full-precision checkpoints and integer exports per arm, initial identities, all nine validation evaluations per arm, and complete resumable last state. Missing candidates fail closed. After both arms finish, freeze exactly one C and one T by validation only, before any scored game. An inferior C or failed screen cannot trigger a seed retry.

## 4. Implementation is part of the delegated preparation

F1 is not already runnable. Implement F1-owned contract, trainer/fixture/selector, opening builder, live controller and analyzer, with documented actual commands and `--help` output in `commands.md`. Command names below describe required capabilities, not existing CLIs. Do not ask the user to supply an adapter that this task assigns to the worker.

Reuse qualified components with tests, not historical dispatchers with relabeled receipts:

- Training mathematics/loader: `scripts/strength_first_trainer.py`; Step 10 recipe/selector as above.
- Context and deterministic CUDA: `scripts/strength_first_step12_context.py`, `strength_first_step12_cuda_runtime.py`, `strength_first_step12_cuda_backward.py`, `.cu`, reference/tests, and Step 12 trainer/backend/fixtures. These contain historical bindings that require an explicit F1 wrapper or tested parameterization.
- Live match/replay: `scripts/strength_first_step18_live.py`, `_host_driver.py`, `_mbp_driver.py`, `_sources.py`, the Step 14 browser workers, and `haitaka_learn/src/bin/strength_first_step14_replay.rs`.
- Exact paired analysis primitives: `scripts/strength_first_confirmation_core.py`. Step 18's A/B-versus-handcrafted contract is **not** the F1 T/C/I contract.

Required entry points: input/calibration freeze, CPU qualification, package/grant gate, CUDA qualification, two-arm train/resume, validation-only select, opening freeze, runtime qualification, development run/analyze, holdout run/analyze, and idempotent closeout. All live commands fail closed on missing/expired grants, changed hashes, stale gates, unsupported opponents or exhausted budgets. Imports and dry runs must not create devices or dispatch jobs. Offline analysis/replay dispatches zero searches.

Preserve the lessons already resolved in Step 12:

- Build pinned trainer revision `61666d9e3653e4df9881b14c23f8fdcc4bf7779b` from the bound archive. Bind a compatible GNU patch executable; do not rely on macOS's unqualified `patch`. Apply the historical Phase 7.1 then R5 patches with the qualified order/options. Include all overlays **and `trainer-patches/evaluate.py`**. Two fresh contexts must have identical resulting source hashes. See the [context assembly procedure](anhoku-nnue-strength-first-step12-training-worker-runbook.md#context-assembly-compatibility-amendment--2026-09-17).
- Use the same qualified deterministic CUDA backward in fixtures and production. The old atomic accumulation failed exact full-precision resume; equal integer exports alone did not fix that. Its later bounded resolution is at `out/anhoku-strength-first/step12-independent-replication-20260916-v1/preflight/deterministic-cuda-v1/resolution.json`. Read the linked actual evidence, not just the status. Historical success does not waive requalification of F1's new loss and environment.
- Do not reopen that resolved work as a requirement to invent another backend, use fixture-only shims on GPU, relax equality to tolerances, or monkeypatch historical roots/checks. A necessary implementation repair requires new bindings and tests; changing the scientific contract requires a prospective amendment before results are viewed.

## 5. Preparation, fixtures and permission gates

After CPU preparation is delegated, prefer MBP `192.168.0.12` (relative CPU 10), then MBA `192.168.0.5` (7), then localhost Ubuntu (1) for CPU-heavy work, per AGENTS.md. Record availability/choice; lightweight local inspection/tests are fine. No full CPU/MPS training fallback is authorized. Match host is MBP only in F1; a host substitution requires a new pre-score binding/qualification, never a mid-campaign blend.

Before requesting GPU permission, finish input/calibration checks, F1 implementation, CPU fixtures, package review and resource forecast. Fixtures cover:

- Exact C loss equivalence, independent float64 P/gradient reference checks, symmetry under simultaneous score sign reversal, zero error, extreme scores, nonfinite rejection, target-only clipping and runtime factor 600. Verify analytical prefactor near zero.
- Training-only temperature provenance; known median examples and degenerate failure cases; changes to forbidden validation/outcome inputs cannot affect tau.
- Identical initial state, loader/cursor/example/LR traces; short final batches; record-weighted validation; loss diagnostics separated from the common selector; nonmonotonic minima and 1e-12 tie boundaries.
- CPU tiny-overfit/resume and authorized same-environment CUDA fixtures for **both losses**. Compare uninterrupted/resumed full-precision model, optimizer/lookahead state, RNG/next draw, cursor/next batches, trace and export exactly, including a fresh-process resume. Include lookahead, epoch/partial-batch and checkpoint-publication boundaries.
- Corrupt/missing input, changed package, absent grant, concurrent dispatch, disconnect, disk-full, interrupted publication and replayed closeout. Persist resource counters across all attempts; no duplicate scored records or automatic training rerun.

Then present the user a dated compatible Vast.ai quote and exact scope: one instance, two 1024-update arms, both CUDA fixtures, at most one exact-state recovery per arm, allowed transfers, all-in caps, archival reserve and instance termination. Request permission explicitly; do not rent while waiting. A fixture-only grant does not authorize production training. Old Step 12 permissions authorize neither.

Proposed ceilings, **not grants**:

| Resource | F1 ceiling |
| --- | --- |
| Active CPU preparation, including implementation/testing and opening generation | 8 hours cumulative; max 4 CPU processes; no games during this allowance |
| Paid lifecycle | One instance; 4 billed hours and USD 5 all-in, whichever first; other paid resources USD 0 |
| Training | Two arms × 1024 updates; one exact-state recovery per arm; no fresh restarts/seeds |
| GPU qualification | At most two complete qualification attempts per loss within the same paid cap; repairs must preserve the frozen mathematics |
| Match/qualification phase | 32 active MBP host-hours including setup, fixtures, failed blocks and retries; max one active game |
| Closeout | 1 active hour |
| Storage | 64 GiB new physical storage across all copies/hosts; maintain 16 GiB additional free reserve on each active host |

Forecast against measured qualified throughput, storage sizes and actual quote; reserve time/cost to archive and terminate. If the complete workflow cannot fit, stop and request a budget amendment **before launch**, not after deleting mandatory evidence or shortening the protocol. Four GPU hours are a proposed upper bound, not a throughput promise. Never infer prices from old rentals.

Use separate execution and rental grants with user-reference/package/config/input hashes and expiration/scope. They may come from one explicit reply, but remain false until actually given. Recheck immediately before dispatch. Rental approval must include shutdown of the owned instance on completion, failure or cap exhaustion. Monitor heartbeat, budget, GPU/RAM/disk and last update at least every 30 seconds; SSH loss is not process exit. Archive and hash-check all mandatory artifacts before termination whenever possible; reserve capacity for failure cleanup. Never kill unrelated jobs or leave a billed instance unattended.

Maintain append-only UTC start/end and monotonic duration ledgers for every phase/attempt, actual billed charges and transfers. Search timer totals are not charged host wall time: Step 18's reported engine-hours are useful planning evidence, not a replacement for this ledger.

## 6. Fresh openings and runtime qualification

Freeze implementation, selected C/T hashes, I, host/runtime/search settings and both opening suites **before development games**. Bind browser version, helper binaries, generator dependencies and full commands. Use the retained history-aware WASM engine at `go movetime 1000`, one thread/game, same fixed hash configuration and fresh search context per game. Each side gets its own `UsiEngine` and loads its own network on that instance; a module-global default is not sufficient. Deliver full `position sfen … moves …` history on every move. No Shogitter UI integration is needed.

Build the exclusion union from the completed Step 18 safe source inventory and add **both Step 18 selection and holdout suites** plus every subsequent known exposed corpus/opening family, including F1 fixtures. Inventory the sources explicitly; no broad historical-result traversal. Step 18's union seed is `out/anhoku-strength-first/step18-model-strength-closeout-20260920-v1/preflight/current-families/union.jsonl` (333810681 bytes, SHA-256 `31224f20c06d25f8bf13eeac956ae6c4a7f6b6bfd02f86eecef8e5a9a246fa61`). Verify its upstream source/normalization bindings; it alone predates Step 18's newly exposed suites.

Canonical family identity uses the qualified normalizer's board SFEN / rotated-color-swapped equivalence. Bind actual helper and dependency hashes. A changed normalization source prevents unqualified cache reuse: rebuild inventories from safe payloads under a single current binding, do not wait for historical pending Step 17 gates or assert equivalence from function names.

Preserve the protected abandoned Step 9/10 outcome prohibition. `scripts/strength_first_step18_sources.py` lists the safe contract chain and whole preregistered suites; exclude those entire suites without inspecting partial results. Never enumerate, hash or open the protected outcome trees to find the played subset.

Freeze the following proposal algorithm and seeds in the F1 contract:

- Development master seed `2026092502`, holdout master seed `2026092503`; proposal index starts at 0 in each phase.
- Derive generator seed as the first eight bytes, big-endian, of SHA-256 of ASCII `f1:PHASE:MASTER:INDEX`, where PHASE is exactly `development` or `holdout` and the numbers are decimal without padding.
- One independent generator invocation per proposal: retained `strength-first-generate-opening-suite`, one position, 16 random plies, **no exclusion file passed inside the generator**. Record full argv, deterministic seed and eligibility result.
- Apply legal-position and fixed outer family-exclusion checks. Development accepts the first 128 eligible proposals. Holdout accepts the first 512, with the fixed exclusion set additionally containing all development families.
- Do not reject a proposal just because an earlier accepted proposal in the **same** phase has that family. Sampling is conditionally with replacement; retained duplicates matter to the independence contract. No rolling in-phase deduplication or hand-picked balanced suite.
- Maximum 100,000 proposals per phase, within the CPU-preparation cap; exhaustion is a stop, not a new seed. Store rejected proposal reasons and accepted order. Freeze both suites/schedules before games; holdout outcomes remain unopened until its authorized phase.

Development uses the same 128 pairs for T–C and T–I. Each pair has two games from the exact same initial position, swapping evaluator colors; it is not two unrelated positions. Deterministically alternate matchup order by pair index and alternate first-leg color order by index. Holdout uses only T–I. Block size eight pairs; retain the fixed complete schedule and phase/block/game IDs.

Before scores, qualify I, C and T: at least 128 native/WASM exact integer parity rows per model, zero mismatches; donor and both-color cases; all 12 retained terminal/history queries; at least eight timed searches per model; eight short end-to-end fixture games covering T–C and T–I, both colors and both first-leg orders, with raw traces and independent replay. Bind deterministic fixture cases in the package. Verify per-instance model isolation, requested/actual timer/root metadata, legal move parsing and full-history replay. Fixture outcomes are not strength data. Preserve corrected FT activation and export scale; any parity or protocol failure stops scored launch.

## 7. Games, advancement and one final claim

| Phase | Matchups | Pairs | Scored games | Decision |
| --- | --- | ---: | ---: | --- |
| Development | T–C and T–I on common openings | 128 each | 512 | T must pass both practical screens below |
| Independent holdout, only if screen passes | Frozen T–I | 512 | 1024 | One artifact-specific superiority test |

Maximum 1536 accepted scored games. No additional handcrafted comparison is needed to claim improvement over I; beating handcrafted again would answer the wrong primary question. Qualification and failed attempts are additional work charged to the same finite wall-time budget.

Use the retained terminal-aware replay rules: true terminal results take precedence; maximum 200 plies. Ordinary game scores from T's perspective are 1 win, 1/2 draw, 0 loss; a genuine nonterminal ply cap counts as 1/2 only in the ordinary descriptive score. Conservative scoring replaces **only each capped leg's score** with 0 for T, leaving its companion leg unchanged. Infrastructure failure, illegal move, missing telemetry or inconsistent replay is not a game draw or cap. Reject the affected block with evidence, not just an inconvenient game.

Allow at most one full-block retry per block, and at most four retried blocks across both phases, within the wall cap. Preserve rejected attempts; no selection of the better attempt. Retry only predetermined technical-invalidity classes established by raw replay, never strength results. If both attempts fail or totals/budget cannot be completed, close as incomplete; do not manufacture a fixed-sample claim from survivors. Before each block, ensure budget for its worst-case 200-ply search allowance plus measured overhead and archival reserve. No unbounded overrun to finish a phase.

For each matchup, report ordinary/conservative game score, W/D/L, caps, pair histogram over `{0,.25,.5,.75,1}` and actual wall time. A pair score is the mean of its two game scores. Development advances T only when its **conservative score is at least 0.52 against both C and I** over all 128 pairs per matchup; ordinary scores must also clear 0.52. Apply the threshold with exact rational arithmetic. This is a predeclared practical screen, not a significance claim. Do not promote C, change T's checkpoint or tune tau if T fails. Close F1 without opening holdout outcomes.

If T passes, run all 512 independent holdout pairs against the same frozen I. No early success stop, optional extra sample, alternate challenger or holdout-based repair/tuning. Primary null: T's expected conservative pair score against I is at most 0.5 under the frozen eligible-opening/runtime distribution. Reuse the tested fixed-bet statistic with lambda 1/8 and alpha .05:

```text
q_i = 4 * conservative_pair_score_i          # integer 0..4
reject null iff product_i(30 + q_i) > 20 * 32^512
```

Equality fails; compute the decision with exact integers, not rounded percentages/logs. Report the conservative lower bound using the existing downward-rounded inversion, plus descriptive ordinary results and cap sensitivity. The evidence requires the specified independent proposal law and verified reset/history behavior; if those assumptions fail, do not call the statistic confirmatory. One planned test of one frozen candidate requires no search across checkpoints on the holdout. Any later experiment needs its own multiplicity/claim policy; do not treat repeated .05 tests as one global .05 search.

This is a budget-limited screen, not a high-power promise for small gains. Inconclusive does not mean equivalent or worse. A positive T–C development result does not establish a generally superior loss recipe; a holdout pass establishes evidence for **this T over I under these conditions**, not a transitive new test against handcrafted or deployment readiness.

## 8. Closeout and handoff

Publish `contract.json`, `calibration.json`, `package.json`, `commands.md`, grants/gates, CPU/CUDA qualification, per-arm initialization/run/validation/selection/archive receipts, opening/source freeze, runtime qualification, raw/replayed games, analysis, resource/instance-disposition receipts, `closeout.json`, and `r0/anhoku-strength-first/f1-loss-ablation-report.md`. Include all failed/partial attempts and explicitly state whether GPU/training/games occurred. Re-running closeout verifies existing artifacts and launches zero searches, jobs or instances.

Close in exactly one category:

- **Preparation/qualification blocked or execution incomplete:** exact blocker, work/cost consumed and safe next gate; no strength conclusion.
- **Screen did not advance:** I remains incumbent; report matched development evidence and its uncertainty; no holdout run.
- **Holdout complete, superiority unconfirmed:** I remains incumbent; report positive-but-unconfirmed or nonpositive point result without claiming equivalence.
- **Holdout superiority supported:** nominate frozen T as the new engine-level incumbent for a separate handoff, preserving I and all evidence. `deploymentAuthorized=false`; no shipped model changes.

Update the parent future-work item with the actual outcome and links. Stop. No automatic replication, Step 19, new data purchase, expanded training or release. A later user-approved replication can test seed/data robustness, but is not quietly appended to this artifact-specific experiment.

## 9. Conditional backlog after F1 — not execution authority

If further work is requested, choose one hypothesis and write a separate bounded contract rather than combining these changes with F1:

1. **Teacher-quality/coverage pilot:** proposed maximum 10,000 existing training-side positions, stratified across donor interactions, tactical, quiet and endgame cases. Freeze sampler/strata, teacher builds and two search budgets before labels. Compare legal/finite label rate, score/rank stability and coverage. Use MBP/MBA CPU preference; no large corpus purchase or full generation yet. Better label stability alone is not playing-strength evidence; decide separately whether a matched small-data training experiment is worth funding.
2. **Result/eval mixing or true probability calibration:** first audit verified completed-game outcomes, winner/draw semantics, side-to-move orientation, terminal rules and masks. Never assign real WDL labels to capped/unfinished games. Split by game/family before fitting; calibration data may not overlap the future strength holdout. Freeze one mixture/scale policy and compare with a teacher-only control. This is optional, not a mandatory missing ingredient.
3. **QAT, LR/regularization or feature changes:** one factor per contract, with parity/export and runtime-cost gates. Width 512 and 2× score scaling have prior inconclusive/unfavorable legacy evidence; consult it before proposing a repeat. Do not infer either that they must help or that they can never help.

The next decision should follow what F1 actually found: optimization/quantization failure calls for diagnosis; promising but unconfirmed games call for an explicit evidence/budget choice; an unpromising matched result permits stopping. None obligates an open-ended sweep.
