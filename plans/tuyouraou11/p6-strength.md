# P6 棋力本検証: 実ブラウザ micro / mini

状態: 完了（desktop対象、実機対象外）。2026-10-31、ユーザーが「本検証を行って」と明示指定。
[凍結プロトコル](p6-strength-protocol.json)を本対局の開始前に保存した。
モデル・Wasm・glue・Web runtime・対局 bridge・native arbiter・開幕の hash を固定する。
24局の予備比較は混ぜず、全く別の400局を実施する。

## 固定条件

- Chromium 153.0.8010.12 / Firefox 142.0.1、各200局、計400局。
- 各 browser で200ms / 1000ms 各100局。P4の50開幕を同じ順序で先後交換。
  開幕 SHA-256 `ce6653c8790157159a44f30e375984451282318e2dce6870de5a65d30e75696c`。
- A=mini / B=micro、depth上限64、Hash64MiB、最大256手で引分、seed1。
- 同じCPU上で各browser2局並列。Chromium=taskset 0,1,4,5、Firefox=2,3,6,7。
  4物理coreへ4対局を割り当てる。アプリ開発サーバー等は保持するため専用の無負荷端末とは扱わない。
- 各model2個の独立Worker。対局前に同じ固定10局面を100msずつwarmupし、TTを消去する。
  毎対局のisready/usinewgameでもTTを消去し、モデルごとの探索状態が混ざらない。
- 本番配信の実SIMD128 Wasmをブラウザmodule Workerで実行する。
  既存Web runtimeのモデル選択・fetch・bytes/SHA検証を通し、UsiEngine sessionへ読み込む。
  USI exportはP4と同じ履歴付き探索/TT64比較条件で、画面botのglobal探索の既定設定とは区別する。
- native CLIは対局の進行・全着手のtry_play・千日手/連続王手等の判定のみを担当する。
  指し手をnativeで探索する構成ではない。全USI入出力をgzipで保存し、各棋譜の全手と照合する。
- 実機はユーザー指定どおり対象外。scalarの整数一致/合法手は既存P6で検証済み、本対局はSIMD対象。

## 判定

勝率ではなく引分を0.5点としてスコア率を算出する。
P4に合わせた開始局面ペア単位・browser/時間別層化bootstrap 10,000回、seed1、percentile 95%区間を主指標とする。
同じ開幕を全条件で使う依存性に対し、共通開幕block単位bootstrapも感度分析として併記する。
総合の両区間の下端が50%を超えた場合のみ「この条件ではminiが上回った」とする。
そうでなければ優劣未確定。結果を見て都合のよい追加対局や条件変更を行わない。
P4の退行条件（区間上端<50%）も別途報告する。

クラッシュ・不正着手・指し手欠損・bridge/RPCエラー・出所不一致は失敗で、普通の敗戦へ混ぜない。
実装修正が必要になった場合はそのrunを無効とし、同じ固定条件の全本検証を再実行する。
両モデルを選択肢として残すユーザー方針は維持する。勝者を1つに絞らず、既定もこの実験では自動変更しない。

## 再現

```bash
# shogitter-3。起動中のdev serverは使用/変更しない。
HAITAKA_CHROMIUM_PATH=/path/to/chrome-headless-shell \
  taskset -c 0,1,4,5 node scripts/tuyouraou11-match.mjs chromium
# 別processで並行。CPU集合を重ねない。
taskset -c 2,3,6,7 node scripts/tuyouraou11-match.mjs firefox

# haitaka-variants、全400局が終わってから
python3 scripts/tuyouraou11-strength-summary.py
```

準備試験は `--smoke`。修正後に両browser各100局×4手を通し、全800手の棋譜/実Worker transcript一致とモデルhashを確認済み。
本対局の出力先は `target/tuyouraou11/p6-strength/{chromium,firefox}/`。
既存結果があればbridgeは起動を拒否し、条件違いや誤った追記を防ぐ。

## 無効化した初回実行

