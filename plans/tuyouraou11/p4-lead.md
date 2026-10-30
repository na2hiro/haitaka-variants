# P4 / LEAD 統合検証

実施: 2026-10-30。状態: ローカル統合・対局検証完了、実機検証はユーザー指示により保留。
P4/初回導入全体の完了・公開採用はまだ宣言しない。

## revision と成果物

- WEB P3: shogitter-3 `5d62198d`（feat/tuyouraou11-p0）。
- 共通 P3 記録: haitaka-variants `b541db5`（feat/tuyouraou11-p2）。
- ENGINE P2: `ebf6a08`。P4 でエンジン評価/探索の実装変更はなし。
- P4 WEB 実装: shogitter-3 `057e84f4`。ENGINE 側の検証例・集計スクリプト・結果は本コミットに収録。無関係な未追跡ファイルを保持。
- `p4-artifacts.json`: 正規同期した 13 variant、native harness、開始局面の bytes/hash。
- `performance-budget.md`: SFNN 性能/対局結果を採否判断する前に desktop 予算を固定。
- `p4-e2e.json`: 両 browser の実画面テスト、prepare のモデル ID/arch、Worker 通信/Network、trace hash。
- `p4-native-smoke.json`: 同じ対局用 binary が handcrafted / TuyouraOu11 micro を切り替えて応答する USI 記録。
- 生の性能計測/棋譜/対局 report: `p4-results/` に保存。実行ログは `target/tuyouraou11/p4*`。実行コマンドは以下。

モデル ID/hash/ライセンスは P1/P3 から変更なし。WEB 配信の provenance と LICENSE を P1 正本へ cmp して一致を再確認した。
同一 origin 配信の bin は application/octet-stream、Cache-Control は public, max-age=31536000, immutable。

## 統合変更

P3 で追加した Worker 内の `~/engines/enginePreparation.js` import は、Vitest/型検査では通るが、
本番の Worker バンドルでは解決されなかった。bot.worker.ts/vendor index.ts を相対 import へ修正して本番 build を確認。
その他の製品挙動・公開契約の変更はなし。同期スクリプトから P2 checkout を明示して全 13 variant を再生成した。
生成物の手動 rename/差替えはしていない。

WEB の追加検証資産:

- `tests/tuyouraou11.spec.ts`、`playwright.tuyouraou11.config.ts`: 実 Wasm の準備 → 着手、2 段階準備表示、通信失敗/再試行、トップページの非取得、同じ standard wrapper 内のルール切替、perft、stop 後の新 Worker。
- `scripts/tuyouraou11-browser.mjs`: 実 runtime/実 Wasm の cold/warm・探索・メモリ・main 応答性の固定手順。
- ENGINE 例 `sfnn_openings.rs`: seed=1 の LCG64 で 50 開始局面を生成。
- ENGINE 例 `sfnn_verify_moves.rs`: ブラウザ探索結果を native の合法手生成で独立検証。
- `scripts/tuyouraou11-p4-summary.py`: 開始局面ペア単位・時間別層化の bootstrap、性能集計。

## 検証環境

Ubuntu 26.04 / kernel 7.0.0-31-generic / Core i7-6700、CPU governor=powersave。
Chromium 153.0.8010.12、Firefox 142.0.1（Playwright の Ubuntu 24.04 互換配布）。headless であり実 Safari/iOS とは別。
Node 26 で Playwright の trace 作成・Firefox 展開が停止したため、Node 24.12.0 へ切替。
Firefox アーカイブはダウンロード後 Python で同じ公式 zip を展開し、Firefox の起動を確認した。

実画面は専用 staging サーバー、port=3099、SQLite DB は`.tmp/p4/dev.db`へのコピー。
ユーザーの`pnpm dev`は起動せず、既存サーバーの終了もしていない。公開デプロイなし。
広告/解析の外部アクセスは E2E だけ遮断する。Wasm/モデル/探索はモックではなく実体を使用。
HTTP 503 テストだけ progress 配信を意図的に失敗させて再試行を検証する。

## 実行コマンド

WEB 作業ディレクトリ: `/home/na2hiro/proj/shogitter-3`。

