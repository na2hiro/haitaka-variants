#!/usr/bin/env bash
set -euo pipefail

source /root/.cargo/env
source /workspace/haitaka-variant-nnue-pytorch/env/bin/activate
cd /workspace/haitaka-variants

v051_root=out/anhoku-v0.5.1
v05_source=out/anhoku-v0.5
v05_rerun=out/anhoku-v0.5-rerun
disk_log=out/corrected-runs-disk.log

monitor_disk() {
    while true; do
        date -Is
        df -h /
        sleep 300
    done
}

monitor_disk >> "${disk_log}" 2>&1 &
disk_monitor_pid=$!
trap 'kill "${disk_monitor_pid}" 2>/dev/null || true; wait "${disk_monitor_pid}" 2>/dev/null || true' EXIT

mkdir -p "${v051_root}/logs" "${v051_root}/artifacts"
df -h / | tee -a "${v051_root}/logs/corrected-runs-disk.log"
cargo rank-existing haitaka_learn.anhoku-v0.5.1.toml \
    --ranking-budget 8192 \
    --output "${v051_root}/artifacts/haitaka-anhoku-v0.5.1.reselected.nnue" \
    2>&1 | tee -a "${v051_root}/logs/vast-reselection.log"
cargo verify haitaka_learn.anhoku-v0.5.1.toml \
    2>&1 | tee -a "${v051_root}/logs/vast-reselection-verify.log"
date -Is > "${v051_root}/artifacts/vast-reselection-complete.txt"

mkdir -p "${v05_rerun}/datasets" "${v05_rerun}/logs" "${v05_rerun}/artifacts"
rsync -a "${v05_source}/datasets/" "${v05_rerun}/datasets/"
for dataset_file in train.bin validation.bin; do
    source_hash=$(sha256sum "${v05_source}/datasets/${dataset_file}" | cut -d ' ' -f 1)
    rerun_hash=$(sha256sum "${v05_rerun}/datasets/${dataset_file}" | cut -d ' ' -f 1)
    if [[ "${source_hash}" != "${rerun_hash}" ]]; then
        printf 'dataset checksum mismatch: %s\n' "${dataset_file}" >&2
        exit 1
    fi
    printf '%s  %s\n' "${rerun_hash}" "${dataset_file}"
done | tee "${v05_rerun}/datasets/source-sha256.txt"

df -h / | tee -a "${v05_rerun}/logs/corrected-runs-disk.log"
if [[ -f "${v05_rerun}/artifacts/training-started.txt" ]]; then
    cargo train haitaka_learn.anhoku-v0.5.toml --storage-saver \
        2>&1 | tee -a "${v05_rerun}/logs/vast-train.log"
else
    date -Is > "${v05_rerun}/artifacts/training-started.txt"
    cargo train haitaka_learn.anhoku-v0.5.toml --no-resume --storage-saver \
        2>&1 | tee -a "${v05_rerun}/logs/vast-train.log"
fi
cargo verify haitaka_learn.anhoku-v0.5.toml \
    2>&1 | tee -a "${v05_rerun}/logs/vast-verify.log"
df -h / | tee -a "${v05_rerun}/logs/corrected-runs-disk.log"
date -Is > "${v05_rerun}/artifacts/vast-complete.txt"