`target/tuyouraou11/p6-strength-invalid-transport-1/` に初回の記録と旧プロトコルを保持する。
両browserの200ms各100局は対局CLIでは正常終了したが、Chromium bridgeが対局後のproxy終了による
`ECONNRESET`を検証エラーとして扱ったため、この実行全体を除外した。Firefoxの1000ms途中分も混ぜない。
対局後の正常な切断を記録できるようbridgeを修正し、100局×2browserの再接続試験と全手監査を通した。
モデル・探索・開幕・持ち時間・サンプル数・CPU/並列度・判定法は変更せず、revision3のbridge hashで全400局を最初から実施する。
2回目もreadline iterator終了後の未処理例外が見つかり、`p6-strength-invalid-transport-2/`へ隔離した。
終了後・返信後・探索中の強制切断を各100回、計300回再現し、正常終了と実エラーの区別を確認した。
恒久的なerror listenerとエラー発生時点の状態保持を追加。最終bridgeで両browser各100短期対局を通した。
探索/RPCの実エラーや未完了対局は引き続き失敗とする。統計境界値と棋譜改変検知の4テストも合格。

## 外部中断からの継続

revision3では200ms各100局と1000ms Chromium34局/Firefox35局の棋譜を保存した後、
対局processが外部実行環境の終了とともに停止し、metadataのエラー・終了処理は残らなかった。
model/search/bridgeの実装不具合は記録されていない。途中のgzipにもfooterがない。
元ファイルは変更せずhashを`protocol.resume.interruptedArtifacts`に固定した。

gzipの圧縮済み領域から完全なJSON行だけを復元し、全手を検証できた各browserの200ms100局・1000ms32局、計264局を保持する。
1000msの33局目以降（Chromium2局/Firefox3局）は全手のlogが回収できないため除外した。
勝敗を使った選択ではなく、両browserとも16開幕ペアまでという同じ境界を適用する。
保持264局28,226手はmodel別Workerの実応答と一致した。

残り34開幕ペア（元開幕index16..49）を各browser68局ずつ実施する。
`p6-strength-remaining-openings.sfen`は凍結した50開幕の末尾34個で、元の順序を維持する。
continuation bridgeのWorker sourceは元bridgeと完全一致。配信Wasm/glue/runtime/model、depth64/Hash64、
1000ms/max256、2局並列/core割当・warmup・統計法も同じ。実行/出力先だけを分けた。
protocolに継続理由・hash・採用範囲を記録し、継続開始時に検証する。
今後のturn中断でもprocessが保持されるよう、tmuxの専用2sessionで起動した。
元データは`target/tuyouraou11/p6-strength/`、継続分は`target/tuyouraou11/p6-strength-resume/`。

```bash
# shogitter-3、起動時に既存継続結果があれば拒否する
HAITAKA_CHROMIUM_PATH=/path/to/chrome-headless-shell \
  taskset -c 0,1,4,5 node scripts/tuyouraou11-match-resume.mjs chromium
taskset -c 2,3,6,7 node scripts/tuyouraou11-match-resume.mjs firefox
# haitaka-variants
python3 scripts/tuyouraou11-strength-summary.py \
  --continuation-dir target/tuyouraou11/p6-strength-resume
```

## 確定結果

**本検証400局・43,814手の実ブラウザ応答照合が合格。**
miniから見て248勝147敗5分。スコア率62.625%、総合95%区間58.00〜67.25%。
共通開幕block感度分析の95%区間57.625〜67.50%。いずれも下端>50%で、事前基準に従い
「このdesktop SIMD/USI条件の総合ではminiがmicroを上回った」と判定する。

| ブラウザ | 持ち時間 | mini勝 / micro勝 / 分 | miniスコア率 | 95%区間 |
|---|---:|---:|---:|---:|
| chromium | 200ms | 66 / 33 / 1 | 66.5% | 57.0〜75.5% |
| chromium | 1000ms | 65 / 35 / 0 | 65.0% | 56.0〜73.0% |
| firefox | 200ms | 45 / 52 / 3 | 46.5% | 36.5〜57.0% |
| firefox | 1000ms | 72 / 27 / 1 | 72.5% | 63.0〜81.5% |

