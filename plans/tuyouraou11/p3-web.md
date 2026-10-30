# P3 / WEB 結果

実施: 2026-10-30。P3 の終了条件を充足。初回導入全体と実 Wasm 統合は未完了。
仕様は [contract.md](contract.md)、操作手順は [shogitter 側記録](../../../shogitter-3/docs/plans/tuyouraou11-p3.md)。

## revision と変更の所在

- shogitter-3: branch `feat/tuyouraou11-p0`、HEAD `b9c857b9beae5ecf3873b28b6615673358ed7f3d` に対する未コミット差分。
- P1 資産の参照 checkout: haitaka-variants、branch `feat/tuyouraou11-p1`、HEAD `5d8dee3b635a98df3feee2b0dae4615f3d854537` と P1 作業中の未コミット provenance/モデル取得成果物。
- P0 契約の型・エラー・ルール制約は変更していない。ENGINE のソース・fixture・共有 status.md は変更していない。既存未追跡ファイルを保持。

shogitter 側の変更:

- `app/engines/enginePreparation.ts`、`wasmBindgenRuntime.ts`: 2 ファイル descriptor、遅延 prepare、HTTP/長さ/hash 照合、export/返り値検証、abort、取得上限、同時処理集約、失敗保持。legacy nnueUrl を維持。
- `app/bot/vendors/haitaka/index.ts`、全 variant の wrapper/型定義: ruleId に応じた prepare/dispose。SFNN は ruleId=0 だけ。perft からモデルを取得しない。
- `app/bot/{WorkerBot,WebWorkerBot,bot.worker}.ts`: import-ready と prepare-ready の分離、main の 60 秒上限、世代照合、再試行、stop/dispose、ルール変更と busy restart 後の再準備。既存 random/possibilityist/UserEngine 経路を維持。
- `app/components/shogi/SelfBotShogi.tsx`: 準備状態の購読、2 段階の準備表示、失敗説明と再試行、準備後の thinking/探索、キャンセルの抑制。
- `scripts/{sync-haitaka-wasm,haitaka-model}.mjs`: 明示的な engine/model ソース指定、実資産検証とコピー、版固定資産の上書き拒否、wrapper テンプレート更新、`--wrappers-only`/`--verify-model`。旧 NNUE の暗黙コピーを廃止。
- `server.ts`、`package.json`、`.github/workflows/deploy.yml`: bin/Wasm の MIME、実資産の CI 検証。既存の 1 年 immutable 配信を利用。
- `standard/model.json`、`public/engines/haitaka/tuyouraou11micro/<id>/`、関連テストと WEB 記録。

## 実資産

モデル ID: `tuyouraou11micro-20261001-epoch0010-7f9545dbf73b-a2d18f056463`。
arch: `SFNN_ka2_2048_8_64_progress8`、FV_SCALE=24。

| ファイル     |     bytes | SHA-256                                                          |
| ------------ | --------: | ---------------------------------------------------------------- |
| nn.bin       | 3,864,610 | 7f9545dbf73b87acf312146daabf59c4f17d0845c87d587fa19caed51707dddb |
| progress.bin | 1,003,104 | a2d18f0564639c945b5d6469e9b437cd64a526ad4fbcef02ab1d8f547d08e5f1 |

P1 の `target/tuyouraou11/model/` から実体・provenance.json・LICENSE.txt を検証して配置。
provenance/license は `plans/tuyouraou11/{provenance.json,MODEL-LICENSE.txt}` とも同一であることを cmp で確認した。
archive SHA-256 は `d1326e9a620c30d40ae553259dd0b6828d2f4e9fca3b6ab9440566579ab0e8c2`、
参照実装 commit は `7d9743646f1006ff6d84da45d6d6160048a5a416`（P1 provenance から転記）。
配布記事による MIT 指定と tag.json の作者情報を P1 のライセンス文面のまま保持し、著作権年を創作していない。
画像・GPL 参照エンジン・旧 shogi.nnue は配信資産に含めていない。
P1 の scalar/fixture 一致の判定は ENGINE の責任であり、WEB の資産検証から合格と推定していない。

## 実行コマンドと結果

以下の作業ディレクトリは `/home/na2hiro/proj/shogitter-3`。WEB のため Cargo feature 指定なし。

```bash
HAITAKA_ENGINE_DIR=../haitaka-variants \
HAITAKA_MODEL_DIR=../haitaka-variants/target/tuyouraou11/model \
node scripts/sync-haitaka-wasm.mjs --wrappers-only

pnpm bot:verify:haitaka-model
pnpm exec vitest run app/bot app/engines/wasmBindgenRuntime.test.ts app/engines/haitakaModelSync.test.ts app/components/shogi/SelfBotShogi.test.ts
pnpm typecheck:app
git diff --check
```

結果: 同期・配置済み実資産検証 PASS。Vitest 11 files / 80 tests PASS、skip なし。型検査・差分検査 PASS。
HTTP 失敗、片方失敗、サイズ/hash 不一致、export 不足、load 失敗、arch 返り値不一致、取得 timeout、同時 prepare、
失敗保持、明示再試行、準備中 stop/dispose、遅延応答破棄、ルール変更、busy restart、movetime 維持、準備 UI、
perft/変則ルールの非取得、同期テンプレート再生成、実モデル資産の完全性を検証した。

変更したアプリファイルの静的検査も PASS:

```bash
pnpm exec eslint app/bot/WorkerBot.ts app/bot/WebWorkerBot.ts app/bot/WebWorkerBot.test.ts app/bot/bot.worker.ts app/bot/bot.worker.test.ts app/bot/vendors/haitaka/index.ts app/bot/vendors/haitaka/index.test.ts app/components/shogi/SelfBotShogi.tsx app/components/shogi/SelfBotShogi.test.ts app/engines/enginePreparation.ts app/engines/wasmBindgenRuntime.ts app/engines/wasmBindgenRuntime.test.ts app/engines/haitakaModelSync.test.ts --max-warnings=0
```

Prettier は変更ファイルに限定して適用し、最終の check で確認。

## P4 / LEAD への引継ぎ

- status.md の P3 は本記録を参照して更新すること（共有進捗の更新は LEAD 担当）。P1 最終引継ぎでも上記資産 snapshot の ID/hash/provenance を照合する。
- **P2 実 Wasm は未接続。** 既存同梱 Wasm は load_sfnn を export しない。現在の通常将棋 AI は export 不足のエラーを表示し、従来評価へ自動フォールバックしない。
- P2 完了後、`HAITAKA_ENGINE_DIR=../haitaka-variants pnpm bot:sync:haitaka` で実 Wasm を再生成する。P3 では Cargo build や P2 のソース変更を実施していない。
- P4 の実 Wasm E2E、モデル使用の証拠、Network 記録、実機性能、対局は未実施。合法手や評価一致をモック結果から主張しない。
- 配信データは bin 合計 4,867,714 bytes。既存 Express 配信は 1 年 immutable。JS 受信 buffer、Wasm コピー・重み展開、progress Q16、TT/accumulator のメモリ実測と cold/warm の許容値は P4 で確定する。
- 公開デプロイ・dev server 起動・コミットは実施していない。

P4追記: 上記はP3実施時点の記録。P3はshogitter `5d62198d` / 本記録 `b541db5` にコミット済み。
P4で実Wasmを同期してローカル統合・対局検証を完了した。最新結果と実機保留は [p4-lead.md](p4-lead.md) を参照。
