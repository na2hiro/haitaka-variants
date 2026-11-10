# 追加測定の原本

実装はエンジン58d5802、将棋ったーc0d003b0。後続の測定・レポート追加は実装を変更していない。
[測定表と定義](../../../../docs/historical-shogi-benchmarks.md)を参照。

- historical-all-perft.json: 本将棋平手＋古将棋14種、Native、初期局面、3秒以上×3回。天竺大・大局は深さ3。
- allrounder-comparison.json / .csv: 現行オールラウンダーAIとの同じ深さでの実測と算術比。大大・泰将棋は葉数が異なる参考値。
- honshogi-impact.json: 古将棋追加前9d90ef3→58d5802。本将棋の通常/SIMD WASMサイズとNative/Chrome perft NPS。5秒以上×5回。
- honshogi-wasm-original.json: 最初の通常WASM初期局面で揺れがあった元試行。warmupを15秒へ延長した全再測定はhonshogi-wasm-remeasure.json。
- local-play-19.json: 起動中の将棋ったーで古将棋14種・小盤5種の準備と実応手を確認したChrome19件。
- manifest.json: 全保存ファイルのサイズとSHA-256。

測定スクリプトはdriversに原本を保存。記録されたローカルパスをそのまま残しているため、別環境ではパスの調整が必要。
Allrounder v1は先頭5ルール（3秒warmup/3秒以上の測定）、v2は中将棋以降（10秒warmup/5秒以上の測定）で使用。
どちらも各perftで新しいゲームラッパーを作り、別のperft呼出しとの移動候補キャッシュ共有を避ける。1回のperft内の製品実装のキャッシュは維持。
ルール追加前後の実装は同じ専用checkoutパスへ切り替えてビルドし、選択された依存ツリーの一致も保存した。

既存の再現手順はdocs/large-shogi-performance-2.mdとscripts/perft-benchmark.pyを参照。
測定中にビルド・参照照合・ブラウザ試験等を重ねない。
