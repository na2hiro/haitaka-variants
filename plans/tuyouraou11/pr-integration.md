# PR統合時の検証

最新main `121dda7` とWeb master `abff8be2` を取り込み、エンジンの競合箇所は
SFNN loaderとmainのterminal_notificationsの両APIを保持して解消した。
検証したengineコードは `459c3b9`、再生成済みWebコードは `db92feba`。
この記録の追加はコードを変更しない。

- `cargo test --workspace --release`: 474 passed / 5 ignored。
- `cargo test -p haitaka_wasm --features annan`: library93 passed。SFNNの非対応build拒否も確認。
- micro/mini各236参照局面・2048手、runtime/TT/評価範囲/モデル切替を実モデルで再検証。
- main統合後の診断Wasmをscalar/SIMDで生成し、Nodeで両モデルの236局面・2048手・固定深さ20探索を完全一致確認。
- 統計/改変検知4テストと保存済み400局・43,814手の応答ログ監査を再実行。
- 変更Rustソースのrustfmt確認合格。

本流統合後の配信Wasm（scalar/SIMD）もWeb側へ再同期した。
ハッシュ・診断結果・検証logのSHA-256は [pr-integration.json](pr-integration.json) に記録する。
Web側のUI/E2E結果はWeb PRに記載する。

400局の棋力結果・性能のraw記録は統合前の固定binaryに対応し、当時のhashを保持する。
統合後に400局を再測定したとは扱わない。mainからはUSI終局通知/DFPN証拠のAPI・回帰修正等を取り込んでいる。
旧診断/モデル取得/棋譜記録の再現条件は各記録を参照。

通常debugのworkspace testは既存DFPN回帰の長時間実行中に打ち切り、workspace全体はreleaseで完走した。
実機検証はユーザー指示で対象外。旧HalfKAv2外部実モデルの実ファイル検証は今回追加していない。
