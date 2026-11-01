# P4 の保存データ

`baseline-{chromium,firefox}.json` と `sfnn-{chromium,firefox}.json` はブラウザ測定の未加工出力。
各ファイルに cold/warm 各10回、固定20局面×3回×3時間条件の180探索、browser版、host、HTTP要求を含む。
ファイルのSHA-256は親の `p4-browser-summary.json` に記録しているため、整形して書き換えない。

`match-{200,1000}/self-play-games.jsonl` は、先後交換50ペアの100局ずつの全手と終局理由。
同ディレクトリの `self-play-report.json` は対局CLIの集計。両条件の完了済み全200局を保存した。
採否に用いる95%区間はCLIの別手法を流用せず、共通契約どおりの時間別層化・ペアbootstrapで計算する。

ENGINE repoのルートから再集計できる。

```bash
cargo build -p haitaka_wasm --release --example sfnn_verify_moves
python3 scripts/tuyouraou11-p4-summary.py browser --records-dir plans/tuyouraou11/p4-results
python3 scripts/tuyouraou11-p4-summary.py games --records-dir plans/tuyouraou11/p4-results
```

`browser-summary.json` / `games-summary.json` は上のコマンドで再生成する。
ブラウザ集計は720手の合法性と凍結予算も検証する。対局集計は200局の重複、固定開始局面、先後ペア、
終局理由を検証し、10000回・seed=1のbootstrapを実行する。
