# 大局将棋（ルール64）

将棋ったーの解決済み定義を採用する大局将棋の専用エンジン。Cargo featureとpackage rulesetは`taikyoku`。
対局URLは <http://localhost:3002/rule/64>。既存の開発サーバーを使用する。

## 採用仕様

- 36×36、21ワードの固定長ビット盤。添字はu16で、最終マス1295まで扱う。
- 初期804枚（片側402枚）、209元駒、356駒ID、195通常成り元。
- 成り元が複数ある31種類の成駒は、79種類の成り元を識別する。内部IDは435で、256以上も保持する。
- 王相当は玉将`wz/zx`と太子`cg/uc`。太子`cg`→玉将`zx`、醉象`cf`→太子`uc`。
  同名の駒も別IDとして扱う。最後の王相当を失えば負け、双方の同時喪失は引き分け。
- 敵陣11段での通常成り。成り・不成を選べ、行き所のない駒は成りが強制される。
  成り可否は連続移動を通じて保持し、完了・パス時に適用する。
  摩訶大大将棋・泰将棋方式の捕獲成りや特殊対象による成りは使わない。
- 獅子取り規定の対象は`bl/dh`。付け喰いに歩`aa`と仲人`cq`は使えない。
  非獅子による獅子取り（先獅子）を、次の手の合法性と履歴・ハッシュへ反映する。
- 奔鷲`tn`は14種類の途中状態を持ち、最大4回移動する。
  例えば`[18,18]→[17,17]→[16,16]→[15,15]→[14,14]`は一つの完了手。
  途中の最後の王捕獲では、その時点で終局し、追加の移動や成りを行わない。
- 天狗・鉤行等の曲がる走りは、空きマスで一度直角に曲がれる一段の移動として扱う。
- 鳩槃`ni/zy`の途中の空きマス条件、金翅・大象・奔翅・教王の有限跳び越しを扱う。
  `jumpLimit:3`は移動距離でなく途中の駒の数を制限する。
- 猛龍・大将・角将・飛将・副将・飛鰐のtype100移動では、飛び越えた途中の駒を色に関係なく捕獲する。
  参照定義に列挙された跳び越し禁止駒には着地できず、その先にも進めない。
- 火鬼・四天王の順序付き効果や持ち駒は使わない。`state.shitenno`は空でなければならない。
- 王手放置と王手を受けた状態での移動を許す。王手が掛かっていないステイルメイトは負け。
  四回反復と連続王手の判定には、完了手・先獅子・成り元を含める。

一般的な古将棋解説から推測して補正せず、`packages/core/src/db/rules/taikyokushogi.ts`、
`db/koma.ts`とstrategyの実装を基準にする。

## JSON v5

JSON v4は最大3回移動であり、大局将棋には`haitaka-json-v5`を使う。
ルールIDは64、局面・指し手・探索結果の`schemaVersion`は5。
既存ルールのJSON v1〜v4はそのまま維持する。

```json
{
  "schemaVersion": 5,
  "from": [18, 18],
  "path": [[17, 17], [16, 16], [15, 15], [14, 14]],
  "promotion": false
}
```

`position`は完了済みの局面、`history.initial`と`history.moves`は完了手単位の履歴。
途中からの再開では、完了済み局面に対する`pending: {from, path}`を渡す。
`pending.path`は最長3段。途中の見えている盤面を完了局面として受け入れない。
履歴再生から途中状態・成り可否・成り先を検証する。

曖昧な成駒には`origin`を保持し、ハッシュ・JSON往復・履歴再生へ含める。
将棋ったーのbotでは、完了手を表示用の各移動へ分解し、必要な終了パスも含めて一度の探索結果を実行する。
待った・中断・エンジン切替では残りの移動を破棄する。

Native/WASMは`start_position_json`・`validate_position_json`・`inspect_json`・`legal_moves_json`・
`apply_move_json`・`perft_json`・`search_json`を提供する。
手作り評価を使い、NNUE/SFNN・USI・別ルールID・JSON v4入力は拒否する。

探索の各枝は変更マス・状態・手番を差分で戻す。一括捕獲も全除去マスを戻す。
盤全体のコピーを各枝に追加しない。経路列挙中にも時間とノード制限を調べ、
中断時は合法な完了済みの手へフォールバックする。

