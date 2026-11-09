# 古将棋の合法手生成・perft性能

状態: 第1期完了。この文書は第1期の測定記録。
天竺大・大局の追加高速化を含む最新結果は[第2期の性能記録](large-shogi-performance-2.md)を参照。
段階別の採否・試作・全サンプルは[計画と記録](../plans/historical-shogi-performance/README.md)を参照。

## 測定方法

環境はCore i7-6700、Rustc 1.98.1、Node v26.8.1。論理CPU 7へ固定し、1スレッドで測定した。

NativeのNPSは、評価・枝刈り・置換表・末端一括カウントを使わず、
各手を葉まで実際に適用・復元するperftの葉ノード数/秒。
固定CPU・1スレッド・Releaseでウォームアップ後に3秒以上×3回測り、中央値を使う。
初期局面32ルールは固定の深さと期待葉数を維持し、途中局面・特殊効果・継続中の入力は
[固定入力一覧](../plans/historical-shogi-performance/inputs/positions.json)に由来・SHA・深さ・期待葉数を残す。
JSONの復元と履歴再生はNative内部perftの計時に含めない。

実WASMは同一Node・同一入力・CPU固定で変更前後を交互に測る。
JSON API全体の時間と、APIが返す内部perftの時間を別に記録する。
異なる深さ・盤面・測定範囲のNPSを、同じ仕事量の絶対比較として扱わない。

独立ソースの診断器は本体へ計時器を入れず、包含時間と確保回数を調べるために使う。
包含区間は重なり、計時器はコンパイラの最適化にも影響するので、通常版NPSとは区別する。
確保バイトはalloc/reallocの累積要求量で、ピーク生存メモリではない。

## 実装と互換性

- 和・平安大の王手検査は、全移動先のVecを作らず王マスへ届くかを直接調べる。
- 大型5ルールは探索用の差分キーと履歴順キーを使う。公開FNVハッシュは従来値を保つ。
- 天竺大は手番元の火鬼・四天王情報を共有し、捕獲・成り・焼失・在場順を候補ごとに反映する。
- 大型の実着手は既知の行先だけの情報を保持する。全経路制約を検証し、外部の手を無検証で適用しない。
- 大将棋の検証用コピーは筋の枚数差で省略する。王手検査が必要な古典ルールはコピー判定を維持する。
- 大型の差分は2マスまでinlineに保持し、超過時は全変更を連続したVecへ移す。一括捕獲や焼却も切り詰めない。

scratch試作は和将棋のNPS低下で不採用とした。採否・ソース・測定値は
[第4段階](../plans/historical-shogi-performance/04-memory.md)に保存している。

公開JSON形式・成り元・王の集合・履歴/pending・待った・終局判定との互換性を検証する。
能力ガードもルールごとの既存契約を保つ。平安3種のSFEN/USIと、
残る古将棋のJSON v1〜v5を同一形式へ統一する変更は行わない。

## 再現

最適化前の比較元はe7ab6efc40e8838d62185e6a69d96a3ad28041e8。
Cargo.lockはGit管理外なので、[保存した依存ロック](../plans/historical-shogi-performance/inputs/dependency-Cargo.lock)で前後の依存を固定する。
SHA-256は21e149581bcd526d06d4f0ad905d5de4aee1f1e1fb0c94730e8d0700421a0262。
比較用checkoutが既にある場合はHEADが上記の最適化前コミットと一致するものを再利用する。

~~~sh
cp plans/historical-shogi-performance/inputs/dependency-Cargo.lock Cargo.lock
git worktree add --detach target/perft-original e7ab6efc40e8838d62185e6a69d96a3ad28041e8
cp Cargo.lock target/perft-original/Cargo.lock
python3 scripts/perft-benchmark.py --engine-dir target/perft-original --output target/perft-original.json
git -C target/perft-original switch --detach d95f3b3
python3 scripts/perft-benchmark.py --engine-dir target/perft-original \
  --compare target/perft-original.json --output target/perft-final.json

