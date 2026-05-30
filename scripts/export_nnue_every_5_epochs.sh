#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/export_nnue_every_5_epochs.sh [options]

Exports checkpoints whose Lightning epoch is 4, 9, 14, ... to separate .nnue
files. By default this targets the Anhoku v0.4 run.

Options:
  --config PATH          Training config. Default: haitaka_learn.anhoku-v0.4.toml
  --feature NAME         Cargo feature to pass to haitaka_learn. Default: anhoku
  --checkpoint-dir DIR   Checkpoint directory. Default:
                         <output_dir>/logs/lightning_logs/version_0/checkpoints
  --output-dir DIR       Export destination. Default:
                         <output_dir>/artifacts/epoch-exports
  --start-epoch N        First epoch to export. Default: 4
  --end-epoch N          Last epoch to export, inclusive. Default: no limit
  --step N               Epoch interval. Default: 5
  --release              Use cargo run --release.
  -h, --help             Show this help.
USAGE
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
config="haitaka_learn.anhoku-v0.4.toml"
feature="anhoku"
checkpoint_dir=""
output_dir=""
start_epoch=4
end_epoch=""
step=5
release=0

while (($#)); do
  case "$1" in
    --config)
      config="$2"
      shift 2
      ;;
    --feature)
      feature="$2"
      shift 2
      ;;
    --checkpoint-dir)
      checkpoint_dir="$2"
      shift 2
      ;;
    --output-dir)
      output_dir="$2"
      shift 2
      ;;
    --start-epoch)
      start_epoch="$2"
      shift 2
      ;;
    --end-epoch)
      end_epoch="$2"
      shift 2
      ;;
    --step)
      step="$2"
      shift 2
      ;;
    --release)
      release=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

cd "$repo_root"

config_dir="$(cd "$(dirname "$config")" && pwd)"

if [[ ! "$start_epoch" =~ ^[0-9]+$ ]] || [[ ! "$step" =~ ^[1-9][0-9]*$ ]]; then
  echo "--start-epoch must be non-negative and --step must be positive" >&2
  exit 2
fi
if [[ -n "$end_epoch" ]] && [[ ! "$end_epoch" =~ ^[0-9]+$ ]]; then
  echo "--end-epoch must be non-negative" >&2
  exit 2
fi
if [[ -n "$end_epoch" ]] && ((end_epoch < start_epoch)); then
  echo "--end-epoch must be greater than or equal to --start-epoch" >&2
  exit 2
fi

toml_string() {
  local section="$1"
  local key="$2"
  awk -v section="$section" -v key="$key" '
    $0 ~ "^[[:space:]]*\\[" section "\\][[:space:]]*$" { in_section = 1; next }
    $0 ~ "^[[:space:]]*\\[" { in_section = 0 }
    in_section && $0 ~ "^[[:space:]]*" key "[[:space:]]*=" {
      value = $0
      sub(/^[^=]*=[[:space:]]*/, "", value)
      sub(/[[:space:]]*(#.*)?$/, "", value)
      gsub(/^"|"$/, "", value)
      print value
      exit
    }
  ' "$config"
}

output_root="$(toml_string paths output_dir)"
export_name="$(toml_string export output_name)"

if [[ -z "$output_root" ]]; then
  echo "could not read paths.output_dir from $config" >&2
  exit 1
fi
if [[ -z "$export_name" ]]; then
  echo "could not read export.output_name from $config" >&2
  exit 1
fi
if [[ "$output_root" != /* ]]; then
  output_root="$config_dir/$output_root"
fi

checkpoint_dir="${checkpoint_dir:-$output_root/logs/lightning_logs/version_0/checkpoints}"
output_dir="${output_dir:-$output_root/artifacts/epoch-exports}"
artifact_dir="$output_root/artifacts"
exported_nnue="$artifact_dir/$export_name"
exported_json="$artifact_dir/export.json"
export_stem="${export_name%.nnue}"

if [[ ! -d "$checkpoint_dir" ]]; then
  echo "checkpoint directory does not exist: $checkpoint_dir" >&2
  exit 1
fi

mkdir -p "$output_dir"

cargo_args=(run -p haitaka_learn)
if [[ "$release" -eq 1 ]]; then
  cargo_args+=(--release)
fi
if [[ -n "$feature" ]]; then
  cargo_args+=(--features "$feature")
fi
cargo_args+=(-- export --config "$config")

mapfile -t checkpoints < <(
  find "$checkpoint_dir" -maxdepth 1 -type f -name '*.ckpt' -printf '%f\t%p\n' |
    awk -F '\t' -v start="$start_epoch" -v end="$end_epoch" -v step="$step" '
      {
        name = $1
        if (name !~ /epoch=[0-9]+/) {
          next
        }
        sub(/^.*epoch=/, "", name)
        sub(/[^0-9].*$/, "", name)
        epoch = name + 0
        if (end != "" && epoch > end) {
          next
        }
        if (epoch >= start && ((epoch - start) % step) == 0) {
          printf "%08d\t%s\n", epoch, $2
        }
      }
    ' |
    sort -n
)

if [[ "${#checkpoints[@]}" -eq 0 ]]; then
  echo "no matching checkpoints found under $checkpoint_dir" >&2
  exit 1
fi

for entry in "${checkpoints[@]}"; do
  epoch_padded="${entry%%$'\t'*}"
  checkpoint="${entry#*$'\t'}"
  epoch="$((10#$epoch_padded))"
  epoch_label="$(printf '%03d' "$epoch")"
  target_nnue="$output_dir/${export_stem}-epoch-${epoch_label}.nnue"
  target_json="$output_dir/export-epoch-${epoch_label}.json"

  echo "exporting epoch $epoch from $checkpoint"
  cargo "${cargo_args[@]}" --checkpoint "$checkpoint"

  if [[ ! -f "$exported_nnue" ]]; then
    echo "expected export was not written: $exported_nnue" >&2
    exit 1
  fi

  mv "$exported_nnue" "$target_nnue"
  if [[ -f "$exported_json" ]]; then
    mv "$exported_json" "$target_json"
  fi

  echo "wrote $target_nnue"
done
