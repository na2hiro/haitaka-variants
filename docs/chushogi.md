# 中将棋（ルール48）

12×12・21種類92枚から始める中将棋に対応する。成り駒を含めた39個の駒IDを区別し、
獅子・角鷹・飛鷲の二段移動、居食い、途中終了を、探索では完了した1手として扱う。
採用仕様は将棋ったーの `chushogi.ts`、駒定義、Chu成り・Shishi捕獲制御。

対局: <http://localhost:3002/rule/48>。「相手」で「将棋ったーAI」を選ぶ。
手作り評価で指す。既存NNUE/SFNN・DFPN・USIは使わない。

## 駒・盤面・勝敗

- 盤面は `Position<Chushogi>`、色別占有は実際に `MultiBitBoard<3,144>` を使う。
  未使用48ビットは常に0。走り駒の利きは盤形に沿った走査で求め、巨大なMagic表は作らない。
- 玉 `ah` と太子 `cg` が王相当。両方あれば片方を失っても続行する。
  最後の王相当を失うか、指せる手がなくなると負ける。王手・詰みの放置は合法。
- 持ち駒は使わない。同じ動きでも、例えば初期の竜馬 `am` と角行の成り `dl`、
  初期の獅子 `bl` と麒麟の成り `dh` は別のIDを維持する。
- 敵陣4段のChu成りを使う。敵陣への侵入、敵陣内での捕獲、捕獲しながらの退出で成れる。
  不成で侵入後、敵陣内を捕獲せず動くだけでは成れない。歩・香は最奥段でも成れるが任意。
  行き所のない生駒も許す。
- 足付きの獅子、隣接する獅子、獅子かげの足、付け喰い、先獅子を捕獲判定へ反映する。
  付け喰いに歩・仲人は使えない。二段目での足の判定は、参照元と同じく開始盤面と
  途中盤面を使い分ける。
- 同一局面4回で千日手。完了した手番だけを数え、先獅子の有無を区別する。
  相手の王相当を連続して攻撃し続けた側は反則負けになる。

## JSON v2

形式名・能力宣言は **`haitaka-json-v2`**。小将棋のJSON v1は変更しない。
SFEN/USIへ新しい文字を割り当てず、数値座標と将棋ったーの駒IDを使う。

```json
{
  "schemaVersion": 2,
  "ruleId": 48,
  "position": {
    "sideToMove": 0,
    "pieces": [
      { "at": [5, 5], "piece": "bl", "color": 0 },
      { "at": [6, 5], "piece": "aa", "color": 1 },
      { "at": [7, 5], "piece": "ad", "color": 1 },
      { "at": [12, 1], "piece": "ah", "color": 1 },
      { "at": [12, 12], "piece": "ah", "color": 0 }
    ],
    "hands": [{}, {}],
    "state": { "senjishi": false }
  },
  "history": null,
  "pending": null
}
```

座標は1始まりの `[筋, 段]`、色は先手0・後手1。`pieces` は空マスを含めず、正規化時に筋・段の順へ並べる。
未知の駒・状態・フィールド、盤外、重複、成り元を合算した在庫超過、持ち駒を拒否する。
片側に王相当がない終局局面は有効。双方とも王相当がない局面は拒否する。

`state.senjishi` は、直前の操作で獅子以外が獅子（`bl` / `dh`）を取ったかどうか。
二段移動の途中でも更新され、次の捕獲の合法性に使われる。

```json
{ "schemaVersion": 2, "from": [5, 5], "path": [[6, 5], [7, 5]], "promotion": false }
```

`path` は1個か2個の着地点。二枚取りの途中の捕獲、居食いで元のマスへ戻る手、
一段目だけで終える手も区別する。成りは経路を完了したときだけ確定する。
玉を取ってその場で終局した場合は、一段目までの経路が完了手になる。
手番と探索深さ、千日手履歴は経路全体で1回だけ進む。

- `history: null`（省略も可）は履歴なしの解析開始局面。
  正規化後は `{ "initial": 開始局面, "moves": [完了経路, ...] }` になる。
  経路を順に適用した局面が `position` と一致することを、ルール状態も含めて検証する。
  終局後の続行も拒否する。
