#!/usr/bin/env bash
set -euo pipefail

repo_root=${HAITAKA_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}
cd "${repo_root}"

v1=out/anhoku-v0.7-phase11b-seed80-v1/artifacts/haitaka-anhoku-v0.7-phase11b-seed80-v1-step16.nnue
v2=out/anhoku-v0.7-phase11b-seed80-v2/artifacts/haitaka-anhoku-v0.7-phase11b-seed80-v2-step16.nnue
gate_root=out/anhoku-v0.7-phase11b-seed80-v2/artifacts/phase11b-gate/seed80
initial=${gate_root}/batch-1024/self-play-report.json
report_dir=${gate_root}/batch-3072

printf '%s  %s\n' f7111caf885db66e528c56f23ffe9446609daf1f9a1b3a13cc1c2043b1a66632 "${v1}" \
    | sha256sum --check
printf '%s  %s\n' 7e94100c24c495265fed01c06c4f9359f44aa52182c8481b46bf936f63c63a31 "${v2}" \
    | sha256sum --check
jq -e '
    .summary.games == 1024 and
    .summary.pairCount == 512 and
    .summary.pairedElo95Ci[0] <= 0 and
    .summary.pairedElo95Ci[1] >= 0 and
    .command.movetimeMs == 100 and
    .command.openingRandomPlies == 4 and
    .command.seed == 1180 and
    .command.maxPlies == 200 and
    .engines[0].nnueSha256 == "7e94100c24c495265fed01c06c4f9359f44aa52182c8481b46bf936f63c63a31" and
    .engines[1].nnueSha256 == "f7111caf885db66e528c56f23ffe9446609daf1f9a1b3a13cc1c2043b1a66632"
' "${initial}" >/dev/null
[[ ! -e "${report_dir}/self-play-report.json" ]]
printf '%s  target/release/haitaka_cli\n' "$(cut -d' ' -f1 "${gate_root}/engine-sha256.txt")" \
    | sha256sum --check

target/release/haitaka_cli self-play \
    --games 3072 \
    --threads 20 \
    --a-eval nnue \
    --b-eval nnue \
    --a-nnue "${v2}" \
    --b-nnue "${v1}" \
    --movetime-ms 100 \
    --opening-random-plies 4 \
    --seed 12180 \
    --max-plies 200 \
    --report-dir "${report_dir}" \
    2>&1 | tee "${gate_root}/batch-3072.log"
