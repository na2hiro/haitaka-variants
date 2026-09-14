# Anhoku NNUE: strength-first plan

Created: 2026-09-06. Updated: 2026-09-14. Status: the independent-data confirmation is complete. Model A passed the separately frozen 100 ms endpoint, but model B failed; the required conjunction therefore failed. Handcrafted remains the production evaluator, no deployment is authorized, and this candidate lane is closed.

## Objective and scope

Build a standalone NNUE evaluator that beats handcrafted evaluation in the actual Anhoku production search at **1,000 ms/move (1 second)**, then improve it through measured teacher, data, and runtime iterations. A hybrid evaluator may be an explicitly named diagnostic, but does not satisfy the standalone NNUE objective.

This is a new experimental program, not another R5 repair amendment. On adoption, it replaces future execution routing in the original strength plan, workflow reboot, and R5 repair assignment. Their failures remain failures; their artifacts and useful implementations remain available. It does not set any historical R5/R6 authorization flag to true. Existing processes and paid resources must be inventoried before execution; this document does not authorize stopping them or renting a GPU.

The immediate deliverable is a short, working loop: train a model, retain it, export it, play trustworthy development games against handcrafted, and choose the next experiment from the result. Final independent confirmation remains strict. Development experiments need not satisfy promotion-level statistical requirements before another hypothesis can be tested.

## Current decision and timing policy — 2026-09-08

Use **1,000 ms per move as the primary fixed-time budget** for future development selection and final confirmation. Give both opponents the same budget. Keep 100 ms as a secondary responsiveness/short-budget regression check; it is not a second promotion endpoint. Equal-node games remain supporting diagnostics. One second is a search budget, not a required artificial delay when a terminal result or completed search returns earlier.

This policy applies prospectively. Completed 100 ms experiments and their frozen contracts remain unchanged and retain their original interpretation. Future contracts must explicitly set `movetimeMs: 1000`, specify the search depth cap, and budget the longer run before launch. Reserve up to ten times the 100 ms search time for the same paired schedule until a measured throughput forecast is available; reduce optional secondary work rather than silently changing a frozen primary sample size.

The completed [scale/time experiment](../r0/anhoku-strength-first/step5-result-report.md) ran 384 development games on the MBP at `192.168.0.12`. The original-scale candidate scored 67.2% against handcrafted at 100 ms and 65.6% at 1 second; doubled scale scored 60.9% and 57.8%. Doubled scale scored 30–34 against the original at both budgets and did not improve restricted teacher regret. Retain the selected width-256 candidate at **scale 1×**. These are reused development openings with eight concurrent native games, not controlled production-path confirmation. The paired intervals do not establish a relative strength benefit from the longer budget.

A separate 16-search-per-budget browser smoke test using the archived compatible WASM runtime peaked at 103.1 ms and 1,002.8 ms, without missing moves or emergency fallbacks. This supports testing 1 second but does not qualify the current production artifact, all target devices, or model load time. Production configuration is unchanged by this plan update.

The artifact name “Step 5 scale/time” refers to the completed follow-up to the Step 4 diagnosis; it does **not** mean the corpus-growth stage below has been completed or authorized.

## Runtime correction discovered during execution

The 2026-09-08 export audit found a serializer/runtime mismatch: the feature transformer is serialized in activation units at scale 127, but the runtime and historical integer oracles applied an extra six-bit right shift before clipping. The working runtime and oracles now remove that FT-only shift; hidden affine-layer shifts remain unchanged. The model bytes and 1× output scale are unchanged. Historical result files remain intact and describe the legacy implementation.

On 128 development parents excluding the previously labeled 128 canonical boards, the corrected independent integer reference, native full/incremental evaluation, native traces, and WASM agreed exactly on 8,278 positions. Nonterminal full-precision MAE fell from 550.3 to 44.8 score units, and static move agreement rose from 71/127 to 119/127. The separate 64-game native comparison was 27–37 for corrected versus legacy, with a 95% opening-group score interval of 31.2%–53.1%; a game-strength improvement is not established. The controlled browser baseline remains the next strength measurement.

