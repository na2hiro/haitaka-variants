# Anhoku NNUE v0.6 Phase 8 Preparation

Phase 8 remains gated on the Phase 7 result. This preparation defines the two
new teacher lanes, calibrates their common node budget, and prevents a rare
incomplete depth-1 label from aborting a multi-day generation run. The Phase
8A launch identity is frozen in the separate [Phase 8A handoff](nnue-training-anhoku-v0.6-phase8a.md);
bounded data generation and its quality decision are still pending. It does
not start training.

## Prepared Lanes

The Phase 7 depth-3 root dataset is the control. The new configs are:

- `haitaka_learn.anhoku-v0.6-phase8-root.toml`: fixed-node root positions;
- `haitaka_learn.anhoku-v0.6-phase8-leaf.toml`: fixed-node qsearch-PV leaves.

Persistent bounded data pilots use 200 train and 40 validation games:

- `haitaka_learn.anhoku-v0.6-phase8-root.pilot.toml`;
- `haitaka_learn.anhoku-v0.6-phase8-leaf.pilot.toml`.

These pilots exercise the production data contracts and rejection audit but are
not strength experiments. Generate them from a committed revision with:

```bash
cargo generate haitaka_learn.anhoku-v0.6-phase8-root.pilot.toml
cargo generate haitaka_learn.anhoku-v0.6-phase8-leaf.pilot.toml
```

The Phase 8A configs now use the reviewed 64-ID `anhoku-v2` suite and freeze
`anhoku-v2-053` through `anhoku-v2-064` as OOD-v2. They also set
`max_candidate_roots_per_game = 64`, so root and leaf cannot drift by asking
for replacement roots after leaf rejection. Each shard and final manifest
records a `candidate_identity_sha256`; the two final lanes must match it
before training.

Both new configs now use the 50,000-node re-pilot budget, depth cap 64, and
`incomplete_label_policy = "reject-position"`. Their
opening suite, generation seed, grouped split, shuffle, sampling, rollout,
training, verification, and selection settings are copied from the Phase 7
config. Only output/export identity and the teacher budget, incomplete-label
policy, and position policy may differ. Run the machine-readable identity check
with:

```bash
python3 scripts/phase8_prepare.py check \
  --output out/anhoku-v0.6-phase8-preflight.json
```

The report contains hashes for all three configs and a canonical hash of the
shared non-teacher variables. Production manifests must later reproduce the
same suite, generation, split, and shuffle identity; config equality alone is
not accepted as evidence.

## Fixed-Node Calibration Gate

Phase 4 established that 5,000 nodes can expire before depth 1 completes. The
strict calibration reproduced the failure at 5,000, 10,000, and 20,000 nodes;
50,000 passed 496 candidates but later failed once in a 2,237-candidate tail
run. No practical finite budget therefore guarantees that a 1M run will finish
under the default `error` policy.

The new opt-in `reject-position` policy is permitted only with a fixed-node
budget and `uniform-rollout-v1`. It records the full search counters, increments
`rejected_incomplete_label_positions`, skips the unusable label, and still uses
the independent rollout search for the game move. Other configurations retain
the fail-closed default. Run a bounded calibration with:

```bash
python3 scripts/phase8_prepare.py calibrate --reject-incomplete \
  --output out/anhoku-v0.6-phase8-node-calibration.json
```

The default matrix tries 5,000, 10,000, 20,000, and 50,000 nodes on 12 train
and 12 validation games with at most 24 labels per game. It records failures,
label count, rejection count, exact node use, CPU time, and wall time. Use
`--position-policy qsearch-pv-leaf` for the leaf smoke.

The 12+12-game root matrix measured:

| Nodes | Candidates | Rejected incomplete | Rate | Label CPU seconds |
| ---: | ---: | ---: | ---: | ---: |
| 5,000 | 504 | 17 | 3.37% | 14.43 |
| 10,000 | 500 | 5 | 1.00% | 26.96 |
| 20,000 | 496 | 1 | 0.20% | 51.04 |
| 50,000 | 496 | 0 | 0.00% | 124.08 |

An extended 48+48-game 20,000-node run attempted 2,301 labels, stored 2,296,
and rejected 5 (0.22%) in 255.09 label CPU seconds. This was the initial common
budget: it reduced the observed tactical-tail rejection rate by about 15x
versus 5,000 nodes while remaining 2.4x cheaper than 50,000 in the bounded
matrix. The matching 20,000-node leaf smoke attempted 499 labels,
rejected 1 incomplete label (0.20%), 17 terminal leaves, and 10 mate-saturated
leaves, storing 471. Its incomplete rate matches the root smoke while the
terminal and mate filters remain separate Phase 5 counters.

