# Anhoku NNUE v0.6 Phase 7.1 evaluation-repair result

Status: training and strength evaluation are complete; the original match
evidence is invalidated and preserved. The repaired automatic winner is lane C,
step 16, NNUE SHA-256
`049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0`.

Its repaired 256-game selection result was `+27.2 Elo` with paired 95% CI
`[+0.2, +54.2]`. The independent seed-7104 confirmation was `+7.8 Elo` with
paired 95% CI `[-5.9, +21.5]`, so the written non-inferiority gate (lower bound
greater than `-10 Elo`) passes. The same confirmed model scored `-128.4 Elo`
against handcrafted in the conditional 1,024-game comparison, CI
`[-150.4, -106.3]`; it is not a general-strength promotion candidate.

Phase 8 remains **blocked** because the required reviewed OOD-v2 suite plan
with at least 64 opening IDs and at least 12 held-out IDs is not yet ready.
The repaired strength gate is therefore recorded separately from the Phase 8
approval decision.

## Scope and identity

- Worktree and branch: `strengthen-phase-3`, HEAD `6c59328`.
- Correct ancestry: `1c251f5` is the direct parent of `6c59328`.
- No training was repeated; all 32 preserved exports were reused.
- The Phase 7 trainer revision was unavailable. The reviewed overlay was run
  on base revision `61666d9e3653e4df9881b14c23f8fdcc4bf7779b`, with transferred
  source snapshot SHA-256
  `5100dd3e65d6cbb6c84a7a9f975ba0d97a643e1547fb3f059308ecd277cf36ed`.
- Host: RTX 4060 Ti 16,380 MiB, 128 vCPUs, driver `580.76.05`, CUDA toolkit
  `12.8`; matcher calibration used 20 workers.
- Corrected v0.5.1 anchor SHA-256:
  `1c6ffefb34fe53137d33c3ccd5668dc507c4b11e4841cf6c6670167a4d26380f`.

The original recovery archive remains unchanged:
`out/anhoku-v0.6-phase7.1/haitaka-anhoku-phase7.1-results.tar.zst`, SHA-256
`d126f1337f8e0d54d1aa8aa0211f32b5025b3eeff7f45af0c13dc4ddd212dc04`.

## Training inputs and invariants

The deterministic split and all pre-training gates remain valid and were not
changed by this repair:

| Binary | Entries | SHA-256 |
| --- | ---: | --- |
| `datasets/train.bin` | 1,661,320 | `26780bc630a19ed891e11bc441eda34d8019e237c2816a42691497c3c15173d4` |
| `datasets/id-validation.bin` | 184,566 | `30910ec58e615cba8374b4d8a39190e94dae873be9903df471ab9e6306195d79` |
| `datasets/legacy-ood-validation.bin` | 265,571 | `48dd24e7648e4da71cd68256d314f51cd4a49067b8a5f0e691abe67037c80751` |

The split used `split_seed=7101`, lowest-SHA shard selection, `shuffle_seed=7102`,
and 65,536-record bounded chunks. Both train/ID distribution and leakage gates
passed. Legacy OOD was retained as a diagnostic only, not as a selector.

| Set | Draw | Mate-score | Mean absolute score | Score/result agreement |
| --- | ---: | ---: | ---: | ---: |
| Train | 26.33% | 6.91% | 3,232.38 | 57.98% |
| ID validation | 27.43% | 6.95% | 3,229.83 | 58.43% |
| Legacy OOD | 0.00% | 10.32% | 4,446.43 | 71.40% |

All lanes used `HalfKAv2^+DonorSingleEff`, batch size 16,384, nominal epoch
1,000,000, validation size 100,000, random-FEN skipping 3, trainer seed 1,
checkpoint/validation interval 2, and `max_steps=16`.

| Lane | Initialization | Initial LR | Lambda |
| --- | --- | ---: | ---: |
| A | fresh | 0.0015 | 0.8 |
| B | fresh | 0.0003 | 0.8 |
| C | corrected v0.5.1 warm start | 0.00015 | 0.8 |
| D | corrected v0.5.1 warm start | 0.00015 | 1.0 |

