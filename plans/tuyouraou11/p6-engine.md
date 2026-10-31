# P6 ENGINE: micro SIMD

> 2026-10-31 更新: mini の追加が明示指定された。下記 10/30 の SIMD-only 記録は履歴。
> 現在の実装・検証・選択方針は [p6-mini.md](p6-mini.md) を正とする。
> 今回の実機検証はユーザーが明示的に対象外とした。
> 棋力本検証400局も完了: [p6-strength.md](p6-strength.md)。

実施: 2026-10-30。ユーザー指定は SIMD のみ。mini と package v2 は対象外。
開始 revision: `1638152`。P4 の実機検証保留はユーザー指示を継続する。

## 実装

- SFNN の全結合層を既存の `AffineKernel` に接続。native は AVX2/NEON の能力判定、Wasm はビルド時の SIMD128 を使う。
- `sfnn_kernels.rs` に accumulator の i16 加減算と積活性化の SIMD128 を実装。
  加減算は saturate せず wrap し、活性化は 0..254 に clamp、積の下位 16bit を unsigned で 9bit 右シフトする。
  `254²=64516` を符号付き 16bit として算術右シフトしない。
- dense 層は u8/i8 を i16 へ拡張してから乗算・i32 和にする既存カーネルを使用。
  飽和する中間和や relaxed SIMD、浮動小数近似を使わない。
- `sfnn-force-scalar` は検証用 oracle。モデル形式・識別子・load_sfnn ABI・探索条件は変更しない。
- Web へ配信するのは単一スレッドの scalar/SIMD128 の二成果物。SharedArrayBuffer は不要。

## 検証

- native の既存テスト 96 件、SFNN/既存 NNUE の整数カーネルを含めて合格。
- 実モデルの 236 参照局面と 2,048 手（捕獲 349、打ち 318、成り 92、玉移動 187、bucket 変更 258）が
  最適化版・scalar 固定版とも公式参照値/全再計算と完全一致。
- グローバル/セッション別モデル交換、失敗時 TT 保持、mate 範囲のテスト合格。
  グローバル TT を共有する ignored tests は `--test-threads=1` で実行する。
- Annan feature の SFNN 拒否と既存回帰合格。
- Wasm の SFNN 境界値/非整列アクセス/端数・wrap/clamp 比較と、既存 HalfKAv2 の SIMD dot 比較合格。
  旧 NNUE の外部実モデルは未配置。合格範囲は合成カーネル/通常回帰であり、外部ファイルを読んだとは扱わない。
- scalar/SIMD の実 Wasm で 236 参照局面、2,048 手、固定深さ 2 の 20 局面を Node/Chromium/Firefox で照合。
  progress 和・bucket・評価値に加え、固定深さの指し手・評価値・探索ノード数も一致。
  Firefox の全再計算照合は 120 秒を超えたため、検証ハーネスの上限を 600 秒へ拡大した。
  探索の movetime は変更していない。

## 再現

```bash
HAITAKA_SFNN_MODEL_DIR="$PWD/target/tuyouraou11/model" \
  cargo test -p haitaka_wasm --release --test sfnn_reference -- --ignored --nocapture
HAITAKA_SFNN_MODEL_DIR="$PWD/target/tuyouraou11/model" \
  cargo test -p haitaka_wasm --release --lib sfnn -- --ignored --test-threads=1
HAITAKA_SFNN_MODEL_DIR="$PWD/target/tuyouraou11/model" \
  cargo test -p haitaka_wasm --release --features sfnn-force-scalar \
  --test sfnn_reference -- --ignored
PATH="$HOME/.cache/.wasm-pack/wasm-bindgen-9daeccfa8e093372:$PATH" \
HAITAKA_CHROMIUM_PATH="$HOME/.cache/ms-playwright/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell" \
  node scripts/tuyouraou11-simd.mjs --build --browser
```

Node 24.12.0 / rustc 1.98.1 / wasm-bindgen 0.2.118。
検証成果物は `target/tuyouraou11/p6/`。配信版のハッシュと性能/採用判断は `p6-lead.md` を参照。