Execution is tracked by the [correction and browser contract](../r0/anhoku-strength-first/step6-browser-export-v1.json) and its [reporter-recovery contract](../r0/anhoku-strength-first/step6-browser-export-v2.json). Its “Step 6” artifact name is an experiment sequence number, not completion of the independent-confirmation stage below. This correction does not authorize deployment, invalidate retained raw historical measurements, or establish that the training recipe is now optimal.

## Corrected-runtime result and decision — 2026-09-10

The controlled browser/WASM primary completed 64 color-swapped opening pairs at 1,000 ms per move, one concurrent game. The corrected width-256 candidate scored 67–61 against handcrafted, or 52.34%, with a 95% opening-paired bootstrap interval of 43.75%–60.94%. All 5,987 search records qualified: every nonterminal move was searched, and each terminal record explicitly reported no legal move. There were no missing moves, emergency fallbacks, capped games, or unsearched moves. P95 and p99 elapsed time were 1,000.1 ms, maximum elapsed time was 1,003.2 ms, and measured peak Chrome process-group RSS was 1,849.7 MiB on this Ubuntu host.

The matched 16-pair secondary check at 100 ms scored 17–15, or 53.12%, with a 34.38%–71.88% interval. Across the 16 openings shared by both schedules, the 1,000 ms minus 100 ms relative score difference was −6.25 percentage points with a −34.38 to +21.88 point interval. This does not demonstrate a relative benefit from the longer budget; it does establish that the 1-second browser path meets the measured timing behavior on this host.

The export diagnosis favored the serializer-consistent correction but remained development evidence. On 8,277 nonterminal positions, full-precision MAE fell from 550.3 to 44.8 score units, and integer reference, native full/incremental/trace, and WASM evaluation agreed exactly on all 8,278 audited positions. Mean restricted teacher regret fell from 0.035458 to 0.012169; the corrected-minus-legacy opening-cluster interval was −0.054589 to +0.000993. The separate native corrected-versus-legacy match scored 27–37, with a 31.25%–53.12% interval, so a game-strength improvement from the correction is not established.

Keep the serializer-consistent correction in the working runtime. The 1-second browser score interval crosses 50%, uses reused development openings, and does not authorize NNUE promotion or a production timing change. Before any new training, the next CPU-only selection step is to re-screen at most two retained checkpoints under the corrected runtime, because their earlier selection used the legacy implementation. New GPU work still requires an explicit Vast.ai rental request with a concrete cost envelope. The complete report is [step6-result-report.md](../r0/anhoku-strength-first/step6-result-report.md).

## Corrected-runtime checkpoint selection — 2026-09-10

The predeclared two-checkpoint rescreen is complete. Update 2,048 scored 76–52 against handcrafted in 64 color-swapped browser/WASM opening pairs at 1,000 ms per move, or 59.38%, with a 95% opening-paired bootstrap interval of 51.56%–67.19%. Update 1,920 had scored 67–61, or 52.34%, on the same opening groups and execution protocol. The paired update-2,048 minus update-1,920 difference was +7.03 percentage points, with a −3.12 to +17.19 point interval.

The frozen conservative point-estimate rule therefore selects **update 2,048** as the development checkpoint. All 5,912 challenger searches qualified, with no missing, fallback, unsearched, or capped games. P95 and p99 elapsed time were 1,000.1 ms and maximum elapsed time was 1,003.2 ms. Its full-precision/integer/native/WASM qualification passed on 1,024 positions with zero mismatches; corrected export quantization MAE was 18.44 score units.

This selection uses checkpoints, openings, and matches already exposed during development. The result nominates update 2,048 for the next bounded experiment; it is not independent confirmation and does not authorize production use. Stop retained-checkpoint screening here. Any next training treatment that needs a GPU requires a concrete cost envelope and explicit Vast.ai rental permission. The complete report is [step7-result-report.md](../r0/anhoku-strength-first/step7-result-report.md).

## Independent-seed development replication — 2026-09-11

The third and final initial-tournament run completed from a fresh width-256 scratch seed on the unchanged audited 880k training and 60k validation records. It kept the features, objective, optimizer, schedule, 4,096-update exposure, and development openings fixed. The lowest validation loss was 0.012594 at update 2,688. The frozen equal-node comparison selected update 2,688 over update 2,048: their 256-game scores against handcrafted were 57.03% and 49.22%.

