# 通常移動を使う追加ルール

古将棋・５五系のJSON v3基盤を再利用し、次の28ルールを追加した。
持ち駒不使用54は最新mainの `fushiyo` を再利用するため、新しい駒取り処理は実装しない。
将棋ったー側ではこれらに既存の持ち駒系6種を加え、計35ルールをHaitakaに接続する。

| 段階 | ルールID | feature |
| --- | --- | --- |
| 2 | 不成111・二歩あり138 | `narazu`, `nifu-ari` |
| 3 | 小京左京46・右京47・仲人65・クイーン王147 | `sho-kyo-left`, `sho-kyo-right`, `nakodo`, `queen-king` |
| 4 | 取らず銀30・相取らず銀31・ざりかに153・ざりかにとんぼ165 | `torazu-gin`, `ai-torazu-gin`, `zarikani`, `zarikani-tonbo` |
| 5 | 狭い79・せっまい90・広い91 | `semai`, `semmai`, `hiroi` |
| 6 | どうぶつしょうぎ17 | `dobutsu` |
| 7 | 取る一手28 | `toruichi` |
| 8 | 飛車92・角103・精霊棋159・京左京44・右京45 | `hisha`, `kaku`, `seireigi`, `kyo-left`, `kyo-right` |
| 9 | 突撃88 | `totsugeki` |
| 10 | 相八方桂21・八方桂20 | `ai-happokei`, `happokei` |
| 11 | Patrol112・Lortap113 | `patrol`, `lortap` |
| 12 | 獅子王22・奔王vs獅子117 | `shishio`, `hono-shishi` |
| 13 | 二玉163 | `nigyoku` |

## ルール固有の挙動

- 不成は盤上で行き所がなくなる移動も許す。行き所のない駒を打つことは禁止する。
- 二歩ありは筋ごとの歩数制限だけを除き、通常の成り・打ち歩詰めなどの制約を保つ。
- 仲人を打って詰ませる手は合法で、歩だけを対象とする打ち歩詰め制限を適用しない。
- 捕獲制限は合法手だけでなく、王駒への攻撃判定にも適用する。成銀は取らず銀の保護対象にしない。
- 取らず銀は後手から始める。JSON内のsideと盤上のcolorは先後の向きであり、将棋ったーの手番インデックスをそのまま渡さない。
- ざりかに系では幼王は捕獲できず、王手放置を許す。矩形7×6盤を使う。
- どうぶつしょうぎは王手放置・ライオンの捕獲を許し、安全な最奥段へのライオンの移動をトライ勝ちとする。持ち駒・ひよこの成りも再現する。
- 取る一手はcoreの擬似捕獲判定を使う。ピンされた非王駒でも捕獲義務が生じる一方、自玉が王手中なら免除する。王しか取れず、候補がすべて敵に守られている場合も強制しない。

## 構造と再生成

`classical/ordinary.rs` がルールの方針、`ordinary_defs.rs` が生成された駒・配置の定義を持つ。
盤・持ち駒・成り・棋譜・探索は既存のClassicalGameを再利用する。評価はhandcraftedで、9×9 NNUE/SFNNはロードしない。

第1巡の定義は `inputs/definitions.json`、全28種の最新の定義は
`plans/next-rule-groups/inputs/phase2-definitions.json` に固定した。
再生成時は将棋ったー側の `nextRulesDefinitions.test.ts` で定義を出力し、次を実行する。

```sh
python3 scripts/generate-next-rules.py plans/next-rule-groups/inputs/phase2-definitions.json \
  --through 13 --web ../shogitter-3
rustfmt --edition 2024 haitaka/src/extended/classical/ordinary_defs.rs
```

各featureを単独でビルドする。通常のSFEN/USI形式は `fushiyo` のみで、この表の28種はJSON v3のCLI・WASM APIを使う。

```sh
cargo build --release -p haitaka_cli --features dobutsu
cargo run --release -p haitaka_cli --features dobutsu -- json startpos
```

## 検証

新規15ルールの1,113局面・35,167遷移で、coreとnativeの合法手集合・盤面・持ち駒・勝敗が一致した。
同梱WASMも同じ固定／乱数局面をcoreと照合し、時間予算が小さい場合も合法手を返すことを確認した。
持ち駒不使用は80局面でcoreとWASMの合法手数・持ち駒なし・打ち駒なしを確認した。
ブラウザーでは16ルールすべてでモデル準備とAIの応手を確認した。

coreとnativeの照合は将棋ったー側の `ordinaryRulesReference.test.ts`、coreと同梱WASMの照合は
`ordinaryRulesWasm.test.ts`、画面上の対局確認は `haitaka-ordinary.spec.ts` にある。
原本・WASMハッシュ・ビルド元commitは `plans/next-rule-groups/results/verification.json` と同ディレクトリのログに保存した。

第2巡の13種は1,044局面・44,783遷移でnative/WASMとcoreが一致した。
既存の持ち駒系6種も588局面・32,531遷移でWASMとcoreを照合し、19種のブラウザー対局を確認した。
詳細は `plans/next-rule-groups/phase2.md` と `results/phase2/verification.json` を参照。

第2巡では `CHECK`（自玉の王手放置を禁ずる）と `CHECKMATE`（詰みを判定する）を分離した。
獅子は `LION`（複数段移動）と `LION_CAPTURE_RULES`（中将棋固有の捕獲制限）を分離し、
獅子王と奔王vs獅子では後者を無効にする。Patrol/Lortapは捕獲前の盤上の幾何学的な味方の利きを使う。
