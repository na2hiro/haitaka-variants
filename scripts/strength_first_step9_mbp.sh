#!/usr/bin/env bash
set -euo pipefail

workspace=$(cd "$(dirname "$0")/.." && pwd)
cd "$workspace"

prior_pid=${1:-0}
config=haitaka_learn.anhoku-strength-first-step9-model-b-source-v2.toml
expected_config_sha=36a8521f1dde00e94db687a34c7b716060847aa78a1095c1cda3920586a2311e
actual_config_sha=$(shasum -a 256 "$config" | cut -d ' ' -f 1)
if [[ "$actual_config_sha" != "$expected_config_sha" ]]; then
  echo "config identity mismatch: $actual_config_sha" >&2
  exit 1
fi

if [[ "$prior_pid" != 0 ]]; then
  while kill -0 "$prior_pid" 2>/dev/null; do
    sleep 5
  done
fi

cargo build --release -p haitaka_learn --features anhoku
target/release/haitaka_learn validate-openings --config "$config"
target/release/haitaka_learn generate-data --config "$config" --jobs 10

source_root=out/anhoku-strength-first/step9-model-b-source-20260911-v2/datasets
frozen_root=out/anhoku-strength-first/step9-model-b-corpus-20260911-v1
target/release/haitaka_learn strength-first-freeze-model-b-corpus \
  --model-a inputs/model-a/training.bin \
  --model-a inputs/model-a/validation.bin \
  --model-a inputs/model-a/development.bin \
  --source-train "$source_root/train.bin" \
  --source-validation "$source_root/validation.bin" \
  --output-dir "$frozen_root" \
  --train-records 880000 \
  --validation-records 60000 \
  --selection-seed 2026091107

shasum -a 256 \
  "$source_root/train.bin" \
  "$source_root/train.json" \
  "$source_root/validation.bin" \
  "$source_root/validation.json" \
  "$frozen_root/training.bin" \
  "$frozen_root/training.json" \
  "$frozen_root/validation.bin" \
  "$frozen_root/validation.json" \
  "$frozen_root/model-b-corpus.json" \
  > "$frozen_root/sha256sums.txt"
