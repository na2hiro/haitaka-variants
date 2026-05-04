#!/usr/bin/env sh
set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: sh scripts/prepare_anhoku_training_bundle.sh <config.toml>" >&2
  exit 2
fi

config="$1"

if [ ! -f "$config" ]; then
  echo "config not found: $config" >&2
  exit 1
fi

output_dir="$(awk '
  /^\[paths\]/ { in_paths = 1; next }
  /^\[/ { in_paths = 0 }
  in_paths && /^[[:space:]]*output_dir[[:space:]]*=/ {
    sub(/^[^=]*=[[:space:]]*/, "")
    gsub(/"/, "")
    print
    exit
  }
' "$config")"

bootstrap_nnue="$(awk '
  /^\[paths\]/ { in_paths = 1; next }
  /^\[/ { in_paths = 0 }
  in_paths && /^[[:space:]]*bootstrap_nnue[[:space:]]*=/ {
    sub(/^[^=]*=[[:space:]]*/, "")
    gsub(/"/, "")
    print
    exit
  }
' "$config")"

if [ -z "$output_dir" ]; then
  echo "paths.output_dir is missing in $config" >&2
  exit 1
fi

if [ -z "$bootstrap_nnue" ]; then
  bootstrap_name=""
else
  if [ ! -f "$bootstrap_nnue" ]; then
    echo "bootstrap NNUE not found: $bootstrap_nnue" >&2
    exit 1
  fi
  bootstrap_name="$(basename "$bootstrap_nnue")"
fi

datasets_dir="$output_dir/datasets"
if [ ! -d "$datasets_dir" ]; then
  echo "datasets directory not found: $datasets_dir" >&2
  echo "run generate-data for $config first" >&2
  exit 1
fi

stem="$(basename "$config" .toml)"
bundle="$(pwd)/anhoku-training-input-${stem}.tgz"
tmpdir="${TMPDIR:-/tmp}/anhoku-training-input-${stem}-$$"

rm -rf "$tmpdir"
mkdir -p "$tmpdir/haitaka"

cp "$config" "$tmpdir/haitaka/"
mkdir -p "$tmpdir/haitaka/$output_dir"
cp -R "$datasets_dir" "$tmpdir/haitaka/$output_dir/"
if [ -n "$bootstrap_name" ]; then
  cp "$bootstrap_nnue" "$tmpdir/$bootstrap_name"
fi

if [ -n "$bootstrap_name" ]; then
  tar -czf "$bundle" -C "$tmpdir" haitaka "$bootstrap_name"
else
  tar -czf "$bundle" -C "$tmpdir" haitaka
fi
rm -rf "$tmpdir"
echo "wrote $bundle"
