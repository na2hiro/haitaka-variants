#!/usr/bin/env bash
set -euo pipefail

log=/workspace/haitaka-variants/out/corrected-runs-disk.log
while supervisorctl status haitaka_corrected_runs | grep -q RUNNING; do
    date -Is >> "${log}"
    df -h / >> "${log}"
    sleep 300
done
