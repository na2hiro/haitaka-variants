#!/usr/bin/env bash
# Run the search-heavy portions of the frozen strength-first Step 4 diagnostic.

set -euo pipefail

MODE=${1:?usage: strength_first_step4_remote.sh build|initial|selected|labels [repo-root] [run-root]}
REPO_ROOT=${2:-$PWD}
RUN_ROOT=${3:-$REPO_ROOT/out/anhoku-strength-first/step4-objective-search-diagnostic-20260907-v1}
SHARDS=8
RUNNER=target/release/strength_first_step4_diagnostic
BIN_DIR=$RUN_ROOT/remote-bin
BIN_C16=$BIN_DIR/strength-first-step4-c16
BIN_CANDIDATE=$BIN_DIR/strength-first-step4-candidate256
PARENTS=$RUN_ROOT/parent-pool/parents.jsonl
SELECTED=$RUN_ROOT/selection/selected-parents.jsonl
UNION=$RUN_ROOT/union/union-plan.jsonl
C16_MODEL=$REPO_ROOT/out/anhoku-v0.6-phase7.1-preserved/lane-c-step-16.nnue
CANDIDATE_MODEL=$REPO_ROOT/out/anhoku-strength-first/step3-compact-scratch-seed-20260906-v1/exports/selected-u00001920.nnue

cd "$REPO_ROOT"

require_file() {
  test -f "$1" || { echo "required file is missing: $1" >&2; exit 1; }
}

verify_completed_shard() {
  python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert d.get("passed") is True' "$1/receipt.json"
}

run_shards() {
  local binary=$1
  local evaluator=$2
  local model=$3
  local model_label=$4
  local nodes=$5
  local input=$6
  local output_parent=$7
  local total chunk start end shard status pid
  local -a pids=()

  require_file "$binary"
  require_file "$input"
  if [[ -n "$model" ]]; then
    require_file "$model"
  fi
  total=$(wc -l < "$input" | tr -d ' ')
  test "$total" -gt 0
  chunk=$(( (total + SHARDS - 1) / SHARDS ))
  mkdir -p "$output_parent"
  for ((shard=0; shard<SHARDS; shard++)); do
    start=$(( shard * chunk ))
    end=$(( start + chunk ))
    if (( start >= total )); then
      break
    fi
    if (( end > total )); then
      end=$total
    fi
    local output_dir
    output_dir=$(printf '%s/shard-%02d' "$output_parent" "$shard")
    if [[ -e "$output_dir" ]]; then
      verify_completed_shard "$output_dir"
      continue
    fi
    if [[ -n "$model" ]]; then
      "$binary" --mode probe --input "$input" --output-dir "$output_dir" \
        --evaluator "$evaluator" --model "$model" --model-label "$model_label" \
        --nodes "$nodes" --start "$start" --end "$end" &
    else
      "$binary" --mode probe --input "$input" --output-dir "$output_dir" \
        --evaluator "$evaluator" --model-label "$model_label" \
        --nodes "$nodes" --start "$start" --end "$end" &
    fi
    pids+=("$!")
  done
  status=0
  for pid in "${pids[@]}"; do
    wait "$pid" || status=1
  done
  test "$status" -eq 0
}

run_label_shards() {
  local total chunk start end shard status pid output_dir
  local -a pids=()
  require_file "$BIN_C16"
  require_file "$UNION"
  total=$(wc -l < "$UNION" | tr -d ' ')
  test "$total" -gt 0
  chunk=$(( (total + SHARDS - 1) / SHARDS ))
  mkdir -p "$RUN_ROOT/remote/labels"
  for ((shard=0; shard<SHARDS; shard++)); do
    start=$(( shard * chunk ))
    end=$(( start + chunk ))
    if (( start >= total )); then
      break
    fi
    if (( end > total )); then
      end=$total
    fi
    output_dir=$(printf '%s/remote/labels/shard-%02d' "$RUN_ROOT" "$shard")
    if [[ -e "$output_dir" ]]; then
      verify_completed_shard "$output_dir"
      continue
    fi
    "$BIN_C16" --mode label --input "$UNION" --output-dir "$output_dir" \
      --evaluator handcrafted --nodes 250000 --start "$start" --end "$end" &
    pids+=("$!")
  done
  status=0
  for pid in "${pids[@]}"; do
    wait "$pid" || status=1
  done
  test "$status" -eq 0
}

case "$MODE" in
  build)
    mkdir -p "$BIN_DIR"
    cargo build --release -p haitaka_learn --bin strength_first_step4_diagnostic --features anhoku
    cp "$RUNNER" "$BIN_C16"
    cargo build --release -p haitaka_learn --bin strength_first_step4_diagnostic --features anhoku,nnue-l1-256
    cp "$RUNNER" "$BIN_CANDIDATE"
    python3 - "$BIN_C16" "$BIN_CANDIDATE" "$BIN_DIR/build-receipt.json" <<'PY'
import hashlib, json, pathlib, platform, subprocess, sys

def artifact(path):
    path = pathlib.Path(path)
    return {"path": str(path), "bytes": path.stat().st_size,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}

receipt = {
    "schema": "haitaka-strength-first-step4-remote-build-v1",
    "platform": platform.platform(),
    "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
    "c16": artifact(sys.argv[1]),
    "candidate256": artifact(sys.argv[2]),
    "buildFeatures": {"c16": ["anhoku"], "candidate256": ["anhoku", "nnue-l1-256"]},
    "passed": True,
}
pathlib.Path(sys.argv[3]).write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
PY
    ;;
  initial)
    require_file "$PARENTS"
    run_shards "$BIN_C16" handcrafted "" handcrafted-10000 10000 "$PARENTS" "$RUN_ROOT/remote/initial/handcrafted"
    run_shards "$BIN_C16" nnue "$C16_MODEL" c16-10000 10000 "$PARENTS" "$RUN_ROOT/remote/initial/c16"
    run_shards "$BIN_CANDIDATE" nnue "$CANDIDATE_MODEL" candidate-10000 10000 "$PARENTS" "$RUN_ROOT/remote/initial/candidate"
    ;;
  selected)
    require_file "$SELECTED"
    run_shards "$BIN_C16" handcrafted "" handcrafted-deep 250000 "$SELECTED" "$RUN_ROOT/remote/selected/handcrafted-deep"
    run_shards "$BIN_CANDIDATE" nnue "$CANDIDATE_MODEL" candidate-low 2500 "$SELECTED" "$RUN_ROOT/remote/selected/candidate-low"
    run_shards "$BIN_CANDIDATE" nnue "$CANDIDATE_MODEL" candidate-high 40000 "$SELECTED" "$RUN_ROOT/remote/selected/candidate-high"
    ;;
  labels)
    require_file "$UNION"
    run_label_shards
    if [[ ! -e "$RUN_ROOT/remote/features" ]]; then
      "$BIN_CANDIDATE" --mode features --input "$UNION" --output-dir "$RUN_ROOT/remote/features"
    else
      verify_completed_shard "$RUN_ROOT/remote/features"
    fi
    ;;
  *)
    echo "unknown mode: $MODE" >&2
    exit 2
    ;;
esac
