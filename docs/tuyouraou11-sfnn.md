# TuyouraOu11 micro: scalar and incremental runtime

`haitaka_wasm::sfnn::SfnnModel` loads the two micro parameter files and evaluates
an ordinary shogi `Board` from scratch or from an incremental state. P2 connects
this evaluator to search and exports `load_sfnn` at module and UsiEngine scope.

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
real-model Rust tests, 2,048-move delta comparisons, runtime replacement/TT and
score-range tests. It writes the native trace corpus to target/ for Wasm comparison. Missing models are an error. Ordinary `cargo test` reports
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
Late random play is not a curated endgame tactical benchmark. P2 incremental-state and engine Worker comparisons are described below; P4
still needs application E2E, strength and supported-device checks.

All 236 positions match both Q16 partial sums, their combined sum, bucket and
static score exactly. Tests additionally cover all seven progress thresholds at
before/equality/after, ±halfway Q16 rounding, saturated finite extremes, malformed
LEB128, truncated/extra files, header/description/marker mismatch, missing/nonfinite
progress, unsupported arch and ruleset. Do not interpret P1 as Wasm integration,
difference evaluation, atomic runtime model replacement, or a strength claim.

## P2 runtime and WebAssembly verification

The production APIs are synchronous and preserve the P0 signature:

```ts
load_sfnn(nn_bytes: Uint8Array, progress_bytes: Uint8Array, arch: string): string;
new UsiEngine().load_sfnn(nn_bytes: Uint8Array, progress_bytes: Uint8Array, arch: string): string;
```

Parse both files before replacing the evaluator. Successful installation clears
that evaluator's TT; failures leave the previous evaluator and TT intact.
Global and session models remain independent, and loading a HalfKAv2 model
through `load_nnue` replaces an installed SFNN model in that scope. Model weights
are immutable Arc data, not copied per node. Position states own just their two
accumulators and Q16 sums. Full-refresh search remains available as a native
verification mode.

K+A2 updates only moved/captured/dropped feature rows, including separate king
rows. It never rebases a transformer on king movement. Progress is king-relative:
only the moved king's progress perspective is refreshed, while the other sum
receives nonking deltas. Bucket selection is recomputed from the updated sums.

SFNN's reference static clip is ±31753; Haitaka's search reserves mate/TT ranges
below that. Only the SFNN *search* evaluation is clamped to ±28975, immediately
below MATE_TT_THRESHOLD=28976. Full-refresh/delta diagnostics continue to return
the exact reference static values. A synthetic high-bias model tests this bound.

Build the default, non-SIMD web package and the test-only diagnostic package:

```bash
python3 scripts/tuyouraou11.py verify
wasm-pack build haitaka_wasm --target web --out-dir ../target/tuyouraou11/pkg-diagnostics \
  --release --no-opt --features sfnn-diagnostics
node scripts/tuyouraou11-wasm.mjs
wasm-pack build haitaka_wasm --target web --out-dir ../target/tuyouraou11/pkg \
  --release --no-opt
node scripts/tuyouraou11-wasm.mjs target/tuyouraou11/pkg --production
```

Both packages expose load_sfnn on the module and UsiEngine with matching generated
TypeScript signatures. Only the diagnostic feature exposes sfnn_trace; it checks
full accumulator equality on every move and returns integer diagnostic traces.
Keep that feature out of production. The corpus is generated by the explicit
native verification job, not a hand-maintained file with expected values.

Add `--browser` to either Node command to also run the exact Wasm in a Chromium
module Web Worker, comparing the diagnostic corpus or production API smoke test.
The script uses `@playwright/test` from the adjacent shogitter-3 checkout; set
SHOGITTER_DIR if the checkout is elsewhere. Use HAITAKA_CHROMIUM_PATH to select
an already installed Chromium executable when Playwright's default is absent.
No browser download is performed by this script. In this environment:

```bash
HAITAKA_CHROMIUM_PATH="$HOME/.cache/ms-playwright/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell" \
  node scripts/tuyouraou11-wasm.mjs --browser
HAITAKA_CHROMIUM_PATH="$HOME/.cache/ms-playwright/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell" \
  node scripts/tuyouraou11-wasm.mjs target/tuyouraou11/pkg --production --browser
```

The script serves only its test inputs on an ephemeral localhost port and closes
the server/browser afterward. The page's main-thread timer continues while the
Worker evaluates. This is an engine Worker test, not Shogitter's P4 application
E2E, mobile/Safari compatibility, cold/warm measurement or a strength result.

Native coverage is 2,048 moves with seed=1 LCG64: 349 captures, 318 drops,
92 promotions, 187 king moves and 258 bucket changes, plus independent sibling
branches. Accumulators, Q16 sums, buckets and evaluation values agree exactly.
Four representative positions at depth 2 have equal full-refresh/incremental
search result and node count. The actual web-target Wasm agrees with the native
corpus and all 236 external-reference static fixtures in Node and Chromium.
The production bundle separately passes module/session load, failure retention,
fixed-depth search and movetime search. The startpos depth-1 score is 12 with
handcrafted evaluation and 58 with micro (bestmove is 7g7f for both); the static
root score 52 is a separate full-refresh quantity.

## P4 native evaluator selection harness

```bash
cargo build -p haitaka_wasm --release --example sfnn_usi
# Handcrafted, USI commands on stdin/stdout:
target/release/examples/sfnn_usi
# Same engine/search with micro:
target/release/examples/sfnn_usi --model-dir target/tuyouraou11/model
```

The harness loads before replying to isready and exits on a load error, without
falling back. It uses the existing UsiSession and movetime/depth/node search,
with per-session TT. P4 can use this same executable for both sides of
haitaka_cli self-play, providing `--a-engine-arg=--model-dir` and a second
`--a-engine-arg` with the absolute model directory for micro. Do not pass the
SFNN bin to the old CLI --nnue option. The P4 200-game paired experiment is still
outstanding; the P2 harness only establishes a comparable evaluation path.

## P6 exact SIMD kernels

The micro evaluator now uses detected integer affine kernels (AVX2/NEON on native,
SIMD128 in the SIMD Wasm build). Accumulator row updates and product activation
also use SIMD128 in that build. The scalar Wasm is compiled with `-simd128` and
remains the browser fallback. There are no relaxed SIMD instructions or new
shared-memory requirements. `sfnn-force-scalar` is a verification-only Cargo
feature for comparing the same native search with scalar affine kernels.

Run `node scripts/tuyouraou11-simd.mjs --build --browser` with wasm-bindgen on PATH
and the existing model/P2 corpus to verify 236 reference positions, 2,048 moves,
and 20 fixed-depth searches in Node and Chromium/Firefox Workers. The browser
trace timeout is 600 seconds for full-refresh diagnostics; movetime budgets
remain unchanged. P6 records and adoption details: `plans/tuyouraou11/p6-lead.md`.


## P6 mini profile

The exact same K+A2/progress8 arithmetic also accepts the pinned mini profile
`SFNN_ka2_4096_8_64_progress8`. Only the transformer/first-layer width changes
(2048 to 4096); the description and all file boundaries are checked for the
selected profile. Each model owns its biases, feature rows and affine strides.
SIMD/scalar selection is independent of the profile. `load_sfnn` and USI session
loading return the selected arch and clear only the relevant TT on success.

`plans/tuyouraou11/mini-provenance.json` pins the archive, both parameter hashes
and the same external reference commit. Use `scripts/tuyouraou11.py` with
`--profile mini` for fetch/reference/fixtures/verify; builds clean the external
reference first so changing the edition cannot reuse micro object files.
`node scripts/tuyouraou11-simd.mjs --mini --build --browser` checks the mini
fixtures, incremental traces and fixed-depth searches in both backends.
Results and selection policy: `plans/tuyouraou11/p6-mini.md`.
