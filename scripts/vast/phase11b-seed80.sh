#!/usr/bin/env bash
set -euo pipefail

mode=${1:-preflight}
if [[ "${mode}" != "preflight" && "${mode}" != "train" ]]; then
    printf 'usage: %s [preflight|train]\n' "$0" >&2
    exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "${repo_root}"

v1_config=haitaka_learn.anhoku-v0.7-phase11b-seed80-v1.toml
v2_config=haitaka_learn.anhoku-v0.7-phase11b-seed80-v2.toml
v1_root=out/anhoku-v0.7-phase11b-seed80-v1
v2_root=out/anhoku-v0.7-phase11b-seed80-v2
trainer_root=../haitaka-variant-nnue-pytorch
python=${trainer_root}/env/bin/python
expected_trainer=61666d9e3653e4df9881b14c23f8fdcc4bf7779b

git merge-base --is-ancestor c26e4fd HEAD
expected_haitaka=$(<phase11b-input-audit/bundle-source-commit.txt)
[[ $(git rev-parse HEAD) == "${expected_haitaka}" ]]
# bundle-pretrain intentionally rewrites these two tracked configs to use its
# archive-local bootstrap. No other tracked file may differ on the rented host.
git diff --quiet -- . \
    ":(exclude)${v1_config}" ":(exclude)${v2_config}"
git diff --cached --quiet -- . \
    ":(exclude)${v1_config}" ":(exclude)${v2_config}"
[[ $(git -C "${trainer_root}" rev-parse HEAD) == "${expected_trainer}" ]]

normalize_config() {
    sed -E \
        -e '/^[[:space:]]*#/d' \
        -e '/^(output_dir|features|output_name|description) = /d' \
        "$1"
}
diff -u <(normalize_config "${v1_config}") <(normalize_config "${v2_config}")

check_lane_hashes() {
    local lane_root=$1
    printf '%s  %s/datasets/train.bin\n' \
        aa2fc9decbb767d170c10a523ccefb9bb01ef3a39dc7d2e36606a34fb5e85599 "${lane_root}"
    printf '%s  %s/datasets/validation.bin\n' \
        36e1360e75c81af311efca4497bc611e99fd6bb01fbad8cb2be8bac605bdb2e6 "${lane_root}"
    printf '%s  %s/datasets/train.json\n' \
        0bc3d8459f0adb379ab182f589a7495aa96b94ad42f28724a43eebadae4961ee "${lane_root}"
    printf '%s  %s/datasets/validation.json\n' \
        e029633a6f86fb7277c652b9634f447f93bdbc1f47e6de8b270e7d00e7c08895 "${lane_root}"
}
{
    printf '%s  bootstrap/lane-c-step-16.nnue\n' \
        049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0
    check_lane_hashes "${v1_root}"
    check_lane_hashes "${v2_root}"
} | sha256sum --check

available_kib=$(df -Pk . | awk 'NR == 2 {print $4}')
minimum_kib=$((25 * 1024 * 1024))
if (( available_kib < minimum_kib )); then
    printf 'need at least 25 GiB free; only %s KiB available\n' "${available_kib}" >&2
    exit 1
fi

[[ -x "${python}" ]]
"${python}" - <<'PY'
import torch
assert torch.cuda.is_available(), "PyTorch cannot see CUDA"
props = torch.cuda.get_device_properties(0)
assert props.total_memory >= 12 * 1024**3, "Phase 11-B requires at least 12 GiB VRAM"
print(f"torch={torch.__version__}")
print(f"gpu={props.name}")
print(f"vram_gib={props.total_memory / 1024**3:.1f}")
PY

mkdir -p "${v1_root}/artifacts" "${v1_root}/logs" \
    "${v2_root}/artifacts" "${v2_root}/logs"
nvidia-smi >"${v1_root}/artifacts/vast-nvidia-smi-preflight.txt"
cp "${v1_root}/artifacts/vast-nvidia-smi-preflight.txt" \
    "${v2_root}/artifacts/vast-nvidia-smi-preflight.txt"

cargo run -p haitaka_learn --release --features anhoku -- \
    verify-donor-receiver-pair-v2-trainer \
    --config "${v2_config}" \
    --output "${v2_root}/artifacts/trainer-feature-parity-remote.json"

