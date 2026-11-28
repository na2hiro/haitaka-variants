# 通常将棋の PVS・1 ply LMR 実験結果

判定の一覧は [README](README.md)。区間が0を外れた比較も、比較の数が多いため追試前の候補として扱う。

計測日 2026-11-21。通常将棋の release ビルドで手作り評価を使用した。現行探索は変更前の `7f29506d`。既定オフの `search-pvs` と `search-lmr` で実験を切り替える。計測したコードは `feat/handcrafted-eval-experiments`（`7f29506`）に載せた未コミットの実装。このブランチでは最新 main に移植し、既定ビルドと実験ビルドの探索結果が計測時から変わらないことを照合した（[README](README.md#移植後の照合)）。

## 固定深さの検証

PVS 単体は、開始局面、既存の序盤4局面、戦術3局面、詰み、王手回避、千日手と連続王手の成立直前の局面で現行探索とルートスコアが一致した。局面ごとに固定した2深さを、通常の qsearch 設定とデルタ枝刈り無効の両方で照合した。探索ごとに TT と手順付けを初期化し、期限なしで実行した。成立済みの千日手と連続王手も、終局値と試し読みゼロを確認した。

LMR の回帰テストでは5手目の TT 深さが1 ply 浅いこと、短縮 fail-low の親で TT 保存を省くこと、`alpha` 超過と短縮時の詰み値から通常深さへ戻ることを確認した。通常の再探索中のノード予算切れと、qsearch 上限からの再実行中の中断を用い、確定済みのルート手だけが残って履歴が復元されることを確認した。

安南と NekoNeko は `search-lmr` を有効にして深さ4の USI 統計を確認し、PVS と LMR の各カウンタはすべてゼロだった。

| 構成 | 成功 | 既存の無効化済みテスト |
|---|---:|---:|
| 現行 | 103 | 4 |
| PVS | 106 | 4 |
| PVS＋LMR | 110 | 4 |
| 安南＋実験 feature | 106 | 4 |
| NekoNeko＋実験 feature | 100 | 4 |

各構成について `cargo test --offline --release -p haitaka_wasm --lib` を実行した。実験 feature 付きの `wasm32-unknown-unknown` と、学習データ生成に使う既定構成の `haitaka_learn` は `cargo check --offline` を通過した。USI の `go depth`、`go movetime`、`go nodes` から実験カウンタを取得できることも確認した。

## 固定深さの計測

Intel Core i7-6700、rustc 1.98.1。各局面と深さについてビルドの順番を交替しながら3回計測した。各回で USI プロセスを起動し直し、初期 TT と初期手順付けで `go depth` を実行した。実時間は `go` 送信から `bestmove` 受信までの中央値。ノード数、スコア、選択手、再探索件数は3回一致した。スコアは手番側から見た `lastCompletedIterationValue`。

各セルは **スコア / 選択手 / 主探索ノード / qnodes / 実時間 ms** の順。

| 局面 | 深さ | 現行 | PVS | PVS＋LMR |
|---|---:|---|---|---|
| startpos | 4 | 2 / `7g7f` / 7,143 / 6,230 / 12.66 | 2 / `7g7f` / 7,527 / 6,389 / 12.80 | 2 / `7g7f` / 7,895 / 6,611 / 13.86 |
| startpos | 5 | 2 / `1g1f` / 39,238 / 36,632 / 62.33 | 2 / `1g1f` / 39,042 / 34,324 / 63.74 | 2 / `1g1f` / 24,452 / 22,273 / 47.36 |
| opening-1 | 4 | 204 / `3c2b` / 44,143 / 60,528 / 106.75 | 204 / `3c2b` / 41,705 / 53,218 / 90.60 | 204 / `3c2b` / 37,771 / 51,384 / 90.88 |
| opening-1 | 5 | 208 / `3c2b` / 205,246 / 634,331 / 1391.96 | 208 / `3c2b` / 196,759 / 486,259 / 1098.24 | 204 / `3c7g` / 155,943 / 385,543 / 828.83 |
| opening-2 | 4 | 220 / `2e2d` / 5,020 / 4,844 / 12.14 | 220 / `2e2d` / 6,032 / 5,833 / 13.51 | 220 / `2e2d` / 5,325 / 5,160 / 12.65 |
| opening-2 | 5 | 210 / `2e2d` / 47,047 / 42,883 / 71.64 | 210 / `2e2d` / 49,604 / 44,523 / 83.21 | 210 / `2e2d` / 25,308 / 22,386 / 46.50 |
| opening-3 | 4 | -10 / `7g7f` / 4,258 / 3,721 / 8.07 | -10 / `7g7f` / 4,419 / 3,839 / 7.96 | -10 / `7g7f` / 4,592 / 3,916 / 8.25 |
| opening-3 | 5 | -8 / `7g7f` / 31,750 / 29,308 / 56.81 | -8 / `7g7f` / 39,296 / 34,094 / 73.33 | -8 / `7g7f` / 25,731 / 22,882 / 52.34 |
| opening-4 | 4 | 10 / `7g7f` / 3,426 / 3,152 / 7.11 | 10 / `7g7f` / 4,544 / 4,159 / 8.41 | 10 / `7g7f` / 3,930 / 3,578 / 7.84 |
| opening-4 | 5 | 4 / `5h5i` / 23,098 / 21,278 / 33.86 | 4 / `5h5i` / 21,398 / 18,981 / 32.79 | 4 / `5h5i` / 13,863 / 12,180 / 24.43 |
| rook-capture | 4 | 2088 / `5e4e` / 2,389 / 42,582 / 48.12 | 2088 / `5e4e` / 2,764 / 36,498 / 40.92 | 2088 / `5e4e` / 2,454 / 36,308 / 46.91 |
| rook-capture | 5 | 2088 / `5e4e` / 29,193 / 116,477 / 128.11 | 2088 / `5e4e` / 42,725 / 122,685 / 135.44 | 2088 / `5e4e` / 37,760 / 113,649 / 126.41 |
| silver-promotion | 4 | 566 / `5i4h` / 560 / 828 / 1.42 | 566 / `5i4h` / 538 / 756 / 1.35 | 566 / `5i4h` / 642 / 878 / 1.50 |
| silver-promotion | 5 | 568 / `5i4h` / 3,462 / 3,707 / 4.62 | 568 / `5i4h` / 3,529 / 3,640 / 4.57 | 568 / `5i4h` / 2,549 / 2,980 / 3.98 |
| bishop-promotion | 4 | 942 / `5i4h` / 1,540 / 4,065 / 5.16 | 942 / `5i4h` / 1,506 / 3,919 / 5.04 | 942 / `5i4h` / 1,506 / 3,919 / 4.90 |
| bishop-promotion | 5 | 942 / `5c3a+` / 14,079 / 13,581 / 13.90 | 942 / `5c3a+` / 13,588 / 13,045 / 13.32 | 942 / `5c4d+` / 5,979 / 8,077 / 9.20 |

再探索率の分母は PVS の試し読み件数または LMR の短縮件数。

| 局面 | 深さ | PVS の再探索率 | PVS＋LMR の PVS 再探索率 | PVS＋LMR の LMR 再探索率 |
|---|---:|---:|---:|---:|
| startpos | 4 | 10/402 (2.49%) | 13/486 (2.67%) | 0/104 (0.00%) |
| startpos | 5 | 26/639 (4.07%) | 33/781 (4.23%) | 5/800 (0.62%) |
| opening-1 | 4 | 46/1,011 (4.55%) | 52/1,103 (4.71%) | 0/274 (0.00%) |
| opening-1 | 5 | 123/3,858 (3.19%) | 110/3,650 (3.01%) | 323/1,937 (16.68%) |
| opening-2 | 4 | 7/234 (2.99%) | 7/234 (2.99%) | 0/50 (0.00%) |
| opening-2 | 5 | 13/610 (2.13%) | 17/707 (2.40%) | 2/908 (0.22%) |
| opening-3 | 4 | 6/206 (2.91%) | 8/232 (3.45%) | 1/49 (2.04%) |
| opening-3 | 5 | 34/755 (4.50%) | 35/810 (4.32%) | 15/599 (2.50%) |
| opening-4 | 4 | 8/253 (3.16%) | 8/253 (3.16%) | 0/40 (0.00%) |
| opening-4 | 5 | 9/317 (2.84%) | 11/407 (2.70%) | 1/485 (0.21%) |
| rook-capture | 4 | 5/228 (2.19%) | 5/228 (2.19%) | 0/17 (0.00%) |
| rook-capture | 5 | 39/524 (7.44%) | 39/524 (7.44%) | 1/208 (0.48%) |
| silver-promotion | 4 | 2/52 (3.85%) | 4/72 (5.56%) | 0/1 (0.00%) |
| silver-promotion | 5 | 9/176 (5.11%) | 9/176 (5.11%) | 0/75 (0.00%) |
| bishop-promotion | 4 | 6/136 (4.41%) | 6/136 (4.41%) | — |
| bishop-promotion | 5 | 3/88 (3.41%) | 6/109 (5.50%) | 0/353 (0.00%) |

8局面の合計に対する増減率。実時間は各局面の中央値を合計した。

| 深さ | 比較 | 主探索ノード | qnodes | 合計ノード | 実時間 |
|---|---|---:|---:|---:|---:|
| 4 | PVS 対 現行 | +0.8% | -9.0% | -5.5% | -10.3% |
| 4 | PVS＋LMR 対 PVS | -7.1% | -2.5% | -4.2% | +3.4% |
| 5 | PVS 対 現行 | +3.3% | -15.7% | -9.9% | -14.7% |
| 5 | PVS＋LMR 対 PVS | -28.2% | -22.1% | -24.2% | -24.3% |

PVS のスコアと選択手は全16ケースで現行と一致した。PVS＋LMR は opening-1 の深さ5で 208 から 204 になり、手が `3c2b` から `3c7g` に変わった。デルタ枝刈り無効でも同じスコア差が出た。短縮を外した参照テストでは 208 に戻った。bishop-promotion の深さ5でも手が変わり、スコアは 942 だった。

## 先後を入れ替えた自己対局

各比較は200局、同一初手順を先後入れ替えした100ペア。全4比較で seed 1、ランダム初手順4 ply、4ワーカー、対局上限200 plyを使用した。棋譜から、4比較が同じ100初手順を使い、全ペアで先後が反転したことを照合した。Elo と95%信頼区間は CLI のペア単位推定。棋譜からペアの得点分布を再計算し、CLI の数値と照合した。

| A 対 B | 1手 | A 勝–B 勝–引分 | A 得点率 | ペア Elo | 95% CI |
|---|---:|---:|---:|---:|---:|
| PVS 対 現行 | 20 ms | 85–114–1 | 42.75% | -50.7 | [-96.7, -4.8] |
| PVS＋LMR 対 PVS | 20 ms | 96–102–2 | 48.50% | -10.4 | [-51.2, +30.3] |
| PVS 対 現行 | 100 ms | 113–87–0 | 56.50% | +45.4 | [+0.7, +90.2] |
| PVS＋LMR 対 PVS | 100 ms | 101–96–3 | 51.25% | +8.7 | [-39.1, +56.5] |

PVS 対現行は20 msでマイナス、100 msでプラスだった。95%信頼区間の0に近い側の端はそれぞれ −4.8 Elo、+0.7 Elo だった。PVS は固定深さでスコアを変えず、効果は探索速度を通じたものに限られる。持ち時間で向きが逆になり、どちらも区間の端が0に近いため、偶然のばらつきの可能性が高いとみて判定保留とする。LMR の追加効果は両持ち時間で信頼区間が0 Eloを含むため、強さの差の判定は保留とする。

200 plyの対局上限に達した4局と千日手2局は引き分けとして集計した。

## 実行コマンドと保存先

現行バイナリは実装前にビルドした。

```bash
run_dir="$PWD/target/search-pvs-lmr"
mkdir -p "$run_dir"
cargo build --offline --release -p haitaka_cli
cp target/release/haitaka_cli "$run_dir/baseline"
cargo build --offline --release -p haitaka_cli --features search-pvs
cp target/release/haitaka_cli "$run_dir/pvs"
cargo build --offline --release -p haitaka_cli --features search-lmr
cp target/release/haitaka_cli "$run_dir/lmr"
```

固定深さ計測に使用したスクリプトと生データは [`receipts/standard/fixed-depth.py`](receipts/standard/fixed-depth.py) と [`receipts/standard/fixed-depth.json`](receipts/standard/fixed-depth.json)。たとえば開始局面の深さ5は以下で再現できる。

```bash
for build in baseline pvs lmr
do
    printf '%s\n' isready 'position startpos' 'go depth 5' quit |
        "$run_dir/$build" usi --eval handcrafted
done
```

固定局面は既存の `ORDERING_TEST_OPENINGS` の順と qsearch の戦術フィクスチャ3件。

| 局面 | SFEN |
|---|---|
| opening-1 | `lnsg1gsnl/1r2k2b1/1ppppp+Bpp/p8/9/2P6/PP1PPPPPP/7R1/LNSGKGSNL b P 5` |
| opening-2 | `lnsgkgsnl/2r4b1/ppppp1ppp/5p3/7P1/9/PPPPPPP1P/1B5R1/LNSGKGSNL b - 5` |
| opening-3 | `lnsgk1snl/1r4gb1/pp1pppppp/2p6/9/9/PPPPPPPPP/1B2GK1R1/LNSG2SNL b - 5` |
| opening-4 | `lns1k1snl/1r1g1g1b1/ppppppppp/9/9/9/PPPPPPPPP/1B1RK4/LNSG1GSNL b - 5` |
| rook-capture | `9/9/k8/9/4Rr3/9/9/9/4K4 b - 1` |
| silver-promotion | `4k4/9/4S4/9/9/9/9/9/4K4 b - 1` |
| bishop-promotion | `4k4/9/4B4/9/9/9/9/9/4K4 b - 1` |

実際のコマンド（`*.command`）と対局ごとの集計（`*.report.json`）は [`receipts/standard/`](receipts/standard/) にある。棋譜（`self-play-games.jsonl`）、ログ、バイナリは計測した機械の `target/search-pvs-lmr/` にだけ残し、コミットしていない。4比較は順番に実施した。一部の対局は低優先度のテストビルドと並行した。

```bash
run_match() {
    local time_ms=$1 challenger=$2 opponent=$3
    "$run_dir/baseline" self-play \
        --games 200 --threads 4 --movetime-ms "$time_ms" \
        --opening-unit pair --opening-random-plies 4 --seed 1 --max-plies 200 \
        --a-engine "$run_dir/$challenger" --a-engine-arg usi --a-engine-arg=--eval --a-engine-arg handcrafted \
        --b-engine "$run_dir/$opponent" --b-engine-arg usi --b-engine-arg=--eval --b-engine-arg handcrafted \
        --report-dir "$run_dir/$challenger-vs-$opponent-${time_ms}ms"
}
run_match 20 pvs baseline
run_match 20 lmr pvs
run_match 100 pvs baseline
run_match 100 lmr pvs
```

| release バイナリ | SHA-256 |
|---|---|
| baseline | `e6a646e8679577c1bb816b39714c91dce21ed01418fa02f84faecb9bd698cee8` |
| pvs | `bca00ce155f8b6fe344d121dcae563ba4f8acb0d6136ecc8096fffb28eafd956` |
| lmr | `54fa057cfc2a374e97589029bb37533d2134135391ea50d194124b8531aba0c9` |

## 追加実験

変則5ルールの100 ms対局を別に集計した。 [代表ルールの実験結果](variants.md)

### 本将棋の1秒対局

`--movetime-ms 1000` で追試した。前回と同じ release バイナリ・手作り評価と、seed 1・ランダム初手順4 ply・4ワーカー・上限200 plyを使用した。各比較は200局、100先後ペア。棋譜から100 ms対局と同じ100初手順であること、全ペアで先後を反転したこと、集計値が一致することを確認した。最初の比較の一部は変則ルールのビルドと対局が並行した。

| A 対 B | A 勝–B 勝–引分 | A 得点率 | ペア Elo | 95% CI |
|---|---:|---:|---:|---:|
| PVS 対 現行 | 101–93–6 | 52.00% | +13.9 | [-31.2, +59.0] |
| PVS＋LMR 対 PVS | 114–79–7 | 58.75% | +61.4 | [+14.2, +108.6] |

PVS 単体の信頼区間は0 Eloを含んだ。LMR の追加は信頼区間の全体がプラスだった。ただし計測は1回で下限は +14.2 Elo と0に近い。既定化の前に、事前に決めた基準（SPRT など）で追試する。 200 plyに達した13局は引き分けとして集計した。

```bash
run_match 1000 pvs baseline
run_match 1000 lmr pvs
```

両比較の `*.command` と集計は [`receipts/standard/`](receipts/standard/) にある（`standard-1000ms-summary.json` は棋譜から再集計した値、`audit-1000ms.py` はその照合スクリプト）。棋譜とログはコミットしていない。