The selected model passed full-precision/integer/native/WASM qualification on 1,024 positions. On the MacBook Pro under the frozen one-game-at-a-time primary protocol, it scored 163–93 against handcrafted over 256 games at 1,000 ms per move, or 63.67%, with a grouped 95% interval of 55.58%–71.77%. This replicates the recipe's development strength across initialization seed. Because it reuses the corpus and opening groups, it is model A evidence rather than the independent-data model B required for final confirmation. Handcrafted remains the production recommendation. The complete report is [step8-result-report.md](../r0/anhoku-strength-first/step8-result-report.md).

## Independent-data replication and confirmation — 2026-09-14

The Step 9 lane generated and froze an independent 880,000-record training corpus and 60,000-record validation corpus under [step9-model-b-data-contract-v2.json](../r0/anhoku-strength-first/step9-model-b-data-contract-v2.json). Model B then completed all 4,096 optimizer updates. Its preregistered update-2,688 network passed the 1,024-position full-precision, integer-reference, native, and WASM parity qualification with zero mismatches.

The initial 1,000 ms confirmation was stopped before completion to reduce elapsed time. Its partial results remain unopened and excluded. A new prospective [100 ms contract](../r0/anhoku-strength-first/step9-confirmation-100ms-contract-v1.json) froze two fresh disjoint 2,048-pair suites, the same two predesignated u2,688 models, 4,096 games per model, one concurrent game, and the unchanged one-sided paired success rule before either new game ran.

Both complete schedules were kept sealed until they finished. Model A scored 2,324–1,771–1, or 56.7505%, with a one-sided 95% lower bound of 55.5928%, and passed. Independent-data model B scored 1,992–2,103–1, or 48.6450%, with a lower bound of 47.4931%, and failed. All 8,192 games and artifact bindings passed the frozen analyzer with zero fallbacks or maximum-ply games. Because both models had to pass, the confirmation failed. The complete record is [step9-result-report.md](../r0/anhoku-strength-first/step9-result-report.md).

This 100 ms native result does not establish the original 1,000 ms production claim or qualify the shipped WASM path. Those limitations cannot change the decision: the only completed prospective confirmation failed its frozen conjunction. Handcrafted remains the production recommendation, and no deployment is authorized. These confirmation schedules must not be extended or rerun to rescue the confirmation; the opened suites may be reused only as exploratory development history under the linked diagnosis runbook.

The next optional work is the bounded exploratory diagnosis in the [Step 9 post-confirmation diagnosis runbook](anhoku-nnue-strength-first-step9-diagnosis-runbook.md). It first audits existing identities, recipes, and corpus summaries, then uses fixed matched 512-opening subsets for crossover and Model B u768 checks on the MBP. This work cannot rescue the failed confirmation and must stop after its diagnostic report. The completed [diagnosis report](../r0/anhoku-strength-first/step9-diagnosis-report.md) and [machine-readable report](../r0/anhoku-strength-first/step9-diagnosis-report.json) record the bounded outcomes and remaining unresolved hypotheses.

The exact selected checkpoint, selected integer network, update-2,048 checkpoint, final checkpoint, and evaluation receipts are preserved locally. Two early historical full-precision checkpoints were not fully transferred before the training instance was destroyed. A bounded recovery replay confirmed the same initial predictions and boundary metrics but could not reproduce post-update CUDA artifacts byte for byte. This limits complete archival rehashing of the historical graph; it does not change the selected-model result. Both Vast instances are destroyed. Training and recovery charges totaled $2.266.

## Next execution sequence

1. **Completed: establish the controlled 1-second production-path baseline (CPU only).** The Step 6 browser/WASM run and its 100 ms secondary check record the required fixed-time score, search qualification, timing, memory, load, and provenance evidence. A second supported target device is still required before production use.
2. **Completed: diagnose integer-export versus full-precision disagreement and reselect the retained checkpoint (CPU only).** Step 6 identified and fixed the feature-transformer scale defect; Step 7 stopped after the two predeclared checkpoints and selected update 2,048 under the corrected runtime.
3. **Completed: replicate the compact recipe across initialization seed.** Step 8 trained the final initial-tournament seed, selected update 2,688, passed parity, and beat handcrafted under the 256-game primary development protocol.
4. **Completed: prepare and train independent-data model B.** The separately generated 880k/60k corpus passed its exclusions and audits, Model B completed training, and the preregistered u2,688 export passed parity qualification.
5. **Completed: run independent confirmation.** The separately frozen 100 ms native endpoint completed without opening interim results. Model A passed and model B failed, so the conjunction failed. End this candidate lane with handcrafted retained in production. Any future NNUE work starts from a new development hypothesis and prospective contract.

