# Anhoku NNUE Training

This runbook documents the practice path that successfully trained and loaded a
small Anhoku NNUE using local data generation on an Apple Silicon Mac and
PyTorch training/export on a temporary vast.ai CUDA instance.

Use the pilot config first. Move to the v0 config only after the pilot proves
that transfer, CUDA, training, export, local verification, and browser loading
all work.

## Configs

- `haitaka_learn.anhoku-pilot.toml` is the cheap pipeline check.
- `haitaka_learn.anhoku-v0.toml` is the first useful model attempt.

Expected dataset sizes:

- Pilot: about 96k train positions and 9.6k validation positions.
- v0: about 3.2M train positions and 320k validation positions.
- Each row is 72 bytes before compression, so the v0 dataset should be small
  enough to transfer comfortably.

Both checked-in configs train from random initialization. Do not set
`paths.bootstrap_nnue` to a Fairy-Stockfish `.nnue` for these Anhoku runs unless
you have confirmed that `variant-nnue-pytorch/serialize.py` can import that file
with `--features HalfKAv2^`.

## What Did Not Work

Using the downloaded Fairy-Stockfish shogi NNUE as `bootstrap_nnue` failed while
converting it to `bootstrap.pt`:

```text
RuntimeError: shape '[152847, 8]' is invalid for input of size 744840
```

The file can still be a valid NNUE evaluation file, but `bootstrap_nnue` is not
just an inference load. It asks `variant-nnue-pytorch` to import the `.nnue` into
the current trainable PyTorch model.

The checked-in trainer overlay uses shogi pockets and factorized `HalfKAv2^`:

- Real runtime features: `150903`.
- Factorized trainer features: `152847`.
- Difference: `1944` virtual factor features.

Exported `.nnue` files are coalesced runtime artifacts. They are good for
Haitaka loading/search, but they are not the safest way to resume training.
Resume from a Lightning `.ckpt` or a compatible `.pt` if you need continuation.
For first Anhoku practice runs, random initialization is simpler and worked.

## Local Mac

Generate the pilot dataset:

```bash
cargo run -p haitaka_learn --features anhoku -- generate-data --config haitaka_learn.anhoku-pilot.toml
```

Generate the v0 dataset after the pilot succeeds:

```bash
cargo run -p haitaka_learn --features anhoku -- generate-data --config haitaka_learn.anhoku-v0.toml
```

Create a transfer bundle for either config:

```bash
sh scripts/prepare_anhoku_training_bundle.sh haitaka_learn.anhoku-pilot.toml
sh scripts/prepare_anhoku_training_bundle.sh haitaka_learn.anhoku-v0.toml
```

The script writes a `.tgz` file in the repository root and includes:

- The selected config.
- The config's generated `datasets/` directory.
- The configured bootstrap NNUE only when `paths.bootstrap_nnue` is present.

## Vast.ai Setup

The successful practice run used:

- PyTorch Vast template.
- 1x RTX 5070 Ti.
- 80 GB container size.
- On-demand instance.
- Direct SSH.

RTX 50-series hosts need CUDA 12.8 wheels. Install the CUDA 12.8 requirements,
not the default CUDA 11.8 requirements:

```bash
cd /workspace
git clone <haitaka repo url> haitaka
git clone https://github.com/fairy-stockfish/variant-nnue-pytorch.git

cd variant-nnue-pytorch
python3 -m venv env
source env/bin/activate
pip install --upgrade pip
pip install --default-timeout=1000 --retries=10 --no-cache-dir -r requirements-CUDA128.txt
```

If the large PyTorch wheel times out, retry the same command. The timeout is a
host/network problem, not necessarily a CUDA problem.

Install Rust and build tools if `cargo`, `cmake`, or a compiler is missing:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

apt update
apt install -y build-essential cmake pkg-config
```

Verify CUDA:

```bash
source /workspace/variant-nnue-pytorch/env/bin/activate
python - <<'PY'
import torch
print(torch.__version__)
print(torch.cuda.is_available())
print(torch.cuda.get_device_name(0))
PY
```

## Transfer And Train

Copy the local bundle to `/workspace`. If the Vast SSH command is:

```bash
ssh -p PORT root@HOST
```

then upload from the local Mac with:

```bash
scp -P PORT anhoku-training-input-haitaka_learn.anhoku-pilot.tgz root@HOST:/workspace/
```

Unpack and train on Vast:

```bash
cd /workspace
tar -xzf anhoku-training-input-*.tgz

source "$HOME/.cargo/env"
source /workspace/variant-nnue-pytorch/env/bin/activate

cd /workspace/haitaka
cargo run -p haitaka_learn --features anhoku -- train --config haitaka_learn.anhoku-pilot.toml
cargo run -p haitaka_learn --features anhoku -- export --config haitaka_learn.anhoku-pilot.toml
```

The config includes:

```toml
extra_args = ["--threads", "8", "--accelerator", "gpu", "--devices", "1"]
```

These flags are important. Without them, Lightning may print:

```text
GPU available: True (cuda), used: False
AssertionError: feature_indices_0.is_cuda
```

If the installed Lightning version rejects `--accelerator` or `--devices`, use
the older fallback:

```toml
extra_args = ["--threads", "8", "--gpus", "1"]
```

## Download Results

The essential outputs are:

- `haitaka_learn-out/anhoku-*/artifacts/*.nnue`
- `haitaka_learn-out/anhoku-*/artifacts/export.json`
- `haitaka_learn-out/anhoku-*/datasets/train.json`
- `haitaka_learn-out/anhoku-*/datasets/validation.json`

Lightning checkpoints under `logs/**/*.ckpt` can be gigabytes. They are useful
only if you plan to resume training, so do not download them for a normal model
handoff.

Example `rsync` download from the local Mac:

```bash
cd /Users/na2hiro/proj/engine/haitaka

mkdir -p haitaka_learn-out/anhoku-pilot/artifacts
mkdir -p haitaka_learn-out/anhoku-pilot/datasets

rsync -avP -e 'ssh -p PORT' \
  root@HOST:/workspace/haitaka/haitaka_learn-out/anhoku-pilot/artifacts/haitaka-anhoku-pilot.nnue \
  haitaka_learn-out/anhoku-pilot/artifacts/

rsync -avP -e 'ssh -p PORT' \
  root@HOST:/workspace/haitaka/haitaka_learn-out/anhoku-pilot/artifacts/export.json \
  haitaka_learn-out/anhoku-pilot/artifacts/

rsync -avP -e 'ssh -p PORT' \
  root@HOST:/workspace/haitaka/haitaka_learn-out/anhoku-pilot/datasets/train.json \
  haitaka_learn-out/anhoku-pilot/datasets/

rsync -avP -e 'ssh -p PORT' \
  root@HOST:/workspace/haitaka/haitaka_learn-out/anhoku-pilot/datasets/validation.json \
  haitaka_learn-out/anhoku-pilot/datasets/
```

After confirming the files are downloaded, destroy the Vast instance to avoid
ongoing charges.

## Local Verification

After downloading the artifacts into the matching local output directory:

```bash
cargo run -p haitaka_learn --features anhoku -- verify --config haitaka_learn.anhoku-pilot.toml
cargo run -p haitaka_learn --features anhoku -- verify --config haitaka_learn.anhoku-v0.toml
```

For reporting or sharing, keep:

- Config file.
- `train.json` and `validation.json`.
- Exported `.nnue`.
- `export.json`.
- `verify.json`.
- `variant-nnue-pytorch` commit.
- Haitaka engine commit from the dataset manifests.
- Vast GPU model, VRAM, hourly price, and training duration.
