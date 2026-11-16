# 平安将棋9×9のつようら王転用実験

## 結果

既存のつようら王11micro・miniを平安9×9で動かすことはできたが、今回の
ネイティブCPU・1手100msの対戦では手書き評価が圧倒的に優位だった。
この条件から、既定の手書き評価を置き換えることは勧めない。

勝・敗はつようら王側。得点率は（勝＋引分×0.5）÷局数。

| 評価器 | 勝 | 敗 | 引分 | 得点率 | ペアbootstrap参考95%区間 | 平均手数 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| つようら王11micro | 0 | 99 | 1 | 0.5% | 0.0–1.5% | 110.68 |
| つようら王11mini | 1 | 99 | 0 | 1.0% | 0.0–3.0% | 117.45 |

microの引分1局は512手打切り。microは先手で49敗1引分、後手で50敗。
miniは先手で50敗、後手で1勝49敗。先後交換した序盤ペアではmicroが50敗、
miniが49敗1分で、勝ち越したペアはなかった。対局エラー・代替着手は両方0。

区間は50序盤ペアを単位とする10,000回percentile bootstrap（seed=330011）。
結果が0%付近に集中した小標本の参考区間であり、真の得点率がこの狭い範囲に確実に
収まるという意味には使わない。二つのモデルを比較した探索的実験で、多重比較補正はしていない。

本将棋モデルを転用した結果であり、専用学習、長い思考時間、WASM・ブラウザーでの
棋力対戦は未測定。miniがmicroより強いことを直接対戦で示したものでもない。
負けた原因を、モデルの分布外入力・評価速度・平安特有の評価要素などへ分離する診断はしていない。

## 実装と正しさの確認

計測対象のエンジンソースは `feb12d6b448ecaede97983bf91911f2b27b4b43a`。
SFNN専用の許可を分離し、平安9×9だけでmicro・miniを許可した。
平安8×8・9×8、従来NNUE、学習用SFENの未検証形式は拒否したまま。

取り捨て捕獲では、捕獲駒の盤上特徴を削除して持駒を加算するのではなく、欠けた物理駒の
スロット（特徴0）を加える。金への成り、玉移動、進行度も全再計算と一致させる。

- 平安9×9のコア6件・WASMライブラリ5件、他の平安2形状の8件、本将棋SFNNの3件が成功。
- 特徴0を意図的に非ゼロにした合成パラメーターで、両手番の捕獲・金成り・玉による捕獲を検証。
- 実micro・miniの各2,304遷移で差分状態・進行度・評価値が全再計算と整数単位で完全一致。
  各モデルで捕獲257件、金への成り95件を含む。固定深さ探索、USI、玉対玉の0点終局も確認。
- 将棋ったーの実配布WASMを再生成し、関連143件と実Chrome12件が成功。
  モデル切替・応手、平安3種の成り・取り捨て・引き分けを確認。

## 条件と証跡

[条件・元棋譜・集計・ハッシュ監査](../plans/heian-sfnn/20261116/)。
通常版・miniで同じ50序盤を使用したこと、全100ペアが揃うこと、勝敗とレポートの整合を照合した。
元棋譜を含む各ファイルのSHA-256は `audit.json` に記録。

- 各100局、50序盤の先後交換、ランダム序盤8手、seed=330011。
- 1手100ms、最大512手、4対局並列。深さ上限64、既定Hash 16 MiB。
- 両者とも同じ `sfnn_usi` 実行ファイル。Aだけモデルをロードし、Bは手書き評価。
  モデル読込みは持ち時間外で、対局ごとに新しいセッションを作る。
- CPUはIntel Core i7-6700、CPU affinity=0,1,2,3（異なる物理4コア）。
  計測中に別のビルドや学習は実行していない。
- Rust 1.98.1、native release。モデル・実行ファイルのSHA-256とソースコミットを固定。
- micro NN SHA-256: `7f9545dbf73b87acf312146daabf59c4f17d0845c87d587fa19caed51707dddb`。
- mini NN SHA-256: `45323c65135da65d6c2c364eea93a5af6a285c27ebfb07eab4afbe737ef74508`。
- 共通progress SHA-256: `a2d18f0564639c945b5d6469e9b437cd64a526ad4fbcef02ab1d8f547d08e5f1`。

## 再現

過去の[ルール比較手順](tuyouraou11-rule-comparison.md)と、残っていた持駒ルール用
`target/tuyouraou11-hand-comparison-main-20261112/driver.py` を参照し、既存の
`haitaka_cli self-play` と `sfnn_usi` を使うドライバーを追加した。
既存手順の256手上限を、今回の平安将棋では512手にして打切りの影響を抑えた。

```sh
# 検証済みmicro/miniモデルを用意し、追跡対象のソース変更をcommitしてから実行する。
# CPU番号とモデルパスは実行環境に合わせる。
taskset -c 0,1,2,3 python3 scripts/compare-heian-sfnn.py all \
  --micro-model target/tuyouraou11/model \
  --mini-model target/tuyouraou11/mini-model \
  --output target/heian-sfnn-new-run

# 保存した200局のペア単位集計を再計算する。
python3 scripts/compare-heian-sfnn.py summarize \
  --output plans/heian-sfnn/20261116
```

実モデルの正しさの検証は次の通り（miniではprofileとモデルディレクトリを切り替える）。

```sh
HAITAKA_SFNN_MODEL_DIR="$PWD/target/tuyouraou11/model" \
  cargo test --release -p haitaka_wasm --features heian-9x9 \
  --test sfnn_heian -- --ignored --nocapture
```
