#!/usr/bin/env bash
set -euo pipefail

workspace="${1:-$(pwd)}"
trainer_checkout="${2:-${workspace}/../engine/variant-nnue-pytorch}"
bundle="${3:-${workspace}/target/pretrain-bundles/strength-first-step9-model-b-invariants-v1.tgz}"
pinned_trainer=61666d9e3653e4df9881b14c23f8fdcc4bf7779b

if [[ "$(git -C "${trainer_checkout}" rev-parse HEAD)" != "${pinned_trainer}" ]]; then
  echo "trainer checkout is not at ${pinned_trainer}" >&2
  exit 1
fi

stage="$(mktemp -d)"
trap 'rm -rf "${stage}"' EXIT
root="${stage}/model-b-workspace"
repo="${root}/haitaka-variants-anhoku"
mkdir -p "${repo}" "${root}/engine" "$(dirname "${bundle}")"

cd "${workspace}"
files=(
  scripts/strength_first_model_b_trainer.py
  scripts/strength_first_trainer.py
  scripts/r5_trainer.py
  trainer-patches/variant-nnue-pytorch-phase7.1.patch
  trainer-patches/variant-nnue-pytorch-r5.patch
  trainer-patches/evaluate.py
  haitaka_learn/trainer_overlay/feature_set.py
  haitaka_learn/trainer_overlay/features.py
  haitaka_learn/trainer_overlay/donor_features.py
  haitaka_learn/trainer_overlay/training_data_loader.cpp
  haitaka_learn/openings/anhoku-v3.tsv
  out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/training.bin
  out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/training.json
  out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/training.jsonl
  out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/validation.bin
  out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/validation.json
  out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/validation.jsonl
  out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/model-b-corpus.json
  out/anhoku-strength-first/step9-model-b-corpus-20260911-v1/sha256sums.txt
)

for file in "${files[@]}"; do
  test -f "${file}"
  rsync -aR "${file}" "${repo}/"
done

git clone --quiet --no-hardlinks "${trainer_checkout}" "${root}/engine/variant-nnue-pytorch"
git -C "${root}/engine/variant-nnue-pytorch" checkout --quiet "${pinned_trainer}"

cd "${root}"
find haitaka-variants-anhoku engine/variant-nnue-pytorch \
  -type f -not -path '*/.git/*' -print0 \
  | sort -z \
  | xargs -0 shasum -a 256 > bundle-invariants.sha256
printf '%s\n' "${pinned_trainer}" > trainer-revision.txt

tar -czf "${bundle}" -C "${stage}" model-b-workspace
shasum -a 256 "${bundle}" > "${bundle}.sha256"
tar -tzf "${bundle}" >/dev/null
cat "${bundle}.sha256"
