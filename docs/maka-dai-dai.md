# 摩訶大大将棋（ルール61）

2026-11-08。エンジン`d787c40`、将棋ったー連携`35491d15`。
将棋ったーの`makadaidaishogi.ts`と解決済み駒定義を採用する。
19×19盤・6ワードのビット盤・添字u16、76駒ID、初期192枚（片側96枚）。
金将の成り元22種を区別し、内部では98種類の駒を使う。手作り評価で探索する。

対局URL: <http://localhost:3002/rule/61>。相手に「将棋ったーAI」を選ぶ。
多段移動を続ける場合は次の移動先を直接選び、現在の駒を再クリックするとそこで終了する。

## 採用する成りと勝敗

- 取り捨て。通常は駒を取った移動駒が強制的に成り、すでに成っていればそのまま。
  静かな移動では成らない。行き所のない未成駒も保存・復元できる。
- 無明`ih`・法性`iz`を取ると法性`iz`、提婆`ii`・教王`jb`を取ると教王`jb`になる。
  通常の成り先がない駒や、すでに成った駒にもこの変化が起こる。
- 王将`ah`・王子`iy`は特殊成りせず同じ駒種を保つ。
  醉象`cf`は特殊成りを避け、通常成りで王子になる。
- 王将は普通の駒を取ると、王相当でない自在天王`ix`になる。
  ほかに王子がなければ王相当を失って負けになる。双方が失えば引き分け。
- 獅子・狛犬・法性・教王などの多段移動では、成りを完了・パスまで遅らせる。
  後から取った特殊駒の成り先を優先し、普通の駒の捕獲やパスでは直前の特殊成り先を保つ。
- 継続可能な途中の移動で最後の敵の王相当を取れば、その時点で終局し、遅延中の成りを追加しない。
- 王相当は王将と王子の集合。王手放置を許す。王手が掛かっていない場合の手詰まりを負けにする。
  四回反復・連続王手は完了した手で判定し、先獅子と成り元を局面の区別に含める。

自在天王は任意の空きマスへ移動でき、守られていない敵駒を取れる。
守りを調べる際は敵の自在天王の利きを除き、ほかの敵駒の参照定義どおりの利きを使う。
鉤行の曲がる走り、狛犬の三回移動・帰還・飛び越し条件、獅子取りの例外も扱う。
付け喰いには歩`aa`と仲人`cq`を使えない。

## JSON・成り元・途中再開

`haitaka-json-v4`、`schemaVersion: 4`、`ruleId: 61`を使う。
独自の駒IDをSFEN/USI文字に置き換えない。座標は1始まりで、[19,19]の添字は360。

```json
{"schemaVersion":4,"from":[9,9],"path":[[9,8],[9,7]],"promotion":true}
```

要求は`position`、`history: {initial, moves}`、`pending`。
状態は`{"senjishi":false,"shitenno":[]}`で、持ち駒は空。
`promotion:true`は通常成り・特殊成りの実行を表し、成り先は捕獲列から決まる。
すでに法性の駒が無明を取って再び法性になる場合もtrueになる。

金将への通常成りには`origin`を保持する。

```json
{"at":[9,8],"piece":"do","color":0,"origin":"ag"}
```

特殊成りは成り元を消す。複数の元駒が同じ特殊駒へ変わり得るため、
駒種別の初期枚数を上限にせず、片側96枚の総数で検証する。
未知の駒・成り元、空でない持ち駒・四天王状態は拒否する。

途中再開は最後の完了局面と`pending: {from, path}`を渡す。
履歴と捕獲列から成り先を再計算し、保存された`promotionTarget`とも照合する。
workerは一度選択した完了経路を保持し、残りの移動・終了パスを順に送る。
検索の適用・復元は変更マスと状態の差分を使い、各枝で盤全体をコピーしない。

## 参照実装で修正したパス・駒クラス処理

多段移動の終了パスを、盤上の自分自身の捕獲と誤認していた処理を修正した。
これにより、法性・教王の静かな移動が成り扱いになる問題と、
取った相手から決まる特殊成り先が自分の種類で上書きされる問題を防ぐ。
また、通常成りで設定した成り元をパスの後処理で消さないようにした。
特殊成りで成り元を消す処理は特殊成り自身が行う。専用の回帰テストで確認する。
成りで自在天王へ入る・そこから特殊成りで出る際には、同じ駒オブジェクトのクラスも更新する。
保存・再読込を挟まなくても、新しい駒種の動きをその直後から使える。
パスの5ケースとクラス切り替えの2ケース、計7ケースで検証する。

## ビルドと再検証

```sh
cargo build --release -p haitaka_cli --features maka-dai-dai
cargo run --release -p haitaka_cli --features maka-dai-dai -- json startpos
cargo build --release -p haitaka_wasm --target wasm32-unknown-unknown --features maka-dai-dai
wasm-bindgen --target web --out-dir target/wasm-maka-dai-dai target/wasm32-unknown-unknown/release/haitaka_wasm.wasm
```

CLIの`package --wasm-dir target/wasm-maka-dai-dai --output target/haitaka-maka-dai-dai.tgz`で配布できる。
ブラウザ組込用は`wasm-bindgen --out-name haitaka_engine`で直接生成する。
NNUE/SFNN、USI、別ルールIDの指定は拒否する。
駒定義は将棋ったーの`app/bot/makaDaiDaiReference.test.ts`を`MAKA_DAI_DAI_DEFINITIONS=...`で出力し、
`python3 scripts/generate-tenjiku-dai-defs.py target/maka-dai-dai-definitions.json --ruleset maka-dai-dai`で再生成する。

```sh
MAKA_ORACLE=1 MAKA_POSITIONS=128 pnpm exec vitest run app/bot/makaDaiDaiReference.test.ts
MAKA_PERFT=1 pnpm exec vitest run app/bot/makaDaiDaiReference.test.ts
node scripts/verify-maka-dai-dai.mjs target
```

全参照比較は通常テストと分けて明示実行する。
WASM初期化5秒、線形メモリ64MiB、対局思考5秒を予算とし、1msの制限も検証する。


## 検証結果

396固定局面と128ランダム局面、計524局面・33,383完了手遷移が参照と一致した。
全76駒の先後別移動、47通常成り元と22金将成り元を先後とも確認した。
本比較520局面に、桂馬の捕獲成り2局面と自在天王の変化2局面を補い、
クラス切り替えに関わる既存局面も再検証している。
独立perftは80 / 6,400。実WASMでも全524局面の遷移・結果・ハッシュ・履歴・探索が一致した。

WASMは345,527 bytes、線形メモリ1,245,184 bytes、最終照合の初期化約20.5ms、
1ms制限の実測約1.00msで、設定した予算を満たした。
Chromeで新規8ケース・既存56ケースが通った。
core 2,577件、AI CI範囲434件、アプリ通常709件と型検査・lintも通った。
Rust releaseワークスペースと拡張36件が通り、大大将棋20,014遷移・天竺大将棋77,025遷移は
変更前と完全一致。本将棋のバイナリもバイト単位で一致した。

[機械可読の検証記録](../plans/historical-shogi/05-maka-dai-dai-verification.json)を参照。
第5段階は進行中。次は泰将棋（63）。
