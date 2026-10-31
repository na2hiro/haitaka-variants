# P6 mini 追加: 実装・検証・選択方針

実施: 2026-10-31。ユーザーが mini 追加と両モデルの選択を明示指定。
実機検証は今回「しない」と指定したため対象外。P4 の実機性能を検証済みとは扱わない。
10/30 の SIMD-only 記録は履歴として保持し、この追補を現在の判断とする。

## 提供方針

「設定」→「モデル」で **つようら王11micro / つようら王11mini / 選択肢評価** を選べる。
ルールページとノートの両方で使え、選択は同じタブの sessionStorage に保持する。
micro を既定に維持し、mini を選択した場合のみ mini のパラメーターを取得する。
一方を削除して勝者だけを残す方針にはしない。

対応する 18 ルールは micro と共通: 平手、駒落ち 13 種、歩なし、将棋120系の 3 種。
対象外ルールでは選択欄を表示せず選択肢評価を使う。
モデル切替では Worker を破棄して新規準備する。具体的な modelId/arch を ready で返し、
破損・通信失敗・未配置 mini を別モデルで隠さず、既存の明示再試行へ渡す。
待機表示は従来どおり実際の bot 手番で待つ場合にだけアバター付近へ表示する。

## モデルの出所と ENGINE

- [mini-provenance.json](mini-provenance.json)、[MINI-MODEL-LICENSE.txt](MINI-MODEL-LICENSE.txt)。
- 元記事の MIT 指定、配布 archive の作者情報を保持。archive に独立 LICENSE はなく、banner は配信しない。
- archive SHA-256: `bd5cd719e000ee3e8c4f158ecd7b56913655569fa73911bd95209f55bbb772ea`。
- mini nn.bin: 7,711,372bytes、SHA-256 `45323c65135da65d6c2c364eea93a5af6a285c27ebfb07eab4afbe737ef74508`。
- progress.bin: 1,003,104bytes。micro と同じ SHA-256 `a2d18f0564639c945b5d6469e9b437cd64a526ad4fbcef02ab1d8f547d08e5f1`。
- 合計 8,714,476bytes（約 8.31MiB）、micro は 4,867,714bytes（約 4.64MiB）。モデル版固定 URL へ配信する。
- 参照: YaneuraOu commit `7d9743646f1006ff6d84da45d6d6160048a5a416`、`TARGET_CPU=OTHER`、FV_SCALE=24。
  mini 用 edition を clean build して 236 局面を生成。旧 micro 参照値は上書きしない。
  GPL 参照ソース/実行ファイルは ignored target/ にのみ置き、製品へリンク/同梱しない。

受理 arch は micro 2048 と mini 4096 の完全識別子に限定。
モデル自身の幅に従って特徴変換・差分更新・最初の全結合層を処理し、
ヘッダー記述と選択 arch が違えば拒否する。progress8 と後段 8→16→64→1、整数演算は共通。
探索中の変換用バッファは最大 4096 要素の固定領域で、評価ごとの追加 heap 確保は行わない。
scalar/SIMD 選択はモデル選択と独立し、両方で同じファイルを読める。

## 検証

結果は [p6-mini-results/](p6-mini-results/) に保存する。P4 と 10/30 SIMD の記録は保持する。

- mini: 236 参照局面の progress 和/bucket/評価値が公式実装と完全一致。
  2,048 手の差分更新・兄弟局面が全再計算と一致。固定深さ探索でも差分/全再計算が一致。
  破損・末尾/不足・未知 arch・既知 arch とモデル幅の取り違えを拒否する。
- micro: 同じ 236 局面・2,048 手と探索を再検証。通常 Rust 96 テスト、ignored runtime/TT/評価範囲 3 テスト合格。
  micro→mini→micro の session/global 読み込み、返却 arch、TT 初期化と合法手も確認。
  Annan feature では両 arch を拒否する既存 3 テストが合格。
- Node: mini と micro それぞれで実 Wasm scalar/SIMD の 236 局面・2,048 手・20 固定深さ探索が完全一致。
  mini は Chromium/Firefox の実 Worker でも両 backend が全項目一致。
  [ブラウザ一致記録](p6-mini-results/browser-comparison.json)。
- 拡張ルール: 実配信 SIMD で 166 初形（駒落ち13種、歩なし、将棋120全初形、他2種各16初形）から480合法手。
  全初形が公式 mini 参照の progress/評価値と完全一致。
- Web: 関連ユニット91件、型検査、ESLint 合格。両モデルの実公開ファイル bytes/hash/descriptor を省略なく照合。
  LAN HTTP の開発環境で Chromium/Firefox × scalar/SIMD の mini 選択・着手・micro への切替を確認。
- 本番 build と Chromium/Firefox 各18件、計36件の画面 E2E 合格。mini の両 backend の実配信 Wasm hash も照合。
  ルール→ノートへの設定保持、選択肢評価への切替、読み込み中表示、HTTP 失敗/明示再試行、
  各拡張ルールでの着手、ルール切替、perft、待ったを確認。
  [E2E 記録](p6-mini-results/e2e.json)。本番試験用サーバー/隔離コピーは終了し、dev は保持。

## 性能と比較対局

P4 と同じ CPU/core、同じ固定20局面と200/1000/3000ms、各3反復を使う。
mini Chromium→mini Firefox→micro Chromium→micro Firefox の順で直列測定する。
80準備/720探索。cold は新 context、warm は同 context の新 Worker。各組の探索は専用 Worker で実行する。
同じホストで build/E2E/参照一致/対局も並行した（対局は taskset 2,3,6,7）。無負荷の専用ホスト測定とは扱わない。

