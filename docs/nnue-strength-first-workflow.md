# Strength-first NNUE workflow

This document records the parts of the Anhoku experiment that are useful when
training an NNUE for another Haitaka ruleset. The concrete runner and receipts
remain deliberately strict: they reproduce one experiment and reject silent
configuration changes. Treat them as an executable example to copy and adapt,
not as a universal configuration format.

The central rule is to close the entire loop early:

1. bind the source, data, openings, recipe, seeds, budgets, and output paths;
2. test the real model, optimizer, loader, resume path, and serializer on CPU;
3. measure GPU throughput before approving the full horizon;
4. retain named checkpoints rather than only the final state;
5. prove full-precision to integer to native and WASM parity before games;
6. use paired development games to select a checkpoint;
7. export the selected checkpoint explicitly; and
8. reserve fresh data and openings for independent confirmation.

This order catches integration and strength failures while an experiment is
still cheap. Validation loss is useful for nominating checkpoints, but it does
not replace games through the shipped search path.

## Files in the reference implementation

- `plans/anhoku-nnue-strength-first-plan.md` defines the development and
  confirmation boundary.
- `r0/anhoku-strength-first/step3-experiment-v1.json` is the closed-world,
  hash-bound experiment contract.
- `scripts/strength_first_trainer.py` implements finite shuffled exposure,
  validation, checkpoints, atomic resume state, staged continuation gates, and
  explicit selected export.
- `scripts/strength_first_candidate_parity.py` creates quantization, runtime,
  game, comparison, and selection receipts.
- `haitaka_learn/src/bin/strength_first_candidate_parity.rs` evaluates the
  integer model with the native runtime.
- `scripts/strength_first_candidate_wasm_parity.mjs` evaluates the same corpus
  through the built WASM artifact.
- `scripts/strength_first_browser_harness.mjs` and
  `scripts/strength_first_browser_worker.js` exercise the production browser
  interface and record timing and memory.
- `scripts/strength_first_preflight.py` inventories inputs and recomputes game
  totals from raw JSONL instead of trusting summaries.
- `r0/anhoku-strength-first/step3-result-report.md` records the resulting
  Anhoku development decision.

Large datasets, checkpoints, NNUE files, raw games, and receipt snapshots live
under `out/` and are intentionally excluded from Git. Publish them separately
when another person must reproduce the exact numeric result. Record their byte
lengths and SHA-256 hashes in the committed contract or report.

## Reproduce the local gates

The dedicated runner reuses the pinned trainer checkout preparation in
`scripts/r5_trainer.py`. Start with checks that do not rent a GPU:

```bash
python3 scripts/strength_first_trainer.py \
  --config r0/anhoku-strength-first/step3-experiment-v1.json validate

python3 scripts/strength_first_trainer.py \
  --config r0/anhoku-strength-first/step3-experiment-v1.json self-test

python3 scripts/strength_first_trainer.py \
  --config r0/anhoku-strength-first/step3-experiment-v1.json \
  real-fixture --output-dir out/strength-first-real-fixture

python3 -m unittest scripts/test_strength_first_preflight.py
```

`validate` checks the contract and every declared input hash. `self-test`
checks schedule, finite partial batches, resume, receipt, stopping, and
selection logic without importing PyTorch. `real-fixture` imports the actual
NNUE, Ranger optimizer, sparse loader, and serializer, then compares a resumed
run with an uninterrupted run.

The real fixture does not qualify CUDA state. On a GPU host, begin with the
contract's short probe boundary:

```bash
python3 scripts/strength_first_trainer.py \
  --config r0/anhoku-strength-first/step3-experiment-v1.json \
  train \
  --approval-receipt path/to/gpu-approval.json \
  --stop-after-updates 128
```

Recompute the remaining time and cost from that measurement. Continue only to
a boundary covered by the resulting continuation receipt:

```bash
python3 scripts/strength_first_trainer.py \
  --config r0/anhoku-strength-first/step3-experiment-v1.json \
  train --resume \
  --approval-receipt path/to/gpu-approval.json \
  --continuation-receipt path/to/continuation.json \
  --stop-after-updates 256
```

The reference workflow repeats this at every scheduled game boundary. An
interrupted process resumes from `last-state.pt`; named scheduled and
best-validation checkpoints remain immutable inputs to later selection.

## Export and parity

Export the exact checkpoint under evaluation:

```bash
python3 scripts/strength_first_trainer.py \
  --config r0/anhoku-strength-first/step3-experiment-v1.json \
  candidate-export \
  --checkpoint out/my-run/checkpoints/scheduled-u00000256.pt
```

Build native and WASM evaluators with the same ruleset and NNUE width:

```bash
cargo build --release -p haitaka_learn \
  --bin strength_first_candidate_parity \
  --features anhoku,nnue-l1-256

wasm-pack build haitaka_wasm --release --target nodejs \
  --features anhoku,nnue-l1-256 \
  --out-dir out/strength-first-wasm/pkg
```

Create a fixed corpus that covers ordinary positions, checks, captures,
promotions, drops, donor or other ruleset-specific states, and both sides to
move. The reference run used 1,024 positions. Produce three score streams:

