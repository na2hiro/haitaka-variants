# 平安将棋3種

古将棋対応の第1段階。将棋ったーに実装された平安将棋の3サイズに対応する。

| Cargo feature / package ruleset | ルールID | 盤 | 格納 | perft深さ1 / 2 / 3 |
| --- | ---: | --- | --- | --- |
| `heian-8x8` | 12 | 8×8 | u64 | 22 / 484 / 10296 |
| `heian-9x8` | 13 | 9×8 | u128 | 26 / 676 / 17108 |
| `heian-9x9` | 14 | 9×9 | u128 / 既存Magic | 26 / 676 / 17108 |

## ルールと表現

- 歩・香・桂・銀・金・玉を使う。捕獲した駒は取り捨てで、持ち駒と駒打ちはない。
- 敵陣3段への出入りで歩・香・桂・銀が通常の金に成る。歩香の最奥段、桂の最奥2段では強制成り。
- 成った駒のSFENは `G/g`。元の駒の種類へ戻す必要がないため `+P` 等の入力は拒否する。
- 金の初期枚数を超える局面を許し、総駒数と未成の駒の上限を検証する。未使用の駒種や持ち駒は拒否する。
- 玉対玉は引き分け。終局は通常探索・静止探索・直接API・USIで扱い、追加の手を生成しない。
- 千日手は既存の履歴規則に従う。4回の同一局面は引き分け、連続王手は王手側が負け。
- DFPNと既存NNUE/SFNNは未対応として拒否する。9×9版も既存モデルを使用しない。
  パッケージはDFPNの能力を広告しない。

USIでは `info string gameover only-kings-draw rules ...` で玉対玉の引き分けを通知し、
手を返さない既存の終局慣例として `bestmove resign` を続ける。独自対局ツールはgameover通知を
先に解釈すること。将棋ったーは自身のルール判定で引き分けを確定し、AIに次の手を求めない。

`RuleSpec` の捕獲、成り先、使用駒、金への集約を考慮した枚数検査、終局、モデル・DFPNの設定を使う。
金への集約検査と取り捨ては別の設定であり、今後の持ち駒なしルールがすべて金に成るとは仮定しない。
標準の成り表の場合は既存の高速な判定と配列参照を使う。

## ビルドと対局

```sh
cargo run -p haitaka_cli --release --features heian-8x8 -- usi
cargo run -p haitaka --release --features heian-9x8 --example perft -- 3
cargo xtask package --ruleset heian-9x9
```

一度に1ルールのfeatureを指定する。ほかのルールとの同時指定はコンパイルエラーになる。
出力は `target/haitaka-variants-<ruleset>.tgz`。SFEN / USIを利用し、モデルは含めない。

内部の `compact-board` は生成座標と方向別利き、`scalar64` は64ビット格納、
`variant-board` は既存9×9用テスト・文書例の区別に使う。既存の `small-board` は従来の
ミニ将棋系featureを維持する。9×8はcompact座標とu128を組み合わせる。
補助featureは単独で指定せず、具体的なルールを指定する。

将棋ったーの連携は `/home/na2hiro/proj/shogitter-3` の `feat/haitaka-heian-shogi` で行った。
開発サーバーはユーザーが起動しているものを利用する。調査・確認時のURLは
`http://localhost:3002/rule/12`、`/rule/13`、`/rule/14`。
各画面で相手に「将棋ったーAI」を選ぶ。

## 独立比較と固定局面

参照ルールはShogitter `67ea4f76a02721608f2ceb3fe19657b35f1d805d` のcore。
平安将棋ルールに継承される成り・王手判定、取り捨て、玉対玉の引き分けも照合した。
Haitakaの変更前基準は `9d90ef3`。

| ルール | 比較局面数 | 全合法手の遷移数 |
| --- | ---: | ---: |
| heian-8x8 | 151 | 2888 |
| heian-9x8 | 151 | 3662 |
| heian-9x9 | 151 | 3689 |

