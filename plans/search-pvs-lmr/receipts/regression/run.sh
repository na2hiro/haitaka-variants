#!/usr/bin/env bash
# Build and regression-test feat/search-pvs-lmr against origin/main and the measured builds.
set -euo pipefail
branch=/home/na2hiro/proj/haitaka-search-pvs-lmr
mainwt=/home/na2hiro/proj/haitaka-main-1c87fe3
out=$branch/target/regress
mkdir -p "$out"
export CARGO_TARGET_DIR=$branch/target

build() { # dir name features
    local dir=$1 name=$2 features=$3 opts=()
    [[ -n "$features" ]] && opts=(--features "$features")
    (cd "$dir" && cargo build --release -p haitaka_cli "${opts[@]}" > "$out/build-$name.log" 2>&1) \
        || { tail -40 "$out/build-$name.log"; exit 1; }
    cp "$CARGO_TARGET_DIR/release/haitaka_cli" "$out/$name"
    echo "built $name"
}
build "$mainwt" main-standard ""
build "$mainwt" main-chushogi "chushogi"
build "$branch" standard ""
build "$branch" standard-pvs "search-pvs"
build "$branch" standard-lmr "search-lmr"
build "$branch" chushogi "chushogi"
build "$branch" chushogi-pvs "chushogi,search-variant-experiments,search-pvs"
build "$branch" chushogi-lmr "chushogi,search-variant-experiments,search-lmr"
# Representative ordinary rules: try win, forced capture, lion, no check rule (x2), plain.
ordinary="dobutsu toruichi shishio happokei zarikani narazu"
for rule in $ordinary; do
    build "$mainwt" "main-$rule" "$rule"
    build "$branch" "$rule" "$rule"
done

cd "$branch"
t() { # label cargo-args...
    local label=$1; shift
    cargo test --release "$@" > "$out/test-$label.log" 2>&1 || { tail -60 "$out/test-$label.log"; exit 1; }
    printf '%-40s %s\n' "$label" "$(grep '^test result:' "$out/test-$label.log" | awk '{p+=$4; f+=$6; i+=$8} END {print p" passed, "f" failed, "i" ignored"}')"
}
t wasm-standard -p haitaka_wasm --lib
t wasm-standard-pvs -p haitaka_wasm --lib --features search-pvs
t wasm-standard-lmr -p haitaka_wasm --lib --features search-lmr
for rule in annan nekoneko minishogi heian-9x9 fushiyo oute; do
    t "wasm-$rule" -p haitaka_wasm --lib --features "$rule"
    t "wasm-$rule-lmr" -p haitaka_wasm --lib --features "$rule,search-variant-experiments,search-lmr"
done
t haitaka-chushogi -p haitaka --lib --features chushogi
for rule in chushogi shoshogi kyoto tenjiku-dai taikyoku; do
    t "haitaka-$rule-lmr" -p haitaka --lib --features "$rule,search-variant-experiments,search-lmr"
done
for rule in $ordinary; do
    t "haitaka-$rule" -p haitaka --lib --features "$rule"
    t "haitaka-$rule-lmr" -p haitaka --lib --features "$rule,search-variant-experiments,search-lmr"
done
# Every ordinary rule must compile with each experiment build.
for rule in $(sed -n '/^# Ordinary movement/,/^$/p' haitaka/Cargo.toml | grep -o '^[a-z-]*'); do
    for exp in search-pvs search-lmr; do
        cargo check --release -p haitaka --features "$rule,search-variant-experiments,$exp" > "$out/check-$rule-$exp.log" 2>&1 \
            || { tail -40 "$out/check-$rule-$exp.log"; exit 1; }
    done
    echo "checked $rule"
done
t cli-chushogi -p haitaka_cli --features chushogi --test chushogi
t cli-chu-usi -p haitaka_cli --features chushogi,search-variant-experiments,search-lmr chu_usi
# The v3 model and setup.json were copied from the measuring machine; SHA256 matches chu-minase/manifest.json.
export HAITAKA_MINASE_V3_NNUE=$out/models/reference.nnue
export HAITAKA_MINASE_NNUE=/tmp/minase-v260/crates/minase/nets/nnue.bin
t minase-nnue -p haitaka --lib --features chushogi extended::chu::minase_nnue -- --include-ignored
t minase-nnue-lmr -p haitaka --lib --features chushogi,search-variant-experiments,search-lmr extended::chu::minase_nnue -- --include-ignored

python3 -I -B "$branch/plans/search-pvs-lmr/receipts/regression/compare.py" "$out" $ordinary

CARGO_TARGET_DIR=$branch/target/wasm wasm-pack build haitaka_wasm --mode no-install --target web --release \
    --no-opt --no-pack --out-dir "$out/wasm-chushogi" -- --features chushogi > "$out/build-wasm.log" 2>&1 \
    || { tail -40 "$out/build-wasm.log"; exit 1; }
for file in chushogi-reference-input.jsonl chushogi-reference-output.jsonl; do
    ln -sf /home/na2hiro/proj/haitaka-variants/target/$file "$out/$file"
done
ln -sf "$out/chushogi" "$out/haitaka-chushogi"
node scripts/verify-chushogi.mjs "$out" > "$out/verify-chushogi.log" 2>&1 || { tail -30 "$out/verify-chushogi.log"; exit 1; }
tail -5 "$out/verify-chushogi.log"
echo ALL-OK
