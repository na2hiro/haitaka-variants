# 泰将棋（ルール63）

2026-11-08。エンジン`2ab6d3a`、将棋ったー連携`c9f3f5ce`。
将棋ったーの`taishogi.ts`と解決済み駒定義を採用する。別名は無上泰将棋。
25×25盤、10ワードのビット盤、添字u16。初期354枚（片側177枚）、成駒を含む130駒IDを使う。
金将の成り元51種と大象の成り元2種を区別するため、内部では183種類の駒を使う。

対局URL: <http://localhost:3002/rule/63>。相手に「将棋ったーAI」を選ぶ。
多段移動を続ける場合は次の移動先を直接選び、現在の駒を再クリックするとそこで終了する。

## 採用規則

- 取り捨て。通常は駒を取った移動駒が強制的に成る。静かな移動では成らない。
- 無明`ih`・法性`iz`を取ると法性`iz`、提婆`ii`・教王`jb`を取ると教王`jb`になる。
  既成駒や通常の成り先がない駒も、この特殊成りの対象になる。
- 王相当は自在天王`ix`、初期配置の太子`cg`、醉象の成駒の太子`uc`。
  `cg`と`uc`は参照では同名だが別IDであり、成り先・初期配置・王の残存を混同しない。
- 王相当の3種類は特殊成りせず同じ駒種を保つ。醉象`cf`は特殊成りを避けて`uc`に成る。
- 自在天王は王相当の駒であり、任意の空きマスへ移動できる。
  敵の自在天王以外の利きで守られていない敵駒を取れる。
- 獅子・狛犬・法性・教王・角鷹・飛鷲などの継続状態を使い、完了した1手を最長3段の経路で表す。
  捕獲による成りを完了・パスまで遅らせ、後から取った特殊駒の成り先を優先する。
  普通の捕獲やパスでは直前の特殊成り先を保つ。
- 継続可能な途中の移動で最後の王相当を取れば、その時点で終局し、遅延中の成りを追加しない。
  王相当が他に残っていれば対局を続ける。王手放置を許容する。
- 王手が掛かっていない場合の手詰まりを負けとする。四回反復・連続王手は完了した手単位で判定する。
  獅子取りの対象は`bl`と`dh`。付け喰いには歩`aa`と仲人`cq`を使えない。
- 鉤行・天狗の曲がる走り、有限走り、飛び越し条件、同名の別ID・成り元を解決済み定義どおりに扱う。

## JSONと途中再開

`haitaka-json-v4`、`schemaVersion: 4`、`ruleId: 63`。
独自駒をSFEN/USI文字に置き換えない。座標は1始まりで、[25,25]の添字は624。

```json
{"schemaVersion":4,"from":[13,13],"path":[[13,12],[13,11],[13,10]],"promotion":true}
```

要求は`position`、`history: {initial, moves}`、`pending`。
状態は`{"senjishi":false,"shitenno":[]}`、持ち駒は空。
通常成りで金将`do`や大象`id`になった駒には、成り元の`origin`を保持する。

```json
{"at":[14,12],"piece":"id","color":0,"origin":"gn"}
```

特殊成りは成り元を消す。変化によって駒種別の枚数が増え得るため、初期の駒種別上限を使わず、
片側177枚の総数で検証する。未知の駒・成り元・ルール設定、持ち駒、空でない四天王状態は拒否する。

途中再開には最後の完了局面と`pending: {from, path}`を渡す。
履歴から成り先・成り資格を復元し、保存データの`promotionTarget`・`promoElig`と照合する。
workerは選んだ経路を保持し、途中で再探索しない。
JSONの捕獲変化処理は、ルールID・盤サイズ・駒定義を渡す形で摩訶大大将棋と共通化している。

## 大盤の処理と検証方法

探索では変更したマス・状態・手番の差分を保存・復元し、各枝で625マスの盤全体をコピーしない。
自在天王の移動先が空きマスと分かっている履歴の検証では、敵の守りの全盤計算を省く。
敵駒を取る場合は必要な守りを調べ、全合法手を生成した結果と一致することをテストする。
火鬼・四天王の効果を持たないルールでは、その計算も省く。

全合法候補を参照実装で実行し、完了した手の駒配置・成り元・効果状態・結果を比較する。
ランダム局面は実初期盤から合法な候補を選んで完了手単位で作る。
大きなNative出力はストリームで保存し、比較時も1局面ずつ読む。
長い独立perft計算は定期的にイベントループへ制御を戻す。

## ビルドと再検証

```sh
cargo build --release -p haitaka_cli --features taishogi
cargo run --release -p haitaka_cli --features taishogi -- json startpos
cargo build --release -p haitaka_wasm --target wasm32-unknown-unknown --features taishogi
wasm-bindgen --target web --out-dir target/wasm-taishogi target/wasm32-unknown-unknown/release/haitaka_wasm.wasm
```

CLIの`package --wasm-dir target/wasm-taishogi --output target/haitaka-taishogi.tgz`でパッケージ化する。
ブラウザ組込用は`wasm-bindgen --out-name haitaka_engine`で直接生成する。
まず手作り評価を使い、NNUE/SFNN・USI・別ルールIDの指定は拒否する。

将棋ったーの`app/bot/taishogiReference.test.ts`を`TAISHOGI_DEFINITIONS=...`で実行して駒定義を出力し、
共通スクリプトで再生成する。

```sh
python3 scripts/generate-tenjiku-dai-defs.py target/taishogi-definitions.json --ruleset taishogi
TAI_ORACLE=1 TAI_POSITIONS=128 pnpm exec vitest run app/bot/taishogiReference.test.ts
TAI_PERFT=1 pnpm exec vitest run app/bot/taishogiReference.test.ts
node scripts/verify-taishogi.mjs target
```

全参照比較・独立perftは通常の単体テストと分けて明示実行する。
WASM初期化5秒・線形メモリ64MiB・対局思考5秒を予算とし、1ms制限も確認する。


## 検証結果

796固定局面と128ランダム局面、計924局面・74,259完了手遷移が参照と一致した。
全130駒・88通常成り元、金将51種・大象2種の成り元を先後とも確認した。
独立した初期局面perftは304 / 92,676。実WASMでも全遷移・結果・ハッシュ・履歴・探索が一致した。

WASMは342,832 bytes、線形メモリ1,507,328 bytes、初期化約3.0ms、
1ms制限の実測約1.00msで予算内。Chrome新規8件・既存64件が通った。
127手の履歴復元はNativeのCLI起動を含む8回の中央値で約67.9msから約2.8msになり、
全924局面・74,259遷移が最適化前と一致した。

Rust releaseワークスペースと拡張38件、core 2,577件、AI CI範囲434件、アプリ通常716件、
JSON v4連携33件と型検査・lintが通った。
摩訶大大・大大・天竺大将棋の計130,422遷移も変更前と一致し、本将棋のバイナリも同一だった。

[機械可読の検証記録](../plans/historical-shogi/05-taishogi-verification.json)を参照。
第5段階は進行中。次は大局将棋（64）。