| 持ち時間 | Chromium mini/micro NPS比 | Firefox mini/micro NPS比 |
|---|---:|---:|
| 200ms | 0.448 | 0.656 |
| 1000ms | 0.415 | 0.541 |
| 3000ms | 0.389 | 0.575 |

同一局面・時間・反復のNPS比の中央値（各60組）。到達深さ中央値は200/1000/3000ms順で、
Chromium micro=2/3/4、mini=2/3/3、Firefox micro=1/2/3、mini=1/1/2。
準備往復の最大は chromium: micro 252.8ms / mini 510.9ms、firefox: micro 583.0ms / mini 1179.0ms。
Wasm memory最大はmicro 55,771,136bytes（53.2MiB）、mini 60,620,800bytes（57.8MiB）。
engine時間超過最大 3.0ms、main往復超過最大 9.7ms。
全720応答が独立した合法手生成器で合法、main timerも全件進み、既存予算を通過した。

micro/miniの4測定は同一の配信SIMD Wasm hashを持つことを集計時にも検査した。
miniは計算量/メモリが増える。既定をmicroのままにする根拠とし、強さの優劣をNPSから推定しない。
[全生データと集計](p6-mini-results/models-summary.json)、[ソース/成果物hash](p6-mini-results/artifacts.json)。
再集計: `python3 scripts/tuyouraou11-p6-summary.py models --records-dir plans/tuyouraou11/p6-mini-results`。


P4 の既存 desktop 上限（準備10秒、Wasm memory128MiB、engine超過100ms、main往復超過200ms）を変更せず適用する。
上限内はローカルHTTPのこの測定条件での判定で、携帯回線/CDN/低速通信の保証ではない。
NPS比には評価器の計算コストに加えて、モデルによる探索木の違いも含まれる。
ブラウザ総 process RSS/JS heap peak は未測定で、Wasm memoryと同一視しない。
旧HalfKAv2の外部実モデルも今回の配置にはなく、通常/合成カーネル回帰を実ファイル検証へ読み替えない。

同じ native 実行ファイル（runtime AVX2 検出）で A=mini、B=micro とし、
P4 の先頭6開幕を先後交換。200ms/1000ms 各12局、max256手、seed1、threads2で比較した。
mini から見て200msは8勝4敗、1000msは7勝5敗、計15勝9敗・引分0。
スコア率62.50%、時間別層化・開始局面ペア bootstrap 10,000回（seed1）の95%区間は45.83〜79.17%。
クラッシュ、不正着手、指し手欠損は0。全手の合法性は対局ハーネスが検証した。

24局の探索的比較であり、棋力向上や優劣確定の根拠にはしない。これはnativeでのモデル比較で、
Wasm SIMDの対局やP4の200局再実施ではない。デフォルトはmicroのまま、miniもユーザーが選択できる。
[対局集計](p6-mini-results/model-matches-summary.json)、`p6-mini-results/match-{200,1000}/`。
再集計: `python3 scripts/tuyouraou11-p6-summary.py model-matches --records-dir plans/tuyouraou11/p6-mini-results`。


## 完了範囲

mini追加、micro/mini選択、SIMD/scalar互換、desktop/browserの動作・性能検証は完了。
24局の予備比較に続き、[p6-strength.md](p6-strength.md)のブラウザ本検証400局を完了。
全43,814手が実Worker応答と一致し、miniは248勝147敗5分・スコア62.625%（95%区間58.00〜67.25%）。
共通開幕block区間も50%を上回り、今回のdesktop条件の総合ではminiが上回った。
Firefox200msでは優位を確認できず、全環境へ一般化しない。両モデルの選択とmicro既定を維持する。
実機はユーザーの明示指定で対象外。P4の実機保留や未測定メモリを検証済みへ変えない。



## 再現

```bash
# haitaka-variants: 既存 micro の取得済みファイルに加えて mini を取得
python3 scripts/tuyouraou11.py fetch --profile mini
python3 scripts/tuyouraou11.py reference --profile mini
python3 scripts/tuyouraou11.py fixtures --profile mini
python3 scripts/tuyouraou11.py verify --profile mini
node scripts/tuyouraou11-simd.mjs --mini --build --browser

# shogitter-3: wasm-bindgen を PATH へ設定。両 model descriptor/実ファイルを照合する
HAITAKA_MINI_MODEL_DIR=../haitaka-variants/target/tuyouraou11/mini-model \
HAITAKA_ENGINE_DIR=../haitaka-variants node scripts/sync-haitaka-wasm.mjs --standard-only
node scripts/sync-haitaka-wasm.mjs --verify-model
BASE_URL=http://localhost:3098 SKIP_WEB_SERVER=1 \
  pnpm exec playwright test -c playwright.tuyouraou11.config.ts --trace off
# 開発サーバーと同じ profile の診断 Wasm/参照ビルドを使用
node scripts/tuyouraou11-rules.mjs mini

# 同一core・固定20局面・200/1000/3000ms・各3反復、各組180探索
# mini と micro を各 browser で直列に測る（同時測定しない）
taskset -c 0,4 node scripts/tuyouraou11-browser.mjs sfnn chromium simd mini
taskset -c 0,4 node scripts/tuyouraou11-browser.mjs sfnn firefox simd mini
taskset -c 0,4 node scripts/tuyouraou11-browser.mjs sfnn chromium simd micro
taskset -c 0,4 node scripts/tuyouraou11-browser.mjs sfnn firefox simd micro
```

Chromium の executablePath を上書きする場合は `HAITAKA_CHROMIUM_PATH` を設定する。
参照ビルドは profile ごとの edition として作り直す。micro 再生成時は `reference --profile micro` を先に実施。
実 bin/展開 archive/参照 GPL 実行ファイルは ignored target/、Web 配信の MIT モデルは public/ へ置く。
コミット・公開デプロイは今回行わない。既存開発サーバー 3002 番は保持する。