Production must report the rejection rate per split. If either lane exceeds
1%, pause before training and audit the bias rather than accepting the dataset.

## Historical 20,000-Node Persistent Pilot

The committed revision `b894685` generated both initial persistent pilots on
2026-08-20. The table preserves their result; the ignored `out/` paths were
subsequently replaced by the 50,000-node re-pilot below.

| Lane/split | Candidates | Stored | Incomplete | Terminal | Mate | Split seconds | Dataset SHA-256 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| root train | 7,880 | 7,785 | 95 (1.21%) | 0 | 0 | 298.52 | `ed488344a0d56c0392523d51bd59a420471c3bc5ff7aaf48ed0232d1d83331d1` |
| root validation | 2,182 | 2,120 | 62 (2.84%) | 0 | 0 | 136.02 | `06af4d3031c712b735619933050ca57ce77a2b2843670527238ab5dcb311631e` |
| leaf train | 7,917 | 7,429 | 95 (1.20%) | 256 | 137 | 303.71 | `ccde65c81b5d01e9cc15bfadac64656e0724cd2b2b4ae8ea824ebef8768b19d1` |
| leaf validation | 2,186 | 1,910 | 62 (2.84%) | 182 | 32 | 141.02 | `e8a580f34b397354e639ab0042f91d785cac6009dab70be7f5dd65fc9e14ce14` |

Both lanes used the same engine revision and reported zero train/validation
opening-group overlap and zero samples before the opening boundary. The equal
incomplete counts in corresponding splits support traced/untraced fixed-node
parity; the leaf lane attempts a few replacement candidates after its separate
terminal and mate rejections.

The data-quality audit is not launch-passing:

| Lane/split | Black / white | Win / loss among decisive | Status |
| --- | ---: | ---: | --- |
| root train | 50.38% / 49.62% | 49.38% / 50.62% | balance pass; rejection fail |
| root validation | 44.43% / 55.57% | 47.83% / 52.17% | side and rejection fail |
| leaf train | 43.00% / 57.00% | 41.76% / 58.24% | side and rejection fail |
| leaf validation | 56.65% / 43.35% | 36.54% / 63.46% | side, outcome, and rejection fail |

Those results were suitable for loader/trainer plumbing tests, not Phase 8
strength training. They triggered the higher-budget re-pilot and detailed
leaf-selection audit below.

The re-pilot instrumentation records candidate root side; stored root-to-leaf
side transitions; even/odd leaf distance; incomplete, terminal, and mate
rejections by root side; terminal/mate rejections by leaf side when a trace is
available; rejected positions by eventual root-relative win/loss/draw; and the
complete selection breakdown per opening ID. The audit report exposes these
counters under
`position_trace.selection_by_side_parity_and_result` and
`position_trace.selection_by_opening`. The telemetry is versioned so shards
without it cannot be silently reused or merged into the re-pilot.

## 50,000-Node Re-pilot Result And Diagnosis

Revision `db7eeb7` regenerated both lanes from scratch with `--no-resume` on
2026-08-20. Both manifests report the same engine revision, opening-suite hash,
generation seed, grouped split, and shuffle identity. Generation completed in
24:53 for root and 24:54 for leaf while the two lanes shared the machine.

| Lane/split | Candidates | Stored | Incomplete | Terminal | Mate | Dataset SHA-256 |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| root train | 7,880 | 7,857 | 23 (0.29%) | 0 | 0 | `88ec71ce839ec1bca8d0a6831dfe53a2730c2422fdaffee2d2b10129ea639018` |
| root validation | 2,162 | 2,132 | 30 (1.39%) | 0 | 0 | `a74f1c545cef2a64d6ceb1ee64044121282cd52d211d4e332d0a424297d95700` |
| leaf train | 7,931 | 7,417 | 23 (0.29%) | 315 | 176 | `7206e937e14816a0d5c12755cd54d71e9055f7748bcc0642eecafa23ad67dc7e` |
| leaf validation | 2,164 | 1,910 | 30 (1.39%) | 192 | 32 | `eab469718fa6bb308d45573ba6e5f1cedf586e5a7d447cbf4b25507dc380ca84` |

The 50,000-node budget passes the incomplete-label gate in train but still
fails it in validation. The matching 23 and 30 incomplete counts in the root
and traced lanes confirm that trace collection does not change fixed-node
search completion. Raising the budget reduced the historical rejection rates,
but did not make Phase 8 launch-ready.