- 一段目まで進んだ画面から再開する場合、`position` は**その手の開始時点の確定局面**を渡し、
  `pending: { "from": [5,5], "via": [6,5] }` を添える。
  合法な継続移動の一段目であることを検証し、そこからの完了手だけを探索する。
  最善手には一段目を含む完全な経路を返す。`pending` は省略時もnull扱い。
- 将棋ったー連携は棋譜の差分から完了経路と開始盤面を復元する。
  選んだ経路を盤面・手番・棋譜に結び付けて保持し、二段目やパスのために再探索しない。
  待った・停止・別局面・対局切替では保持した経路を破棄する。
- `inspect_json` の `positionHash` はルールID・盤形・手番・全駒IDと色・先獅子・未完了経路を含む。
  完了局面は固定FNV-1a、未完了経路は開始盤面のハッシュへ経路座標を混ぜる。
  `contextHash` は確定局面の履歴順も含み、置換表のキーに使う。表示時刻は含めない。

## Native・WASM

```bash
cargo build --release -p haitaka_cli --features chushogi
./target/release/haitaka_cli json startpos > /tmp/chushogi.json
./target/release/haitaka_cli json moves < /tmp/chushogi.json
./target/release/haitaka_cli json search --depth 8 --timeout-ms 1000 --nodes 10000 < /tmp/chushogi.json
./target/release/haitaka_cli json perft --depth 2 < /tmp/chushogi.json
cargo pack-chushogi
```

Rust入口は `haitaka::extended::chu`。`start_position_json`, `validate_position_json`,
`legal_moves_json`, `apply_move_json`, `inspect_json`, `perft_json`, `search_json` はNative/WASMで共通。
WASMの関数名と引数は小将棋と同じで、v2のJSON文字列を入出力する。

探索は小将棋と共用した反復深化・αβ探索・捕獲と成りの静止探索・履歴付き置換表。
途中の捕獲も手の並べ替えと静止探索へ反映する。深さ1～100、正のミリ秒、ノード予算0は無制限。
予算は反復全体へ適用し、経路の列挙中も停止を確認する。中断時も完了した合法手だけを返す。
完了前に中断すれば合法な代替手、終局なら `bestMove: null`。perftの深さは0～4。
既存の14駒種の `Board` / `Piece` と通常ルールの高速経路は維持している。

## 参照元の修正

将棋ったー側にも次の回帰修正を入れた。

- 中将棋の敵陣内での捕獲成りを候補手生成が漏らしていたため、捕獲の有無を成り判定へ渡す。
- 保存済みの盤面ハッシュの形式は維持し、千日手の比較とAIキャッシュに先獅子を加える。
  先獅子が解除された同じ配置と合算して、1手早く千日手になる問題を防ぐ。
- 連続王手の千日手を調べるとき、獅子の途中盤面を数えない。

## 検証

242局面（全39駒種の両向き・特殊捕獲・成り・先獅子履歴・ランダム128局面・途中1局面）で、
参照実装と16,121個の完了経路・全遷移・勝敗が一致した。Nativeと実WASMのハッシュ、JSON正規化、
低予算探索も一致。初期局面のperftは深さ1=36、深さ2=1,296。

実Chromeでは中将棋6ケース（AI二枚取り・待った、途中再開、太子への成り、玉捕獲後の継続と
途中終了、最後の王捕獲、初期局面の応手）を確認した。小将棋4ケース・平安将棋9ケースも通過。
観測した中将棋WASMは289,483 bytes、線形メモリは1,310,720 bytes。
低予算1msの探索は約1.00msで完了手を返した（このマシンのNodeでの測定）。

本将棋の探索結果・ノード数は変更前後で一致した。時間がばらついた最初の20ペアから、
改めて40ペアで測定すると開始局面-0.75%、終盤+0.09%（各ペアの時間増減率の中央値）。
Chrome perft40ペアもほぼ同じ時間だった。正値は遅いことを表す。

主な再現コマンド:

```bash
cargo test --release -p haitaka -p haitaka_cli -p haitaka_wasm --features chushogi
# ビルドからCLIをtarget/haitaka-chushogiへ、WASMグルーをtarget/wasm-chushogiへ配置後:
# shogitter-3で
CHU_PERFT=1 pnpm exec vitest run app/bot/chushogiReference.test.ts
# Haitakaで
node scripts/verify-chushogi.mjs
```

計画と検証記録: [第3段階](../plans/historical-shogi/03-chu.md)。
