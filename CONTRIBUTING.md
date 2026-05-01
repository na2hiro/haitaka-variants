# Contributing to Haitaka Variants

変則将棋のAIを盛り上げていきたいと思い、このリポジトリをオープンソースにすることにしました。個別のルールごとの分野ではあまり活発でなかったとしても、似たようなルールなど隣接する領域での手法や知見が集約でき、メリットが大きいのではないかと考えました。

やっていきましょう！

## 貢献が望まれる分野

不明な点などはissuesを上げたり、[将棋ったーDiscord](https://discord.com/invite/CYRWPQMGXu)の #変則将棋ai チャンネルで気軽に質問してください！

### 合法手生成ロジックの正しさの確認
生成された合法手に漏れがないか、非合法な手がないかなど、ある程度テストはしてありますが、完璧ではありません。

- 【テスト】 [将棋ったー上でHaitaka Variantsと対局する](https://shogitter/com/rule?bots=builtin%3Ahaitaka)だけでも、エラーやおかしな挙動を発見できる場合があり、助かります。
- 【コーディング】さらに怪しそうなテストケースを追加するなどして網羅性を高め、ロジックの正しさを確認し将来のコード変更による挙動の破壊を防止する

### 個別のルールにおける機械学習、NNUEモデルの出力・シェア
Haitaka Variants では、NNUEという割とスタンダードなニューラルネットワークの学習・評価ができるようになっています。haitaka_learnディレクトリ配下に機械学習関連のコードがあり、ここで学習して得られたモデル(.binファイル)を探索時に使用することができます。

- 【機械学習】現状、CUDAに対応したGPUが必要ですが、既に学習できるような環境が整っているはずです（未検証）。具体的には、データセットの生成、NNUEの学習、そして結果の検証などのステップがあります。

ぜひ世界初(?)の安南・安北・安東西・（今後もどんどん増えていく予定）のAIを作成し、世に出してみませんか？

### 合法手生成ロジックのさらなる高速化
高速な合法手生成には、ドメイン知識、つまりそれぞれの将棋ルール特有の知識への理解が重要です。例えば本将棋では「両王手がかかっている局面では、合法手は王を動かす手のみに限られる」と仮定できるため、王以外を動かす手を考えなくていいというような最適化がなされています。安南など役割が変化するルールの実装においては、役割が変化している駒と役割を提供している駒による両王手の場合は後者を取ることで２つの王手を同時に取り除ける場合があるため、両王手の最適化自体を無効化しています。

- 【コーディング】これを部分的に復活させることで合法手生成を効率的にできるのではないかと思います。
- 【提案する】【コーディング】また、これに限らず、他の箇所で合法手生成の効率化の余地があるはずです。コードを理解する必要はありますが、アイデアを出してissueを立てる（やったら良さそうなことを提案する）だけで貢献できる分野だと思います。具体的な分野：
  - 詰ルーチンdf-pn部分
  - 探索部分(alphabeta)の効率化(例えば有効そうな子ノードから優先して探索するなど)

### それ以外の周辺機能の拡充、アーキテクチャレベルでの改良など
na2hiroは将棋ったーの開発歴やWeb開発歴は長いものの、コンピュータ将棋に関しては素人です。コンピュータ将棋のリポジトリとしては足りないものが多いのではないかと思います。

- 【提案する】【コーディング】もし、ルールの拡張性を保ったまま採用できるようなコード効率化、機械学習パイプラインの改善などを含めた様々なレベルでの改善案などがあれば教えていただけると嬉しいです。

### na2hiroが今後やろうとしているけど貢献があると助かること
na2hiroは、上のことより先に次のことに注力しようと思っています。それでも、この分野で興味があるものがあるならissuesに書き込んでいただければと思います。

* 手書きの[駒得・合法手数ベースのナイーブな評価関数](https://github.com/na2hiro/haitaka-variants/blob/16837421ac9c89a454d220f9e2aa57d640b3f259/haitaka_wasm/src/lib.rs#L1011-L1023)を改善し、NNUEがなくても中終盤でそれなりの手を指せるようにする https://github.com/na2hiro/haitaka-variants/issues/4
* 他のルールの合法手を生成できるようにし、他のルールでも同様に早い探索やNNUEによる学習を開始できるようにする。
  * 他の役割変化系ルールへのさらなる対応。 https://github.com/na2hiro/haitaka-variants/issues/5
  * どういったルールへの対応が比較的簡単かを検討する https://github.com/na2hiro/haitaka-variants/issues/6

### その他

[ROADMAP.md](./ROADMAP.md)に俯瞰した方針が示してあるので、そちらもご覧ください。

## Development Commands

Run tests:

```bash
cargo test
cargo test -p haitaka --features annan
cargo test -p haitaka_wasm --features annan
cargo test -p haitaka_learn --features annan
cargo test -p haitaka_cli --features annan
```

Local engine smoke tests:

```bash
cargo run -p haitaka_cli -- play --human none --depth 2
cargo run -p haitaka_cli -- self-play --games 2 --a-depth 2 --b-depth 1
cargo run -p haitaka_cli -- package --allow-missing-wasm
```

Benchmarks:

```bash
cargo bench -p haitaka --bench legals -- --noplot
cargo bench -p haitaka --bench perft -- --noplot
cargo bench -p haitaka --bench dfpn -- --noplot
```

## Pull Requests

- Keep rule changes small and include SFEN-based regression tests when possible.
- Include benchmark numbers for move generation or search performance changes.
- Use the manual `Benchmarks` workflow when a PR needs CI-side performance
  validation but did not touch files covered by the benchmark path filters.
- Say whether the change was tested with `--features annan`.
- Do not include generated training outputs or large model files directly in PRs.
- Agree that your code will be shipped under MIT License before submitting PRs.
