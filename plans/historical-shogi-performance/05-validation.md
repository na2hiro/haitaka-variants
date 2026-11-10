# 第5段階と各段階の検証条件

状態: 完了。全14ルールの新Native/WASM・6,209局面/526,295遷移、
Rust/core/AI/app/型/lint、Chrome82件、Native32・固定16・実WASM11の前後比較、
全14ルールの通常5秒思考と64MiB制約が通過した。
[最終記録](results/05-final.json) / [比較表](../../docs/historical-shogi-performance.md)。

## 正しさを先に確認

- 修正前の合法手・遷移・結果を独立した参照実装と照合する。perftの葉数一致だけでは完了しない。
- [古将棋の記録](../historical-shogi/README.md)からルール別のビルド・参照比較・WASM再生成手順を読む。
  共通処理を変更した場合、直接の対象だけでなく呼出元のルールへ回帰を広げる。
- 固定特殊局面と各ルール128以上の固定seed局面を使い、適用・取り消し・成り元・王の集合・終局を比較する。
- Native/WASMで全合法手と全遷移、ハッシュ/履歴/pendingの復元、固定ノード探索を確認する。
- Nativeの結果を生成せず古いファイルを使う場合、そのファイルが変更前比較用であることを明記する。
  実装後のNative/WASM一致に古いNative出力を誤用しない。

## 測定

初期局面32ルールはscripts/perft-benchmark.pyを使い、保存済みの深さ・葉数を固定する。
末端まで適用するperftを最低3秒×3回測り、全サンプルと中央値を保存する。
追加で固定seedの途中局面、全駒の利きが増える局面、獅子/奔鷲の継続中、特殊効果直前を選び、
入力JSON/SFEN・履歴・SHA-256・深さ・期待葉数を固定する。途中局面を測れる入力オプションは
測定器へ必要になった段階で追加し、初期局面と同じタイミング範囲を使う。

実WASMは同じChrome/Node・同じ入力を使い、局面復元込みのJSON APIと内部perftを区別する。
初期化5秒・線形メモリ64MiB・通常思考5秒・低予算1msの既存基準を維持する。
1msの実測超過も値を記録し、途中局面を含め合法な完了手を返すことを必須にする。
NPS向上をそのまま棋力向上とは扱わない。

## 実行の例

```sh
cargo test --release --workspace
cargo test --release -p haitaka --features washogi extended
cargo test --release -p haitaka --features heian-dai extended
# 他ルールも1featureずつ同様に実行する。featureを同時指定しない。
python3 scripts/perft-benchmark.py --output target/perft-final.json
```

大局将棋の全比較を行う場合はNative/WASMを当該変更から再生成し、専用バイナリを保存してから
`scripts/verify-taikyoku-reference.mjs`・`verify-taikyoku-coverage.mjs`・`verify-taikyoku.mjs`を使う。
既存の圧縮oracleは容量を確認してから展開し、参照記録のSHA-256を確認する。
その他のルールは対応するdocs・plansにある専用スクリプトとfixtureを使う。

将棋ったーを変更した場合はcore・AI CI・通常app・typecheck・lintを実行する。
ブラウザは既存 http://localhost:3002 を使い、pnpm devを追加起動しない。
通常対局・途中再開・4回移動・待った・終局・ルール切替を新旧ルールで確認する。

## 完了条件と記録

各段階の採用変更について、正しさの不一致ゼロ、同じ入力/葉数での前後性能、非対象への回帰確認が揃う。
採用しなかった試作も理由を記録する。前後の結果・コマンド・環境・コミットをresultsへ保存し、
READMEの進捗を更新する。最後に全対応表・ビルド手順・API互換性・計測済み改善率を更新する。
対象変更だけを明示的にコミットし、学習関連など既存のユーザー変更は残す。

## 最終比較の実行条件

最適化前のe7ab6efをtarget/perft-originalに独立checkoutした。
scripts/perft-benchmark.pyの--engine-dirで、そのcheckoutと最終実装を同一Rustハーネスで測る。
Rust側の計時ループ・葉まで適用する条件は変更しない。

途中局面はscripts/perft-position-benchmark.pyを使う。
元のperft-benchmark.rsのimportsとmeasure()をそのまま共有し、JSONの復元を計時から除く。
参照コーパスから固定seed対局の途中、疎な多駒移動/特殊効果、獅子・奔鷲のpendingを選び、
入力JSON・SHA・由来のコーパスと行番号を保存する。--calibrateを最適化前のcheckoutで実行し、
期待葉数を凍結してから前後を各3秒以上×3回で比較する。

フロントエンドは既存のlocalhost:3002を利用した。generatedのWASMとbindgen JSを同時更新し、
core・AI CI・通常app・typecheck・lintと、古将棋77件と小盤5件、計82件のChromeテストを行う。
ユーザーの指示により第5段階まで続け、クリティカルな問題以外で段階ごとに停止しない。

最終Native比較はtarget/perft-originalをe7ab6efとd95f3b3へ順に切り替え、
両版を同一パスでビルドする。crate識別子・コード配置へパス差が混じる条件を避ける。
初期32ルールも固定16局面も、engineDirectory・依存ロック・ハーネス・CPUを一致させる。
独立checkoutを使うため、ユーザーの作業ブランチや学習関連変更は切り替えない。

初期32ルールの最終比較は全葉数が一致し、各3秒以上×3回、全サンプル幅5%以内。
対象8ルールは1.39〜4.11倍、非対象に5%以上の低下なし。
非対象24ルール中23は前後バイナリも完全一致した。
[変更前](results/05-native-before.json) / [変更後](results/05-native-after.json) /
[生成物の一致と非対象確認](results/05-native-binary-identity.json)。
固定16局面・実WASM11ケースの比較も完了し、全サンプル幅5%以内。
通常5秒予算も全14ルールで合法な完了手、初期化5秒・64MiB以内を確認した。
