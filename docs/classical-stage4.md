# 禽・和・平安大・大将棋

2026-11-08。採用仕様は将棋ったーの解決済みルール定義と駒定義に合わせる。
実装コミットは`5aae64d`、将棋ったー連携は`e12907e2`。
実装ブランチは`feat/classical-stage4`、将棋ったー側は`feat/haitaka-classical-stage4`。

| ルール | ID | feature / ruleset | 盤 | 持ち駒 | 王相当 |
| --- | ---: | --- | --- | --- | --- |
| 禽将棋 | 16 | `torishogi` | 7×7 / u64 | あり | 鵬 `av` |
| 和将棋 | 50 | `washogi` | 11×11 / u128 | なし | 靏玉 `dw` |
| 平安大将棋 | 15 | `heian-dai` | 13×13 / 3ワード | なし | 玉 `ah` |
| 大将棋 | 49 | `daishogi` | 15×15 / 4ワード | なし | 玉 `ah`、太子 `cg` |

## 採用した規則

禽は敵陣2段で強制成り。駒打ちでは成らず、行き所のない燕は打てない。
同じ筋の不成の燕は2枚まで。打ち燕詰めは禁止し、合法な応手を再帰的に確認する。
鵰は鷹、雁は燕へ戻って持ち駒になる。左右の鶉は捕獲しても種類を交換しない。
所有者が変わるので、盤上と双方の持ち駒を合わせた初期総数で在庫を検証する。

和は取り捨て・王手放置禁止・指す手がなければ負け。雲鷲 `eh` は王ではない。
平安大も王手放置は禁止で、玉対玉になれば引き分け。
大は王手放置を許容し、最後の玉・太子を取れば勝ち。獅子取りの履歴規定を使う。
大の成りは敵陣5段のNormal方式で、中将棋のChu方式とは異なる。

禽の鵰、和の風馬・雲鷲、大の猛牛・飛龍の有限走りは途中の駒で遮られる。
獅子の二回移動・跳躍とは別に処理する。平安大の金と大の金将は成り元を保持する。

## Native / WASM / パッケージ

```sh
cargo build --release -p haitaka_cli --features torishogi
cargo run --release -p haitaka_cli --features torishogi -- json startpos
cargo run --release -p haitaka_cli --features torishogi -- json search --depth 3 --timeout-ms 1000 < position.json
cargo build --release -p haitaka_wasm --target wasm32-unknown-unknown --features torishogi
```

featureを`washogi`、`heian-dai`、`daishogi`に置き換えて各ルールを作る。
複数ルールのfeature同時指定は禁止。通常の14駒のBoardと拡張駒のバックエンドは分離する。
WASMは`wasm-bindgen --target web`で生成する。配布には各専用CLIの
`package --wasm-dir target/wasm-torishogi --output target/haitaka-torishogi.tgz`を使う。
既存NNUE/SFNNとUSI/SFENは受け付けず、手作り評価とJSON APIを使用する。

## JSON v3

プロトコル名は`haitaka-json-v3`。既存の小将棋v1、中将棋v2は維持する。
要求には`schemaVersion: 3`、`ruleId`、`position`、`history`、`pending`を渡す。
盤上駒は`{at: [file, rank], piece, color, origin?}`、持ち駒は双方のID→個数のマップ。
`origin`は複数の成り元がある駒の識別子で、盤面・履歴・ハッシュで保持する。
大の`state`は`{senjishi: boolean}`、他の3ルールは`{}`。

```json
{"schemaVersion":3,"kind":"move","from":[5,5],"path":[[6,5],[7,5]],"promotion":false}
{"schemaVersion":3,"kind":"drop","piece":"bb","to":[2,2]}
```

通常移動も獅子も`path`で完了した1手を表す。二回移動は手番を一度だけ進める。
`history`は初期局面と完了手の配列。全遷移を検証して現在局面と一致することを要求する。
大の移動途中は、最後の完了局面と`pending: {from, via}`を渡して残りの合法な完了手に絞る。
将棋ったーのworkerは選んだ完了手を保持し、二回目の操作で再探索しない。
棋譜差分にも成り元を保存し、持ち駒を含めて再開・待った・プレビューを検証する。
千日手では大の獅子取り状態と成り元を区別し、既存の保存ハッシュ形式は維持する。

## 検証と再現

[第4段階の機械可読記録](../plans/historical-shogi/04-other-classical-verification.json)を参照。
各128ランダム局面に全駒・先後反転・成りの固定局面を加え、合法な完了手と全遷移を
参照実装→Native→実WASMで比較した。独立した参照perftも深さ2まで一致する。

将棋ったー側の`app/bot/classicalReference.test.ts`で定義・比較データを生成する。
`CLASSICAL_RULE=torishogi`等で1ルールを選び、`CLASSICAL_POSITIONS`でランダム局面数を指定できる。
解決済み定義の出力には`CLASSICAL_DEFINITIONS`を使い、
`python3 scripts/generate-classical-defs.py target/classical-definitions.json`でRust表を再生成する。
その後`cargo fmt --all`を実行する。

```sh
node scripts/verify-classical.mjs target torishogi
python3 scripts/verify-classical-packages.py target
```

Chromeの`tests/haitaka-classical.spec.ts`は通常対局、途中再開、終局、燕打ち、
成り元保持、大の獅子二枚取り・移動途中からの再開の16ケースを検証する。
次の大型・複雑なルールは[第5段階](../plans/historical-shogi/05-large-classical.md)で扱う。
