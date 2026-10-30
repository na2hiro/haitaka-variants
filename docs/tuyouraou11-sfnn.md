# TuyouraOu11 micro: P1 scalar evaluator

`haitaka_wasm::sfnn::SfnnModel` loads the two micro parameter files and evaluates
an ordinary shogi `Board` from scratch. P1 does not install this evaluator into
search or add wasm-bindgen exports. Those are P2 work.

## Fixed artifacts and reference

- Model: [provenance](../plans/tuyouraou11/provenance.json).
- Reference: YaneuraOu commit `7d9743646f1006ff6d84da45d6d6160048a5a416`.
- Parameters: `nn.bin` 3,864,610 bytes; `progress.bin` 1,003,104 bytes.
- Model ID: `tuyouraou11micro-20261001-epoch0010-7f9545dbf73b-a2d18f056463`.
- Both parameter files have the article's explicit MIT designation. The archive
  contains five files but no separate LICENSE or copyright notice. Attribution
  from tag.json and the explicit designation are preserved in
  [MODEL-LICENSE.txt](../plans/tuyouraou11/MODEL-LICENSE.txt); no year is invented.
  banner.png is not redistributed. The reference program is GPLv3, built only
  under ignored target/, and is not linked into Haitaka. Its source license is
  retained in the fetched source. Haitaka's evaluator is an independent Rust
  implementation of the file format, feature numbering and integer arithmetic.

## Reproduce

From the haitaka-variants root, prerequisites are Rust/Cargo, Python 3.12+ with
`jsonschema`, system `libarchive`, GNU make and g++. No 7z executable is required.
No commands modify the system or install packages. Network is needed only for
fetching the fixed model/reference. Build logs and binaries stay in target/.

```bash
python3 scripts/tuyouraou11.py fetch
python3 scripts/tuyouraou11.py reference
python3 scripts/tuyouraou11.py fixtures
python3 scripts/tuyouraou11.py verify
cargo test -p haitaka_wasm --release
cargo test -p haitaka_wasm --features annan sfnn --lib
```

`fetch` verifies the distribution archive and both extracted file SHA-256 values
and writes the two parameters, provenance.json and LICENSE.txt to
`target/tuyouraou11/model/`. WEB can consume that directory after running the same
fetch command; the files are not added to Git. It can also extract the two bins
with a normal 7z tool and compare them with the committed provenance.

`reference` downloads the exact source commit, builds
`YANEURAOU_ENGINE_SFNN_ka2_2048_8_64_progress8`, `TARGET_CPU=OTHER`,
`COMPILER=g++`, `PYTHON=python3`. OTHER means 64-bit scalar on this host; NO_SSE
is the upstream **32-bit** target and is not used. A diagnostic-only command is
inserted from `scripts/tuyouraou11-reference-command.inc`. Model parsing,
feature extraction, progress selection and evaluation remain upstream code.

`fixtures` generates legal positions with the explicit LCG64 algorithm in
`haitaka_wasm/examples/sfnn_positions.rs`, seed=1, four games, at most 256 plies,
sampling plies congruent to 0 or 1 modulo 8. The generator file hash is included
in the fixture metadata. It sends each SFEN to the external reference with
Threads=1, USI_Hash=16, FV_SCALE=24, BookFile=no_book and the model's EvalDir.
The generated JSON is validated against the P0 schema before saving it.
Never change fixture values to agree with the Rust evaluator.

`verify` validates schema/provenance and hashes, then explicitly runs the
real-model Rust test. Missing models are an error. Ordinary `cargo test` reports
this integration test as ignored with its reason; that is not a real-model pass.
Equivalent explicit invocation:

```bash
HAITAKA_SFNN_MODEL_DIR="$PWD/target/tuyouraou11/model" \
  cargo test -p haitaka_wasm --release --test sfnn_reference -- --ignored --nocapture
```

## File format and scalar arithmetic

