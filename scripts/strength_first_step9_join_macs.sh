#!/usr/bin/env bash
set -euo pipefail

workspace=$(cd "$(dirname "$0")/.." && pwd)
mbp=192.168.0.12
mba=192.168.0.5
mbp_workspace=/Users/na2hiro/anhoku-step9-model-b-c3109e8
mba_workspace=/Users/na2hiro/anhoku-step9-model-b-2e8e45d
source_rel=out/anhoku-strength-first/step9-model-b-source-20260911-v2
mba_copy_rel=out/anhoku-strength-first/step9-model-b-source-20260911-v2-mba-lane
local_mba_copy="$workspace/$mba_copy_rel"
frozen_rel=out/anhoku-strength-first/step9-model-b-corpus-20260911-v1
config=haitaka_learn.anhoku-strength-first-step9-model-b-source-v2.toml

while ! ssh "$mbp" test -f "$mbp_workspace/$source_rel/mbp-lane.complete"; do
  sleep 300
done
while ! ssh "$mba" test -f "$mba_workspace/$source_rel/mba-lane.complete"; do
  sleep 300
done

mkdir -p "$local_mba_copy"
rsync -a "$mba:$mba_workspace/$source_rel/" "$local_mba_copy/"
ssh "$mbp" mkdir -p "$mbp_workspace/$mba_copy_rel"
rsync -a "$local_mba_copy/" "$mbp:$mbp_workspace/$mba_copy_rel/"

ssh "$mbp" "cd '$mbp_workspace' && \
  target/release/haitaka_learn merge-data \
    --config '$config' \
    --input '$source_rel' \
    --input '$mba_copy_rel' && \
  target/release/haitaka_learn audit-data \
    --bin '$source_rel/datasets/train.bin' \
    --manifest '$source_rel/datasets/train.json' \
    --output '$source_rel/artifacts/train-audit.json' && \
  target/release/haitaka_learn audit-data \
    --bin '$source_rel/datasets/validation.bin' \
    --manifest '$source_rel/datasets/validation.json' \
    --output '$source_rel/artifacts/validation-audit.json' && \
  target/release/haitaka_learn strength-first-freeze-model-b-corpus \
    --model-a inputs/model-a/training.bin \
    --model-a inputs/model-a/validation.bin \
    --model-a inputs/model-a/development.bin \
    --source-train '$source_rel/datasets/train.bin' \
    --source-validation '$source_rel/datasets/validation.bin' \
    --output-dir '$frozen_rel' \
    --train-records 880000 \
    --validation-records 60000 \
    --selection-seed 2026091107 && \
  shasum -a 256 \
    '$source_rel/datasets/train.bin' \
    '$source_rel/datasets/train.json' \
    '$source_rel/datasets/validation.bin' \
    '$source_rel/datasets/validation.json' \
    '$source_rel/artifacts/train-audit.json' \
    '$source_rel/artifacts/validation-audit.json' \
    '$frozen_rel/training.bin' \
    '$frozen_rel/training.json' \
    '$frozen_rel/training.jsonl' \
    '$frozen_rel/validation.bin' \
    '$frozen_rel/validation.json' \
    '$frozen_rel/validation.jsonl' \
    '$frozen_rel/model-b-corpus.json' \
    > '$frozen_rel/sha256sums.txt'"
