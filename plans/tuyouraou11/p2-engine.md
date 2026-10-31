# P2 / ENGINE 結果

実施: 2026-10-30。状態: 完了。初回導入全体は未完了。
P1を428e32fにコミット後、feat/tuyouraou11-p2を作成（base=428e32f）。
P2成果物は本ブランチのP2コミットに収録。WEBの変更・P3報告は変更していない。

## 実装と公開契約

- sfnn.rs: immutableなモデルと、2つのaccumulator/Q16和だけを所有するSfnnPositionStateを分離。合法手の差分更新・独立分岐を実装。モデル重みはノードごとに複製しない。
- K+A2の玉featureは定数行の差分更新。progressは動いた玉側の和のみ全再計算し、他方は非玉featureの差分。成り/捕獲/持駒/打ち/先後交代/bucket変更を扱う。
- lib.rs: Handcrafted/HalfKAv2/SFNNをEvaluationStrategyで切替。既存HalfKAv2状態はinlineのまま保持し、SFNN状態を探索・qsearchへ接続。
- moduleおよびUsiEngineに同期 `load_sfnn(nn_bytes, progress_bytes, arch): string` を公開。microのarchを返す。nativeのload_sfnn_impl/UsiSession::load_sfnnも同じ順序・エラー契約。
- NNとprogressの検証後に一括交換。成功時に所有TTをclear、失敗時に前モデル/TTを維持。globalとsessionは独立。旧load_nnue ABIを維持し、読み込んだ評価器がそのscopeの前評価器を置換する。
- SFNN静的値は参照どおり±31753。探索へ渡す際だけ±28975へ制限し、mate/TT変換へ誤分類されないようにした。静的fixtureの値は変えない。
- sfnn_usi example: 同じUsiSession/探索でhandcraftedとmicroを選べるP4用native harness。package v2の変更なし。
- sfnn-diagnosticsはテスト専用feature。通常配布版にtrace exportなし。型定義で両load_sfnnのUint8Array/Uint8Array/string→stringを確認。

## 検証結果

作業ディレクトリ: ~/proj/haitaka-variants。必要モデルはtarget/tuyouraou11/model/。
コマンドの詳細・再現手順はdocs/tuyouraou11-sfnn.mdにも記録。

| 検証 | 実行 | 結果 |
|---|---|---|
| P1参照+差分/探索+runtime | python3 scripts/tuyouraou11.py verify | integration 2件、runtime 2件PASS。ignoreを合格扱いしていない |
| 通常回帰 | cargo test -p haitaka_wasm --release | 95 PASS、0 fail。実モデル専用4件は別途上でPASS |
| Annan回帰 | cargo test -p haitaka_wasm --release --features annan | 91 PASS、0 fail |
| NekoNeko回帰 | cargo test -p haitaka_wasm --release --features nekoneko | fixture修正後85 PASS、0 fail |
| 既存256幅NNUE | cargo test -p haitaka_wasm --release --features nnue-l1-256 | 95 PASS、0 fail |
| 修正終局fixture | 通常/Annanの --lib usi_session_marks_terminal_resign_as_no_legal_move | 各1 PASS。NekoNekoは上の全suiteでPASS |
| Native harness | cargo build -p haitaka_wasm --release --example sfnn_usi、およびUSI smoke | 両評価器で合法bestmove、score 12/58 |
| 診断Wasm | wasm-pack build haitaka_wasm --target web --out-dir ../target/tuyouraou11/pkg-diagnostics --release --no-opt --features sfnn-diagnostics | PASS |
| 通常Wasm | wasm-pack build haitaka_wasm --target web --out-dir ../target/tuyouraou11/pkg --release --no-opt | PASS |
| Node実Wasm | node scripts/tuyouraou11-wasm.mjs | 236静的fixture + 2048差分手 + runtime API PASS |
| Chromium実Worker | 同コマンドに --browser（HAITAKA_CHROMIUM_PATH指定） | Chromium 153.0.8010.12、同236/2048一致、main thread timer継続 |
| 通常配布版実Wasm | node scripts/tuyouraou11-wasm.mjs target/tuyouraou11/pkg --production --browser | Node/Chromiumで両load API、保持、session分離、深さ/時間探索PASS |
| 書式/構文 | rustfmt --check、node --check、Python AST、git diff --check | PASS |

差分検証はLCG64 seed=1、2,048手。捕獲349、打ち318、成り92、玉移動187、bucket変更258。
毎手の全accumulator/Q16和/bucket/評価値と、同じ親からの独立分岐が全再計算と完全一致。
代表4局面・深さ2で全再計算/差分のroot resultとノード数が一致。
P1の236静的fixtureも参照値に完全一致したまま。

RuntimeテストはTTへ識別entryを入れて失敗時保持/成功時消去を確認。global交換でsession TTを消さず、session交換でglobal TTを消さない。モデルのArc identityも独立。別モデルの新しいsessionはhandcraftedで始まる。
巨大な出力biasの合成モデルで、参照静的clipと探索mate範囲の分離も確認。

ログはtarget/tuyouraou11/p2-{reference,runtime,all-tests,annan,nekoneko,compact,final-native,browser-wasm,production-wasm}.logなど。通常テストに出る既存dead_code等のwarningは今回の失敗ではない。

## 回帰テストの既存fixture修正

NekoNekoの既存終局テストは、王に隣接する敵飛車から王が飛車の動きを受け取り、1a6aで逃げられる配置だった。P1コミット428e32fの隔離snapshotでも同じテスト失敗を再現（p2-nekoneko-baseline.log）。

終局通知の期待値や製品ルールは変えず、飛車を1bではなく1dへ打つ詰み配置へテストデータだけ修正した。通常/Annan/NekoNekoで正しい終局を確認。haitakaの合法手生成・変則ルール本体に変更なし。

## P4への引継ぎ

- 通常Wasm/glue/型定義: target/tuyouraou11/pkg/。診断版: pkg-diagnostics/。
- サイズ/hash/build version: p2-artifacts.json（現在のP2作業treeから生成、SIMD要求なし）。
- P3の同期scriptから再生成する場合は、このP2 checkoutをHAITAKA_ENGINE_DIRで指定する。生成物のみ手でrename/差替えしない。既存同期scriptはout-name haitaka_engineで生成する。
- この環境のwasm-bindgen 0.2.118は ~/.cache/.wasm-pack/wasm-bindgen-9daeccfa8e093372/ にある。同期scriptのPATHにwasm-bindgenがない場合は、このcache dirをPATHに足すか同じ版を用意する。wasm-packはcacheを利用済み。
- モデルID/provenance/2bin hashはP1から変更なし。WEB側のP3とAPIの契約差分なし。
- P4 native比較はtarget/release/examples/sfnn_usiの引数有無で選択。--model-dirは絶対パスを推奨。
- ENGINEのP2終了条件を全て満たす。P3完了報告を前提に、次はLEAD/P4の実Wasm同期・将棋ったー画面E2E・実機予算/測定・200局比較。
- ここで実行したのは独立したengine Worker検証。将棋ったー本体のブラウザE2EやSafari/iOS/Android実機検証、cold/warm予算、200局対局、公開デプロイは未実施。P4完了とはしない。
