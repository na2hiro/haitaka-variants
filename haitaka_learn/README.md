# haitaka_learn

`haitaka_learn` is the local CLI/orchestrator for:

- generating Haitaka-native NNUE training data
- invoking upstream `variant-nnue-pytorch`
- exporting a `.nnue`
- verifying that the exported net loads and searches inside Haitaka

It keeps Haitaka's inference side compatible with Fairy-Stockfish-style `HalfKAv2^` networks while letting us generate training data for:

- standard shogi
- handicap shogi on the standard 9x9 geometry
- Annan shogi
- Anhoku shogi
- Antouzai shogi
- Taimen shogi (enemy-donor, in front)
- Haimen shogi (enemy-donor, behind)

Ruleset-to-feature-set mapping:

- standard / handicap: `HalfKAv2^`
- Annan / Anhoku / Taimen / Haimen: `HalfKAv2^+DonorSingleEff`
- Antouzai: `HalfKAv2^+DonorPairSlots`

## What Is Already Prepared

The example config expects the upstream trainer checkout at:

`../variant-nnue-pytorch`

The example config at [/haitaka_learn.toml](../haitaka_learn.toml) already points there.

Important environment note:

- Upstream `variant-nnue-pytorch` is a CUDA-first trainer.
- On macOS / Apple Silicon, the upstream `requirements.txt` is not the happy path because it installs CUDA wheels and `train.py` currently calls `.cuda()` directly.
- This means the current machine is good for data generation and verification, but actual training should happen on a Linux machine with a CUDA-capable GPU.

## Directory Layout

Typical outputs go under the configured `output_dir`, by default:

`./haitaka_learn-out`

Generated artifacts:

- `datasets/train.bin`
- `datasets/train.json`
- `datasets/validation.bin`
- `datasets/validation.json`
- `artifacts/bootstrap.pt`
- `artifacts/haitaka.nnue`
- `artifacts/export.json`
- `artifacts/verify.json`
- `logs/` for Lightning checkpoints and TensorBoard logs

## Prerequisites

### For data generation and verification

- Rust toolchain that can build the Haitaka workspace

### For training and export

- Python 3.9+
- `cmake`
- C++17 compiler
- NVIDIA GPU with CUDA support
- upstream trainer checkout:
  - `../variant-nnue-pytorch`

Recommended CUDA-machine setup inside the trainer checkout:

```bash
cd ../variant-nnue-pytorch
python3 -m venv env
source env/bin/activate
pip install -r requirements.txt
```

The upstream trainer README says CUDA 11.8 wheels are the default path.

## Config

Start from:

[haitaka_learn.toml](../haitaka_learn.toml)

Key fields:

- `[rules]`
  - `ruleset = "standard" | "handicap" | "annan" | "anhoku" | "antouzai" | "taimen" | "haimen" | "tenkyo" | "tenjiku" | "anki"`
  - `handicap = "two-piece" | "four-piece" | "six-piece"` when `ruleset = "handicap"`
  - `rule_id` defaults to the built-in registry for standard, handicap presets, Annan, Anhoku (`55`), Antouzai (`95`), Taimen (`72`), Haimen (`74`), Tenkyo (`151`), Tenjiku (`56`), and Anki (`94`)
  - set `rule_id` explicitly when using a custom handicap `opening_sfen` without a named preset, or when matching an external registry
  - `opening_sfen` can override the default opening for any ruleset
- `[paths]`
  - `trainer_checkout`
  - `bootstrap_nnue`
  - `output_dir`
- `[data]`
  - self-play and sampling parameters
  - `search_depth` labels sampled positions
  - `rollout_search_depth` chooses non-labeling self-play moves after `opening_random_plies`; keep this shallow, for example `1`, when running expensive label depths
  - `jobs = 0` uses all available CPU cores; this is the default and the recommended setting for serious generation runs unless memory or thermals force a lower value
  - `shard_games` controls resumable shard size
  - `progress_every_percent` controls stdout progress and ETA frequency
  - `resume = true` reuses completed shard files after interruptions. Each shard records the
    git revision and a config-file hash; if a resumed shard's revision or config hash differs
    from the current run (e.g. a local patch that doesn't affect data generation, or a
    comment-only config edit), `generate-data` reports how much of the run's data is affected
    and prompts: abort, resume reusing the mismatched shards, or discard and regenerate them.
    Pass `--ignore-identity-mismatch` to reuse them non-interactively (e.g. on sharded/CI runs).
    Throughput (`speed`) and `eta` are computed from freshly generated games only, so restored
    shards no longer distort them.
- `[training]`
  - `features` defaults to the recommended family for the selected ruleset
  - standard / handicap keep `HalfKAv2^`
  - Annan / Anhoku / Taimen / Haimen / Tenkyo / Tenjiku use `HalfKAv2^+DonorSingleEff`
  - Antouzai uses `HalfKAv2^+DonorPairSlots`
  - Anki uses `HalfKAv2^+DonorKnight8Slots`
  - upstream trainer args like batch size and epoch count
- `[export]`
  - output name and description string
- `[verify]`
  - smoke-search settings

## Standard And Handicap Workflow

Use the default build for standard shogi and handicap shogi.

### 1. Generate data

```bash
cd haitaka-variants
cargo generate haitaka_learn.toml --jobs 0
```

This:

- reads `[rules].ruleset` from the config and runs `haitaka_learn` with the
  matching Cargo feature when one is required