git -C target/perft-original switch --detach e7ab6efc40e8838d62185e6a69d96a3ad28041e8
python3 scripts/perft-position-benchmark.py --engine-dir target/perft-original \
  --positions plans/historical-shogi-performance/inputs/positions.json --output target/positions-original.json
git -C target/perft-original switch --detach d95f3b3
python3 scripts/perft-position-benchmark.py --engine-dir target/perft-original \
  --positions plans/historical-shogi-performance/inputs/positions.json \
  --compare target/positions-original.json --output target/positions-final.json
~~~

Rustのcrate識別子やコード配置にパスの差が混じらないよう、同じcheckoutパスを旧版・現行版へ切り替える。
測定器は前後のengineDirectory・依存ロック・ハーネス・CPU・設定の一致を検査する。
現行ソースの測定コミットはd95f3b3で、エンジン実装はc19c890と同一。

期待葉数は既に固定してある。再校正する場合だけ、変更前checkoutと--calibrateを使い、
その後は入力・深さ・期待葉数を変更せずに前後を測る。
ビルドは測定器がすべて終えてから計時する。測定中に別ビルド・参照照合・ブラウザ試験を走らせない。

## 互換性の検証

最終ソースから14ルールのNative/WASMを再生成し、全6,209局面・526,295遷移が一致した。
JSONの11ルールは、以前に独立したShogitterと照合したコーパスへ新Nativeを実行し、
その全合法手・遷移・勝敗・ハッシュと新WASMを比較した。
大型5ルールは公開position/contextハッシュを旧配布WASMとも照合し、履歴/pendingの再読込を確認した。
平安3種は新Nativeの453局面・10,239遷移をShogitterと独立比較し、
検証専用WASMラッパーの全出力ともバイト単位で比較した。配布版のUSI/perft/searchも別に検証した。

Rustワークスペース477件と14feature別テスト、将棋ったーcore 2,580件・AI CI 436件・app 726件、
型検査とlintが通過した。実Chromeでは古将棋77件と小盤5件の計82件が通過し、
通常対局・途中再開・4段移動・待った・終局・ルール切替とpageerrorなしを確認した。
[全検証の記録](../plans/historical-shogi-performance/results/05-correctness.json)。

配布先は将棋ったーのperf/historical-shogi-movegen、WASM更新コミットは3cbfc427。
全14ルールを同じraw WASMからbindgen JS/WASMの組として再生成した。
[生成物ハッシュ](../plans/historical-shogi-performance/results/05-frontend-assets.json)。
既存fixture7件の整形は0fcc1506に分離し、JSONの意味が同一であることをSHA-256で確認した。

## 最終の測定結果

Nativeは同じ専用checkoutパスをe7ab6efとd95f3b3へ切り替えて測った。
両版のCPU・コンパイラ・依存ロック・Rustハーネス・深さ・葉数が一致し、
各バッチ3秒以上×3回の中央値を使う。全サンプル幅は5%以内。
非対象ルールに5%以上の低下はない。過去の段階別倍率を掛け合わせず、この直接比較を最終値とする。

### Native・初期局面32ルール

