# 小将棋（ルール43）

小将棋の醉象・太子、持ち駒を使わない捕獲、複数の王相当、王手放置を扱う。
採用ルールは将棋ったーの `packages/core/src/db/rules/shoshogi.ts`。
本将棋の `Board` / `Piece` / SFEN / USI は維持し、新しい処理は
`haitaka::extended::{Shoshogi, ShoGame}` と `shoshogi` featureで選択する。
このfeatureではCLI・WASMから従来のSFEN/USI探索を使えない。

## 対局

このマシンの将棋ったー: <http://localhost:3002/rule/43>。
「相手」で「将棋ったーAI」を選ぶ。手作り評価で、既存NNUE/SFNNとDFPNは使わない。
初期局面だけでなく、太子への成りや玉を失った途中局面から再開できる。

## 採用ルール

- 9×9、双方21枚。通常の20枚に醉象を5八・5二へ追加する。
- 醉象 `cf` は真後ろ以外の隣接7マス。敵陣3段の通常の成りで太子 `cg` になる。
- 太子は隣接8マスへ動く。玉 `ah` と異なるIDを保つ。
- 捕獲した駒は除外し、持ち駒にしない。歩・香・桂の行き所のない移動は強制成り。
- 王相当は盤上の玉と太子から導く。最後の王相当を失った側が負ける。
  玉か太子の片方だけを失っても、もう片方が残れば続行する。
- 王手放置・詰み放置・王同士の隣接・ピンされた駒の移動も合法。
  盤外移動や味方駒の捕獲は非合法。
- 同一局面4回で千日手。連続して相手の王相当のいずれかを攻撃し続けた側は反則負け。
  両側が連続攻撃している場合は、参照実装と同じく直前の着手側が負ける。
- 玉対玉の自動引き分けはない。参照ルールは、指せる手がないだけで自動終局させない。
  エンジンが手を返せない場合は対局アダプターが投了する。

## 拡張ルール用JSON v1

位置・指し手形式名と能力宣言は **`haitaka-json-v1`**。
各入力オブジェクトは未知のフィールドを拒否する。すべてのフィールドが必須。
例は初期配置ではなく、途中局面を示す。

```json
{
  "schemaVersion": 1,
  "ruleId": 43,
  "position": {
    "sideToMove": 0,
    "pieces": [
      { "at": [4, 4], "piece": "cf", "color": 0 },
      { "at": [5, 9], "piece": "ah", "color": 0 },
      { "at": [5, 1], "piece": "cg", "color": 1 }
    ],
    "hands": [{}, {}],
    "state": {}
  },
  "history": []
}
```

- 座標は `[筋, 段]` の1始まり整数、向きは先手0・後手1。
  `sideToMove` は向きであり、表示上の手番番号ではない。
- `pieces` は空マスを含めない。正規化では筋、段の順へ並べる。
  駒IDは将棋ったーの `aa`～`an` と `cf`, `cg`。
  成り元を含む同一向きの在庫上限、座標、重複、行き所のない生駒も検証する。
- `hands` は両側とも空オブジェクト、`state` は空オブジェクトのみ対応。
  王リストや追加のルール状態を黙って捨てない。
- 王相当が片側にいない終局局面は有効。双方ともいない局面は拒否する。
- `history` は現在局面を**末尾に含む**時系列の位置配列。
  空配列は「履歴なしの解析開始局面」を意味し、正規化で現在局面1個へ変換する。
  再開対局で千日手を維持するには、対局開始時点からの履歴を渡す。
  各遷移が合法な完了した1手であること、末尾と現在局面の一致、終局後の続行がないことを検証する。
- 成り・捕獲・手番変更は適用後の盤面から復元できる。追加の王リストは持たない。
  将棋ったー連携は棋譜の盤面差分を逆順にたどり、履歴を再構成する。
  取り消し・再開した棋譜も現在局面と差分の整合性を検証する。