The warm-start importer was repaired before lanes C/D were rerun: a donor-family
anchor must be imported with the exact configured feature family rather than
plain `HalfKAv2`. The focused trainer test covers this path. Train loss was not
persisted by the older Lightning logger because the pilot stopped within the
first epoch; optimizer step, accepted positions, LR, ID loss, and OOD loss are
available in the event files. Offline losses below use the deterministic
unfiltered evaluator and remain distinct from runtime validation scalars.

## Checkpoint/offline evaluation summary

The table maps every saved checkpoint to its preserved NNUE export. Multiple
steps in one row have identical export bytes and therefore share one repaired
match report. `—` is the unavailable train-loss scalar.

| Checkpoints | Accepted positions | LR | Train loss | ID loss | OOD loss | NNUE SHA-256 |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| A/2, A/4 | 32,768; 65,536 | .0015 | — | .04073303; .04072624 | .04220089; .04220088 | `135e5a1653b3ddd22df107f91adff111569c47afddbb2f2b8d4ae8fc93ab0eff` |
| A/6 | 98,304 | .0015 | — | .04068387 | .04219684 | `bd710a4c6f259967f5dea6d52e2c9943304220a1d4131c24da8ea3a9e5350347` |
| A/8 | 131,072 | .0015 | — | .04042707 | .04217329 | `e26dbe489f5ea563a5b294ba243ddaeb597c75d210855fad11c3bf2fcc36ee5f` |
| A/10 | 163,840 | .0015 | — | .04011799 | .04215048 | `b68783dac82a30f75395829c8065d1befe992b8d4a682d35e422f0486dd16632` |
| A/12 | 196,608 | .0015 | — | .04020068 | .04216201 | `8f33564f567492916a120adf762bfb6e5201373be1ab917c288aae98d97c1d6b` |
| A/14 | 229,376 | .0015 | — | .03976077 | .04213082 | `bc082f9b534641cfff3af3a6871733a7975793c07a411c66088699e791ac5f8e` |
| A/16 | 262,144 | .0015 | — | .03924389 | .04206749 | `79790c5c7949c1ff62f3e46db0df84018844476bb7184b4b9f6d83b4b5b91ac0` |
| B/2, B/4 | 32,768; 65,536 | .0003 | — | .04130780; .04130645 | .04302212; .04302211 | `36aeb5be3a5e74649fd49ec26c8f100f2f11fc689ebde6b9b2ebce2bc70e50de` |
| B/6 | 98,304 | .0003 | — | .04129351 | .04301888 | `7062915c9568533f2545d71568a9c3ae122090fa1f3d3c1c0db2ef37a7959e08` |
| B/8 | 131,072 | .0003 | — | .04121656 | .04300044 | `74e217624a18cfeac1476f00807de5a51e2f7a594a112231fb7a7f027612db00` |
| B/10 | 163,840 | .0003 | — | .04112141 | .04297690 | `941506b0f25dd9e251928d3d94903fbe8975a20a197056655d6d1012b8bfc199` |
| B/12 | 196,608 | .0003 | — | .04114750 | .04298507 | `cf1d3efc2df71e1fe8ea444c833a0722cd1808429cfea418aca1ab1ce5bc80fc` |
| B/14 | 229,376 | .0003 | — | .04103297 | .04295857 | `40452e02528be8554c96223d8417f390fcffa3ef3d9b4e640ed797cb47c8a753` |
| B/16 | 262,144 | .0003 | — | .04092336 | .04293774 | `77914537e3dfb61ff5c1b17daac56cdb40384bb9cdeb33091292bb55d1d3901d` |
| C/2, C/4, C/6, C/8, C/12 | 32,768; 65,536; 98,304; 131,072; 196,608 | .00015 | — | .03687328; .03687234; .03684255; .03667391; .03651727 | .04040004; .04039935; .04037747; .04025371; .04014131 | `57e1269bc353a8f8afd2a5946e57d79bf28efb02eb74678b51a200ab78053b29` |
| C/10 | 163,840 | .00015 | — | .03645628 | .04009786 | `9e1c771b9897a97fa77449841ba8af83a06483badc8d664dfdd113dc400eed41` |
| C/14 | 229,376 | .00015 | — | .03621730 | .03993075 | `5273d8b2bf7840161922f705c22645d3ca7530792fc64affddb172e195792f3a` |
| C/16 | 262,144 | .00015 | — | .03588859 | .03970238 | `049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0` |
| D/2, D/4, D/6, D/8, D/12 | 32,768; 65,536; 98,304; 131,072; 196,608 | .00015 | — | .03687319; .03687220; .03683809; .03664341; .03646154 | .04040010; .04039951; .04038063; .04027083; .04017018 | `dc14230c1d93d229e46991b2f2c440169fd66738ede3551b601cc9e79d5ddafd` |
| D/10 | 163,840 | .00015 | — | .03639057 | .04013100 | `7a49db5fe893f089e893cc3dbf82f9540413355762413a5034fe4bd7bcff48ff` |
| D/14 | 229,376 | .00015 | — | .03611467 | .03998008 | `447c3dc61fb425af43c8b5c0c9f52154ee0a5dea86340e7b995fecfb2b954e80` |
| D/16 | 262,144 | .00015 | — | .03573285 | .03977364 | `75fc7c2b45d1decf3846fc740dfe9111ee277004902ae5936640f3dd9a346101` |

