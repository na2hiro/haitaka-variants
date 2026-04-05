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

## What Is Already Prepared

The repository now expects the upstream trainer checkout at:

`/Users/na2hiro/proj/engine/variant-nnue-pytorch`

The example config at [/haitaka_learn.toml](../haitaka_learn.toml) already points there.

Important environment note:

- Upstream `variant-nnue-pytorch` is a CUDA-first trainer.
- On macOS / Apple Silicon, the upstream `requirements.txt` is not the happy path because it installs CUDA wheels and `train.py` currently calls `.cuda()` directly.
- This means the current machine is good for data generation and verification, but actual training should happen on a Linux machine with a CUDA-capable GPU.

## Directory Layout

Typical outputs go under the configured `output_dir`, by default:

`/Users/na2hiro/proj/engine/haitaka/haitaka_learn-out`

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
  - `/Users/na2hiro/proj/engine/variant-nnue-pytorch`

Recommended CUDA-machine setup inside the trainer checkout:

```bash
cd /Users/na2hiro/proj/engine/variant-nnue-pytorch
python3 -m venv env
source env/bin/activate
pip install -r requirements.txt
```

The upstream trainer README says CUDA 11.8 wheels are the default path.

## Config

Start from:

[/Users/na2hiro/proj/engine/haitaka/haitaka_learn.toml](/Users/na2hiro/proj/engine/haitaka/haitaka_learn.toml)

Key fields:

- `[rules]`
  - `ruleset = "standard" | "handicap" | "annan"`
  - `handicap = "two-piece" | "four-piece" | "six-piece"` when `ruleset = "handicap"`
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
cd /Users/na2hiro/proj/engine/haitaka
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
cd /Users/na2hiro/proj/engine/haitaka
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
cd /Users/na2hiro/proj/engine/haitaka
cargo run -p haitaka_learn -- export --config haitaka_learn.toml
```

### 4. Verify

```bash
cd /Users/na2hiro/proj/engine/haitaka
cargo run -p haitaka_learn -- verify --config haitaka_learn.toml
```

### 5. One-shot pipeline

```bash
cd /Users/na2hiro/proj/engine/haitaka
cargo run -p haitaka_learn -- pipeline --config haitaka_learn.toml
```

## Annan Workflow

Annan uses the same NNUE feature geometry, but data generation and verification must be built with the Annan feature enabled.

### 1. Switch config

Set:

```toml
[rules]
ruleset = "annan"
rule_id = 26
```

### 2. Generate Annan data

```bash
cd /Users/na2hiro/proj/engine/haitaka
cargo run -p haitaka_learn --features annan -- generate-data --config haitaka_learn.toml
```

### 3. Train / export / verify the Annan run

```bash
cd /Users/na2hiro/proj/engine/haitaka
cargo run -p haitaka_learn --features annan -- train --config haitaka_learn.toml
cargo run -p haitaka_learn --features annan -- export --config haitaka_learn.toml
cargo run -p haitaka_learn --features annan -- verify --config haitaka_learn.toml
```

Use the same `--features annan` flag consistently for Annan data-generation and verification runs.

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
- evaluates fixed standard, handicap, and Annan SFENs
- optionally returns a legal search result for the configured ruleset

The report is written to:

[/Users/na2hiro/proj/engine/haitaka/haitaka_learn-out/artifacts/verify.json](/Users/na2hiro/proj/engine/haitaka/haitaka_learn-out/artifacts/verify.json)

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
