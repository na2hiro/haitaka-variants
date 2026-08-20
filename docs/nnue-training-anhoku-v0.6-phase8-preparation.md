# Anhoku NNUE v0.6 Phase 8 Preparation

Phase 8 remains gated on the Phase 7 result. This preparation defines the two
new teacher lanes, calibrates their common node budget, and prevents a rare
incomplete depth-1 label from aborting a multi-day generation run. It does not
start production generation or training.

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

## Persistent 200+40-Game Pilot Result

The committed revision `b894685` generated both persistent pilots on
2026-08-20. These artifacts are ignored under `out/` but retain their configs,
manifests, shards, binaries, and audit reports.

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

These files are suitable for loader/trainer plumbing tests, not Phase 8
strength training. Before the 1M experiment, rerun the persistent pilots with a
higher common node budget and re-audit the leaf-side selection effect. Do not
relax either bound without a written ruleset-specific justification.

The re-pilot instrumentation records candidate root side; stored root-to-leaf
side transitions; even/odd leaf distance; incomplete, terminal, and mate
rejections by root side; terminal/mate rejections by traced leaf side; rejected
positions by eventual root-relative win/loss/draw; and the complete selection
breakdown per opening ID. The audit report exposes these counters under
`position_trace.selection_by_side_parity_and_result` and
`position_trace.selection_by_opening`. The telemetry is versioned so shards
without it cannot be silently reused or merged into the re-pilot.

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