| Lane/split | Stored black / white | Odd leaf distance | Win / loss among decisive | Status |
| --- | ---: | ---: | ---: | --- |
| root train | 49.97% / 50.03% | 0% | 49.08% / 50.92% | pass |
| root validation | 44.75% / 55.25% | 0% | 47.56% / 52.44% | side and rejection fail |
| leaf train | 43.10% / 56.90% | 57.73% | 39.71% / 60.29% | side and outcome fail |
| leaf validation | 56.96% / 43.04% | 56.60% | 36.02% / 63.98% | side, outcome, and rejection fail |

The rejection audit further reports:

| Lane/split/category | Rejected root black / white | Rejected leaf black / white | Root-relative game win / loss / draw |
| --- | ---: | ---: | ---: |
| root train incomplete | 3 / 20 | unavailable | 9 / 14 / 0 |
| root validation incomplete | 30 / 0 | unavailable | 0 / 30 / 0 |
| leaf train terminal | 177 / 138 | 126 / 189 | 199 / 89 / 27 |
| leaf train mate | 69 / 107 | unavailable | 74 / 102 / 0 |
| leaf validation terminal | 24 / 168 | 190 / 2 | 170 / 22 / 0 |
| leaf validation mate | 11 / 21 | unavailable | 17 / 15 / 0 |

Incomplete labels have no leaf. The completed node-budget iteration did not
retain a training trace for these mate-saturated rejections, so their root-side
and game-result counters are authoritative but their leaf side is unavailable.
The manifest's zero mate-leaf counters therefore mean “no trace available,” not
that zero mate rejections occurred on either side.

This is not an implementation-side or packed-record orientation bug:

- candidate black plus white equals the manifest candidate count in all four
  datasets;
- all four root-to-leaf transition cells sum to stored positions, even plus odd
  distance sums to stored positions, and each odd trace flips side while each
  even trace preserves it;
- transition-derived black/white totals exactly equal the independent binary
  audit, and stored plus every rejection category exactly equals candidates;
- root and leaf lanes reproduce corresponding incomplete counts and game-result
  totals for positions not changed by leaf selection.

The dominant imbalance is legitimate qsearch-PV leaf behavior for this Anhoku
opening distribution. In train, 64.56% of stored black-root traces are odd and
flip to white, versus 50.84% of white-root traces flipping to black. Validation
reverses the correlation: 49.62% of black-root traces and 63.07% of white-root
traces are odd, producing a black-majority leaf set. The effect is strongly
opening-specific: train opening `anhoku-v1-009` stores 294 black versus 1,278
white leaves, while validation opening `anhoku-v1-010` stores 718 black versus
362 white leaves. Terminal filtering does not explain the result: it removes
more white leaves in train (189 versus 126) and far more black leaves in
validation (190 versus 2), moderating rather than creating the observed
majorities. Mate counts are too small to reverse either imbalance.

Two avoidable selection artifacts remain. First, the per-game cap counts
accepted samples, so rejecting a terminal or mate leaf lets the leaf lane try
replacement roots; it attempted 51 more train candidates and two more
validation candidates than root. This is small relative to the side gap but
breaks an exact root-candidate A/B comparison. Second, the grouped validation
split contains only openings `anhoku-v1-010` and `anhoku-v1-011`, so its result
is especially sensitive to opening-specific trace parity. Before another
re-pilot, cap attempted candidates rather than accepted samples and define a
larger or cross-validated held-out opening set. Do not hide the intrinsic leaf
distribution by silently rebalancing records; any weighting or stratification
must be a separately named experiment.

## Training And Evaluation Matrix

After Phase 7 explicitly approves Phase 8 and the node gate passes:

| Lane | Data policy | Initialization seeds | Required comparisons |
| --- | --- | --- | --- |
| Phase 7 control | depth-3 root | existing Phase 7 seeds | reuse unchanged |
| Phase 8 root | fixed-node root | 80, 81, 82 | control, handcrafted |
| Phase 8 leaf | fixed-node qsearch leaf | 80, 81, 82 | control, handcrafted |

The external trainer must record the three initialization seeds in its logs or
run manifests; the current Haitaka config does not itself guarantee PyTorch
initialization seeding. Do not claim the three-seed acceptance criterion until
that evidence exists.

For every seed, preserve all checkpoints and report validation loss, tactical
fixtures, fixed-anchor Elo, handcrafted Elo, and NNUE NPS. The final matches use
the same paired openings under equal-node diagnostics and 100 ms fixed-time
play. Phase 6 SIMD must remain enabled for the fixed-time binaries.

## Remaining Launch Gates

1. Phase 7 passes its dataset/result gate and explicitly approves Phase 8.
2. A representative 50,000-node pilot keeps incomplete-label rejection at or
   below 1% for both position policies.
3. The external trainer demonstrates and records deterministic initialization
   seeds 80, 81, and 82.
4. The generated root and leaf manifests verify identical non-teacher identity
   by hashes before training begins.