Firefoxの200ms条件は区間が50%を跨ぎ、miniの優位を確認できない。短時間・低速な実行環境を含めて
常にminiが強いとは結論しない。全ブラウザ・実機・画面bot既定設定に一般化しない。
Chromium総合スコア65.75%（95%59.5〜72.0%）、Firefox総合59.50%（52.5〜66.5%）。
P4の退行判定（総合区間の上端<50%）には該当しない。

- 4層各100局、各50開始局面の先後交換を照合。重複・不足なし。
- 全43,814着手が該当モデルのWorker USI応答と一致。native arbiterが全手をtry_playし終局判定。
- 採用400局のクラッシュ・不正着手・指し手欠損・RPC失敗は0。
- 5引分のうち3局が最大256手による打切り引分。打切りをモデルの勝敗へ付け替えない。
- 対局WorkerのHash64MiB条件でのWasm memory最大はmini127,729,664bytes、micro116,523,008bytes。
  旧性能測定のglobal探索の53.2/57.8MiBと異なり、比較時TT設定に由来する。総process RSSではない。
- 採用手のWorker往復探索超過は最大8ms、broker往復超過は最大約16.0ms。
  元の実行終了時の失敗2runと、外部中断時に全手を回収できなかった5局は含めない。
- frozen protocolのモデル/生成Wasm/glue/Web runtime/能力判定/native arbiter hashを照合。
  interrupted retained264局とcontinuation136局の対応、model別slotも確認した。

micro/miniの両方を選べる方針を維持し、既定はmicroのまま。
今回の結果はminiの追加選択肢を裏付ける。単独の勝者への絞り込みや自動切替はしない。
ユーザーが対象外としたiOS/Android実機は検証済みと扱わない。

[機械集計・全監査](p6-strength-results/strength-summary.json)、[固定条件](p6-strength-protocol.json)、
[ファイルmanifest](p6-strength-results/manifest.json)。
採用400局の一覧は`p6-strength-results/accepted-games.jsonl`（元ファイル/局番号付き、継続分の開幕番号を補正）。
rawは`p6-strength-results/retained/`と`continuation/`に保存した。
retainedのgzipは中断時のbyte列をそのまま保存しておりfooterがないが、固定hashと完全JSON行だけを使って監査する。
retainedの1000ms棋譜には除外した33局目以降も元のまま含むため、生ファイルの行数を採用局数と誤認しない。
集計スクリプトが採用範囲を厳密に制限する。

```bash
# 保存済み記録だけから再監査・再集計
PYTHONDONTWRITEBYTECODE=1 python3 scripts/tuyouraou11-strength-summary.py \
  --records-dir plans/tuyouraou11/p6-strength-results/retained \
  --continuation-dir plans/tuyouraou11/p6-strength-results/continuation
```

状態: 今回指定の棋力本検証とdesktop対象のP6は完了。実機は対象外。コミット/公開デプロイなし。


## Elo差への換算

スコア率 p=(勝+0.5×分)/局数を `ΔElo=400 log10(p/(1-p))` で換算する。
絶対レーティングや対人段級ではなく、固定した対局条件での相対差を表す。
95%区間は既存の開幕ペアbootstrapスコア区間に同じ単調変換を適用する。

| 対局条件 | mini − micro Elo差 | 95%区間 |
|---|---:|---:|
| Chromium 200ms | +119 | +49〜+196 |
| Chromium 1000ms | +108 | +42〜+173 |
| Firefox 200ms | −24 | −96〜+49 |
| Firefox 1000ms | +168 | +92〜+258 |
| 全400局 | +90 | +56〜+125 |

共通開幕block感度分析での総合区間は+53〜+127 Elo。
比較用の既存P4 native対局ではmicroが手書き評価に188勝12敗、+478 Elo相当（95%区間+392〜+604）。
200ms92勝8敗は+424（+330〜+604）、1000ms96勝4敗は+552（+424〜+798）。
P4 nativeはP6 SIMD前の条件で、今回のbrowser条件と異なるため、+90と+478を足してmini対手書き評価の値にはしない。
mini対手書き評価の直接の本検証はこの換算記録にはない。

[換算データ](p6-elo-summary.json)。再現: `python3 scripts/tuyouraou11-elo.py`。
