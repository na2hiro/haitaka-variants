#!/usr/bin/env bash
set -euo pipefail
root=/home/na2hiro/proj/haitaka-variants
impact="$root/target/honshogi-historical-impact"
source_dir="$impact/source"
binder=/home/na2hiro/.cache/.wasm-pack/wasm-bindgen-9daeccfa8e093372/wasm-bindgen
for side in before after; do
 revision=9d90ef3
 if [[ "$side" == after ]]; then revision=58d5802; fi
 git -C "$source_dir" switch --detach "$revision"
 cp "$root/Cargo.lock" "$source_dir/Cargo.lock"
 cp "$root/Cargo.lock" "$impact/native-bench/Cargo.lock"
 mkdir -p "$impact/$side"
 cargo build --offline --release --manifest-path "$impact/native-bench/Cargo.toml" --target-dir "$impact/build" > "$impact/$side/native-build.log" 2>&1
 cp "$impact/build/release/honshogi-historical-impact" "$impact/$side/native"
 cp "$impact/native-bench/Cargo.lock" "$impact/$side/native-effective-Cargo.lock"
 for variant in scalar simd; do
  feature=-simd128
  if [[ "$variant" == simd ]]; then feature=+simd128; fi
  mkdir -p "$impact/$side/$variant"
  RUSTFLAGS="-C target-feature=$feature" cargo build --offline --release --target wasm32-unknown-unknown --manifest-path "$source_dir/Cargo.toml" --target-dir "$impact/build" -p haitaka_wasm --no-default-features > "$impact/$side/$variant/build.log" 2>&1
  cp "$impact/build/wasm32-unknown-unknown/release/haitaka_wasm.wasm" "$impact/$side/$variant/raw.wasm"
  "$binder" --target web --out-dir "$impact/$side/$variant" --out-name haitaka_wasm "$impact/$side/$variant/raw.wasm"
  echo "$side $variant WASM ready"
 done
 cp "$source_dir/Cargo.lock" "$impact/$side/workspace-effective-Cargo.lock"
 cargo tree --offline --manifest-path "$source_dir/Cargo.toml" -p haitaka_wasm --no-default-features --target wasm32-unknown-unknown --edges normal,build --prefix none > "$impact/$side/wasm-dependencies.txt" 2> "$impact/$side/tree.log"
 git -C "$source_dir" rev-parse HEAD > "$impact/$side/source-commit.txt"
 echo "$side build completed"
done