for lane in v1 v2; do
    config_var=${lane}_config
    root_var=${lane}_root
    config=${!config_var}
    lane_root=${!root_var}
    if [[ ! -s "${lane_root}/artifacts/bootstrap.pt" ]]; then
        cargo run -p haitaka_learn --release --features anhoku -- \
            prepare-bootstrap --config "${config}"
    fi
done

{
    printf 'preflight_completed=%s\n' "$(date -Is)"
    printf 'haitaka_commit=%s\n' "$(git rev-parse HEAD)"
    printf 'trainer_commit=%s\n' "$(git -C "${trainer_root}" rev-parse HEAD)"
    printf 'v1_config_sha256=%s\n' "$(sha256sum "${v1_config}" | awk '{print $1}')"
    printf 'v2_config_sha256=%s\n' "$(sha256sum "${v2_config}" | awk '{print $1}')"
    printf 'vast_hourly_usd=%s\n' "${VAST_HOURLY_USD:-not-recorded}"
} | tee "${v1_root}/artifacts/vast-preflight.txt" \
    "${v2_root}/artifacts/vast-preflight.txt"

printf 'Phase 11-B preflight passed; no optimizer step has run.\n'
if [[ "${mode}" == "preflight" ]]; then
    exit 0
fi

run_lane() {
    local label=$1
    local config=$2
    local lane_root=$3
    local output_name=$4
    local train_log=${lane_root}/logs/vast-train-step16.log

    printf 'starting %s at %s\n' "${label}" "$(date -Is)" | tee -a "${train_log}"
    cargo run -p haitaka_learn --release --features anhoku -- \
        train --config "${config}" 2>&1 | tee -a "${train_log}"

    mapfile -t step16_checkpoints < <(
        find "${lane_root}/logs" -type f -name '*step=16.ckpt' \
            -printf '%T@ %p\n' | sort -nr
    )
    if (( ${#step16_checkpoints[@]} == 0 )); then
        printf '%s produced no step-16 checkpoint\n' "${label}" >&2
        exit 1
    fi
    local checkpoint=${step16_checkpoints[0]#* }
    local output=${lane_root}/artifacts/${output_name}
    cargo run -p haitaka_learn --release --features anhoku -- \
        export-checkpoint --config "${config}" \
        --checkpoint "${checkpoint}" --output "${output}"
    cargo run -p haitaka_learn --release --features anhoku -- \
        verify --config "${config}"
    {
        printf 'lane=%s\n' "${label}"
        printf 'completed=%s\n' "$(date -Is)"
        printf 'checkpoint=%s\n' "${checkpoint}"
        sha256sum "${checkpoint}" "${output}"
    } | tee "${lane_root}/artifacts/step16-identity.txt"
}

run_lane v1 "${v1_config}" "${v1_root}" \
    haitaka-anhoku-v0.7-phase11b-seed80-v1-step16.nnue
run_lane v2 "${v2_config}" "${v2_root}" \
    haitaka-anhoku-v0.7-phase11b-seed80-v2-step16.nnue

mapfile -t v1_final_checkpoints < <(
    find "${v1_root}/logs" -type f -name '*step=16.ckpt' \
        -printf '%T@ %p\n' | sort -nr
)
mapfile -t v2_final_checkpoints < <(
    find "${v2_root}/logs" -type f -name '*step=16.ckpt' \
        -printf '%T@ %p\n' | sort -nr
)
v1_final_checkpoint=${v1_final_checkpoints[0]#* }
v2_final_checkpoint=${v2_final_checkpoints[0]#* }

results_archive=anhoku-v0.7-phase11b-seed80-results.tgz
tar --exclude='bootstrap.pt' \
    --exclude='bootstrap-donor-receiver-pair-v2.nnue' \
    -czf "${results_archive}" \
    "${v1_config}" "${v2_config}" \
    "${v1_root}/datasets/train.json" \
    "${v1_root}/datasets/validation.json" \
    "${v1_root}/artifacts" \
    "${v1_root}/logs/vast-train-step16.log" \
    "${v1_final_checkpoint}" \
    "${v2_root}/datasets/train.json" \
    "${v2_root}/datasets/validation.json" \
    "${v2_root}/artifacts" \
    "${v2_root}/logs/vast-train-step16.log" \
    "${v2_final_checkpoint}"
sha256sum "${results_archive}" | tee "${results_archive}.sha256"
printf 'training complete; download %s and %s before destroying the instance\n' \
    "${results_archive}" "${results_archive}.sha256"