## Evidence behind the reset

| Observation | Implication |
| --- | --- |
| The original plan used a coupled C/16 teacher/initialization, very short training, repeated or stochastic validation, and matches contaminated by missing moves/fallbacks. | Its negative Elo does not establish an NNUE ceiling or justify a feature redesign. |
| Replacement R4 has a passing report and a 1M-record pilot: 880k training, 60k validation, 60k development, handcrafted 10k-node labels. | Reuse this audited starting corpus; do not regenerate it just to rename the workflow. |
| R5 seed 17 reduced development Huber loss from 0.016006 for C/16 to 0.015180, yet scored 20–44 against C/16 in 64 repaired equal-node games. | Scalar teacher fidelity is not a sufficient checkpoint selector. The reported approximate interval supports a serious regression, not a precise strength estimate. |
| The same candidate went 7–9 against handcrafted in only 16 games. | Its actual handcrafted gap is unresolved. Do not infer it transitively from C/16 matches on other openings. |
| All R5 recipes stopped at 768 updates before their plateau criterion. | More training is a hypothesis worth testing, not an established cure. |
| Compact width 256 exports about 83 MB versus 161 MB at width 512; native incremental medians were 2.64 versus 4.18 microseconds. | Test compact strength at fixed time early. Microbenchmarks do not prove browser speed or Elo. |
| R5 repair v2–v3f accumulated terminal, incomplete-label, identity, quota, cache, and replenishment repairs around exhaustive child labeling. | Preserve the actual correctness fixes, but remove exhaustive regret construction from the mandatory training path. |

Code inspection adds two practical findings. `scripts/r5_trainer.py` hardcodes batch size, score normalization, and parts of the LR schedule; a JSON edit alone does not implement a new recipe. It repeatedly writes `last.pt` and finally exports that checkpoint, so earlier metric rows do not guarantee recoverable earlier weights. The new runner must retain the actual selected checkpoints and prove resume works.

The current handcrafted evaluator is material plus legal-move mobility (`haitaka_wasm/src/lib.rs::handcrafted_static_eval`). Gold-like feature identity collisions are already known. Neither the importance of those collisions nor the optimal learning objective has been established by a valid strength ablation.

Local evidence inspected: the three requested plans; R1 and R9; `r0/anhoku-reboot/r5-fixed-data-exposure-contract.json`; replacement R4's gate; R5's phase-status and runtime reports; the trainer, NNUE runtime, and reusable diagnostic worker. These reports were read, not freshly reproduced or comprehensively rehashed. The v3f amendment exists in the dirty plan, but its named output directory was absent during this inspection; do not call v3f completed.

## Keep the foundations; simplify the experiment

Keep independent generation/teacher/initialization identities, finite validation, explicit score orientation and mate semantics, unknown-result masking, full/integer/runtime parity, legal interrupted-search results, paired games, and immutable model/data identities. Requalify affected components after a relevant change; do not rerun every historical phase for an unrelated trainer change.

Use one new experiment directory per run with a resolved configuration, source/build/input hashes, metrics, selected full-precision weights, matching export, last resumable state, raw games, and a short decision. Retain selected and last checkpoints locally before releasing rented compute. Missing historical weights remain missing; exports can be measured only as integer models.

Retire these prerequisites for this new program:

- Completing R5R-01 through R5R-04 before any different experiment.
- A 0.005 Huber improvement over C/16 as permission to test strength or add data.
- Three training seeds before learning whether the first candidate plays well.
- All-legal-child 250k-node labels and power calculations before every pilot.
- Exact convergence as a validity condition. A budget-limited run is valid evidence at its actual exposure, with convergence explicitly unproven.
- Mandatory 10M → 50M → 100M scaling. Scale only when strength evidence supports it.

Correctness failures still invalidate affected measurements. An ordinary losing model, incomplete optional label, or statistically inconclusive development match is an experimental outcome, not a reason to rebuild the workflow.

