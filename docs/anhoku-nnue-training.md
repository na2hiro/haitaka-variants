# Anhoku NNUE Training

This runbook documents the practice path that successfully trained and loaded a
small Anhoku NNUE using local data generation on an Apple Silicon Mac and
PyTorch training/export on a temporary vast.ai CUDA instance.

Use the smoke config first. Move to the pilot config only after smoke proves the
local loop works, then move to v0 after the pilot proves transfer, CUDA,
training, export, local verification, and browser loading all work.

## Configs

- `haitaka_learn.anhoku-smoke.toml` is the quick local generator/trainer check.
- `haitaka_learn.anhoku-pilot.toml` is the cheap pipeline check.
- `haitaka_learn.anhoku-v0.toml` is the first useful model attempt.

Expected dataset sizes:

- Smoke: about 2.4k train positions and 480 validation positions.
- Pilot: about 18k train positions and 3.6k validation positions.
- v0: about 3.2M train positions and 320k validation positions.
- Each row is 72 bytes before compression, so the v0 dataset should be small
  enough to transfer comfortably.

The `paths.bootstrap_nnue` field has two different effects:

- During `generate-data`, Haitaka loads the `.nnue` directly and uses it as the
  teacher evaluator inside search.
- During `train`, `haitaka_learn` asks `variant-nnue-pytorch/serialize.py` to
  convert that same file into `bootstrap.pt` and resume PyTorch training from
  it.

These are not equivalent compatibility checks. A `.nnue` can be valid for
Haitaka search while still failing as a PyTorch training seed.

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
Haitaka loading/search, and they can be useful as teachers for generating the
next dataset. They are not automatically safe as PyTorch training seeds.

For continuation, prefer a Lightning `.ckpt` from the same run. For
cross-generation distillation, use the previous `.nnue` as the data-generation
teacher, then train from random initialization unless you have built a compatible
factorized `.pt` seed.

## Bootstrap And Factorization

`HalfKAv2` is the runtime feature layout. For Haitaka shogi geometry, it has
`150903` real features.

`HalfKAv2^` is the factorized training layout. It has the same `150903` real
features plus `1944` virtual factor features, for `152847` total training
features. Export coalesces those virtual factors back into the real runtime
features, so the final `.nnue` does not store the extra `1944` rows.

To try using a compatible runtime `.nnue` as the starting point for a factorized
training run, first convert it as non-factorized `HalfKAv2`, then append the
zero-initialized `HalfKAv2^` virtual factor block:

```bash
cd /workspace/variant-nnue-pytorch
source env/bin/activate

python serialize.py /workspace/haitaka_learn-out/anhoku-v0/artifacts/haitaka-anhoku-v0.nnue \
  /workspace/anhoku-v0-halfkav2.pt \
  --features HalfKAv2

python - <<'PY'
import torch
import features

src = "/workspace/anhoku-v0-halfkav2.pt"
dst = "/workspace/anhoku-v0-halfkav2-factorized.pt"

model = torch.load(src, weights_only=False)
model.set_feature_set(features.get_feature_set_from_name("HalfKAv2^"))
torch.save(model, dst)

print("wrote", dst)
print("features", model.feature_set.name)
print("input weight shape", tuple(model.input.weight.shape))
PY
```

This creates a `.pt` model with the runtime weights preserved and the virtual
factor block initialized to zero.

Current `haitaka_learn` does not yet accept a prebuilt `.pt` bootstrap directly;
`paths.bootstrap_nnue` is treated as a `.nnue` and converted internally. Until
that support is added, use this factorization path only for manual trainer
experiments, or resume from a Lightning `.ckpt`.

Fallback when `train` fails while converting `bootstrap_nnue`:

```bash
cp haitaka_learn.anhoku-v0.toml haitaka_learn.anhoku-v0-train-nobootstrap.toml
sed -i '/bootstrap_nnue/d' haitaka_learn.anhoku-v0-train-nobootstrap.toml

cargo run -p haitaka_learn --features anhoku -- train --config haitaka_learn.anhoku-v0-train-nobootstrap.toml
cargo run -p haitaka_learn --features anhoku -- export --config haitaka_learn.anhoku-v0-train-nobootstrap.toml
```

This keeps the already generated dataset. It only changes the PyTorch
initialization from NNUE bootstrap to random initialization.

## Local Mac

Generate the smoke dataset first:

```bash
cargo run -p haitaka_learn --features anhoku -- generate-data --config haitaka_learn.anhoku-smoke.toml --jobs 2
```

Generate the pilot dataset after smoke succeeds:

```bash
cargo run -p haitaka_learn --features anhoku -- generate-data --config haitaka_learn.anhoku-pilot.toml --jobs 0
```

Generate the v0 dataset after the pilot succeeds:

```bash
cargo run -p haitaka_learn --features anhoku -- generate-data --config haitaka_learn.anhoku-v0.toml --jobs 0
```

Generation is resumable. If you stop it with Ctrl-C or kill the process, rerun
the same command; completed shard files under `datasets/shards/` are reused.
Progress prints to stdout at the configured percent interval with elapsed time,
ETA, positions, and games/minute.

To split v0 data generation across the M4 Mac and the Dell, run one shard lane
on each machine:

```bash
# M4
cargo run -p haitaka_learn --features anhoku -- generate-data --config haitaka_learn.anhoku-v0.toml --jobs 0 --shard-index 0 --shard-count 2

# Dell
cargo run -p haitaka_learn --features anhoku -- generate-data --config haitaka_learn.anhoku-v0.toml --jobs 0 --shard-index 1 --shard-count 2
```

Copy the two output directories back to the Mac with different wrapper names,
then merge:

```bash
cargo run -p haitaka_learn --features anhoku -- merge-data \
  --config haitaka_learn.anhoku-v0.toml \
  --input haitaka_learn-out/anhoku-v0-m4 \
  --input haitaka_learn-out/anhoku-v0-dell
```

Create a transfer bundle for either config:

```bash
sh scripts/prepare_anhoku_training_bundle.sh haitaka_learn.anhoku-smoke.toml
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
