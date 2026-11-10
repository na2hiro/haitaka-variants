#!/usr/bin/env bash
# Stage and complete the frozen Step 4 CPU diagnostic when the assigned MBP is online.

set -euo pipefail

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
CONTRACT=$REPO_ROOT/r0/anhoku-strength-first/step4-diagnostic-v1.json
LOCAL_RUN=$REPO_ROOT/out/anhoku-strength-first/step4-objective-search-diagnostic-20260907-v1
REMOTE_HOST=192.168.0.12
REMOTE_HOST_KEY_ALIAS=mbp
REMOTE_REPO=/tmp/haitaka-strength-first-step4/repo
REMOTE_RUN=$REMOTE_REPO/out/anhoku-strength-first/step4-objective-search-diagnostic-20260907-v1
CONNECT_DEADLINE_SECONDS=43200
SEARCH_DEADLINE_SECONDS=172500
REMOTE_PATH='export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"'

mkdir -p "$LOCAL_RUN"
exec >> "$LOCAL_RUN/controller.log" 2>&1
echo "$(date --iso-8601=seconds) controller start"

PYTHONDONTWRITEBYTECODE=1 python3 "$REPO_ROOT/scripts/strength_first_step4.py" validate --contract "$CONTRACT"

connect_started=$(date +%s)
while ! ssh -o HostKeyAlias="$REMOTE_HOST_KEY_ALIAS" -o BatchMode=yes -o ConnectTimeout=8 "$REMOTE_HOST" true; do
  now=$(date +%s)
  if (( now - connect_started >= CONNECT_DEADLINE_SECONDS )); then
    echo "$(date --iso-8601=seconds) MBP remained unreachable for 12 hours"
    exit 75
  fi
  sleep 60
done
echo "$(date --iso-8601=seconds) MBP reachable"

ssh -o HostKeyAlias="$REMOTE_HOST_KEY_ALIAS" "$REMOTE_HOST" "mkdir -p '$REMOTE_REPO' '$REMOTE_RUN/parent-pool'"
rsync -e "ssh -o HostKeyAlias=$REMOTE_HOST_KEY_ALIAS" -a \
  "$REPO_ROOT/Cargo.toml" "$REPO_ROOT/Cargo.lock" \
  "$REPO_ROOT/haitaka" "$REPO_ROOT/haitaka_cli" "$REPO_ROOT/haitaka_learn" \
  "$REPO_ROOT/haitaka_types" "$REPO_ROOT/haitaka_wasm" "$REPO_ROOT/xtask" \
  "$REPO_ROOT/scripts" "$REPO_ROOT/r0" \
  "$REMOTE_HOST:$REMOTE_REPO/"
(
  cd "$REPO_ROOT"
  rsync -e "ssh -o HostKeyAlias=$REMOTE_HOST_KEY_ALIAS" -aR \
    out/anhoku-v0.6-phase7.1-preserved/lane-c-step-16.nnue \
    out/anhoku-strength-first/step3-compact-scratch-seed-20260906-v1/exports/selected-u00001920.nnue \
    out/anhoku-strength-first/step4-objective-search-diagnostic-20260907-v1/parent-pool/parents.jsonl \
    out/anhoku-strength-first/step4-objective-search-diagnostic-20260907-v1/parent-pool/summary.json \
    "$REMOTE_HOST:$REMOTE_REPO/"
)

remote_phase() {
  local phase=$1
  local remaining
  remaining=$(( SEARCH_DEADLINE_SECONDS - ($(date +%s) - search_started) ))
  if (( remaining <= 0 )); then
    echo "Step 4 search wall deadline reached before phase $phase" >&2
    exit 124
  fi
  echo "$(date --iso-8601=seconds) remote phase $phase start; ${remaining}s remain"
  timeout --signal=TERM --kill-after=30 "$remaining" \
    ssh -o HostKeyAlias="$REMOTE_HOST_KEY_ALIAS" "$REMOTE_HOST" "$REMOTE_PATH; cd '$REMOTE_REPO'; bash scripts/strength_first_step4_remote.sh '$phase' '$REMOTE_REPO' '$REMOTE_RUN'"
  echo "$(date --iso-8601=seconds) remote phase $phase complete"
}

search_started=$(date +%s)
remote_phase build
remote_phase initial

mkdir -p "$LOCAL_RUN/remote/initial"
rsync -e "ssh -o HostKeyAlias=$REMOTE_HOST_KEY_ALIAS" -a "$REMOTE_HOST:$REMOTE_RUN/remote/initial/" "$LOCAL_RUN/remote/initial/"

