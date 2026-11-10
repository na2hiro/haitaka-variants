# 反転・両面打ちの５五系ルール

将棋ったーの「５五系」9種のうち、未対応だった4種を追加した。既存の5五・ジャドケンス・ゴロゴロ2種・5五金銀と合わせ、カテゴリ全体に対応する。

| Cargo feature / package | ID | 将棋ったーの名前 | 盤 | 成り規則 | perft(2) |
| --- | ---: | --- | --- | --- | ---: |
| `kyoto` | 19 | 京都銀閣将棋（京都将棋） | 5×5 | 毎手 | 137 |
| `kinkei-mai` | 123 | 金桂舞将棋 | 5×5 | 毎手 | 157 |
| `microshogi` | 98 | マイクロ将棋 | 4×5 | 捕獲時 | 80 |
| `kyoto-minishogi` | 125 | 京都将棋vs５五将棋 | 5×5 | 駒種に応じて毎手／敵陣1段 | 172 |

## ルールと状態

- 王以外の反転対象駒だけが反転する。駒を打った直後には反転しない。
- 持駒は表裏を一枚の物理的な駒として数える。JSONの持駒は表側のIDだけで表し、打ち手には使う面のIDを指定する。将棋ったーの表裏それぞれの持駒カウンターは境界で正規化し、不一致を拒否する。
- 京都は二歩なし・行き所のない面も可。金桂舞は金と桂が交互に反転する。
- マイクロは捕獲時だけ必ず反転する。coreのCapturePromotionに合わせ、盤上移動で行き所のない面になることは許すが、打つ面の行き所制約は有効。二歩・打歩詰は許す。
- 混合ルールでは普通の駒と `0l`〜`0s` の京都用駒を区別し、捕獲して所有者が変わっても駒種固有の成り規則を維持する。通常の歩だけに二歩・打歩詰の規定を適用する。非対称な初期配置の駒数は両者合計で検証する。
- 反転後の駒配置で自玉の安全性・相手の王手を判定する。四回反復と連続王手には履歴を使う。将棋ったーに合わせ、王手でない手詰まり自体は自動終局にしない。

既存のルール別駒・物理持駒・完了手探索を再利用し、通信は `haitaka-json-v3`。SFEN/USIへ独自の駒文字を追加せず、モデル読み込みとDFPNは公開しない。既存本将棋の盤表現と小盤5種のSFEN/USIは維持する。

## ビルドと再検証

```sh
cargo build --release -p haitaka_cli --features kyoto
cargo run --release -p haitaka_cli --features kyoto -- json startpos
cargo test --release -p haitaka --features kyoto --lib extended::
cargo xtask package --ruleset kyoto
```

他の3種にも表のfeature名を使う。将棋ったーでは次のように必要なルールだけを再生成できる。

```sh
HAITAKA_ENGINE_DIR=/path/to/haitaka-variants pnpm bot:sync:haitaka \
  --rules kyoto,kinkei-mai,microshogi,kyoto-minishogi
```

`wasm-bindgen` がPATHに必要。参照照合は各featureでCLIをビルドして `target/haitaka-<name>` に置き、将棋ったーで以下を実行する。

```sh
HAITAKA_ENGINE_DIR=/path/to/haitaka-variants CLASSICAL_RULE=kyoto \
  CLASSICAL_POSITIONS=128 pnpm exec vitest run app/bot/classicalReference.test.ts
```

今回の照合は625局面・8,034遷移。全合法手、捕獲後の物理持駒、反転、終局、履歴再生、実WASMの着手結果、固定1,000ノード探索をNative/coreと比較した。1ms指定でも合法手を返すことを確認した。

Rustの反転固有8件を含む共有古将棋42件、本将棋63件、既存5五27件、xtask21件が通過。将棋ったーはAI64件・app730件、型検査・変更箇所のlint、Chromeで4ルールのWASM準備から実応手まで通過した。最大盤面（大局将棋）のwasm32コンパイルも通過。

[検証集計とソース・ログSHA](../plans/small-shogi-flips-verification.json)。検証はコミット前の作業ツリーで行った。基準コミットと変更ソースのSHAは集計内を参照する。

## ローカル対局

[京都](http://localhost:3002/rule/19) / [金桂舞](http://localhost:3002/rule/123) / [マイクロ](http://localhost:3002/rule/98) / [京都vs５五](http://localhost:3002/rule/125)
