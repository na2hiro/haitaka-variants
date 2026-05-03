# Anhoku NNUE Training

This runbook trains an Anhoku NNUE with local data generation on an Apple
Silicon Mac and PyTorch training/export on a temporary vast.ai CUDA instance.

Use the pilot config first. Move to the v0 config only after the pilot proves
that transfer, CUDA, training, export, and local verification all work.

## Configs

- `haitaka_learn.anhoku-pilot.toml` is the cheap pipeline check.
- `haitaka_learn.anhoku-v0.toml` is the first useful model attempt.

Expected dataset sizes:

- Pilot: about 96k train positions and 9.6k validation positions.
- v0: about 3.2M train positions and 320k validation positions.
- Each row is 72 bytes before compression, so the v0 dataset should be small
  enough to transfer comfortably.

Both configs keep `features = "HalfKAv2^"` for Haitaka/Fairy-Stockfish
compatibility and use `../shogi-878ca61334a7.nnue` as the bootstrap teacher.

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
- The configured bootstrap NNUE.

## Vast.ai

Recommended instance:

- Official PyTorch template.
- One NVIDIA GPU with at least 12 GB VRAM.
- Prefer RTX 3090, RTX 4090, RTX A5000, A40, L40, or A100 when prices are
  reasonable.
- 50 GB disk minimum; 80 GB is more comfortable.
- Prefer direct SSH when available.

After connecting:

```bash
cd /workspace
git clone <haitaka repo url> haitaka
git clone https://github.com/fairy-stockfish/variant-nnue-pytorch.git
cd variant-nnue-pytorch
python3 -m venv env
source env/bin/activate
pip install -r requirements.txt
```

Verify CUDA:

```bash
python - <<'PY'
import torch
print(torch.__version__)
print(torch.cuda.is_available())
print(torch.cuda.get_device_name(0))
PY
```

Copy the bundle to `/workspace`, unpack it from `/workspace`, then run the
matching commands:

```bash
cd /workspace
tar -xzf anhoku-training-input-*.tgz

cd /workspace/haitaka
cargo run -p haitaka_learn --features anhoku -- train --config haitaka_learn.anhoku-pilot.toml
cargo run -p haitaka_learn --features anhoku -- export --config haitaka_learn.anhoku-pilot.toml
```

For v0, replace `haitaka_learn.anhoku-pilot.toml` with
`haitaka_learn.anhoku-v0.toml`.

Download these outputs before destroying the instance:

- `haitaka_learn-out/anhoku-*/artifacts/*.nnue`
- `haitaka_learn-out/anhoku-*/artifacts/export.json`
- `haitaka_learn-out/anhoku-*/logs/`

## Local Verification

After downloading the artifacts into the matching local output directory:

```bash
cargo run -p haitaka_learn --features anhoku -- verify --config haitaka_learn.anhoku-pilot.toml
cargo run -p haitaka_learn --features anhoku -- verify --config haitaka_learn.anhoku-v0.toml
```

Keep these reporting artifacts outside git:

- Config file.
- `train.json` and `validation.json`.
- Exported `.nnue`.
- `export.json`.
- `verify.json`.
- `variant-nnue-pytorch` commit.
- Haitaka engine commit from the dataset manifests.
- vast.ai GPU model, VRAM, hourly price, and training duration.
