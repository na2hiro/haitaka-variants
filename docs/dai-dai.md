# 大大将棋（ルール60）

2026-11-08。エンジン`cf92024`、将棋ったー連携`ee96c693`。
将棋ったーの解決済みルール`daidaishogi.ts`と駒定義を採用する。
17×17盤、5ワードのビット盤、添字u16。64種類の元駒と21種類の成駒を区別し、
初期配置は192枚（片側96枚）。取り捨て、王将`ah`の捕獲で勝利し、王手放置を許容する。

対局URL: <http://localhost:3002/rule/60>。相手に「将棋ったーAI」を選ぶ。
多段移動の継続は次の移動先を直接選ぶ。現在の駒を再クリックするとそこで終了する。
駒を取った移動駒に成り先があれば、終了時に自動で成る。

## 採用規則

- 捕獲した移動駒に成り先があれば強制成り。静かな移動では成らない。
  成り表がnullの駒、すでに成った駒は捕獲後も同じ駒種を保つ。
- 多段移動では、最初に捕獲しても元の駒種の動きを続け、最後の移動またはパスで成る。
  継続可能な段階で王を取れば、その時点で成りの前に終局する。
- Capture方式では成らない歩・香も最奥段に残れる。復元時も行き所のない未成駒を拒否しない。
- 獅子取りの対象は`bl`と`dh`。獅子の成駒の奮迅`ie`や狛犬`go`/`lf`は対象外。
  先獅子、足・影の足、歩を使えない付け喰いを扱う。
- 狛犬は方向別の継続状態を使って同一直線上を最大3回移動し、途中で止まる・帰還することもできる。
  2・3マス先への直接移動には`mustNotBeEmpty`が指定した味方駒が必要。
  必要マスは参照のベクトルと符号計算をそのまま採用し、左右対称であるとは仮定しない。
- 天狗・鉤行（成駒の別IDも含む）は、走りの途中の空きマスで一度直角に曲がれる。
  途中では捕獲せず、終点でのみ捕獲する。曲がる地点を追加操作する方式ではなく、
  type102に合わせて終点までを一つの移動として扱う。
- 有限の走り、味方による遮蔽、飛び越し条件、成駒の別IDを個別に保持する。
- 王手が掛かっていないときの手詰まりをステイルメイト負けとする。
  四回反復と連続王手を完了した手単位で判定する。

## API・状態・探索

JSON v4（`haitaka-json-v4`）、`schemaVersion: 4`、`ruleId: 60`を使う。
駒を独自のSFEN/USI文字へ割り当てない。座標は1始まりで、[17,17]の添字は288。

```json
{"schemaVersion":4,"from":[8,8],"path":[[8,7],[8,6],[8,5]],"promotion":true}
```

要求は`position`、`history: {initial, moves}`、`pending`を持つ。
局面の状態は`{"senjishi":false,"shitenno":[]}`。天竺大将棋と共通の形式を使うが、
大大将棋に四天王は存在せず、空でない`shitenno`は拒否する。
持ち駒も空を要求する。履歴を最初から検証し、現在配置・手番・先獅子が一致することを確認する。

途中から再開するときは、最後の完了局面と、一段目または二段目までの
`pending: {from, path}`を渡す。workerは選択した完了経路を保持し、途中で再探索しない。
王の捕獲で手が途中終了した場合にも、追加の移動やパスを送らない。

検索は手作り評価。変更したマスと状態・手番を差分で保存・復元し、
各枝で289マスの盤面を丸ごとコピーしない。通常ルール用のNNUE/SFNN・USIは使用できない。

## ビルドと再検証

```sh
cargo build --release -p haitaka_cli --features dai-dai
cargo run --release -p haitaka_cli --features dai-dai -- json startpos
cargo run --release -p haitaka_cli --features dai-dai -- json search --depth 8 --timeout-ms 5000 < position.json
cargo build --release -p haitaka_wasm --target wasm32-unknown-unknown --features dai-dai
wasm-bindgen --target web --out-dir target/wasm-dai-dai target/wasm32-unknown-unknown/release/haitaka_wasm.wasm
```

CLIの`package --wasm-dir target/wasm-dai-dai --output target/haitaka-dai-dai.tgz`でパッケージ化する。
将棋ったーへ組み込む場合は、`wasm-bindgen --out-name haitaka_engine`で直接生成する。
JavaScriptファイルだけを置換しても、WASM内のインポート名は変更されない。

駒表は将棋ったーの`app/bot/daiDaiReference.test.ts`を
`DAI_DAI_DEFINITIONS=...`で実行して出力し、共通の表生成スクリプトで作る。

```sh
python3 scripts/generate-tenjiku-dai-defs.py target/dai-dai-definitions.json --ruleset dai-dai
```

将棋ったー側の全参照比較と独立perftは、通常テストとは別に明示して実行する。

```sh
DAIDAI_ORACLE=1 DAIDAI_POSITIONS=128 pnpm exec vitest run app/bot/daiDaiReference.test.ts
DAIDAI_PERFT=1 pnpm exec vitest run app/bot/daiDaiReference.test.ts
```

Haitaka側は`node scripts/verify-dai-dai.mjs target`で、全局面・全完了手・結果・ハッシュ・
履歴再生・perft・固定ノード探索を実WASMとNativeで照合する。
ブラウザ検証は将棋ったーの`tests/haitaka-dai-dai.spec.ts`で行う。
初期化5秒以内、線形メモリ64MiB以内、対局思考5秒を予算とし、1ms制限も検証する。

詳細は[第5段階の計画](../plans/historical-shogi/05-large-classical.md)を参照。


## 検証結果

355固定局面と128ランダム局面、計483局面・20,014完了手遷移が参照と一致した。
全85駒の先後別移動と、全21成り元の先後別成りを実際の遷移で覆った。
独立perftは63 / 3,951。実WASMでも全遷移、ハッシュ、履歴、固定ノード探索が一致した。

WASMは339,703 bytes（ブラウザ組込用は339,725 bytes）、線形メモリ1,245,184 bytes、
初期化約2.4ms、1ms制限の実測約1.01ms。設定した予算を満たした。
Chromeで新規8ケースと既存48ケースが通った。Rustワークスペース・拡張35件、core 2,570件、
AIのCI範囲434件、app全体729件、最終プロトコル9件と型検査・lintも通った。
天竺大将棋の352局面・77,025遷移は変更前と完全一致し、本将棋のバイナリもバイト単位で一致した。

[機械可読の検証記録](../plans/historical-shogi/05-dai-dai-verification.json)を参照。
第5段階全体は進行中。次の対象は摩訶大大将棋（61）。
