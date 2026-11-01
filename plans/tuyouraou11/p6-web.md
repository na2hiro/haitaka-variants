# P6 WEB: SIMD 配信と scalar 経路

> 2026-10-31 更新: mini の追加が明示指定された。下記 10/30 の SIMD-only 記録は履歴。
> 現在の実装・検証・選択方針は [p6-mini.md](p6-mini.md) を正とする。
> 今回の実機検証はユーザーが明示的に対象外とした。
> 棋力本検証400局も完了: [p6-strength.md](p6-strength.md)。

実施: 2026-10-30。WEB 開始 revision: `4fd0b35c`。

- standard wrapper で小さな実 Wasm SIMD モジュールを `WebAssembly.validate` し、
  対応時だけ `generated-simd/`、非対応/判定例外時は `generated/` を dynamic import する。
- 正規同期スクリプトは standard に対し `-C target-feature=-simd128` と `+simd128` の両成果物を生成する。
  `--standard-only` で対象を限定でき、他の変則ビルドを不要に変更しない。
- 既存のモデル descriptor・hash・対応ルール・Worker 世代管理を維持。モデル失敗を従来評価へ切り替えない。
  SIMD 判定後の取得/初期化エラーも既存の説明・明示再試行へ渡す。
- 本番ビルドを隔離したコピーで作成し、開発サーバー 3002 番を保持。
  実配信 Wasm と生成ファイルの SHA-256 を比較する画面 E2E を追加した。
- capability 判定だけを無効化した試験で、モデルとエンジンをモックせず scalar の合法応答を確認。
  Chromium の非 secure LAN 開発環境では bootstrap 書換方式で起動できなかったため、開発時は能力判定モジュールだけを差し替える。
  Firefox と本番 bundle では Worker bootstrap へ判定無効化を挿入する。これはテスト側の制御で製品の特例ではない。
- native/ブラウザの測定は P4 corpus を読むだけにし、P4 生記録・局面・予算を上書きしない。

```bash
# shogitter-3
PATH="$HOME/.cache/.wasm-pack/wasm-bindgen-9daeccfa8e093372:$PATH" \
HAITAKA_ENGINE_DIR=../haitaka-variants node scripts/sync-haitaka-wasm.mjs --standard-only
BASE_URL=http://localhost:3098 SKIP_WEB_SERVER=1 \
  pnpm exec playwright test --config playwright.tuyouraou11.config.ts --trace off
# P4と同じCPU/時間条件。4組を直列測定する。
taskset -c 0,4 node scripts/tuyouraou11-browser.mjs sfnn chromium scalar
taskset -c 0,4 node scripts/tuyouraou11-browser.mjs sfnn chromium simd
taskset -c 0,4 node scripts/tuyouraou11-browser.mjs sfnn firefox scalar
taskset -c 0,4 node scripts/tuyouraou11-browser.mjs sfnn firefox simd
```

最終検証件数と採用判断は `p6-lead.md`。公開デプロイは行わない。
