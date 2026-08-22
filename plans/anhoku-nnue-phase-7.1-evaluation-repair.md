# Anhoku NNUE Phase 7.1 evaluation repair

## Objective

Repair the Phase 7.1 strength evaluation without retraining the existing four
lanes. Reuse the 32 preserved NNUE exports, rerun statistically valid paired
matches, regenerate automatic selection, and update the Phase 8 gate.

Work only in the `strengthen-phase-3` worktree. Do not make these changes on
the current `strengthen` branch.

## Current verdict

Phase 7.1 is **training complete, strength evaluation incomplete**.

The data split, four training lanes, 32 NNUE exports, verifier results, hashes,
and deterministic offline evaluations are usable. The current match-derived
claims are not usable:

- all 41 reports used one start SFEN, `openingRandomPlies=0`, seed `7103`,
  `threads=64`, and 100 ms;
- 1,319 of 5,120 games (25.8%) had one side search zero nodes and ended after
  one played move;
- the 64-game screens were worst: 1,124 of 2,048 games (54.9%) had a zero-node
  side;
- C/step-2 and C/step-4 have the same NNUE SHA-256 but scored `+13.6` and
  `-71.6 Elo` in separate 256-game runs under nominally identical settings;
- D/step-8's 64-game report has aggregate NPS 0 and CI `[0,0]`;
- all candidate screening, extension, and confirmation used the same opening
  seed, so checkpoint selection also contains winner's-curse bias.

Therefore do not select or promote B/step-10 from its reported `+52.0 Elo`, and
do not use its CI lower bound for the Phase 8 gate. The `-281.6 Elo` handcrafted
number is also invalid as a precise estimate, although its negative direction
is consistent with the earlier healthy approximately `-234 Elo` benchmark.

## Mandatory repair

### 1. Preserve and mark the old evidence

- Do not delete or overwrite `out/anhoku-v0.6-phase7.1/matches/`.
- Mark every existing match report as invalid in the result document because
  of CPU starvation, a single repeated opening, and no independent winner
  confirmation.
- Put repaired matches in a new directory such as
  `out/anhoku-v0.6-phase7.1/matches-rerun-v2/`.
- Preserve the 32 NNUE files and verify their hashes before starting.

### 2. Calibrate the match host before screening

Run corrected v0.5.1 against itself first:

- 64 games / 32 color-swapped pairs;
- `--movetime-ms 100`;
- `--threads 20` or fewer;
- `--opening-random-plies 4`;
- seed `7103`;
- `--max-plies 200`;
- unchanged Anhoku start SFEN and unchanged corrected v0.5.1 anchor hash
  `1c6ffefb34fe53137d33c3ccd5668dc507c4b11e4841cf6c6670167a4d26380f`.

Calibration acceptance:

- 32 unique paired start SFENs;
- zero games with either side's `totalNodes == 0`;
- nonzero and comparable NPS for both sides;
- no protocol or failure state;
- paired CI contains 0 Elo.

If any zero-node game occurs, discard the run and reduce workers to 10, then 5
if necessary. Do not continue screening until calibration passes.

Also run one cheap deterministic smoke twice with one worker and equal fixed
depth. The two reports should be byte-equivalent after excluding timestamps and
paths. This checks the harness separately from movetime scheduling.

### 3. Rerun the fixed-anchor screen

Screen every **unique NNUE SHA-256** once against corrected v0.5.1:

- 64 games / 32 paired openings;
- the calibrated worker count;
- 100 ms;
- random opening plies 4;
- seed `7103`;
- max plies 200.

If several checkpoints have the same NNUE SHA-256, map the one match report to
all those checkpoints. Do not rerun identical bytes and then interpret timing
noise as a checkpoint difference.

Every screen must have 32 unique start SFENs and zero zero-node games. Treat a
failed quality gate as an invalid run, not as a model loss.

### 4. Extend candidates and select automatically

- Within each lane, choose the two best valid screen estimates that also have
  competitive ID loss.
- Extend each to 256 games / 128 paired openings with the same seed and opening
  policy.
- Preserve per-game JSONL, aggregate reports, pentanomial bins, nodes, NPS,
  timing, engine hashes, and exact command settings.
- Rank by valid paired strength; ID loss is a guardrail, not the final metric.

### 5. Use an independent confirmation set

After choosing the overall winner, run a new confirmation that was not used to
select it:

- at least 1,024 games / 512 paired openings;
- seed `7104`;
- random opening plies 4;
- 100 ms and the calibrated worker count;
- corrected v0.5.1 fixed anchor.

The Phase 7.1 non-inferiority gate passes only if this independent paired 95% CI
has lower bound greater than `-10 Elo`. Do not use the seed-7103 selection CI as
the final gate. Continue toward 4,096 games if the independent result remains
inconclusive and the written sequential rule requires it.

### 6. Run handcrafted only after the independent anchor gate

Only if the independent v0.5.1 gate passes, compare the confirmed winner to the
handcrafted evaluator:

- 1,024 games;
- random opening plies 4;
- a fixed recorded seed not used to select the winner;
- 100 ms;
- calibrated worker count;
- zero zero-node games and diverse paired starts.

## Interpretation after rerun

- B outperforming A with valid independent strength evidence supports the
  lower-learning-rate hypothesis.
- C degrading from the anchor while ID loss improves supports a harmful v0.6
  data-gradient or objective-calibration hypothesis.
- C versus D cannot be decided by comparing their combined loss values because
  lambda changes the metric itself. A lambda conclusion requires `loss_eval`
  and `loss_result` on the same fixed examples.
- Runtime validation and offline validation must remain separate: runtime used
  smart filtering and random-FEN skipping 3, while offline evaluation used an
  unfiltered loader and skipping 0.

## Secondary instrumentation gap

Train loss was not persisted. This does not block the mandatory strength
repair. If strict completion of every Phase 7.1 instrumentation item or the
lambda hypothesis is required, run a separate, explicitly labelled C/D 16-step
diagnostic with per-step `train_loss`, `loss_eval`, and `loss_result` on one
fixed ID/OOD sample. Do not silently mix newly trained exports with the original
32-candidate selection.

## Result-document corrections

Update `docs/nnue-training-anhoku-v0.6-phase7.1.md` after the repaired matches:

- replace the current answer-first conclusion and all selection tables;
- label the old 41 match reports invalid and retain their paths for audit;
- report calibration results and match-quality gates;
- list duplicate checkpoint-to-NNUE-hash mappings;
- report the independent seed-7104 gate separately from selection;
- recompute the automatic winner and handcrafted condition;
- keep Phase 8 blocked unless all written gates pass;
- correct the ancestry statement: `1c251f5` is the direct parent of `6c59328`,
  not a non-ancestor;
- preserve the existing recovery archive and create a new checksum-recorded
  archive containing the repaired reports and updated document.

## Completion criteria

- old invalid evidence is preserved and clearly excluded;
- host calibration passes with zero zero-node games;
- every unique NNUE hash receives one valid fixed-anchor screen;
- all 32 checkpoints map to a screened NNUE hash;
- lane extensions complete under the same valid opening pairs;
- the winner receives an independent seed confirmation;
- handcrafted runs only if the independent anchor gate passes;
- automatic selection and Phase 8 status are regenerated from valid evidence;
- focused tests, `cargo fmt --all -- --check`, and `git diff --check` pass;
- the updated result document clearly distinguishes established facts,
  invalidated claims, and remaining uncertainty.
