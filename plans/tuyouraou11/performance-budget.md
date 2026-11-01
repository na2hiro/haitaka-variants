# P4 性能予算（desktop 限定・採否測定前に固定）

固定: 2026-10-30。SFNN 性能/200局比較の採否を確認する前に記録。
端末: Intel Core i7-6700 / Ubuntu 26.04 x86_64、kernel 7.0.0-31-generic、CPU governor powersave。
ブラウザ: headless Chromium 153.0.8010.12、Playwright Firefox 142.0.1（Ubuntu 24.04互換バイナリ）。
これは desktop のソフトウェア検証であり、実 iOS/Android、画面を持つ Chrome/Firefox の検証へ読み替えない。

計測 CPU は `taskset -c 0,4`（同一物理coreの2 logical CPU）で固定。HTTP はローカル専用サーバー、
モデル版固定 URL、1年 immutable。cold は新 browser context、warm は同じ context 内の新 Worker。
他のビルドや E2E が同じホストで動いたことは生ログと実施記録に残し、専用端末の無負荷値とは主張しない。

固定20局面: `p4-positions.json`。
SHA-256: `14ebf71679d38aba4587652743f0403c32349f7c123b0f51ac1716d957f91e12`。
P1 corpus 236局面から等間隔で選び、SFNN計測前に固定。

## 従来評価の測定根拠

同じ P2 エンジンのモデル未ロード状態を用いる。
Chromium は cold/warm 各10回と 20局面×3回×200/1000/3000ms の180探索を完了。
準備最大261.4ms、Wasm memory 最大48,824,320bytes。
engine時間超過最大14.7ms、main往復の超過最大16.2ms。
Firefox は同じ準備20回（最大271ms）と200msの60探索が完了した時点で本予算を固定した。
残り120探索も同じ基準測定として継続し、数値予算はその結果に合わせて緩めない。
生データは `target/tuyouraou11/p4/baseline-{chromium,firefox}.json`。

## 凍結した数値基準

両 desktop browser へ同じ上限を適用する。

| 項目 | 上限/条件 | 根拠 |
|---|---|---|
| cold/warm 準備の最大 | 10,000ms | 既存最大0.271秒 + 4,867,714bytesを1MiB/sで受ける約4.64秒 + 検証/展開/環境変動に約5秒。通信速度の実測保証ではない |
| Wasm memory.buffer.byteLength 最大 | 134,217,728bytes (128MiB) | baseline約46.6MiB×2 + 配信モデル約4.64MiB×4 + Q16約0.48MiBを切り上げ。配信容量とは別の上限 |
| engine elapsed - movetime の最大 | 100ms | baseline最大14.7msの約6.8倍。200/1000/3000msすべてに適用 |
| main往復 - movetime の最大 | 200ms | baseline最大16.2msの約12倍。Worker通信/スケジューリング分を含む |
| 応答性 | 全探索でmainの10ms timerが進み、実画面でも入力可能 | main threadで同期探索していないことを確認 |
| 安定性 | crash/不正着手/準備失敗0件 | 失敗を通常の敗戦やtimeoutとして隠さない |

JSの明示的モデル受信データ量は 4,867,714bytes。Wasmへのコピーと重み展開はWasm memoryへ含まれる。
ブラウザ内部コピー、JS heap/process RSSのピークはこの測定では取得しておらず、0や受信量と同一と扱わない。
全端末の総プロセスメモリ予算/代替根拠は未決のため、初回導入全体の性能合格とはしない。

ホスティングは既存Fly/Express配信のstatic長期キャッシュを使用する。モデル追加容量は約4.64MiB。
Fly設定にこのファイルサイズを禁止する制約はないが、実CDN/携帯回線での配信性能は未測定。

## 未確定・別判定

- iOS Safari実機、Android Chrome実機: ユーザー指示で今回の検証は後回し。端末/OS/版・省電力状態、従来評価測定と予算は後続作業に残す。
- headful/製品版Chrome・Firefox、モバイル実機の性能、HTTP実回線のcold/warmは未測定。
- 200局の棋力判定はcontract.mdの先後交換・時間別bootstrap基準に従い、この性能予算で置き換えない。
- 未検証必須項目がある限りP4/初回導入全体を完了としない。失敗後に本予算を緩めない。
