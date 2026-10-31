# 進捗・引継ぎ

更新: 2026-10-31。実装契約: [contract.md](contract.md)。

| 作業 | 担当            | 状態                                       | 成果物/依存                                                                         |
| ---- | --------------- | ------------------------------------------ | ----------------------------------------------------------------------------------- |
| P0   | LEAD            | 完了                                       | contract.md、reference-fixture.schema.json、p0-lead.md                              |
| P1   | ENGINE          | 完了                                       | p1-engine.md、provenance.json、reference-fixture.json、scalar 評価器                |
| P2   | ENGINE          | 完了                                       | p2-engine.md、p2-artifacts.json、差分/探索/load_sfnn/実 Wasm 検証                   |
| P3   | WEB             | 完了（WEB 報告、P4 統合待ち）              | p3-web.md 参照。P2 の実 Wasm を P4 で同期                                           |
| P4   | LEAD            | ローカル検証完了・実機保留（全体は未完了） | p4-lead.md。実 Wasm/12 E2E/720 合法応答/200 局比較 PASS。実機はユーザー指示で後回し |
| P5   | 各担当          | 未割当（任意）                             | package v2                                                                          |
| P6   | ENGINE→WEB→LEAD | 完了（SIMD・モデル選択・棋力400局、実機対象外）    | p6-strength.md（棋力400局）/ p6-mini.md（モデル選択）/ p6-lead.md（SIMD履歴）                                |

## ブランチと成果物の所在

- haitaka-variants: feat/tuyouraou11-p0、base d0c29c0c7835f9dfed36323b59aceac06265f603。
- shogitter-3: feat/tuyouraou11-p0、base c94a8d630ba879b59a098719e420e561128e7d49。
- P0 コミット: haitaka 5d8dee3、shogitter b9c857b9。
- WEB P3 は並行作業の p3-web.md 報告を受領。ENGINE は WEB ソースを変更していない。
- ENGINE P1 作業: haitaka の feat/tuyouraou11-p1（base 5d8dee3）。P1 成果物は 428e32f に収録。shogitter 側は変更なし。
- ENGINE P2 作業: feat/tuyouraou11-p2（base 428e32f）。P2 成果物は本ブランチの P2 コミットに収録。通常 Wasm は target/tuyouraou11/pkg/。
- 実 bin と GPL 参照ソース/ビルドは Git 対象外の target/tuyouraou11/。取得・hash 検査をスクリプト化済み。
- 開始時点の他の未追跡ファイルは保持。P0/P1/P2 成果物のみ明示指定してコミット。デプロイなし。

## 未決事項と期限

| 項目                                                      | 解決担当                    | 期限/完了条件                                                                         |
| --------------------------------------------------------- | --------------------------- | ------------------------------------------------------------------------------------- |
| 実 bin の bytes/hash、配布ライセンス詳細、対応上流 commit | ENGINE P1                   | 解決済み。provenance.json / MODEL-LICENSE.txt / p1-engine.md 参照                     |
| progress の視点名/和の合成と参照行                        | ENGINE P1                   | 解決済み。black/white 両和の合計。docs/tuyouraou11-sfnn.md 参照                       |
| desktop/iOS/Android 実機の型番・版の確保                  | LEAD P4                     | 性能予算の固定前。未確保なら P4 完了不可                                              |
| 準備時間/メモリ/探索超過の数値上限                        | LEAD P4、WEB が配信制約提供 | desktop は performance-budget.md で凍結・測定済み。実機予算はユーザー指示により後回し |
| 実機で計測できないメモリ値の代替測定                      | LEAD P4                     | P4 判定前。未測定をゼロ扱いしない                                                     |

次の割当例: 「担当=LEAD、フェーズ=P4」。共通の計画書・contract を必ず読む。