The strict loader currently accepts this micro export, including its exact
architecture description, rather than claiming to support all SFNN models.
Version is `0x7af32f16`, network hash `0x3c203b32`, transformer hash `0x5f134ab8`
and each of eight layer stack hashes is `0x6333718a`. Transformer biases and
feature-major weights use signed LEB128 blocks with a marker and payload byte
count. Validate every boundary, decoded int16 range and complete consumption;
reject trailing payload and trailing file data. Changing a hash or extension
cannot convert HalfKAv2 or another width to this format.

The 1,791 input features combine A2 (1,629) and K (162):

- BonaPiece nonking indices 0..1547: zero for missing physical piece slots;
  hand planes in 1..89 and board planes from 90. Gold and promoted minors share
  one board plane. Horse and dragon have their own planes.
- A2 folds the two king planes into the same 81-square plane at 1548.
- K distinguishes friendly/enemy king planes, offset 1629.
- Squares are file-major and White rotates 180 degrees. Haitaka's `Color` enum
  order is White, Black; always convert explicitly to the reference Black, White
  order. Casting Color to usize for perspective selection is incorrect.

Both perspectives have 2,048 int16 accumulator values. Loaded transformer
parameters are multiplied by two with int16 wrapping; summation also wraps.
Clamp each accumulator half to 0..254 and multiply elementwise across the halves,
shifting the product right by 9. The 1,024 outputs of the side to move precede
those of the other side. Each selected stack is 2048→8, squared/clipped and
clipped activation concatenated to 16, padded input stride 32 for 16→64, then
clipped activation and 64→1. No shortcut is present for H1=8.
Squared activation is min(127, x² >> 19), clipped activation clamp(x >> 6,0,127).
Final integer division by 24 truncates toward zero; clip to ±31753, the reference
VALUE_MAX_EVAL with its default MAX_PLY=246. This is the reference's static score,
not an USI-rendered cp conversion or a search score.

`progress.bin` has 81×1548 little-endian f64 weights. Reject NaN/Infinity. Round
weights times 65536 halfway away from zero, then saturate to int32 **as the
pinned reference does** (finite out-of-range weights are not rejected).
The Black sum uses the Black king and Black BonaPiece list; the White sum uses
the rotated White king and White list. Include 38 nonking slots, excluding both
kings. Add both int64 sums and select a bucket at the seven P0 Q16 thresholds.
Equality goes to the higher bucket.

## Reference diagnostics and coverage

The fixed source locations used by the fixture generator are:

- `source/eval/nnue/evaluate_nnue.cpp:222`: f64→Q16 loading and saturation.
- `source/eval/nnue/evaluate_nnue.cpp:242`: SumQ16, two perspective sums added.
- `source/eval/nnue/evaluate_nnue.cpp:734`: layer stack selection.
- `source/eval/nnue/evaluate_nnue.cpp:780`: propagation, division and score clipping.
- `source/eval/nnue/evaluate_nnue.cpp:966`: compute_eval full-refresh entry.
- `source/eval/nnue/features/{a2.cpp,k.cpp,feature_set.h}`: feature geometry.

Fixture `progress.sumsQ16.black` and `.white` are the two actual partial sums,
`selectedSumQ16` is upstream SumQ16, and `evalCp` is upstream compute_eval. The
instrumentation reads already loaded progress weights and does not substitute a
new evaluator. These diagnostic changes affect only the external test command.

The final fixed LCG fixture has 236 sampled legal positions, 118 per side to move,
32 opening / 96 middle / 108 late random-play positions, 197 with hands and 195
with promoted pieces. Bucket counts 0..7 are 23,19,18,29,30,42,49,26.
Late random play is not a curated endgame tactical benchmark. P2 still needs
incremental-state comparisons and P4 still needs actual strength/browser checks.

All 236 positions match both Q16 partial sums, their combined sum, bucket and
static score exactly. Tests additionally cover all seven progress thresholds at
before/equality/after, ±halfway Q16 rounding, saturated finite extremes, malformed
LEB128, truncated/extra files, header/description/marker mismatch, missing/nonfinite
progress, unsupported arch and ruleset. Do not interpret P1 as Wasm integration,
difference evaluation, atomic runtime model replacement, or a strength claim.