各ルール128局面のseed付き対局に、23の固定局面を加えた。合法手集合と、各手適用後の盤面・手番・
持ち駒・勝敗がすべて一致した。強制成りの操作表現上の重複は、実際の成り結果に基づくUSI手へ正規化した。固定局面は先後の反転、任意／強制成り、成りによる金の増加、
捕獲、玉対玉、詰みを含む。3種の初期perftも将棋ったーの独立生成と一致した。

再現手順:

```sh
# haitaka-variants側
for rule in heian-8x8 heian-9x8 heian-9x9; do
  cargo run -p haitaka --release --features "$rule" --example rule_oracle -- \
    128 "haitaka/tests/fixtures/heian/$rule.sfen" > "target/$rule-transitions.tsv"
done

# shogitter側。ソース位置は実際のワークツリーに合わせる。
HAITAKA_ENGINE_DIR=../haitaka-variants HEIAN_PERFT=1 \
  pnpm exec vitest run app/bot/heianReference.test.ts
```

参照テストは通常実行時に初期配置と千日手も確認する。遷移TSVがない場合はその比較をスキップし、
独立perftは `HEIAN_PERFT=1` で明示実行する。固定局面の履歴判定はNative側でも検証する。

## Native・WASM・ブラウザの検証

```sh
for rule in heian-8x8 heian-9x8 heian-9x9; do
  cargo test -p haitaka -p haitaka-types -p haitaka_wasm -p haitaka_cli --features "$rule"
  wasm-pack build haitaka_wasm --target web --release --features "$rule" \
    --out-dir "../target/wasm-$rule"
done
node scripts/verify-heian.mjs target
```

wasm-bindgenを直接使う場合も、`target/wasm-<ruleset>/haitaka_wasm.js` と対応するWASMを生成する。
将棋ったーの連携先には、同じソースから出力名を `haitaka_engine` にしたものを配置する。

ブラウザでは実WASMの準備・AI応手、保存した途中局面の再開、捕獲して金に成る操作、
持ち駒が増えないこと、玉対玉の引き分けを3種すべてで確認した（9ケース）。

```sh
# shogitter側。起動済みサーバーを利用する。
SKIP_WEB_SERVER=1 BASE_URL=http://localhost:3002 pnpm exec playwright test \
  tests/haitaka-heian.spec.ts --project=chromium --workers=1
```

ローカル検証ではシステムChromeを指定する一時Playwright設定を使った。
ブラウザ用局面を再生成する場合は参照テストに
`HEIAN_BROWSER_FIXTURES=tests/fixtures/heian-positions.json` を指定する。

## 既存ルールへの回帰と性能

平安3種・既存小盤5種の全クレートのテスト、本将棋のreleaseテスト、安南将棋、xtask、
学習クレートを含むworkspace check、将棋ったーの型チェック・AI連携テストを確認した。
Clippyは完了し、既存コードと拡大した定数配列に関する警告は残る。

同じビルド条件で変更前後を交互に測定した。以下はこのマシンでの局所的な計測結果であり、
速度向上を一般に保証するものではない。負の値は処理時間の短縮を示す。

- Native: 各局面1万ノード、20組。最善手・評価・消費ノード一致。
  最終の成り高速経路で初期局面約+0.08%、終盤約-1.30%。
- Chrome WASM: 初期局面perft深さ4（719731）、終盤深さ3（3107043）、各40組。
  最終の成り高速経路で約−3.57%、約−3.23%。ノード数は一致した。
- 成り判定を全面的な汎用表参照にした初期実装では、WASMの初期局面に約3.7%の遅延が出た。
  標準の成り表を使う場合に元の高速経路を維持することで解消した。

Nativeの比較は `scripts/compare-usi-nodes.py`、WASMの比較は次のスクリプトで再現できる。

```sh
node scripts/compare-wasm-perft.mjs BEFORE_DIR AFTER_DIR \
  /path/to/shogitter/node_modules/@playwright/test/index.mjs OUTPUT.json
```

`CHROME_BIN` でChrome実行ファイルを変更できる。変更前後のビルドディレクトリと依存関係を
分けて用意し、コンパイルや別のCPU負荷を同時実行しない。

次段階は [小将棋の計画](../plans/historical-shogi/02-sho.md)。小将棋以降はまだ未実装。
