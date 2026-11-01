# P0 / LEAD 結果

実施: 2026-10-30。状態: 完了（計画に定めるP0終了条件）。初回導入全体は未完了。

## 実施内容

- 両repoで新ブランチfeat/tuyouraou11-p0を作成。既存のtracked変更なし、未追跡ファイルを保持。
- 両AGENTS.md、WorkerBot/Bot/WebWorkerBot/bot.worker、vendor runtime、SelfBotShogi、rule mapping、CLI対局引数、Cargo featureを再確認。
- contract.mdでAPIの同期性/返り値/エラー、2ファイルdescriptorとprovenance、Workerメッセージ、retry/dispose、時間起点、fixture、対局基準を固定。
- standard共有による他ruleへの評価器漏れと、busy restart後の準備漏れを実装上の必須確認に追加。
- reference-fixture.schema.jsonを作成。実値はP1担当が固定参照から生成する。
- shogitter側へ作業開始点と共通契約への参照を置いた。

## 確認コマンド

両repo: `git status --short`、`git branch --show-current`、`git rev-parse HEAD`。
ブランチ作成: 各repoで `git switch -c feat/tuyouraou11-p0`。
文書検査: Pythonでschema JSON読込・Draft202012Validator.check_schema、ダミー構造の適合/異常値拒否と必須成果物確認。結果: PASS。JSON Schema自体の妥当性、64件の合成構造の受理、bucket=8・数値型Q16・mini archの拒否を確認。合成データはschema検査専用で、正解fixtureとして保存していない。

## 終了条件の判定

- 型/準備完了条件/API責任境界: 確定。
- 比較方法: fixtureの完全一致、合法手列、境界テスト、200局の対局条件と区間計算法を固定。
- 未決項目の担当/期限: status.mdへ記録。
- 上流master固定: P1で実binと一致するcommitを固定する責任を明記。
- 数値性能閾値: 実機根拠がなく未確定。計画P0の許容する繰延べとしてP4開始時の測定・採否前の凍結を指定。測定済みとは報告しない。

コード/モデル/ブラウザ動作の検証はP0では実施していない。P1以降へ自動では進まない。

## 最終検査

両repoの現在ブランチがfeat/tuyouraou11-p0であることを確認。git diff --checkは両repoで成功（新規未追跡文書は別途読込確認）。必須成果物の存在確認とschema検査は成功。アプリ/エンジンテストは文書のみのP0のため未実行。P0成果物はユーザー指示により本ブランチへコミット。両repoのコミットIDは各repoで `git log -1 --oneline` により確認できる。
