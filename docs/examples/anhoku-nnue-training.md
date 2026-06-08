# Anhoku NNUE Training

This runbook tracks the Anhoku v0.4.1 training path. v0.4.1 is the first Anhoku run
configured for the donor-aware feature set from this branch.

## Config

Use `haitaka_learn.anhoku-v0.4.1.toml` for the new run.

The important config choices are:

- `rules.ruleset = "anhoku"`.
- `paths.output_dir = "out/anhoku-v0.4.1"` so new datasets,
  checkpoints, and exports do not mix with pre-donor runs.
- No `paths.bootstrap_nnue`. Older Anhoku `.nnue` files were trained/exported
  either without donor features or with the previous shared donor-single hash.
- `training.features = "HalfKAv2^+DonorSingleEff"`.
- `export.output_name = "haitaka-anhoku-v0.4.1.nnue"`.

`training.features` could be omitted because Anhoku maps to
`HalfKAv2^+DonorSingleEff` by default, but keeping it explicit makes the
artifact intent visible in the config and export metadata.

The branch gives Annan and Anhoku different `DonorSingleEff` block hashes:
Annan uses `single-behind`, while Anhoku uses `single-front`. A model exported
for one single-donor mode should not be reused as a bootstrap or runtime model
for the other.

Expected dataset size for the v0.4.1 config is roughly the same scale as v0.3:
about 1.0M to 1.1M train positions and about 100k validation positions. Each
row is 72 bytes before compression, so the dataset bundle should stay small
enough to transfer comfortably.

## Bootstrap Notes

Do not set `paths.bootstrap_nnue` to a Fairy-Stockfish `.nnue` for this run
unless `variant-nnue-pytorch/serialize.py` can import it with:

```bash
python3 serialize.py path/to/input.nnue /tmp/bootstrap.pt --features 'HalfKAv2^+DonorSingleEff'
```

The previous Fairy-Stockfish shogi NNUE import failed with:

```text
RuntimeError: shape '[152847, 8]' is invalid for input of size 744840
```

The file can still be a valid inference NNUE, but `bootstrap_nnue` asks the
trainer to import the file into the current trainable PyTorch shape. For v0.4.1,
start from random initialization unless the bootstrap was produced by this same
Anhoku `HalfKAv2^+DonorSingleEff` stack.

Exported `.nnue` files are coalesced runtime artifacts. They are good for
Haitaka loading/search, but they are not the safest way to resume training.
Resume from a Lightning `.ckpt` or a compatible `.pt` if you need continuation.

## Local Mac

Generate the v0.4.1 dataset:

```bash
HV_VAR=anhoku-v0.4.1
cargo run -p haitaka_learn --release --features anhoku -- generate-data --config haitaka_learn.${HV_VAR}.toml --jobs 0
```

Generation is resumable. If you stop it with Ctrl-C or kill the process, rerun
the same command; completed shard files under `datasets/shards/` are reused.
Progress prints to stdout at the configured percent interval with elapsed time,
ETA, positions, and games/minute.

To split v0.4.1 data generation across the M4 Mac and the Dell, run one shard lane
on each machine:

```bash
# M4
HV_VAR=anhoku-v0.4.1
cargo run -p haitaka_learn --features anhoku -- generate-data --config haitaka_learn.${HV_VAR}.toml --jobs 0 --shard-index 0 --shard-count 2

# Dell
HV_VAR=anhoku-v0.4.1
cargo run -p haitaka_learn --features anhoku -- generate-data --config haitaka_learn.${HV_VAR}.toml --jobs 0 --shard-index 1 --shard-count 2
```

Copy the two output directories back to the Mac with different wrapper names,
then merge:

```bash
HV_VAR=anhoku-v0.4.1
cargo run -p haitaka_learn --features anhoku -- merge-data \
  --config haitaka_learn.${HV_VAR}.toml \
  --input out/${HV_VAR}-m4 \
  --input out/${HV_VAR}-dell
```

Create a transfer bundle after `datasets/train.bin` and
`datasets/validation.bin` exist:

```bash
HV_VAR=anhoku-v0.4.1
tar -czf input.${HV_VAR}.tgz \
  haitaka_learn.${HV_VAR}.toml \
  out/${HV_VAR}/datasets/train.bin \
  out/${HV_VAR}/datasets/train.json \
  out/${HV_VAR}/datasets/validation.bin \
  out/${HV_VAR}/datasets/validation.json
```