| ルール | perft深さ | 葉数/1回 | 前 NPS | 後 NPS | 倍率 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 本将棋 | 5 | 19,861,490 | 18,792,003 | 18,830,504 | 1.00 |
| 5五将棋 | 7 | 132,680,617 | 25,251,125 | 25,321,634 | 1.00 |
| ジャドケンス将棋 | 6 | 51,190,232 | 27,347,047 | 27,285,774 | 1.00 |
| ゴロゴロ将棋 | 6 | 20,552,765 | 25,280,761 | 25,860,951 | 1.02 |
| ゴロゴロ将棋+桂香 | 5 | 36,102,221 | 31,563,224 | 31,648,483 | 1.00 |
| 5五将棋 金銀 | 5 | 36,536,515 | 32,117,483 | 32,548,570 | 1.01 |
| 安南将棋 | 5 | 16,593,022 | 7,187,814 | 7,208,836 | 1.00 |
| 安北将棋 | 5 | 9,673,891 | 6,733,600 | 6,763,085 | 1.00 |
| 安東西将棋 | 5 | 26,211,862 | 5,782,872 | 5,809,791 | 1.00 |
| 対面将棋 | 5 | 19,867,515 | 7,027,895 | 7,013,371 | 1.00 |
| 背面将棋 | 5 | 19,864,557 | 6,706,562 | 6,718,529 | 1.00 |
| ネコ将棋 | 4 | 500,040 | 787,524 | 787,019 | 1.00 |
| ネコネコ将棋 | 4 | 500,274 | 164,764 | 164,409 | 1.00 |
| 横ネコ将棋 | 4 | 626,210 | 785,142 | 787,719 | 1.00 |
| 横ネコネコ将棋 | 4 | 627,302 | 197,546 | 196,121 | 0.99 |
| 点鏡将棋 | 4 | 716,743 | 1,221,608 | 1,233,169 | 1.01 |
| 天竺将棋（9×9） | 4 | 2,187,650 | 3,176,910 | 3,205,414 | 1.01 |
| 安騎将棋 | 4 | 2,272,871 | 2,093,646 | 2,084,810 | 1.00 |
| 平安将棋8×8 | 6 | 96,673,959 | 32,541,540 | 32,534,820 | 1.00 |
| 平安将棋9×8 | 5 | 10,804,053 | 20,007,656 | 19,933,310 | 1.00 |
| 平安将棋9×9 | 5 | 10,799,089 | 19,879,629 | 20,033,559 | 1.01 |
| 小将棋 | 5 | 11,553,437 | 7,582,222 | 7,545,407 | 1.00 |
| 中将棋 | 4 | 2,134,748 | 5,594,354 | 5,595,979 | 1.00 |
| 禽将棋 | 5 | 2,179,749 | 1,089,904 | 1,093,919 | 1.00 |
| 和将棋 | 3 | 126,574 | 664,067 | 1,621,971 | 2.44 |
| 平安大将棋 | 4 | 748,121 | 673,935 | 1,443,162 | 2.14 |
| 大将棋 | 4 | 25,451,513 | 3,472,551 | 4,812,515 | 1.39 |
| 天竺大将棋 | 2 | 17,226 | 104,611 | 430,212 | 4.11 |
| 大大将棋 | 3 | 251,479 | 646,200 | 1,426,758 | 2.21 |
| 摩訶大大将棋 | 3 | 537,260 | 602,996 | 1,716,980 | 2.85 |
| 泰将棋 | 2 | 92,676 | 420,841 | 1,408,141 | 3.35 |
| 大局将棋 | 2 | 79,035 | 88,481 | 302,296 | 3.42 |

[変更前の全サンプル](../plans/historical-shogi-performance/results/05-native-before.json) / [変更後](../plans/historical-shogi-performance/results/05-native-after.json)。

### Native・固定途中局面16件

JSON復元と履歴再生を計時から除き、初期局面と同じmeasure()を使う。
入力・由来・履歴・SHA・期待葉数は[manifest](../plans/historical-shogi-performance/inputs/positions.json)に固定してある。

