# 進捗・引継ぎ

更新: 2026-10-30。実装契約: [contract.md](contract.md)。

| 作業 | 担当 | 状態 | 成果物/依存 |
|---|---|---|---|
| P0 | LEAD | 完了 | contract.md、reference-fixture.schema.json、p0-lead.md |
| P1 | ENGINE | 完了 | p1-engine.md、provenance.json、reference-fixture.json、scalar評価器 |
| P2 | ENGINE | 未着手（開始可能） | 差分評価・探索統合・Wasm API。対局用評価選択harnessも提供 |
| P3 | WEB | 完了（WEB報告、P4統合待ち） | p3-web.md参照。実Wasm API接続はP2/P4待ち |
| P4 | LEAD | 依存待ち | P2/P3完了、実機性能予算固定と実機確保 |
| P5 | 各担当 | 未割当（任意） | package v2 |
| P6 | 各担当 | 未割当（任意） | mini/SIMD |

## ブランチと成果物の所在

- haitaka-variants: feat/tuyouraou11-p0、base d0c29c0c7835f9dfed36323b59aceac06265f603。
- shogitter-3: feat/tuyouraou11-p0、base c94a8d630ba879b59a098719e420e561128e7d49。
- P0コミット: haitaka 5d8dee3、shogitter b9c857b9。
- WEB P3は並行作業のp3-web.md報告を受領。ENGINEはWEBソースを変更していない。
- ENGINE P1作業: haitakaのfeat/tuyouraou11-p1（base 5d8dee3）。P1成果物は本ブランチのP1コミットに収録。shogitter側は変更なし。
- 実binとGPL参照ソース/ビルドはGit対象外のtarget/tuyouraou11/。取得・hash検査をスクリプト化済み。
- 開始時点の他の未追跡ファイルは保持。P0成果物のみ明示指定してコミット。デプロイなし。

## 未決事項と期限

| 項目 | 解決担当 | 期限/完了条件 |
|---|---|---|
| 実binのbytes/hash、配布ライセンス詳細、対応上流commit | ENGINE P1 | 解決済み。provenance.json / MODEL-LICENSE.txt / p1-engine.md参照 |
| progressの視点名/和の合成と参照行 | ENGINE P1 | 解決済み。black/white両和の合計。docs/tuyouraou11-sfnn.md参照 |
| desktop/iOS/Android実機の型番・版の確保 | LEAD P4 | 性能予算の固定前。未確保ならP4完了不可 |
| 準備時間/メモリ/探索超過の数値上限 | LEAD P4、WEBが配信制約提供 | SFNN採否測定前に従来版を測定しperformance-budget.mdを凍結 |
| 実機で計測できないメモリ値の代替測定 | LEAD P4 | P4判定前。未測定をゼロ扱いしない |

次の割当例: 「担当=ENGINE、フェーズ=P2」および「担当=WEB、フェーズ=P3」。共通の計画書・contractを必ず読む。
