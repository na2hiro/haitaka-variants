# 天竺大将棋（ルール59）

2026-11-08。第5段階の最初の対応ルール。将棋ったーの解決済みルールと駒定義を採用する。
実装コミットは`ad4fe1c`、将棋ったー連携は`adc2a83f`。
16×16、初期156枚、成駒・別IDを含む66種類を使う。既存の9×9「天竺将棋」（56）とは別のエンジン。

対局URL: <http://localhost:3002/rule/59>。相手に「将棋ったーAI」を選択する。
多段移動を続ける場合は次の移動先を直接選び、現在の駒を再クリックするとそこで移動を終える。
四天王の隣接捕獲は元のマスへの帰還が必要で、途中では終了できない。

## 採用規則と効果順序

- 取り捨て。王将`ah`と太子`cg`の全てを失えば負け。王手放置を許容する。
- 敵陣5段のNormal成り。多段移動中の成り可否を蓄積し、完了時・パス時に成れる。
- 獅子取りの先獅子・足・付け喰いを扱う。付け喰いに使えない駒の設定は歩`aa`だけ。
- 将の跳越しは駒ごとの`jumpException`で制限する。例外駒の手前で止まり、その駒には着地しない。
- 火鬼・副将は玉の三回歩行を持ち、捕獲すればそこで止まる。走りとは組み合わせない。
- 四天王は隣接駒を飛び越える走り・三回歩行の到達点と、隣接捕獲して戻る居喰いを持つ。
- 四天王どうしは先に在席した駒を優先する。有効な四天王の周囲の敵駒は移動開始時に凍結される。
  多段移動の途中で隣接しても、その移動は続けられる。
- 王手が掛かっていない場合の手詰まりをステイルメイト負けとする。

完了した1手では、通常捕獲と成りの後、手番開始時に有効だった敵火鬼を確認する。
その火鬼が同じ所有者のまま残り、到着駒に隣接していれば、到着駒を先に焼く。
次に四天王の在席順序と凍結を更新し、凍結していない火鬼の焼却対象を集めて同時に取り除く。
最後に在席順序と王相当の残存を更新する。双方の王相当が消えた場合は引き分け。

最後の王相当を継続移動の途中で通常捕獲した場合は、その場で終局し、追加の手番終了時効果は発生しない。
取られた火鬼を同じマスへ移った別の火鬼と取り違える問題は、参照実装側も修正し、回帰テストを追加した。

## JSON v4と復元

プロトコルは`haitaka-json-v4`、`schemaVersion: 4`、`ruleId: 59`。
座標は1始まりで、最長三段の完了経路を使う。SFEN/USIへの独自駒文字の割り当ては行わない。

```json
{"schemaVersion":4,"from":[8,10],"path":[[8,9],[7,10],[7,9]],"promotion":false}
```

要求は`position`、`history: {initial, moves}`、`pending`を持つ。
`position`は駒配置・手番・空の持ち駒と、以下の状態を含む。

```json
{"senjishi":false,"shitenno":[{"at":[8,8],"color":0},{"at":[9,9],"color":1}]}
```

`shitenno`は盤上の全ての四天王を在席順に列挙する。同じ駒配置でも順序が異なれば局面を区別する。
JSONの履歴を最初から検証し、現在局面・順序・先獅子が一致することを要求する。
途中再開では、最後の完了局面を`position`へ渡し、次のように一段目・二段目までを別に渡す。

```json
{"from":[8,10],"path":[[8,9],[7,10]]}
```

将棋ったー側では、保存された手番開始時の火鬼情報も元の完了局面と照合する。
途中情報が欠けた記録を、完了局面として黙って探索しない。
焼却で移動駒が消える棋譜も、全マスの差分と成り表記から復元する。
workerは選んだ完了経路を保持し、二段目・三段目で再探索しない。
既存JSON v1/v2/v3の経路上限と能力指定は維持する。

## ビルド・パッケージ

```sh
cargo build --release -p haitaka_cli --features tenjiku-dai
cargo run --release -p haitaka_cli --features tenjiku-dai -- json startpos
cargo run --release -p haitaka_cli --features tenjiku-dai -- json search --depth 3 --timeout-ms 5000 < position.json
cargo build --release -p haitaka_wasm --target wasm32-unknown-unknown --features tenjiku-dai
```

`wasm-bindgen --target web`で生成したファイルを専用CLIでパッケージ化する。
`package --wasm-dir target/wasm-tenjiku-dai --output target/haitaka-tenjiku-dai.tgz`を使う。
モデルは手作り評価のみ。既存NNUE/SFNN、USI、およびルール56の指定は拒否する。

駒表は将棋ったーの`app/bot/tenjikuDaiReference.test.ts`から`TENJIKU_DAI_DEFINITIONS`で出力し、
`python3 scripts/generate-tenjiku-dai-defs.py target/tenjiku-dai-definitions.json`と`cargo fmt --all`で再生成する。

## 探索と検証

探索の枝では変更マス・手番・効果状態の差分を適用し、巻き戻す。
盤面のコピーは経路列挙開始時などに限定し、各枝で盤面全体をコピーしない。
連続王手の計算は四回反復の成立時に履歴を戻して行い、通常の探索枝では毎回計算しない。
経路列挙中も停止を確認し、時間切れには合法な完了手へフォールバックする。

全66駒の先後別移動、全30成り元の先後別成り、特殊効果・順序・途中再開の固定局面と、
128ランダム局面を比較する。参照側は候補の合法性確認だけ終局判定の重複を省き、
最終遷移の結果は通常の参照判定で照合する。この候補集合が通常のcore合法手と一致することも検証する。
独立した完了手perftは深さ1が132、深さ2が17,226。

将棋ったー側で全比較を行う場合は、通常テストとは別に明示指定する。

```sh
TENJIKU_ORACLE=1 TENJIKU_POSITIONS=128 pnpm exec vitest run app/bot/tenjikuDaiReference.test.ts
TENJIKU_PERFT=1 pnpm exec vitest run app/bot/tenjikuDaiReference.test.ts
```

Haitaka側で実WASMを照合する。

```sh
node scripts/verify-tenjiku-dai.mjs target
```

実WASMでは全遷移・ハッシュ・履歴再生・探索をNative版と照合する。
初期化5秒・線形メモリ64MiB・思考5秒を当初の予算とし、低予算1msも検証する。
Chromeの`tests/haitaka-tenjiku-dai.spec.ts`は8ケース、既存ルールの回帰は35ケースを対象とする。
詳細は[第5段階の計画](../plans/historical-shogi/05-large-classical.md)と[検証記録](../plans/historical-shogi/05-tenjiku-dai-verification.json)を参照。
