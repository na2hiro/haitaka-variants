# 代表ルールの PVS・1 ply LMR 実験結果

判定の一覧は [README](README.md)。

計測日 2026-11-21〜22。既存の100 ms代表ルール検査と、前回の探索実験をもとに、共通の盤面・探索を使う5ルールを選んだ。安南と NekoNeko は駒の動きが周囲の駒に依存する。5五将棋は小盤、持ち駒不使用は取り駒の扱い、王手将棋は勝利条件が異なる。JSON ルールの探索は `haitaka::extended` にある。計測したコードは `feat/handcrafted-eval-experiments`（`7f29506`）に載せた未コミットの実装。このブランチでは最新 main に移植し、既定ビルドと実験ビルドの探索結果が計測時から変わらないことを照合した（[README](README.md#移植後の照合)）。

変則ルールの候補は `search-variant-experiments` と `search-pvs` または `search-lmr` の組合せで有効になる。両候補は実験 feature として切り替える。

## 固定深さ

開始局面と既存の校正用局面を、手作り評価・新しい TT・期限なしの深さ3と4で照合した。NekoNeko の校正用局面では1ケースが180秒を超えたため、開始局面と少駒局面の2局面で再検証した。

| ルール | PVS と基準探索のスコア一致 | PVS の試し読み / 再探索 | PVS＋LMR の短縮 / 再探索 |
|---|---:|---:|---:|
| 安南 | 8/8 | 1,471 / 71 | 273 / 3 |
| NekoNeko | 4/4 | 306 / 11 | 27 / 1 |
| 5五将棋 | 8/8 | 1,174 / 80 | 67 / 8 |
| 持ち駒不使用 | 8/8 | 1,458 / 69 | 246 / 1 |
| 王手将棋 | 8/8 | 1,566 / 57 | 232 / 120 |

安南の開始局面・深さ4は、通常のデルタ枝刈りで基準探索32、PVS 20だった。デルタ枝刈り無効では両方2だった。安南は3構成とも実験 feature を付けてデルタ枝刈りを無効に揃えた。他の4ルールは各ルールの通常の qsearch 設定で比較した。NekoNeko の少駒局面は `4k4/9/9/3p5/9/5P3/9/9/4K4 b - 1`。

安南の実験 feature あり・なしと通常将棋の LMR 構成で、`cargo test --offline --release -p haitaka_wasm --lib` が成功した。安南の実験 feature なしでは試し読みと短縮がゼロだった。

## 100 ms の対局

各行は32局、16先後ペア。手作り評価、seed 1、ランダム初手順4 ply、4ワーカー、対局上限120 ply。ルール内の2比較で同じ初手順と先後の組を使ったことを棋譜から確認し、CLI のペア Elo と95%信頼区間を再計算した。並行して本将棋の1秒対局を実行した計測環境での結果。

| ルール | 比較 | 勝–敗–分 | 打切り | 得点率 | ペア Elo | 95% CI |
|---|---|---:|---:|---:|---:|---:|
| 安南 | PVS 対 基準探索 | 14–17–1 | 1 | 45.31% | -32.7 | [-150.9, +85.6] |
| 安南 | PVS＋LMR 対 PVS | 14–18–0 | 0 | 43.75% | -43.7 | [-150.7, +63.4] |
| NekoNeko | PVS 対 基準探索 | 13–12–7 | 7 | 51.56% | +10.9 | [-74.2, +95.9] |
| NekoNeko | PVS＋LMR 対 PVS | 13–11–8 | 8 | 53.12% | +21.7 | [-76.3, +119.8] |
| 5五将棋 | PVS 対 基準探索 | 15–16–1 | 0 | 48.44% | -10.9 | [-76.6, +54.9] |
| 5五将棋 | PVS＋LMR 対 PVS | 22–8–2 | 0 | 71.88% | +163.0 | [+62.2, +263.8] |
| 持ち駒不使用 | PVS 対 基準探索 | 0–0–32 | 32 | 50.00% | — | — |
| 持ち駒不使用 | PVS＋LMR 対 PVS | 1–0–31 | 30 | 51.56% | +10.9 | [-10.4, +32.2] |
| 王手将棋 | PVS 対 基準探索 | 16–16–0 | 0 | 50.00% | +0.0 | [-87.9, +87.9] |
| 王手将棋 | PVS＋LMR 対 PVS | 15–17–0 | 0 | 46.88% | -21.7 | [-64.5, +21.0] |

5五将棋の LMR がこのスクリーニングで最も高く、PVS 単体に22勝8敗2分だった。16ペアの結果で、区間は各比較についてのもの。比較を10件並べたスクリーニングで、局数も少ないため、差の大きさは当てにならない。局数を増やした追試の候補とする。持ち駒不使用は初手順からの64局中62局が120 plyに達した。

## 持ち駒不使用の追加対局

既存の校正用進行局面8件と少駒局面8件を選び、同じ100 ms・120 plyで各32局を追加した。初手順からの成績と別に集計した。

| ルール | 比較 | 勝–敗–分 | 打切り | 得点率 | ペア Elo | 95% CI |
|---|---|---:|---:|---:|---:|---:|
| 持ち駒不使用 | PVS 対 基準探索 | 2–2–28 | 26 | 50.00% | +0.0 | [-31.1, +31.1] |
| 持ち駒不使用 | PVS＋LMR 対 PVS | 2–2–28 | 24 | 50.00% | +0.0 | [+0.0, +0.0] |

追加対局でも64局中50局が打切りとなり、両比較の得点率は50%。PVS＋LMR 対 PVS の全ペアが1勝1敗または2分で、ペア得点の分散は0だった。この条件で得られる棋力差の情報は限られる。

## 再現コマンド

固定深さ JSON、起動コマンド、対局ごとの集計は [`receipts/variants/`](receipts/variants/) にある（ルール別のディレクトリと、全体の `summary.json`）。バイナリと棋譜は計測した機械の `target/search-pvs-lmr/variants/` にだけ残し、コミットしていない。ビルド例は次のとおり。

```bash
rule=minishogi # annan, nekoneko, fushiyo, oute
out="$PWD/target/search-pvs-lmr/variants/$rule"
mkdir -p "$out"
baseline_features=$rule
if [[ "$rule" == annan ]]; then
    baseline_features="$rule,search-variant-experiments"
fi
cargo build --offline --release -p haitaka_cli --features "$baseline_features"
cp target/release/haitaka_cli "$out/baseline"
cargo build --offline --release -p haitaka_cli --features "$rule,search-variant-experiments,search-pvs"
cp target/release/haitaka_cli "$out/pvs"
cargo build --offline --release -p haitaka_cli --features "$rule,search-variant-experiments,search-lmr"
cp target/release/haitaka_cli "$out/lmr"
```

対局は [`receipts/variants/<rule>/`](receipts/variants/) の `*-100ms.command` で再現できる。各ディレクトリの JSON に実際の対局条件と手順が記録されている。

```bash
"$out/baseline" self-play \
    --games 32 --threads 4 --movetime-ms 100 --max-plies 120 \
    --opening-unit pair --opening-random-plies 4 --seed 1 \
    --a-engine "$out/pvs" --a-engine-arg usi --a-engine-arg=--eval --a-engine-arg handcrafted \
    --b-engine "$out/baseline" --b-engine-arg usi --b-engine-arg=--eval --b-engine-arg handcrafted \
    --report-dir "$out/pvs-vs-baseline-100ms"
```
