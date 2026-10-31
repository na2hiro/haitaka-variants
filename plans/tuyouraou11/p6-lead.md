# P6 LEAD: SIMD 採用とローカル検証

> 2026-10-31 更新: mini の追加が明示指定された。下記 10/30 の SIMD-only 記録は履歴。
> 現在の実装・検証・選択方針は [p6-mini.md](p6-mini.md) を正とする。
> 今回の実機検証はユーザーが明示的に対象外とした。
> 棋力本検証400局も完了: [p6-strength.md](p6-strength.md)。

実施: 2026-10-30。対象は micro の SIMD 化だけ。ユーザーが P6 を明示指定し、
P4 で保留した iOS Safari/Android Chrome 実機検証は引き続き保留する。
ENGINE/WEB の実装・デスクトップ検証と追加 native 対局は完了。
実機の必須項目を満たした全体完了とは扱わない。公開デプロイ/コミットは行っていない。

## 採用

**SIMD128 対応時は SIMD 版、非対応/能力判定例外時は scalar 版を採用する。**
モデル・整数評価・同じ深さの探索結果を変えず、両ブラウザで速度が改善し、
メモリと P4 の時間/応答性予算も維持できたため。mini 追加やモデル変更は行わない。
モデルの選択肢や対応ルールは P4 後の拡張を維持する。

- ENGINE: [p6-engine.md](p6-engine.md)。WEB: [p6-web.md](p6-web.md)。
- ソース/配信 Wasm/native 実行ファイルの SHA-256: [artifacts.json](p6-results/artifacts.json)。
  base revision は dirty tree の開始点であり、その commit 単独でこの成果物を再現したとは記述しない。
- 公開 API/モデル descriptor とデータは変更なし。scalar/SIMD の配信は別々の content hash を持つ。
- 単一 Worker で、追加の SharedArrayBuffer、COOP/COEP 要件はない。

## 正しさと製品回帰

Node および Chromium/Firefox の実 Worker で、scalar/SIMD それぞれが 236 参照局面・2,048 手の
進行度和/bucket/整数評価値と、固定深さ 2 の 20 局面の指し手・評価値・探索ノード数で完全一致した。
[完全一致記録](p6-results/browser-comparison.json)。丸め差の許容はない。

- Rust 通常テスト 96 件、実モデル/TT/評価範囲、Annan 拒否、SFNN/旧 NNUE の Wasm SIMD カーネル合格。
  旧 NNUE の外部モデルは未配置で、その実ファイル回帰が通ったとは扱わない。
- Web 関連ユニット 90 件と、追加した実 Wasm target_features 回帰合格。型検査・ESLint・書式合格。
- 隔離した本番 build 合格。起動中の dev server 3002 番は保持した。
- 本番配信の Chromium/Firefox 各 16 件、計 32 件 E2E 合格。[E2E 記録](p6-results/e2e.json)。
  SIMD 有効/無効の実体ハッシュを照合し、いずれも実モデルで合法手を適用した。
  モデル切替、駒落ち・歩なし・将棋 120 系、HTTP 失敗と再試行、perft、ルール切替、待ったも確認。
- LAN の HTTP 開発環境でも、両ブラウザで scalar 分岐を確認。

## 性能

P4 と同じ Intel Core i7-6700 / Ubuntu、Chromium 153.0.8010.12、Firefox 142.0.1。
CPU は taskset 0,4、governor powersave。モデルのローカル HTTP/immutable 配信、
cold は新 context、warm は同 context の新 Worker。準備各 20 回、20 固定局面 × 各 3 回 ×200/1000/3000ms、
各 backend/browser で 180 探索、計 80 準備/720 探索。
corpus SHA-256 は P4 固定値 `14ebf71679d38aba4587652743f0403c32349f7c123b0f51ac1716d957f91e12`。
scalar/SIMD/browser の順に直列測定した。別 core で build/E2E/native 検証も実行しており、専用の無負荷測定ではない。

同じ局面・同じ movetime・同じ反復の NPS 比を取り、その中央値を集計した。

| movetime | Chromium SIMD/scalar | Firefox SIMD/scalar |
| -------- | -------------------: | ------------------: |
| 200ms    |              2.85 倍 |             3.67 倍 |
| 1000ms   |              2.45 倍 |             3.97 倍 |
| 3000ms   |              3.18 倍 |             4.32 倍 |

到達深さ中央値は Chromium で 200ms が 2→2.5、1000ms が 3→3、3000ms が 3→4。
Firefox は 200ms が 0→1、1000ms が 1→2、3000ms が 2→3。
各値は固定したランダム合法局面集合における結果で、全局面や実機の倍率を保証しない。

- 準備往復の最大: Chromium scalar 266.8ms / SIMD 274.8ms、Firefox scalar 617ms / SIMD 821ms。
- Wasm memory 最大: 4 組とも 55,771,136bytes（約 53.2MiB）。
- engine elapsed - movetime 最大: Chromium は 0 未満、Firefox は 0ms。
- main 往復超過最大: Chromium scalar 3.2ms / SIMD 6.8ms、Firefox は両方 3ms。
- 全 720 応答を独立した native 合法手生成器で照合。全て合法、main timer も全件進行。
- P4 凍結予算（準備 10 秒、memory128MiB、engine 超過 100ms、往復超過 200ms）を全件通過。
- 総 process RSS/JS heap のピークは未計測。Wasm memory と混同しない。

[生データと集計](p6-results/browser-summary.json)。
再集計: `python3 scripts/tuyouraou11-p6-summary.py browser --records-dir plans/tuyouraou11/p6-results`。
P4 の局面・予算・raw 結果は上書きしていない。

## 対局の範囲

同じ micro モデルを native scalar 固定版と runtime 検出による最適化版（この host では AVX2）へ読み込む。
P4 と同じ開幕集合の先頭 6 局面を先後交換し、200ms/1000ms 各 12 局、最大 256 手、seed=1、2 並列。
ハーネスが全手を合法性検証し、crash/手欠損を通常敗戦と混ぜない。
これは追加の小規模回帰で、Wasm SIMD128 の対局や P4 の 200 局比較の再実施ではない。
速度の採用判断は、実ブラウザの NPS 改善と整数/探索結果の完全一致を主な根拠とする。

最適化版から見て 200ms は 7 勝 5 敗、1000ms は 8 勝 3 敗 1 引分、計 15 勝 8 敗 1 引分。
スコア率 64.58%、時間別層化・開始局面ペア bootstrap 10,000 回（seed=1）の 95%区間は
50.00〜79.17%。P4 と同じ「区間上端<50%なら退行」の判定には該当しない。
24 局の小規模比較であり、棋力向上や非劣性を証明したとは扱わない。
crash・不正着手・非終局での指し手欠損は 0 件。全局で CLI が合法性を確認した。

[対局集計](p6-results/matches-summary.json)と`p6-results/match-{200,1000}/`の棋譜を保存した。
再集計: `python3 scripts/tuyouraou11-p6-summary.py matches --records-dir plans/tuyouraou11/p6-results`。

SIMD 実成果物では、駒落ち 13 種・歩なし・全 120 初形と他シャッフル各 16 初形の計 166 局面、
480 手も合法で、公式参照の進行度/評価値と一致した。
[拡張ルールの確認](p6-results/extended-rule-comparison.json)。

実機検証はユーザー指示どおり保留。この保留があるため、実機を含む P6 全体の完了とはしない。