- `inspect_json` の `positionHash` はルールID・盤形・手番・全駒IDと向きを含む
  固定FNV-1aの64bit値（16桁hex）。`contextHash` は履歴の順序も含む。
  反復判定は盤面全体と手番を比較し、置換表キーは履歴を含める。
  v1で状態は空のみ。状態や経路を追加する際はプロトコルの版を更新する。

指し手:

```json
{ "schemaVersion": 1, "from": [4, 4], "to": [4, 3], "promotion": true }
```

成りの有無は明示する。強制成りの移動に `false` を指定すると拒否する。
打ち・途中経路・多段移動はこの版では扱わない。SFEN/USIに新駒文字を割り当てない。

## Native・WASM API

```bash
cargo build --release -p haitaka_cli --features shoshogi
# 以下はすべて同じfeatureのCLIを使う
./target/release/haitaka_cli json startpos > /tmp/shoshogi.json
./target/release/haitaka_cli json moves < /tmp/shoshogi.json
./target/release/haitaka_cli json search --depth 8 --timeout-ms 1000 --nodes 10000 < /tmp/shoshogi.json
./target/release/haitaka_cli json perft --depth 3 < /tmp/shoshogi.json
```

`json validate`, `inspect`, `apply --move '{...}'` もある。
`json oracle` はJSON Lines入力で、合法手と全遷移・勝敗を返す検証用コマンド。

WASMのエクスポートは `start_position_json()`, `validate_position_json(position)`,
`inspect_json(position)`, `legal_moves_json(position)`, `apply_move_json(position, move)`,
`perft_json(position, depth)`, `search_json(position, maxDepth, timeoutMs, nodeBudget)`。
入出力はJSON文字列（`start_position_json`以外の入力もJSON文字列）。
Nativeの同名関数も同じコアを呼ぶ。

探索は反復深化のαβ探索、捕獲・成りの静止探索、履歴を含む置換表、駒価値と前進の手作り評価。
探索深さは1～100、時間は正のミリ秒、ノード予算0は上限なし。
ノード上限は反復を通じた通常探索・静止探索の合計。未完了の反復の値・手は採用せず、
初回の完了前に中断された場合も合法手を返す。終局は `bestMove: null`。
`completedDepth`, `timedOut`, `nodeLimitReached`, `states`, `score`, `outcome` などで結果を確認できる。
perftの深さは0～6。初心者向け棋力調整や小将棋の学習モデルは別段階。

```bash
cargo pack-shoshogi
```

パッケージはJSON形式・ルール43・手作り評価を宣言し、従来のUSI能力やDFPNを宣言しない。
将棋ったーのJSON対応wasm-bindgenランタイムで登録できる。形式不一致・USIランタイム・
JSON能力の欠落・NNUE付きJSONプロファイルは拒否する。

## 検証

- 固定テスト: 醉象・太子の移動、両向き、成り、片方／最後の王の捕獲、王手放置、
  強制成り、スライダーの遮蔽、JSON正規化・未知状態・不正履歴、千日手、中断時の合法手。
- 将棋ったー参照: seed `0x43cf2026`、167局面（ランダム128、固定14、履歴24、終局1）。
  合法手と全4,896遷移・勝敗が一致。Native/WASMの位置ハッシュ・正規化・全遷移も一致。
- 初期局面のperft: 深さ1=26、深さ2=676、深さ3=17,368。
- 実Chrome: 初期局面のAI応手、太子への成り、玉捕獲後の続行、最後の王捕獲の4ケース。

再現用の主な入口:

```bash
cargo test --release -p haitaka -p haitaka_wasm -p haitaka_cli --features shoshogi
# 上のビルドからCLIをtarget/haitaka-shoshogiへコピーし、WASMグルーをtarget/wasm-shoshogiへ生成
# shogitter-3で（HAITAKA_ENGINE_DIRを必要に応じて指定）:
SHO_PERFT=1 pnpm exec vitest run app/bot/shoshogiReference.test.ts
# Haitakaで:
node scripts/verify-shoshogi.mjs
```

機械可読の検証記録は [02-sho-verification.json](../plans/historical-shogi/02-sho-verification.json)。
