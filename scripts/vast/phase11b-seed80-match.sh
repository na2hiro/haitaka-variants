#!/usr/bin/env bash
set -euo pipefail

repo_root=${HAITAKA_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}
cd "${repo_root}"
protocol=${PHASE11B_MATCH_PROTOCOL:-scripts/phase11b-seed80-match-protocol.json}

v1=out/anhoku-v0.7-phase11b-seed80-v1/artifacts/haitaka-anhoku-v0.7-phase11b-seed80-v1-step16.nnue
v2=out/anhoku-v0.7-phase11b-seed80-v2/artifacts/haitaka-anhoku-v0.7-phase11b-seed80-v2-step16.nnue
gate_root=out/anhoku-v0.7-phase11b-seed80-v2/artifacts/phase11b-gate/seed80
report_dir=${gate_root}/batch-1024

printf '%s  %s\n' f7111caf885db66e528c56f23ffe9446609daf1f9a1b3a13cc1c2043b1a66632 "${v1}" \
    | sha256sum --check
printf '%s  %s\n' 7e94100c24c495265fed01c06c4f9359f44aa52182c8481b46bf936f63c63a31 "${v2}" \
    | sha256sum --check
[[ ! -e "${report_dir}/self-play-report.json" ]]

mkdir -p "${gate_root}"
cp "${protocol}" "${gate_root}/match-protocol.json"
cargo build --release -p haitaka_cli --features anhoku
sha256sum target/release/haitaka_cli >"${gate_root}/engine-sha256.txt"
git rev-parse HEAD >"${gate_root}/engine-source-commit.txt"

target/release/haitaka_cli self-play \
    --games 1024 \
    --threads 20 \
    --a-eval nnue \
    --b-eval nnue \
    --a-nnue "${v2}" \
    --b-nnue "${v1}" \
    --movetime-ms 100 \
    --opening-random-plies 4 \
    --seed 1180 \
    --max-plies 200 \
    --report-dir "${report_dir}" \
    2>&1 | tee "${gate_root}/batch-1024.log"