```bash
HAITAKA_ENGINE_DIR=../haitaka-variants \
PATH=/home/na2hiro/.cache/.wasm-pack/wasm-bindgen-9daeccfa8e093372:$PATH \
pnpm bot:sync:haitaka

NODE_ENV=staging pnpm build
# 全variant同期完了後、最終buildも実施。standard WasmのSHA-256一致を確認。

NODE_ENV=staging HOST=http://127.0.0.1:3099 PORT=3099 \
DATABASE_URL=file:/home/na2hiro/proj/shogitter-3/.tmp/p4/dev.db AD_TEST=1 \
node build/server.js

BASE_URL=http://127.0.0.1:3099 SKIP_WEB_SERVER=1 \
HAITAKA_CHROMIUM_PATH=/home/na2hiro/.cache/ms-playwright/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell \
pnpm exec playwright test --config playwright.tuyouraou11.config.ts

pnpm exec vitest run app/bot app/engines/wasmBindgenRuntime.test.ts app/engines/haitakaModelSync.test.ts app/components/shogi/SelfBotShogi.test.ts
pnpm typecheck:app
pnpm bot:verify:haitaka-model
pnpm exec eslint app/bot/bot.worker.ts app/bot/vendors/haitaka/index.ts --max-warnings=0
```

実行時は Node 24 の PATH を使用。性能計測は`taskset -c 0,4`、E2E は`taskset -c 1,5`、最終 build は`taskset -c 2,6`。
専用無負荷マシンではないことを記録。性能の生データは同じ固定 CPU/サーバー条件で比較する。

```bash
HAITAKA_CHROMIUM_PATH=/home/na2hiro/.cache/ms-playwright/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell \
taskset -c 0,4 node scripts/tuyouraou11-browser.mjs baseline chromium
taskset -c 0,4 node scripts/tuyouraou11-browser.mjs baseline firefox
# performance-budget.mdの基準はbaseline計測中に凍結済み。SFNNは同じ条件。
HAITAKA_CHROMIUM_PATH=/home/na2hiro/.cache/ms-playwright/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell \
taskset -c 0,4 node scripts/tuyouraou11-browser.mjs sfnn chromium
taskset -c 0,4 node scripts/tuyouraou11-browser.mjs sfnn firefox
```

ENGINE 作業ディレクトリ: `/home/na2hiro/proj/haitaka-variants`。

```bash
cargo build -p haitaka_wasm --release --example sfnn_openings --example sfnn_usi --example sfnn_verify_moves
cargo build -p haitaka_cli --release
target/release/examples/sfnn_openings > plans/tuyouraou11/p4-openings.sfen

# 同じコマンドでMS=200とMS=1000を各100局、合計200局実行
MS=200
taskset -c 1,2,3,5,6,7 target/release/haitaka_cli self-play \
  --games 100 --threads 3 \
  --a-engine /home/na2hiro/proj/haitaka-variants/target/release/examples/sfnn_usi \
  --a-engine-arg=--model-dir \
  --a-engine-arg=/home/na2hiro/proj/haitaka-variants/target/tuyouraou11/model \
  --b-engine /home/na2hiro/proj/haitaka-variants/target/release/examples/sfnn_usi \
  --movetime-ms "$MS" --openings plans/tuyouraou11/p4-openings.sfen \
  --opening-order sequential --seed 1 --max-plies 256 \
  --report-dir "target/tuyouraou11/p4/match-$MS"

python3 scripts/tuyouraou11-p4-summary.py browser
python3 scripts/tuyouraou11-p4-summary.py games
# 保存済みの生データだけから再集計・検証する場合
python3 scripts/tuyouraou11-p4-summary.py browser --records-dir plans/tuyouraou11/p4-results
python3 scripts/tuyouraou11-p4-summary.py games --records-dir plans/tuyouraou11/p4-results
```

各外部 engine は同一 binary/UsiSession/search を使用。違いは A の load_sfnn だけ。
movetime の深さ上限は双方 64、TT は双方 64MiB、探索に乱数指定はない。seed=1 は開始局面生成・bootstrap に固定。
対局は native harness 条件の比較であり、browser の global 探索で同じ勝率が出るとは主張しない。
開始局面 50 個（LCG64 seed=1、合法 8 手）の SHA-256 は `ce6653c8790157159a44f30e375984451282318e2dce6870de5a65d30e75696c`。
各局面を先後交換し、各時間条件で 100 局。CLI が全手を try_play で検証し、千日手/連続王手の履歴判定を用いる。
crash/不正着手/非終局での手欠損は harness error として終了する。

## 結果（確定分）