if [[ ! -e "$LOCAL_RUN/selection" ]]; then
  PYTHONDONTWRITEBYTECODE=1 python3 "$REPO_ROOT/scripts/strength_first_step4.py" select \
    --contract "$CONTRACT" \
    --parents "$LOCAL_RUN/parent-pool/parents.jsonl" \
    $(printf -- '--handcrafted-probe %q ' "$LOCAL_RUN"/remote/initial/handcrafted/shard-*/probe.jsonl) \
    $(printf -- '--c16-probe %q ' "$LOCAL_RUN"/remote/initial/c16/shard-*/probe.jsonl) \
    $(printf -- '--candidate-probe %q ' "$LOCAL_RUN"/remote/initial/candidate/shard-*/probe.jsonl) \
    --output-dir "$LOCAL_RUN/selection"
fi
ssh -o HostKeyAlias="$REMOTE_HOST_KEY_ALIAS" "$REMOTE_HOST" "mkdir -p '$REMOTE_RUN/selection'"
rsync -e "ssh -o HostKeyAlias=$REMOTE_HOST_KEY_ALIAS" -a "$LOCAL_RUN/selection/selected-parents.jsonl" "$REMOTE_HOST:$REMOTE_RUN/selection/"

remote_phase selected
mkdir -p "$LOCAL_RUN/remote/selected"
rsync -e "ssh -o HostKeyAlias=$REMOTE_HOST_KEY_ALIAS" -a "$REMOTE_HOST:$REMOTE_RUN/remote/selected/" "$LOCAL_RUN/remote/selected/"

if [[ ! -e "$LOCAL_RUN/union" ]]; then
  PYTHONDONTWRITEBYTECODE=1 python3 "$REPO_ROOT/scripts/strength_first_step4.py" union \
    --contract "$CONTRACT" \
    --selected "$LOCAL_RUN/selection/selected-parents.jsonl" \
    $(printf -- '--handcrafted-probe %q ' "$LOCAL_RUN"/remote/initial/handcrafted/shard-*/probe.jsonl) \
    $(printf -- '--c16-probe %q ' "$LOCAL_RUN"/remote/initial/c16/shard-*/probe.jsonl) \
    $(printf -- '--candidate-probe %q ' "$LOCAL_RUN"/remote/initial/candidate/shard-*/probe.jsonl) \
    $(printf -- '--deep-handcrafted-probe %q ' "$LOCAL_RUN"/remote/selected/handcrafted-deep/shard-*/probe.jsonl) \
    $(printf -- '--candidate-low-probe %q ' "$LOCAL_RUN"/remote/selected/candidate-low/shard-*/probe.jsonl) \
    $(printf -- '--candidate-high-probe %q ' "$LOCAL_RUN"/remote/selected/candidate-high/shard-*/probe.jsonl) \
    --output-dir "$LOCAL_RUN/union"
fi
ssh -o HostKeyAlias="$REMOTE_HOST_KEY_ALIAS" "$REMOTE_HOST" "mkdir -p '$REMOTE_RUN/union'"
rsync -e "ssh -o HostKeyAlias=$REMOTE_HOST_KEY_ALIAS" -a "$LOCAL_RUN/union/union-plan.jsonl" "$REMOTE_HOST:$REMOTE_RUN/union/"

remote_phase labels
mkdir -p "$LOCAL_RUN/remote/labels" "$LOCAL_RUN/remote/features"
rsync -e "ssh -o HostKeyAlias=$REMOTE_HOST_KEY_ALIAS" -a "$REMOTE_HOST:$REMOTE_RUN/remote/labels/" "$LOCAL_RUN/remote/labels/"
rsync -e "ssh -o HostKeyAlias=$REMOTE_HOST_KEY_ALIAS" -a "$REMOTE_HOST:$REMOTE_RUN/remote/features/" "$LOCAL_RUN/remote/features/"

if [[ ! -e "$LOCAL_RUN/full-precision" ]]; then
  PYTHONDONTWRITEBYTECODE=1 "$REPO_ROOT/../engine/variant-nnue-pytorch/.venv/bin/python" \
    "$REPO_ROOT/scripts/strength_first_step4.py" full-precision \
    --contract "$CONTRACT" \
    --positions "$LOCAL_RUN/remote/features/positions.jsonl" \
    --checkpoint "$REPO_ROOT/out/anhoku-strength-first/step3-compact-scratch-seed-20260906-v1/checkpoints/best-validation-u00001920.pt" \
    --network "$REPO_ROOT/out/anhoku-strength-first/step3-compact-scratch-seed-20260906-v1/exports/selected-u00001920.nnue" \
    --output-dir "$LOCAL_RUN/full-precision"
fi

if [[ ! -e "$LOCAL_RUN/analysis" ]]; then
  PYTHONDONTWRITEBYTECODE=1 python3 "$REPO_ROOT/scripts/strength_first_step4.py" analyze \
    --contract "$CONTRACT" \
    --union "$LOCAL_RUN/union/union-plan.jsonl" \
    $(printf -- '--labels %q ' "$LOCAL_RUN"/remote/labels/shard-*/labels.jsonl) \
    --full-precision "$LOCAL_RUN/full-precision/scores.jsonl" \
    --output-dir "$LOCAL_RUN/analysis"
fi

echo "$(date --iso-8601=seconds) Step 4 complete"
