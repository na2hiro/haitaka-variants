#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 6 ]]; then
  echo "usage: $0 <generate|pilot|deep-core> <first-shard> <last-shard> <parallelism> <workspace> <output-dir>" >&2
  exit 2
fi

stage=$1
first=$2
last=$3
parallelism=$4
workspace=$5
output_dir=$6
binary="$workspace/target/release/haitaka_learn"
contract="$workspace/r0/anhoku-reboot/r4-data-contract.json"

run_one() {
  local shard=$1
  local padded
  padded=$(printf '%05d' "$shard")
  case "$stage" in
    generate)
      if [[ -f "$output_dir/positions/position-shard-$padded.json" ]]; then return; fi
      "$binary" r4-generate-positions --contract "$contract" --output-dir "$output_dir" --shard-index "$shard"
      ;;
    pilot|deep-core)
      if [[ -f "$output_dir/labels/$stage/labeled-shard-$padded.json" ]]; then return; fi
      "$binary" r4-label-shard \
        --contract "$contract" \
        --input "$output_dir/label-inputs/$stage/label-input-$padded.jsonl" \
        --output-dir "$output_dir" \
        --jobs 1
      ;;
    *)
      echo "unsupported stage: $stage" >&2
      exit 2
      ;;
  esac
}

export stage workspace output_dir binary contract
export -f run_one
seq "$first" "$last" | xargs -P "$parallelism" -n 1 bash -c 'run_one "$1"' _