## Step 1 — Establish today's baseline and runnable loop

Time box: one focused engineering day, followed by the measured CPU match time. Report an overrun with the exact unresolved defect; do not silently add another phase.

1. Snapshot the commit and relevant dirty files without changing the user's work. Inventory the 1M splits, C/16, R5 seed-17/29 exports, and compact export; hash required inputs and verify their manifests. Separate unavailable files from invalid files. Check active local/remote jobs before scheduling work.
2. Reuse R1 feature/sign/export/interruption/history tests and the existing paired harness. Run focused tests against the actual build and model hashes. Verify aggregation from raw games and an identical-engine null/replay fixture. Width 256 must use its matching runtime build.
3. Freeze one reusable development schedule of 128 color-swapped opening pairs. Run C/16, R5 current seed 17, and R5 compact directly against handcrafted at 10k nodes/move. Use the same openings across comparisons. Include seed 29 only if needed to diagnose seed sensitivity.
4. For a new baseline, run 64 development pairs at 1,000 ms/move for C/16 and compact against handcrafted on one quiescent production-class host. Use the production browser/WASM path where available; a native measurement is labeled native and cannot stand in for the shipped path.
5. Record score rate and paired uncertainty, fallback/missing-move counts, termination reasons, combined alpha-beta/qsearch nodes, completed depth, p95/p99 lateness, memory, and model load time. Keep comparisons on different execution environments separate.

Do not wait for every baseline game before preparing the trainer. These are finite development screens, not promotion proofs. Exit with valid baseline artifacts and one command sequence that takes a retained checkpoint through export, parity, and a small paired match. If an existing export already looks competitive, confirm that result before training an elaborate replacement.

## Step 2 — Repair only what the next training experiment needs

Create a separately versioned runner from the reusable pieces of `scripts/r5_trainer.py`; leave historical R5 semantics intact. Use the existing 1M splits and DonorSingleEff features.

Required changes:

- Resolve and validate all actual hyperparameters from configuration, including parameter-group LR multipliers. Reject unused or mismatched settings.
- Preserve finite final partial batches. Use a deterministic seed-and-epoch shuffle for training, with validation consumed once in a fixed order. Record examples, passes, updates, and the data cursor separately.
- Save immutable best-validation, scheduled strength-screen, and final checkpoints, plus an atomic resumable state containing optimizer, schedule, RNG, and data cursor. Test an interrupted run against uninterrupted execution on the same environment.
- Export the specific selected checkpoint, not whichever file was written last. Check full precision → integer reference → native/WASM parity; quantization error is measured separately from exact integer/runtime equivalence.
- Log train/validation loss by phase, score range, tactical/check status, donor state, and feature coverage. Log output scale, saturation, gradient/update magnitudes, and time spent loading, training, validating, and exporting.

Tiny overfit and resume fixtures are sufficient here; do not create a general training framework. Record the cost of a short throughput probe and ask for the required explicit GPU-rental permission with a concrete total envelope before paid execution.

## Step 3 — A bounded training tournament on existing data

Default candidate family: width 256, because it already runs and is cheaper to deploy. Width 512 is the controlled capacity comparator. This is a proposed choice, subject to Step 1's fixed-time evidence, not a claim that compact is stronger.

First train one scratch compact model with the existing robust Huber target, batch 16,384, LR 0.0015, 64-update warmup, then cosine decay to 0.00005 over a maximum 4,096 updates. Preserve existing optimizer group multipliers explicitly. This tests a new optimization recipe; it does not isolate duration from schedule. Select new seeds after inventory. Keep WDL loss disabled.

Evaluate offline every 128 updates; retain exports at 256, 1,024, 2,048, and 4,096, and the best-validation checkpoint. At the first three scheduled checkpoints run 64 development pairs versus handcrafted at 10k nodes. These reused games guide development only. Stop further training if two successive screens have a paired upper 95% score bound below 0.45, or on numerical/correctness failure. A single small losing screen is not proof of failure. Record a budget stop as such, not as convergence.

At completion, compare at most two checkpoints: best validation and best scheduled game screen. Evaluate both on the fixed 128-pair development suite, then give the better one a 128-pair 1,000 ms primary screen and a separately frozen smaller 100 ms secondary check. Prefer fixed-time score, then equal-node score, then lower runtime cost; offline loss breaks remaining ties. Retain all selection history and make no confirmatory significance claim on these reused openings.

