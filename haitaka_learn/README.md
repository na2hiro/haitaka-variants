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
  - `ruleset = "standard" | "handicap" | "annan" | "anhoku" | "antouzai"`
  - `handicap = "two-piece" | "four-piece" | "six-piece"` when `ruleset = "handicap"`
  - `rule_id` defaults to the built-in registry for standard, handicap presets, Annan, Anhoku (`55`), and Antouzai (`95`)
  - set `rule_id` explicitly when using a custom handicap `opening_sfen` without a named preset, or when matching an external registry
  - `opening_sfen` can override the default opening for any ruleset
- `[paths]`
  - `trainer_checkout`
  - `bootstrap_nnue`
  - `output_dir`
- `[data]`
  - self-play and sampling parameters
- `[training]`
  - `features` must stay `HalfKAv2^`
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
cargo run -p haitaka_learn -- generate-data --config haitaka_learn.toml
```

This:

- plays Haitaka self-play games
- samples positions
- labels them with teacher search scores
- writes trainer-compatible `.bin` files plus JSON manifests

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
cargo run -p haitaka_learn -- pipeline --config haitaka_learn.toml
```

## Variant Workflows

Annan, Anhoku, and Antouzai use the same NNUE feature geometry, but each workflow must be built with the matching Haitaka feature enabled.

### 1. Switch config

Set:

```toml
[rules]
ruleset = "annan"   # or "anhoku" / "antouzai"
# rule_id is only needed for a custom registry value, or for handicap+opening_sfen without a preset.
rule_id = 26
```

### 2. Generate variant data

```bash
cd haitaka-variants
cargo run -p haitaka_learn --features annan -- generate-data --config haitaka_learn.toml
cargo run -p haitaka_learn --features anhoku -- generate-data --config haitaka_learn.toml
cargo run -p haitaka_learn --features antouzai -- generate-data --config haitaka_learn.toml
```

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