Runtime validation curves remain in each lane's TensorBoard event files. At
event steps `1,3,...,15`, the ID/OOD series were:

```text
A ID : .094817 .093700 .094205 .093231 .093144 .093479 .093230 .092330
A OOD: .083010 .083947 .082315 .083489 .083834 .083751 .083083 .082934
B ID : .094809 .094981 .094774 .094516 .094792 .094105 .094251 .094110
B OOD: .084075 .083834 .084045 .083891 .084701 .083058 .082579 .083853
C ID : .086632 .086070 .086384 .085759 .086133 .086325 .085772 .085590
C OOD: .084989 .085817 .084935 .085192 .085072 .084878 .084450 .083602
D ID : .037355 .037111 .037134 .037181 .036806 .037041 .036756 .036354
D OOD: .044162 .043572 .043582 .043027 .043710 .043056 .042849 .043552
```

## Why the old match claims are invalid

The original 41 reports under `out/anhoku-v0.6-phase7.1/matches/` are retained
unchanged but excluded from every decision. They all used one start SFEN,
`openingRandomPlies=0`, seed `7103`, 64 workers, and 100 ms. Across 5,120 old
games, 1,319 had at least one side with zero alpha-beta nodes; the old 64-game
screens alone had 1,124 such games out of 2,048. The old reports also reused one
opening, and C checkpoints with identical NNUE bytes received incompatible
point estimates. The B/10 `+52.0 Elo` and `-281.6 Elo` handcrafted claims must
not be used as estimates.

The old paths remain the audit record. The repaired evidence is isolated under
`out/anhoku-v0.6-phase7.1/matches-rerun-v2/`.

## Calibration and harness checks

Corrected v0.5.1 versus itself passed before screening:

| Games | Pairs | Distinct starts | Zero-node sides | Failures | A/B aggregate NPS | Paired Elo (95% CI) |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 64 | 32 | 32 | 0 | 0 | 17,968 / 18,143 | +32.7 `[-3.3, +68.6]` |

The CI contains zero, and the per-side minimum NPS values were 5,215 and
6,270. The report has no warnings or protocol failures.

The first depth-3/200-ply smoke was stopped after proving it was not cheap on
this variant. A replacement two-pass smoke used one worker, equal fixed depth
2, two games, 20 plies, seed 7103, and four random opening plies. Both passes
had identical moves, results, node counts, and search telemetry. Raw reports
differed only in wall-clock-derived elapsed/NPS fields; after removing those
fields plus generated timestamps and paths, the reports were byte-equivalent.

## Repaired fixed-anchor screens

All repaired matches used the unchanged anchor, explicit Anhoku start SFEN,
`threads=20`, `movetime-ms=100`, `max-plies=200`, random opening plies `4`,
seed `7103`, and the alternating color-swapped pair convention. Every screen
had 64 games, 32 pairs, 32 distinct start SFENs, zero zero-node sides, and no
failure state. The 22 unique NNUE hashes below cover all 32 checkpoints.