## ビルドと検証

```sh
cargo build --release -p haitaka_cli --features taikyoku
cp target/release/haitaka_cli target/haitaka-taikyoku
cargo run --release -p haitaka_cli --features taikyoku -- json startpos
cargo build --release -p haitaka_wasm --target wasm32-unknown-unknown --features taikyoku
wasm-bindgen --target web --out-dir target/wasm-taikyoku target/wasm32-unknown-unknown/release/haitaka_wasm.wasm
```

ブラウザ組込用は`wasm-bindgen --out-name haitaka_engine`で直接生成する。
CLIの`package --wasm-dir target/wasm-taikyoku --output target/haitaka-taikyoku.tgz`でパッケージ化する。
同梱マニフェストのpositionFormat/moveFormat/capabilitiesはすべて`haitaka-json-v5`。

将棋ったーの`app/bot/taikyokuReference.test.ts`を`TAIKYOKU_DEFINITIONS=...`で実行し、駒定義を出力する。

```sh
python3 scripts/generate-tenjiku-dai-defs.py target/taikyoku-definitions.json --ruleset taikyoku
node scripts/verify-taikyoku-reference.mjs target ../shogitter-3
TAIKYOKU_PERFT=1 pnpm exec vitest run app/bot/taikyokuReference.test.ts
node scripts/verify-taikyoku-coverage.mjs target
node scripts/verify-taikyoku.mjs target
```

全参照比較と独立perftは通常の単体テストから分離し、明示実行する。
`TAIKYOKU_PERFT`のコマンドは将棋ったーのリポジトリ上で実行する。
参照比較のスクリプトはNative出力を一度生成してから、既定4プロセスで全局面を分担し、
すべての合法手・遷移・勝敗の一致を照合する。
Nativeの大きな出力はストリームで保存・比較する。
`TAIKYOKU_REUSE_ORACLE=1`では保存済み入力が生成した入力と完全一致することを確認して出力を再利用する。

WASM初期化5秒・線形メモリ64MiB・対局思考5秒を予算とし、1ms制限も確認する。
強さの調整・専用モデルの学習は別作業。

大きな履歴のWASM検証は、独立した局面を既定4ワーカーに分ける。
`TAIKYOKU_WASM_WORKERS=1`で逐次実行にもでき、検証内容は同じ。

将棋ったーのステイルメイト判定では、大局将棋に限り、まず合法な応手の存在を調べる。
応手がある場合は王の利きの全盤計算を省ける。応手がない場合は、従来どおり王の移動と王手を確認して判定する。
「王についての条件を満たし、かつ合法な応手がない」という判定の内容を維持し、確認する順序だけを変える。

## 完了記録（2026-11-08）

エンジン`2ba032d`、将棋ったー`bdec1bbd`。
2,071固定局面＋128ランダム局面、2,199局面・253,099完了手遷移が参照実装と一致した。
356駒・195成り元・79成り元識別を先後とも覆い、4回移動280遷移と1〜3段の途中再開を確認した。
独立した参照perftは282 / 79,035。実WASMでも全遷移・結果・ハッシュ・履歴再生がNativeと一致した。

WASMは403,549 bytes、初期化約3.6ms、線形メモリ1,703,936 bytes。
1ms制限の初期局面・途中局面で合法な完了手を返した。
固定1,000ノードのNative/WASM探索も一致した。これは探索の再現性の確認で、棋力の保証ではない。

Chromeで大局将棋10件と既存72件、Rust releaseワークスペース・拡張39件、
core 2,580件、AI CI範囲436件、アプリ通常726件、JSON連携43件、型検査・lintが通った。
既存の天竺大・大大・摩訶大大・泰将棋の計2,283局面・204,681遷移は変更前と完全一致し、
本将棋のバイナリもバイト単位で同一だった。

[機械可読の検証記録](../plans/historical-shogi/05-taikyoku-verification.json)を参照。
[古将棋全14ルールのロードマップ](../plans/historical-shogi/README.md)は第5段階まで完了。

大きなNative検証出力は検証後にgzipで保存し、展開前後のSHA-256を照合した。
キャッシュを再利用する場合は先に`gzip -dk target/taikyoku-reference-output.jsonl.gz`で展開する。