- always uses the release build for data generation
- plays Haitaka self-play games
- samples positions
- labels sampled positions with teacher search scores at `data.search_depth`
- uses `data.rollout_search_depth` for post-opening self-play moves that are not sampled
- writes resumable shard files, then assembles trainer-compatible `.bin` files
  plus JSON manifests

`--jobs 0` is already the default. Pass a smaller value only if the machine becomes memory- or thermally-limited.

Equivalent command with the explicit `jobs` override:

```bash
cargo generate haitaka_learn.toml --jobs 0
```

Generate only one lane of a distributed shard split:

```bash
cargo generate haitaka_learn.toml --jobs 0 --shard 1/2
```

`--shard N/M` is 1-indexed. `--shard 3-5/8` runs the inclusive range covered
by `3/8`, `4/8`, and `5/8`. Shard lanes are contiguous ranges, not modulo
lanes, so the work covered by `--shard 4/4` can later be split between
`--shard 7/8` and `--shard 8/8`.

Pressing Ctrl-C during data generation starts a graceful stop. Already running
shards finish their current `.bin` writes and are kept; no new shards are
started. Press Ctrl-C again to terminate immediately.

Merge shard outputs copied back from multiple machines:

```bash
cargo run -p haitaka_learn -- merge-data --config haitaka_learn.toml --input path/to/machine-a-output --input path/to/machine-b-output
```

`merge-data` fails if shards disagree on the git revision or config hash. When that mismatch is
expected (e.g. a logic-neutral local patch or comment-only config edit), re-run with
`--ignore-identity-mismatch` to skip those two checks.

### 2. Train

Run this on the CUDA machine:

```bash
cd haitaka-variants
cargo run -p haitaka_learn -- train --config haitaka_learn.toml
```

This command:

- temporarily writes shogi-specific `variant.py` and `variant.h` into the upstream trainer checkout
- restores those files afterward
- optionally builds the upstream fast data loader with `cmake`
- converts the bootstrap `.nnue` into `bootstrap.pt`
- launches upstream `train.py`

### 3. Export

```bash
cd haitaka-variants
cargo run -p haitaka_learn -- export --config haitaka_learn.toml
```

### 4. Verify

```bash
cd haitaka-variants
cargo run -p haitaka_learn -- verify --config haitaka_learn.toml
```

### 5. One-shot pipeline

```bash
cd haitaka-variants
cargo run -p haitaka_learn --release -- pipeline --config haitaka_learn.toml
```

## Variant Workflows

Annan, Anhoku, Antouzai, Taimen, Haimen, Tenkyo, Tenjiku, and Anki share the same `HalfKAv2^` base block, but donor-rule runs add ruleset-specific donor geometry and must be built with the matching Haitaka feature enabled. Taimen and Haimen donate movement from an enemy piece (in front / behind) rather than a friendly one.

### 1. Switch config

Set:

```toml
[rules]
ruleset = "annan"   # or "anhoku" / "antouzai" / "taimen" / "haimen" / "tenkyo" / "tenjiku" / "anki"
# rule_id is only needed for a custom registry value, or for handicap+opening_sfen without a preset.
rule_id = 26

[training]
features = "HalfKAv2^+DonorSingleEff" # Antouzai uses DonorPairSlots; Anki uses DonorKnight8Slots
```

### 2. Generate variant data

```bash
cd haitaka-variants
cargo generate haitaka_learn.toml --jobs 0
```

`cargo generate` reads the ruleset from the config, so the same command works
for Annan, Anhoku, Antouzai, Taimen, Haimen, the Neko family, Tenkyo, Tenjiku,
and Anki while still using the matching feature and a release build.

### 3. Train / export / verify the variant run

```bash
cd haitaka-variants
cargo run -p haitaka_learn --features annan -- train --config haitaka_learn.toml
cargo run -p haitaka_learn --features annan -- export --config haitaka_learn.toml
cargo run -p haitaka_learn --features annan -- verify --config haitaka_learn.toml

cargo run -p haitaka_learn --features anhoku -- train --config haitaka_learn.toml
cargo run -p haitaka_learn --features anhoku -- export --config haitaka_learn.toml
cargo run -p haitaka_learn --features anhoku -- verify --config haitaka_learn.toml

cargo run -p haitaka_learn --features antouzai -- train --config haitaka_learn.toml
cargo run -p haitaka_learn --features antouzai -- export --config haitaka_learn.toml
cargo run -p haitaka_learn --features antouzai -- verify --config haitaka_learn.toml
```

Use the same matching feature flag consistently for data-generation, training, export, and verification runs.

## Notes On Labels

Current training entries contain:

- packed position
- teacher score
- ply index
- final game result

Current limitation:

- the trainer's 16-bit move field is not expressive enough for full shogi move encoding, so `haitaka_learn` currently writes `0` there
- this is fine for score/result-driven training, but teacher move match-rate tooling is not meaningful yet

## Verification Behavior

`verify` checks that the exported net:

- parses through Haitaka's `NnueModel::from_bytes`
- evaluates fixed standard, handicap, Annan, Anhoku, and Antouzai SFENs
- optionally returns a legal search result for the configured ruleset

The report is written to:

`haitaka_learn-out/artifacts/verify.json`

## Practical Recommendation

Use this split:

- macOS / local laptop:
  - edit config
  - generate data
  - inspect manifests
  - verify exported nets
- Linux / CUDA trainer box:
  - install upstream trainer deps
  - run `train`
  - run `export`
  - copy resulting `.nnue` back if needed
