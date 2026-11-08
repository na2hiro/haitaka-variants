# 第5段階と各段階の検証条件

状態: 実行中。14ルールの現行Native/WASM再生成、全体テスト、最終の前後測定を行う。

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

## 最終比較の準備（継続実行中）

最適化前のe7ab6efをtarget/perft-originalに独立checkoutした。
scripts/perft-benchmark.pyの--engine-dirで、そのcheckoutと最終実装を同一Rustハーネスで測る。
Rust側の計時ループ・葉まで適用する条件は変更しない。

途中局面はscripts/perft-position-benchmark.pyを使う。
元のperft-benchmark.rsのimportsとmeasure()をそのまま共有し、JSONの復元を計時から除く。
参照コーパスから固定seed対局の途中、疎な多駒移動/特殊効果、獅子・奔鷲のpendingを選び、
入力JSON・SHA・由来のコーパスと行番号を保存する。--calibrateを最適化前のcheckoutで実行し、
期待葉数を凍結してから前後を各3秒以上×3回で比較する。

フロントエンドは既存のlocalhost:3002を利用する。generatedのWASMとbindgen JSを同時更新し、
core・AI CI・通常app・typecheck・lintと、既存の古将棋82件を含むChromeテストを行う。
ユーザーの指示により第5段階まで続け、クリティカルな問題以外で段階ごとに停止しない。