- 正規同期/モデル検証: PASS。standard の生成 Wasm と最終 build 配信資産の SHA-256 一致。
- 本番 build: Worker import 修正後 PASS。最終全 variant 同期後も PASS。
- 実画面 E2E: Chromium/Firefox の各 6 件、計 12 件 PASS、skip なし。
- 関連 Vitest: 11 files / 80 tests PASS、skip なし。
- 型検査/変更アプリの ESLint: PASS。
- 従来評価のブラウザ 360 探索: native 合法手検証 PASS。
- desktop 性能: 従来評価/SFNN×2browser、各 20 準備・180 探索（合計 80 準備/720 探索）PASS。
  全 720 応答を native 合法手生成で照合し、全て合法。凍結した準備/メモリ/時間超過の数値上限を満たす。
- 全 200 局比較: PASS。各時間条件 100 局、50 開始局面の先後交換・全 gameIndex・全 moves/plies・終局理由を照合。重複なし。

対局の確定結果は `p4-games-summary.json`。全 200 局が通常の合法手なしによる終局であり、
crash/不正着手/非終局での手欠損/時間切れによる harness error は 0 件。対局 command は exit=0 で終了した。

| movetime | micro 勝/分/敗 | スコア率 |     95%区間 |
| -------- | -------------: | -------: | ----------: |
| 200ms    |     92 / 0 / 8 |    92.0% | 87.0〜97.0% |
| 1000ms   |     96 / 0 / 4 |    96.0% | 92.0〜99.0% |
| 総合     |   188 / 0 / 12 |    94.0% | 90.5〜97.0% |

契約の退行判定（総合区間の上端<50%）には該当しない。
この固定条件の native 比較結果であり、全局面・全端末の棋力向上/非劣性を証明したとは扱わない。
保存した生データから同じ集計を再実行して一致を確認した。

性能詳細は `p4-browser-summary.json`。準備の往復時間（main から Worker へ prepare を送って ready を受けるまで）と、
Worker 内の Wasm/model 各工程を分離して記録した。実アプリの入力/ready 経路は別途 E2E で確認。

| SFNN     | cold 準備中央値/最大 | warm 準備中央値/最大 | Wasm memory 最大 | engine 時間超過最大 | main 往復超過最大 |
| -------- | -------------------: | -------------------: | ---------------: | ------------------: | ----------------: |
| Chromium |    257.35 / 280.70ms |    235.15 / 253.70ms |  55,771,136bytes |              -0.5ms |             5.7ms |
| Firefox  |          495 / 653ms |        447.5 / 522ms |  55,771,136bytes |                 1ms |               3ms |

これは loopback HTTP・headless・CPU 固定の runtime 測定で、携帯回線や全アプリ初回表示の時間ではない。
JS の明示的モデル受信量は 4,867,714bytes。Wasm memory は非縮小なので処理後の byteLength にもピークが残る。
総 process/JS heap のピークは未取得。
Firefox の固定 corpus では SFNN の到達深さ中央値が 200ms で 0、1000ms で 1、3000ms で 2 だった。
時間上限と合法手応答は満たすが、短い思考時間の探索品質には制約がある。NPS/深さは json へ全条件を残し、
メモリ/時間予算への適合だけから全端末の棋力や快適性が十分と結論しない。SIMD 等の P6 を自動で開始しない。

bootstrap 実装は全勝・全 pair 引分・時間層を全勝/全敗に分ける合成ケースで、それぞれ[1,1]、[.5,.5]、[.5,.5]となることを確認。
実対局は 50 ペア ×2 時間を単位に 10000 回、seed=1、percentile 2.5/97.5%。
標本の局面ペア構造・色交換・固定 suite との一致も集計スクリプトが検証する。

途中試行の不成功は、Worker import の本番 build エラー、Node 26 の trace 停止、テストロケーター誤り、
設定 merge で未導入 WebKit を余分に列挙したこと。原因を修正し、最終の実 Chromium/Firefox の 12 件は skip せず実行。
未導入 WebKit を Safari/iOS の検証と主張しない。

## 保留項目と採否

ユーザー指示により実機検証を後回しとした。iOS Safari 実機/Android Chrome 実機の型番・版、
cold/warm・メモリ・探索・UI 確認、端末ごとの従来評価測定と予算固定が残る。
desktop 結果を実機へ代用しない。総 process/JS heap のピークは今回取得しておらず、0 として報告しない。
desktop は headless であり、製品版 Chrome/Firefox の headful 確認も未実施。

本記録で P4/初回導入全体の完了や公開採用を宣言しない。公開デプロイは実施しない。
今回進められるローカル作業は完了。検証用 staging サーバーと測定 browser、対局 process は全て終了済み。
P4 の実装と検証記録をユーザー指示によりコミット。実機検証は引き続き保留。