`Bins` are `[0-0, 0.5-0, 1-1, 1.5-0, 2-0]` candidate pair scores.

| Checkpoints | NNUE SHA-256 | ID loss | W-L-D | Paired Elo (95% CI) | Bins |
| --- | --- | ---: | ---: | ---: | --- |
| A/2, A/4 | `135e5a1653b3ddd22df107f91adff111569c47afddbb2f2b8d4ae8fc93ab0eff` | .040733/.040726 | 17-47-0 | -176.7 `[-280.2,-73.1]` | 18,0,11,0,3 |
| A/6 | `bd710a4c6f259967f5dea6d52e2c9943304220a1d4131c24da8ea3a9e5350347` | .040684 | 16-48-0 | -190.8 `[-298.7,-83.0]` | 19,0,10,0,3 |
| A/8 | `e26dbe489f5ea563a5b294ba243ddaeb597c75d210855fad11c3bf2fcc36ee5f` | .040427 | 27-36-1 | -49.2 `[-130.6,+32.2]` | 9,1,17,0,5 |
| A/10 | `b68783dac82a30f75395829c8065d1befe992b8d4a682d35e422f0486dd16632` | .040118 | 25-39-0 | -77.2 `[-160.6,+6.1]` | 11,0,17,0,4 |
| A/12 | `8f33564f567492916a120adf762bfb6e5201373be1ab917c288aae98d97c1d6b` | .040201 | 24-40-0 | -88.7 `[-154.0,-23.5]` | 9,0,22,0,1 |
| A/14 | `bc082f9b534641cfff3af3a6871733a7975793c07a411c66088699e791ac5f8e` | .039761 | 22-42-0 | -112.3 `[-191.3,-33.3]` | 12,0,18,0,2 |
| A/16 | `79790c5c7949c1ff62f3e46db0df84018844476bb7184b4b9f6d83b4b5b91ac0` | .039244 | 25-39-0 | -77.2 `[-172.2,+17.7]` | 13,0,13,0,6 |
| B/2, B/4 | `36aeb5be3a5e74649fd49ec26c8f100f2f11fc689ebde6b9b2ebce2bc70e50de` | .041308/.041306 | 16-48-0 | -190.8 `[-298.7,-83.0]` | 19,0,10,0,3 |
| B/6 | `7062915c9568533f2545d71568a9c3ae122090fa1f3d3c1c0db2ef37a7959e08` | .041294 | 16-48-0 | -190.8 `[-290.7,-91.0]` | 18,0,12,0,2 |
| B/8 | `74e217624a18cfeac1476f00807de5a51e2f7a594a112231fb7a7f027612db00` | .041217 | 12-52-0 | -254.7 `[-364.1,-145.4]` | 21,0,10,0,1 |
| B/10 | `941506b0f25dd9e251928d3d94903fbe8975a20a197056655d6d1012b8bfc199` | .041121 | 19-45-0 | -149.8 `[-245.7,-53.9]` | 16,0,13,0,3 |
| B/12 | `cf1d3efc2df71e1fe8ea444c833a0722cd1808429cfea418aca1ab1ce5bc80fc` | .041148 | 24-40-0 | -88.7 `[-186.6,+9.1]` | 14,0,12,0,6 |
| B/14 | `40452e02528be8554c96223d8417f390fcffa3ef3d9b4e640ed797cb47c8a753` | .041033 | 26-38-0 | -65.9 `[-152.4,+20.5]` | 11,0,16,0,5 |
| B/16 | `77914537e3dfb61ff5c1b17daac56cdb40384bb9cdeb33091292bb55d1d3901d` | .040923 | 31-33-0 | -10.9 `[-75.7,+54.0]` | 5,0,23,0,4 |
| C/2, C/4, C/6, C/8, C/12 | `57e1269bc353a8f8afd2a5946e57d79bf28efb02eb74678b51a200ab78053b29` | .036873/.036872/.036843/.036674/.036517 | 35-29-0 | +32.7 `[-14.7,+80.1]` | 1,0,27,0,4 |
| C/10 | `9e1c771b9897a97fa77449841ba8af83a06483badc8d664dfdd113dc400eed41` | .036456 | 30-34-0 | -21.7 `[-82.7,+39.2]` | 5,0,24,0,3 |
| C/14 | `5273d8b2bf7840161922f705c22645d3ca7530792fc64affddb172e195792f3a` | .036217 | 29-35-0 | -32.7 `[-68.6,+3.3]` | 3,0,29,0,0 |
| C/16 | `049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0` | .035889 | 34-30-0 | +21.7 `[-30.9,+74.4]` | 2,0,26,0,4 |
| D/2, D/4, D/6, D/8, D/12 | `dc14230c1d93d229e46991b2f2c440169fd66738ede3551b601cc9e79d5ddafd` | .036873/.036872/.036838/.036643/.036462 | 31-33-0 | -10.9 `[-59.1,+37.4]` | 3,0,27,0,2 |
| D/10 | `7a49db5fe893f089e893cc3dbf82f9540413355762413a5034fe4bd7bcff48ff` | .036391 | 32-32-0 | 0.0 `[-53.0,+53.0]` | 3,0,26,0,3 |
| D/14 | `447c3dc61fb425af43c8b5c0c9f52154ee0a5dea86340e7b995fecfb2b954e80` | .036115 | 33-31-0 | +10.9 `[-26.4,+48.2]` | 1,0,29,0,2 |
| D/16 | `75fc7c2b45d1decf3846fc740dfe9111ee277004902ae5936640f3dd9a346101` | .035733 | 33-31-0 | +10.9 `[-26.4,+48.2]` | 1,0,29,0,2 |