Choose at most two follow-up training lanes from the evidence:

| Finding | Next experiment |
| --- | --- |
| Compact gains strength but seems capacity-limited | Matched width-512 run with the same data, objective, exposure, and seed; compare actual strength/time. |
| Training signal is weak or updates are ineffective | A bounded LR/batch or initialization diagnostic, with exposure and optimizer updates both reported. |
| Loss improves but games do not | Objective/score/search diagnosis in Step 4, before more data. |
| Equal-node play improves but fixed time loses | Profile the shipped runtime and test one measured hotspot; do not assume more labels solve it. |

Initial tournament ceiling: three full training runs, 24 total GPU-hours, and $25 total rental cost, whichever is reached first. These are proposed caps, not authorization or live price estimates. Reduce the horizon before launch if the throughput forecast cannot fit. No seed replication yet unless a recipe produces a credible development gain.

## Step 4 — Diagnose the failure that actually occurred

Use a reusable development set from training/validation-independent sources. It may overlap previously viewed development history if disclosed; it is never relabeled as sealed confirmation evidence.

For move diagnosis, select at most 128 parents from recorded disagreements and representative controls. Compare the moves chosen by handcrafted, C/16, and the candidate, plus a deeper handcrafted root search. Label only that fixed union of at most four legal moves per parent, with equal per-child budget and correct parent score orientation. Call the result restricted-candidate preference/regret: it is not exhaustive legal-move regret. Report incomplete labels and coverage; no quota refill is required. Keep terminal outcomes explicit and compare complete common-support cases without hiding attrition. Biased or sparse coverage makes this diagnostic inconclusive, not a promotion result.

A 250k-node ceiling here is at most 128 million child-search nodes, plus explicitly budgeted root/model searches. Stop at two CPU wall-days; unfinished diagnostic work does not block otherwise valid direct games.

Inspect score/search interaction separately: score slope and offset, near-equal choices, tactical/mate handling, pruning sensitivity, and full/export move differences. A positive score rescaling can change pruning and hence strength. Any calibration experiment changes one declared factor, includes all evaluation-dependent search paths, and is selected on development only.

If scalar loss and move quality disagree, compare one alternative objective on the same corpus: for example, calibrated soft-score loss emphasizing the competitive score range. Derive calibration from training/development evidence and preserve runtime score units. Keep the old objective as control. Do not introduce WDL mixing until real completed outcomes, orientation, and masking are available.

Consider pairwise move supervision only if the preference diagnostic supports it. Consider a minimal piece-identity feature change only if collision attribution supports it. Both require new explicit experiments and matching packer/trainer/serializer/runtime changes; neither is a prerequisite to the first useful model.

## Step 5 — Grow the successful recipe, not just the corpus

Once a candidate shows development strength progress, compare nested 250k and 880k training subsets with group boundaries intact. Use the same recipe and report both fixed-exposure and optimization-completeness limitations. Existing validation/development remains outside every training subset.

If the larger set helps games, generate one additional rung, initially up to 3M total training positions. Freeze trajectory policy separately from label teacher. Include positions encountered by the candidate and handcrafted, with controlled exploration and a retained broad baseline mixture. Deduplicate canonical boards and split entire source games before selecting records. Shared opening strata do not make raw source games independent; preserve both identities and cluster estimates at the defensible source level.

Benchmark label cost and compare 10k versus a deeper budget on a small development sample before buying a larger corpus. A higher node budget is not automatically a better teacher. Search distillation can transfer deeper lookahead into a static evaluator, but whether that improves Anhoku strength must be measured.

After the first successful rung, try 10M only if the measured gain and forecast cost justify it. Later rounds can use a frozen stronger NNUE-search teacher, with replay from earlier data to monitor forgetting. Add trustworthy terminal-result supervision as a separate ablation. Do not recursively train from an unqualified new model or assume self-play improvement.

Each scaling rung has one candidate, one direct incumbent/handcrafted comparison, and a written compute/storage cap before launch. No automatic 50M/100M run.

## Step 6 — Confirm and ship