The bundle includes the config, assembled training data, and dataset manifests.
It intentionally omits `datasets/shards/`; shards are only needed to resume data
generation or run another merge, not to train once `train.bin` and
`validation.bin` exist. It also intentionally does not include a bootstrap NNUE
because v0.4.1 should start from random initialization.

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
git clone https://github.com/na2hiro/haitaka-variants haitaka
git clone https://github.com/na2hiro/haitaka-variant-nnue-pytorch variant-nnue-pytorch

cd variant-nnue-pytorch
python3 -m venv env
source env/bin/activate
pip install --upgrade pip

## IF the machine supports CUDA 12.8 or more
pip install --default-timeout=1000 --retries=10 --no-cache-dir -r requirements-CUDA128.txt
# Otherwise,
pip install --default-timeout=1000 --retries=10 --no-cache-dir -r requirements.txt
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

## set up for 11.8 machine

it was broken. recreated the env.

```
cd /workspace/variant-nnue-pytorch
deactivate 2>/dev/null || true
rm -rf env

python3 -m venv env
source env/bin/activate
pip install --upgrade pip

pip install --default-timeout=1000 --retries=10 --no-cache-dir \
torch==2.7.1 --index-url https://download.pytorch.org/whl/cu118

pip install --default-timeout=1000 --retries=10 --no-cache-dir \
chess matplotlib pytorch-lightning==1.9.5 tensorboard cupy-cuda11x
```

Then verify:

```
python - <<'PY'
import torch
print(torch.__version__)
print(torch.version.cuda)
print(torch.cuda.is_available())
if torch.cuda.is_available():
print(torch.cuda.get_device_name(0))
PY
```

Expected:

```
2.7.1+cu118
11.8
True
```

## Transfer And Train

Copy the local bundle beside the Haitaka checkout on Vast.
Set Vast's host and port to env vars:

```bash
# example: ssh -p 13035 root@ssh2.vast.ai -L 8080:localhost:8080
VAST_HOST=ssh2.vast.ai
VAST_PORT=13035
```

then upload from the local Mac with:

```bash
HV_VAR=anhoku-v0.4.1
scp -P ${VAST_PORT} input.${HV_VAR}.tgz root@${VAST_HOST}:/workspace/
```

Unpack and train on Vast:

```bash
HV_VAR=anhoku-v0.4.1

cd /workspace/haitaka
tar -xzf ../input.${HV_VAR}.tgz

source /workspace/variant-nnue-pytorch/env/bin/activate
source "$HOME/.cargo/env"

cargo run -p haitaka_learn --features anhoku -- train --config haitaka_learn.${HV_VAR}.toml
cargo run -p haitaka_learn --features anhoku -- export --config haitaka_learn.${HV_VAR}.toml
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

- `out/${HV_VAR}/artifacts/haitaka-${HV_VAR}.nnue`
- `out/${HV_VAR}/artifacts/export.json`
- `out/${HV_VAR}/datasets/train.json`
- `out/${HV_VAR}/datasets/validation.json`

Lightning checkpoints under `logs/**/*.ckpt` can be gigabytes. They are useful
only if you plan to resume training, so do not download them for a normal model
handoff.

Example `rsync` download from the local Mac:

```bash
HV_VAR=anhoku-v0.4.1

cd /Users/na2hiro/proj/engine/haitaka

mkdir -p out/${HV_VAR}/artifacts
mkdir -p out/${HV_VAR}/datasets

rsync -avP -e "ssh -p ${VAST_PORT}" \
  root@${VAST_HOST}:/workspace/haitaka/out/${HV_VAR}/artifacts/haitaka-${HV_VAR}.nnue \
  out/${HV_VAR}/artifacts/

rsync -avP -e "ssh -p ${VAST_PORT}" \
  root@${VAST_HOST}:/workspace/haitaka/out/${HV_VAR}/artifacts/export.json \
  out/${HV_VAR}/artifacts/

rsync -avP -e "ssh -p ${VAST_PORT}" \
  root@${VAST_HOST}:/workspace/haitaka/out/${HV_VAR}/datasets/train.json \
  out/${HV_VAR}/datasets/

rsync -avP -e "ssh -p ${VAST_PORT}" \
  root@${VAST_HOST}:/workspace/haitaka/out/${HV_VAR}/datasets/validation.json \
  out/${HV_VAR}/datasets/
```

After confirming the files are downloaded, destroy the Vast instance to avoid
ongoing charges.

## Local Verification

After downloading the artifacts into the matching local output directory:

```bash
HV_VAR=anhoku-v0.4.1
cargo run -p haitaka_learn --features anhoku -- verify --config haitaka_learn.${HV_VAR}.toml
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