| 固定入力 | perft深さ | 葉数/1回 | 前 NPS | 後 NPS | 倍率 |
| --- | ---: | ---: | ---: | ---: | ---: |
| tenjiku-dai-midgame | 2 | 201,294 | 136,908 | 454,033 | 3.32 |
| dai-dai-midgame | 2 | 4,216 | 555,964 | 1,277,108 | 2.30 |
| maka-dai-dai-midgame | 2 | 22,908 | 472,923 | 1,253,966 | 2.65 |
| taishogi-midgame | 2 | 95,766 | 392,533 | 1,225,147 | 3.12 |
| taikyoku-midgame | 2 | 616,706 | 144,757 | 484,234 | 3.35 |
| washogi-midgame | 3 | 156,058 | 638,866 | 1,528,541 | 2.39 |
| heian-dai-midgame | 3 | 41,474 | 661,828 | 1,400,073 | 2.12 |
| daishogi-midgame | 2 | 9,316 | 3,028,205 | 4,418,124 | 1.46 |
| torishogi-midgame | 3 | 11,566 | 1,169,035 | 1,172,831 | 1.00 |
| shoshogi-midgame | 3 | 23,864 | 6,605,748 | 6,617,866 | 1.00 |
| tenjiku-dai-pending-two | 2 | 5,565 | 176,700 | 597,766 | 3.38 |
| taikyoku-pending-three | 3 | 2,025 | 100,561 | 517,031 | 5.14 |
| chushogi-pending-lion | 3 | 5,833 | 6,570,882 | 6,500,132 | 0.99 |
| tenjiku-dai-ordered-fire | 2 | 55,653 | 94,521 | 246,522 | 2.61 |
| taikyoku-transit-ray | 2 | 1,092 | 445,928 | 1,246,605 | 2.80 |
| maka-dai-dai-conversion | 4 | 5,978 | 798,274 | 1,592,993 | 2.00 |

[変更前の全サンプル](../plans/historical-shogi-performance/results/05-positions-before.json) / [変更後](../plans/historical-shogi-performance/results/05-positions-after.json)。

### 配布WASM・Nodeの実JSON API

同じNode・CPUで旧配布物と更新物を交互に3秒以上×3回測った。
比較元は将棋ったーbdec1bbの各ルールの配布バイナリ、更新先は3cbfc427。
Nativeのe7ab6ef→d95f3b3と、WASMの配布物同士の倍率をそれぞれの比較組から計算する。
API NPSはJSON復元・履歴再生・結果JSONを含む。内部perftの計時は別列に示す。
禽将棋はJSON APIの深さ上限4を使うため、Nativeの深さ5と絶対NPSを直接比較しない。
Chromeは機能回帰を確認し、ここのWASM NPSはNodeで測定した。

| 入力 | perft深さ | 葉数/1回 | 前 API NPS | 後 API NPS | 倍率 | 後 内部perft NPS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| torishogi-initial | 4 | 103,857 | 849,558 | 861,726 | 1.01 | 862,946 |
| washogi-initial | 3 | 126,574 | 529,949 | 1,301,232 | 2.46 | 1,306,511 |
| heian-dai-initial | 4 | 748,121 | 507,513 | 1,156,999 | 2.28 | 1,157,295 |
| daishogi-initial | 4 | 25,451,513 | 2,529,542 | 3,411,994 | 1.35 | 3,412,135 |
| tenjiku-dai-initial | 2 | 17,226 | 151,408 | 490,719 | 3.24 | 494,224 |
| tenjiku-dai-pending-two | 2 | 5,565 | 186,940 | 493,237 | 2.64 | 495,275 |
| dai-dai-initial | 3 | 251,479 | 136,836 | 1,095,710 | 8.01 | 1,099,198 |
| maka-dai-dai-initial | 3 | 537,260 | 126,710 | 1,264,999 | 9.98 | 1,266,752 |
| taishogi-initial | 2 | 92,676 | 380,221 | 1,091,061 | 2.87 | 1,095,746 |
| taikyoku-initial | 2 | 79,035 | 123,314 | 295,089 | 2.39 | 297,530 |
| taikyoku-pending-three | 3 | 2,025 | 104,862 | 383,119 | 3.65 | 388,951 |

[WASMの全サンプル・バイナリSHA](../plans/historical-shogi-performance/results/05-wasm-perft.json)。

通常5秒予算では配布WASM全14ルールが合法な完了手を返し、初期化5秒・線形メモリ64MiBの基準を満たした。
1ms予算の実測時間と継続中の合法なfallbackも記録する。
[通常5秒思考](../plans/historical-shogi-performance/results/05-normal-budget.json) / [完了記録](../plans/historical-shogi-performance/results/05-final.json)。

通常5秒思考の内部elapsedMsは約4,990〜5,000ms、API壁時計は約5,001〜5,013msだった。
最大線形メモリは52,822,016バイト（約50.4MiB）。全14ルールが合法な完了手を返した。