```bash
python3 scripts/strength_first_candidate_parity.py reference \
  --config path/to/experiment.json \
  --checkpoint path/to/checkpoint.pt \
  --network path/to/candidate.nnue \
  --corpus path/to/parity-corpus.jsonl \
  --features path/to/parity-features.jsonl \
  --output out/parity/reference.json

target/release/strength_first_candidate_parity \
  --network path/to/candidate.nnue \
  --corpus path/to/parity-corpus.jsonl \
  --output out/parity/native.json

node scripts/strength_first_candidate_wasm_parity.mjs \
  --pkg-dir out/strength-first-wasm/pkg \
  --network path/to/candidate.nnue \
  --corpus path/to/parity-corpus.jsonl \
  --output out/parity/wasm.json
```

Then run `strength_first_candidate_parity.py finalize --help` for the receipt
arguments. The gate requires exact equality among the integer reference,
native full refresh, and WASM full refresh. It also checks native full refresh
against incremental evaluation. Full-precision versus quantized error is
reported separately; it is not expected to be zero.

## Development games and selection

Build the actual search binary for the same feature combination:

```bash
cargo build --release -p haitaka_cli --features anhoku,nnue-l1-256
```

Run complete color-swapped pairs with one thread and no random plies. A
fixed-node screen isolates evaluation quality; a fixed-time screen includes
runtime cost:

```bash
target/release/haitaka_cli self-play \
  --games 128 --threads 1 \
  --nodes-per-move 10000 \
  --a-eval nnue --b-eval handcrafted \
  --a-nnue path/to/candidate.nnue \
  --openings path/to/paired-openings.sfen \
  --opening-order sequential --opening-random-plies 0 \
  --seed 1 --max-plies 200 \
  --report-dir out/screen

target/release/haitaka_cli self-play \
  --games 256 --threads 1 \
  --movetime-ms 100 \
  --a-eval nnue --b-eval handcrafted \
  --a-nnue path/to/selected.nnue \
  --openings path/to/paired-openings.sfen \
  --opening-order sequential --opening-random-plies 0 \
  --seed 1 --max-plies 200 \
  --report-dir out/fixed-time
```

Audit raw `self-play-games.jsonl`. Fail the run on a missing or illegal move,
an emergency fallback, an unsearched move, a broken pair, or an incomplete
game. Aggregate both colors within an opening pair and repeated cycles within
the highest independent opening-source group. Repeated games from the same
opening are not additional independent samples.

The reference selection compared at most two checkpoints: lowest validation
loss and highest scheduled screen score. It selected by a frozen ordering and
then ran fixed time on the winner only. Because those openings had already
influenced selection, the result remained development evidence even when its
interval excluded 50%.

## Porting to another ruleset

Make the ruleset-specific choices explicit before editing the runner:

| Surface | What must change or be revalidated |
| --- | --- |
| Cargo features | Replace `anhoku` in native, learning, CLI, and WASM builds. Ensure the width feature matches the serialized architecture. |
| Feature family | Define the feature rows, cardinality, transformer hash, output buckets, and serializer compatibility for the new rule. |
| Data | Generate ruleset-correct positions and labels. Freeze train and validation identities, counts, byte sizes, and hashes. |
| Corpus | Add positions that exercise the rule's special movement, captures, checks, promotions, drops, terminal states, and color orientation. |
| Openings | Freeze IDs, order, color-swap transformation, and the true independent grouping unit. Do not assume Anhoku's rotation is correct. |
| Search | Verify terminal scoring, repetition/adjudication, node accounting, timer behavior, and that every returned move was searched. |
| Statistics | Set sample sizes and intervals from the new opening groups. The Anhoku constants of 64 groups and 128 pairs are not universal. |
| Runtime | Test the actual native and WASM artifacts at the intended node and time controls. Record load, memory, and tail timing. |
| Confirmation | Use independent training and opening/data seeds, with no checkpoint or sample-size changes after results are opened. |

`strength_first_trainer.py`, `strength_first_candidate_parity.py`, and the
parity binary contain deliberate Anhoku constants, schema names, feature
hashes, row counts, and exact contract assertions. For a new ruleset, copy
them under a new versioned name, update every such binding, and keep
closed-world validation. Loosening the existing validator to accept arbitrary
JSON would make old receipts ambiguous.

## Artifacts to retain

Before releasing a GPU or deleting remote storage, copy and hash-check:

- the resolved contract and source/build identities;
- the approval, throughput, and continuation receipts;
- `run.json`, `metrics.jsonl`, and `last-state.pt`;
- best-validation, scheduled, selected, and final checkpoints;
- every candidate and selected `.nnue` plus serializer metadata;
- full-precision, integer, native, incremental, and WASM parity evidence;
- raw games, match reports, source audits, and selection history; and
- a short decision report that names the next experiment and the limits of the
  claim.

Hash-bound receipts are most useful when their referenced files are retained
together. If artifacts move between a GPU host and a local checkout, preserve
the original bytes and rebind only paths through a validator-aware transport
step. Never rewrite a receipt merely to make a missing path look valid.