Nominate model A from development results. Freeze its recipe and train model B with independent training and newly generated data seeds. B is replication, not a replacement selected after seeing final games.

Before either final suite is opened:

- Freeze separate unseen opening-source groups for A and B, their generation policy and overlap audit, and all model/build hashes. All previously viewed openings are development history. Novel seed numbers alone do not prove independence.
- Exercise the actual shipped WASM artifact, production interface, full search path, repetition/DFPN behavior, and timers on the target host. Freeze numeric memory, load-time, and p95/p99/max-lateness limits from the product's requirements before final testing. These limits remain to be concretized in Step 1; no promotion launch with placeholders.
- Choose a fixed sample size from development pair/group variance for 80% power at a proposed +20 Elo effect and one-sided 5% error. Validate interval coverage and grouping with fixtures/simulation. If the resulting campaign is unaffordable, defer confirmation; do not lower its success threshold after seeing games.
- Freeze adjudication, including a maximum-ply policy. If capped games occur, preserve their status and require a conservative scoring sensitivity bound to remain positive; never turn them into genuine draw labels for training. Missing/illegal moves and emergency fallbacks invalidate the run.

For each model separately, require the lower one-sided 95% bound on paired score versus handcrafted to exceed 0.5 at 1,000 ms/move, equivalently a lower Elo bound above zero. Use the highest independent opening-source group, complete scheduled pairs, and no interim selection or sample extension. Requiring both independent models to pass is the conjunction criterion; freeze the statistical implementation before opening either suite. Report equal-node results as supporting evidence, not the promotion endpoint.

Also require parity, tactical, portability, memory, loading, and timing checks to pass, including a second supported host. Ship the predesignated A only after both confirmations pass. A failed or inconclusive confirmation leaves handcrafted as the production recommendation and that opening block becomes development history. C/16 remains the historical NNUE control until replaced by an explicitly designated incumbent.

## Execution priorities and budget discipline

The first cycle should produce a valid direct baseline and a new model-strength curve within roughly one engineering week, conditional on hardware availability and approval. This is a planning target, not a performance guarantee. Report measured duration after the initial probe.

Use mbp, mba, local Ubuntu, and nubu for suitable independent CPU work according to availability and the recorded approximate 10:7:2:1 capacities. Fixed-time strength evidence uses one concurrent game on a controlled host; do not pool those results with loaded multi-game workers. Paid GPU rental always requires explicit permission under `AGENTS.md`. The user-designated MBP endpoint is currently `192.168.0.12`.

After two engineering days without an executable experiment, cut optional infrastructure scope and identify the precise correctness blocker. After the initial three training runs, end with a candidate or a supported next hypothesis; do not launch another schedule extension automatically. Retain about 25 GiB for the initial new cycle, with a checkpoint-size forecast before execution; historical artifacts are outside this cap and must not be deleted to meet it.

The execution sequence is complete. The confirmation failed and no further compute is scheduled. Handcrafted remains the production evaluator; any future NNUE program requires a new development hypothesis, fresh prospective evidence, and explicit GPU rental permission when applicable.

## Reference anchors

- [Original strength plan](anhoku-nnue-handcrafted-strength-plan.md)
- [Workflow reboot and historical audit](reboot/anhoku-nnue-training-workflow-reboot.md)
- [R5 results and repair history](reboot/reboot-5-fixed-data-exposure-and-1m-workflow-pilot.md)
- [Existing production promotion requirements](reboot/reboot-9-selection-and-promotion.md)
- [R5 machine-readable status](../out/anhoku-reboot-r5-gpu/r5-phase-status.json)
- [R5 measured native runtime](../out/anhoku-reboot-r5-gpu/r5-runtime-diagnostic.json)
- [Current trainer](../scripts/r5_trainer.py)
- [Current native/WASM NNUE implementation](../haitaka_wasm/src/nnue.rs)

External practice is supporting context, not evidence that Anhoku will improve: the [official Stockfish NNUE trainer](https://github.com/official-stockfish/nnue-pytorch) includes match-based network testing, and [Stockfish's testing guide](https://official-stockfish.github.io/docs/fishtest-wiki/Creating-my-first-test.html) describes testing new nets at short and long time controls. This plan adopts early direct strength measurement; it does not copy chess hyperparameters or require a new SPRT implementation.
