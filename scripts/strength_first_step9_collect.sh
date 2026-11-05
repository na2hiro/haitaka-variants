#!/usr/bin/env bash
set -euo pipefail

workspace=$(cd "$(dirname "$0")/.." && pwd)
remote=192.168.0.12
remote_workspace=/Users/na2hiro/anhoku-step9-model-b-c3109e8
source_rel=out/anhoku-strength-first/step9-model-b-source-20260911-v2
frozen_rel=out/anhoku-strength-first/step9-model-b-corpus-20260911-v1

while ! ssh "$remote" test -f "$remote_workspace/$frozen_rel/sha256sums.txt"; do
  sleep 300
done

mkdir -p "$workspace/$source_rel" "$workspace/$frozen_rel"
rsync -a "$remote:$remote_workspace/$source_rel/" "$workspace/$source_rel/"
rsync -a "$remote:$remote_workspace/$frozen_rel/" "$workspace/$frozen_rel/"
rsync -a "$remote:$remote_workspace/step9-model-b-pipeline.log" \
  "$workspace/$frozen_rel/mbp-pipeline.log"
