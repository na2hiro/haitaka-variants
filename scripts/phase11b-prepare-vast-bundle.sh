#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "${repo_root}"

v1_config=haitaka_learn.anhoku-v0.7-phase11b-seed80-v1.toml
v2_config=haitaka_learn.anhoku-v0.7-phase11b-seed80-v2.toml
source_root=out/anhoku-v0.6-phase8d-b-root-262k
v1_root=out/anhoku-v0.7-phase11b-seed80-v1
v2_root=out/anhoku-v0.7-phase11b-seed80-v2
bundle_root=target/pretrain-bundles
paired_bundle=${bundle_root}/anhoku-v0.7-phase11b-seed80-paired.tgz

mkdir -p "${bundle_root}"
expected_hashes=$(mktemp)
pair_stage=$(mktemp -d "${bundle_root}/phase11b-pair.XXXXXX")
cleanup() {
    rm -f "${expected_hashes}"
    rm -rf "${pair_stage}"
}
trap cleanup EXIT

cat >"${expected_hashes}" <<'HASHES'
049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0  out/anhoku-v0.6-phase7.1-preserved/lane-c-step-16.nnue
aa2fc9decbb767d170c10a523ccefb9bb01ef3a39dc7d2e36606a34fb5e85599  out/anhoku-v0.6-phase8d-b-root-262k/datasets/train.bin
36e1360e75c81af311efca4497bc611e99fd6bb01fbad8cb2be8bac605bdb2e6  out/anhoku-v0.6-phase8d-b-root-262k/datasets/validation.bin
0bc3d8459f0adb379ab182f589a7495aa96b94ad42f28724a43eebadae4961ee  out/anhoku-v0.6-phase8d-b-root-262k/datasets/train.json
e029633a6f86fb7277c652b9634f447f93bdbc1f47e6de8b270e7d00e7c08895  out/anhoku-v0.6-phase8d-b-root-262k/datasets/validation.json
c7dd09df94b5c781688bff1c8c14dceb33fd20e201f359b0cd0fc7b9d56faf59  out/anhoku-v0.6-phase8d-b-root-262k/artifacts/phase8d-b1-final-train-audit.json
5d5923b88ed734d07551de14aa8724c5e15d192917d62b97f382ae6d9786d35e  out/anhoku-v0.6-phase8d-b-root-262k/artifacts/phase8d-b1-final-validation-audit.json
HASHES
sha256sum --check "${expected_hashes}"

jq -e '.gates.phase11b_go == true' \
    out/anhoku-v0.7-phase11a/artifacts/phase11a-gate.json >/dev/null

normalize_config() {
    sed -E \
        -e '/^[[:space:]]*#/d' \
        -e '/^(output_dir|features|output_name|description) = /d' \
        "$1"
}
diff -u <(normalize_config "${v1_config}") <(normalize_config "${v2_config}")

grep -Fx 'features = "HalfKAv2^+DonorSingleEff"' "${v1_config}" >/dev/null
grep -Fx 'features = "HalfKAv2^+DonorReceiverPairV2"' "${v2_config}" >/dev/null
for config in "${v1_config}" "${v2_config}"; do
    grep -Fx 'max_steps = 16' "${config}" >/dev/null
    grep -Fx 'extra_args = ["--seed", "80", "--threads", "8", "--accelerator", "gpu", "--devices", "1"]' "${config}" >/dev/null
done

for lane_root in "${v1_root}" "${v2_root}"; do
    mkdir -p "${lane_root}/datasets"
    for name in train.bin validation.bin train.json validation.json; do
        ln -f "${source_root}/datasets/${name}" "${lane_root}/datasets/${name}"
    done
    sha256sum "${lane_root}/datasets/train.bin" \
        "${lane_root}/datasets/validation.bin" \
        "${lane_root}/datasets/train.json" \
        "${lane_root}/datasets/validation.json" \
        >"${lane_root}/datasets/source-sha256.txt"
done

cargo run -p haitaka_learn --release --features anhoku -- \
    validate-openings --config "${v1_config}"
cargo run -p haitaka_learn --release --features anhoku -- \
    validate-openings --config "${v2_config}"

v1_bundle=${bundle_root}/anhoku-v0.7-phase11b-seed80-v1.tgz
v2_bundle=${bundle_root}/anhoku-v0.7-phase11b-seed80-v2.tgz
cargo bundle-pretrain "${v1_config}" --output "${v1_bundle}"
cargo bundle-pretrain "${v2_config}" --output "${v2_bundle}"
tar -xzf "${v1_bundle}" -C "${pair_stage}"
tar -xzf "${v2_bundle}" -C "${pair_stage}"

mkdir -p "${pair_stage}/phase11b-input-audit"
cp "${source_root}/artifacts/phase8d-b1-final-train-audit.json" \
    "${source_root}/artifacts/phase8d-b1-final-validation-audit.json" \
    out/anhoku-v0.7-phase11a/artifacts/phase11a-gate.json \
    out/anhoku-v0.7-phase11a/artifacts/trainer-feature-parity.json \
    "${pair_stage}/phase11b-input-audit/"
git rev-parse HEAD >"${pair_stage}/phase11b-input-audit/bundle-source-commit.txt"

tar -czf "${paired_bundle}" -C "${pair_stage}" .
sha256sum "${paired_bundle}" >"${paired_bundle}.sha256"
tar -tzf "${paired_bundle}" >/dev/null

printf '\nPhase 11-B paired Vast bundle is ready:\n'
ls -lh "${paired_bundle}" "${paired_bundle}.sha256"
cat "${paired_bundle}.sha256"