## Repaired extensions and automatic selection

Within each lane, the two best valid screen estimates with competitive ID loss
were extended to 256 games. Duplicate export bytes were extended once and mapped
back to every equivalent checkpoint. The automatic selection manifest is
`out/anhoku-v0.6-phase7.1/artifacts/selection/repair-v2-selection.json`.

| Candidate | W-L-D | Paired Elo (95% CI) | Bins | Nodes | Time s / NPS |
| --- | ---: | ---: | --- | ---: | ---: |
| A/8 | 103-152-1 | -67.3 `[-107.9,-26.8]` | 41,1,69,0,17 | 22,675,384 | 1,121 / 20,234 |
| A/16 | 114-141-1 | -36.8 `[-78.6,+5.0]` | 37,1,66,0,24 | 18,771,797 | 1,077 / 17,425 |
| B/14 | 101-155-0 | -74.4 `[-115.2,-33.6]` | 43,0,69,0,16 | 19,524,253 | 1,072 / 18,210 |
| B/16 | 102-154-0 | -71.6 `[-111.2,-31.9]` | 41,0,72,0,15 | 19,398,567 | 1,037 / 18,699 |
| C/2, C/4, C/6, C/8, C/12 | 128-128-0 | 0.0 `[-18.5,+18.5]` | 6,0,116,0,6 | 22,196,491 | 1,084 / 20,469 |
| C/16 | 138-118-0 | +27.2 `[+0.2,+54.2]` | 8,0,102,0,18 | 22,565,135 | 1,098 / 20,545 |
| D/2, D/4, D/6, D/8, D/12 | 129-127-0 | +2.7 `[-19.3,+24.7]` | 8,0,111,0,9 | 19,239,761 | 1,068 / 18,012 |
| D/16 | 132-124-0 | +10.9 `[-18.4,+40.1]` | 13,0,98,0,17 | 18,403,776 | 1,059 / 17,372 |

C/16 is the automatic winner by valid paired strength. This is a selection
result, not yet an independent confirmation.

## Independent confirmation and handcrafted comparison

The selected C/16 export was confirmed against the unchanged anchor with seed
7104, 1,024 games, 512 pairs, 20 workers, 100 ms, and four random opening
plies. It had zero zero-node sides and no failure states. There were 511 distinct
start SFENs because one randomized opening position collided; all 512 pair
indices were present. The paired result was:

| Candidate | W-L-D | Paired Elo (95% CI) | Bins | Nodes | Time s / NPS |
| --- | ---: | ---: | --- | ---: | ---: |
| C/16 vs corrected v0.5.1, seed 7104 | 521-498-5 | +7.8 `[-5.9,+21.5]` | 46,3,404,0,59 | 78,028,139 | 4,441 / 17,571 |

The lower bound `-5.9` is greater than the written `-10 Elo` threshold, so the
independent non-inferiority gate passes. The report is valid despite the single
opening collision because it has the required 512 complete pairs, no zero-node
or protocol failures, and the opening policy is substantially diverse.

Only after that gate passed, C/16 was compared with handcrafted using fresh seed
7105, 1,024 games, 512 pairs, and the same calibrated runtime:

| Candidate | W-L-D | Paired Elo (95% CI) | Bins | Nodes | Time s / NPS |
| --- | ---: | ---: | --- | ---: | ---: |
| C/16 vs handcrafted, seed 7105 | 328-690-6 | -128.4 `[-150.4,-106.3]` | 226,5,233,1,47 | 136,785,852 | 4,620 / 29,608 |

This confirms that repaired C/16 is near the corrected v0.5.1 anchor under the
fixed-time protocol but remains materially weaker than the handcrafted evaluator.

## Interpretation and next experiment

- The old lower-LR B hypothesis is not supported by valid repaired strength:
  B/14 and B/16 were both about `-72` to `-74 Elo` in 256 games.
- Warm-start C is the strongest recipe in this repair. C/16 was the automatic
  winner and passed independent non-inferiority, although its confirmation
  point estimate was only `+7.8 Elo` and its CI still includes zero.
- D was mixed and did not beat C: D/16 was `+10.9` in selection while D/12
  was `+2.7`; this does not establish a lambda benefit.
- The large negative handcrafted result means the v0.6 data/label/objective
  pipeline still does not produce a general-strength replacement. ID loss and
  paired strength remain poorly aligned; strength is the decision metric.
- The missing per-step `train_loss`, `loss_eval`, and `loss_result` telemetry
  still prevents a clean lambda conclusion. A separate labelled C/D diagnostic
  would be needed if that hypothesis matters.

The smallest next experiment is to prepare the required OOD-v2 suite first,
then run a controlled warm-start/fresh comparison using depth-2 or fixed-node
rollout labels while retaining the current depth-3 labels as an observational
control. Do not promote C/16 or begin Phase 8 until that suite plan is reviewed.

## Artifacts and exclusions

- Invalid original evidence is retained under `matches/` and is excluded.
- Repaired calibration, smoke, screens, extensions, confirmation, and
  handcrafted reports are under `matches-rerun-v2/`.
- There are 22 unique-hash screen reports for all 32 checkpoints, eight valid
  extension reports, one independent confirmation, and one handcrafted report.
- The first full-depth smoke was aborted as non-cheap; its partial directory is
  retained as a diagnostic. The corrected short smoke passed semantically.
- The first extension loop stopped before a match because of a malformed TSV
  delimiter; no report from that attempt was used. The corrected loop produced
  all eight extension reports.
- All repaired per-game JSONL, aggregate reports, engine hashes, commands,
  nodes, qnodes, NPS, timing, and failure fields are preserved.
- The compact repaired-report transfer archive and its checksum are recorded
  in the final section below.

## Phase 8 gate

The repaired data, verifier, lane invariants, and independent strength gate
pass. Phase 8 is nevertheless **blocked** because the written gate also
requires a reviewed OOD-v2 suite plan with at least 64 opening IDs and at least
12 held-out IDs, and that plan is not present yet. No Phase 8 training or model
promotion was started.

## Repair archive

The repaired evidence archive contains `matches-rerun-v2/`, the automatic
selection manifest, and this result document. It was created after the final
document update:

`out/anhoku-v0.6-phase7.1/anhoku-phase7.1-evaluation-repair-v2.tar.zst`

Its SHA-256 is recorded in the accompanying
`out/anhoku-v0.6-phase7.1/anhoku-phase7.1-evaluation-repair-v2.tar.zst.sha256`
sidecar. The remote match-transfer archive
was SHA-256
`d155d1024e3922adafc219126598f0b8723f49177e2dfcce662e78344b6f3520`.
