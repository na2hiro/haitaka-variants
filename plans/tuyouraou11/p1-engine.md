# P1 / ENGINE 結果

実施: 2026-10-30。状態: 完了。ブランチ: feat/tuyouraou11-p1、base: 5d8dee3。
P1変更は本ブランチのP1コミットに収録。P2/P3/P4の実装やデプロイは行っていない。

## 成果物

- haitaka_wasm/src/sfnn.rs: 独立したMITのscalar SFNN読込・全再計算。既存HalfKAv2と探索経路は変更なし。
- haitaka_wasm/src/lib.rs: nativeのsfnnモジュール公開のみ。wasm-bindgen load_sfnnはP2。
- haitaka_wasm/examples/sfnn_positions.rs: 明示LCG64（seed=1）の合法局面生成器。
- haitaka_wasm/tests/sfnn_reference.rs: 明示実行する実モデル照合と破損拒否テスト。
- scripts/tuyouraou11.py、tuyouraou11-reference-command.inc: 固定モデル取得・外部参照ビルド・fixture生成・検証。
- provenance.json、MODEL-LICENSE.txt、reference-fixture.json: 実体hash/出典と正解データ。
- docs/tuyouraou11-sfnn.md、haitaka_wasm/README.md: 数学仕様・参照行・再実行手順。

## モデルと参照

microのnn.bin=3,864,610 bytes、progress.bin=1,003,104 bytes。両SHA-256と配布archive SHA-256はprovenance.jsonに記録。モデルIDはtuyouraou11micro-20261001-epoch0010-7f9545dbf73b-a2d18f056463。

固定参照はYaneuraOu 7d9743646f1006ff6d84da45d6d6160048a5a416、GNU g++ 15.2.0、TARGET_CPU=OTHER、FV_SCALE=24。モデル読込時にhash mismatchなし。評価やprogressロジックを変更せず診断コマンドだけを追加。GPLソースとbinaryはtarget配下に保持しHaitakaにはリンクしない。

アーカイブの内容はbanner.png、eval_options.txt、nn.bin、progress.bin、tag.json。別のLICENSE/著作権表記はない。記事のMIT明示許諾とtag.jsonの作者（やねうら王プロジェクト）を記録し、著作権年を推定しない。配信対象は2つのbinだけでbannerは含めない。MODEL-LICENSE.txtとprovenanceをbinに添付して渡す。

## 実行と検証結果

作業ディレクトリは全て ~/proj/haitaka-variants。

1. `git switch -c feat/tuyouraou11-p1`: 成功、P0の5d8dee3から作成。
2. 配布物取得/展開、固定参照のmake: 成功。初回はPython名と32bit用NO_SSEの設定で失敗したため、PYTHON=python3/TARGET_CPU=OTHERへ修正。最終コマンドはdocsと専用スクリプトに記録。`python3 scripts/tuyouraou11.py reference` 自体の最終実行も成功（reference/repro-check.log）。
3. `python3 scripts/tuyouraou11.py fetch`: 成功。archive/binのSHA-256とbytesを検証し、配信用provenance/LICENSEもtarget/tuyouraou11/model/へ配置。
4. `python3 scripts/tuyouraou11.py fixtures`: 成功。外部参照から236局面を採取、P0 JSON Schemaに適合。先後各118、序盤32/中盤96/後半108、持駒あり197/成駒あり195。全8bucketを含む。
5. `python3 scripts/tuyouraou11.py verify`: 成功。236局面のblack/white Q16和、合計、bucket、静的整数評価値が全て完全一致。実ファイルの欠損/切断/余剰、NaN/Inf、version/hash/description/marker破損、未知arch/mini拒否を確認。通常テストのignoreを合格結果として扱っていない。
6. `cargo test -p haitaka_wasm`: 95 passed、0 failed（127.94秒）。実モデルテスト1件は明示ignoreであり、別途5で成功。
7. `cargo test -p haitaka_wasm --release`: 95 passed、0 failed（3.51秒）。実モデルテストは別途5。
8. `cargo test -p haitaka_wasm --features annan sfnn --lib`: 3 passed、0 failed。変則ビルドがSFNNを拒否。
9. `rustfmt --edition 2024 --check haitaka_wasm/src/sfnn.rs haitaka_wasm/tests/sfnn_reference.rs haitaka_wasm/examples/sfnn_positions.rs`: 成功。
10. Python AST/JSON Schema、生成器hash、64以上のunique SFEN、bucket 0..7、Q16和の合成整合性、`git diff --check`: 成功。

読込器は符号付きLEB128、microの1791特徴量、2048幅、8/64層、8stack、平方活性化、進行度の外部binを扱う。色はHaitakaのWhite/Black順を参照Black/White順へ明示変換。有限の範囲外progress値は参照に合わせて飽和し、P0の表現を明確化した。公開APIの契約変更はない。

## 終了条件と次担当

P1終了条件（実モデル/参照revisionとライセンス情報の固定、完全一致、異常入力拒否、WEBへの資産受渡し手順）は全て達成。

ENGINE/P2はこのブランチを起点に、差分状態/探索統合/atomic交換・TT消去/トップレベルとUsiEngineのWasm API/対局用評価選択を実装する。P1の公開SfnnEvaluationが全再計算の比較基準となる。

並行WEB/P3の完了報告（p3-web.md）を確認。ENGINEはその成果物を変更していない。P3側snapshotとP1モデルID/hashは一致する。nn.bin/progress.bin/provenance.json/LICENSE.txtの4ファイルをSHA-256で突合して同一と確認。実Wasm統合は引き続きP2/P4。

WEB/P3は `python3 scripts/tuyouraou11.py fetch` でtarget/tuyouraou11/model/のnn.bin、progress.bin、provenance.json、LICENSE.txtを取得できる。両hashとモデルIDは確定。実Wasmのload_sfnnはP2待ちのため準備処理はcontractに基づくモックから着手する。

差分/Wasm/実機/棋力は未検証（P2/P4担当）。mini/SIMD/package v2は範囲外。ランダム合法手列の後半局面を戦略的な終盤テストや棋力の証明とは扱わない。
