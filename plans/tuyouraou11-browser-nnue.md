# つようら王11 NNUEをブラウザで利用する実装計画・将棋ったー変更仕様

作成: 2026-10-30
調査対象: haitaka-variants d0c29c0 / shogitter-3 c94a8d63
状態: P0・P1完了。共通契約は[contract.md](tuyouraou11/contract.md)、進捗は[status.md](tuyouraou11/status.md)。P1のscalar評価器・モデル実体検証は完了（[結果](tuyouraou11/p1-engine.md)）。差分評価・探索/Wasm統合・ブラウザ実測は未実施。

## agentsへの割当方法（実行指示）

agentには「この計画書を読み、担当=ENGINE、フェーズ=P1を実施してください」のように渡す。担当IDとフェーズIDの組を作業単位とする。下表の割当以外を自動的に開始しない。H1〜H4/S1〜S5は技術仕様の参照番号であり、実行順序は本節のP0〜P6に従う。

| 担当ID | 責任 | 変更可能な範囲 |
|---|---|---|
| LEAD | 共通仕様固定、依存関係管理、統合判定 | 本計画書、plans/tuyouraou11/配下の共通仕様・進捗・統合記録。P4では両repoの統合修正 |
| ENGINE | SFNN実装と正しさ、Wasm API | haitaka-variantsのエンジン・テスト・ビルド・ドキュメント |
| WEB | 配信、runtime、Worker、画面 | shogitter-3の実装・テスト・配信設定・ドキュメント |

各agentは対象repoのAGENTS.mdを読み、既存の未コミット変更を保持する。別担当のファイルを変更する必要がある場合は、変更要求を引継ぎ記録に書く。P4のLEADは統合のため両repoを変更できる。依存するAPIや仕様を一方だけで変更しない。

共通の引継ぎ先はhaitaka-variantsの `plans/tuyouraou11/` とする。各作業は `<phase>-<owner>.md`（例: p2-engine.md）に結果を残す。LEADは `status.md` へ各作業の未着手/進行中/完了/依存待ち、成果物のパス、repoのcommitまたは未コミット差分の所在を記録する。

全担当共通の終了手順:

1. 本フェーズの終了条件を全て確認する。必要な実モデルや実機がない場合、モックやskipで代替して完了扱いしない。
2. 引継ぎ記録へ変更ファイル、実行コマンドと結果、参照revision、成果物、未解決事項を残す。コマンドは作業ディレクトリ・feature・モデル指定を含め再実行可能にする。
3. 終了条件を満たした項目と未達項目を報告して終了する。前提未成立は「依存待ち」、一部検証未実施は「未完了」とし、次フェーズへ自動で進めない。
4. 公開デプロイは本割当に含めない。ローカル・テスト環境で実装と検証を完了する。

### 実行順序と並列化

| フェーズ | 割当 | 開始条件 | 主な成果物 |
|---|---|---|---|
| P0 | LEAD | なし | contract.md、status.md、fixture仕様、検証基準 |
| P1 | ENGINE | P0完了 | 固定モデル、provenance、参照fixture、scalar評価器 |
| P2 | ENGINE | P1完了 | 差分評価・探索・実Wasm API |
| P3 | WEB | P0完了。実モデル資産の確定はP1に依存 | 配信・runtime・Worker・UI。P2まではモックで進行可 |
| P4 | LEAD | P2/P3完了 | 両repoを接続した実Wasm E2E・実機/対局記録・導入判定 |
| P5（任意） | ENGINE → WEB → LEAD | P4完了、package対応が明示的に割当済み | package v2のwriter/readerと統合検証 |
| P6（任意） | ENGINE → WEB → LEAD | P4完了、mini/SIMDが明示的に割当済み | 最適化・追加モデルと比較記録 |

初回導入の完了はP0〜P4。P5/P6は初回の必須条件ではない。P3はP2と並行できるが、P1のprovenanceを未確定値で埋めて完了にはしない。

### P0 / LEAD: 共通契約と検証基準を固定

作業:

- 両repoの現行コードと本計画を再確認し、`plans/tuyouraou11/contract.md` を作成する。
- H3のload_sfnnの引数・戻り値・エラー、S2のdescriptor（bytesも含む）、モデルID/配置規則、準備状態メッセージ、再試行/dispose、思考時間の起点を具体的な型と順序で固定する。
- 初回の受理archはmicroの完全識別子だけとする。miniを含む本書の記述はP6へ繰り延べる。未知archは拒否する。
- 参照fixtureのJSON形式を固定する。SFEN、参照commit、モデルhash、progress和/bucket、最終評価値を含める。
- 評価値とprogress整数値は完全一致を既定基準とする。差が出た際に許容差を事後で緩めず、理由を調べて仕様変更として記録する。
- P4の対局条件を固定する。最低200局、同じ開始局面の先後交換、同一movetime、seed固定、micro対従来評価。勝率と95%区間を記録し、50%を上端が下回る場合は退行として不合格、それ以外でも棋力向上が証明されたとは扱わない。
- P4の対象端末・browser、許容準備時間・メモリ・探索時間超過の数値基準を、既存実装の測定と配信先制約を根拠に固定する。根拠未確定ならその基準を未決としP4の判定前に解決する。

成果物: contract.md、status.md、p0-lead.md。

終了条件: ENGINE/WEBが型・モデル受渡し・準備完了の意味を推測せず実装できる。比較方法と未決項目の解決担当が記録されている。公開master参照はP1でcommitへ固定することが明記されている。

### P1 / ENGINE: microの参照データとscalar実装

参照仕様: H1、H2の読込/全再計算、検証1/3。

作業:

- microアーカイブを取得・展開し、実ファイルのbytes/hash/ライセンスと参照実装commitを固定する。miniの取得は不要。
- 参照エンジンで正解fixtureを生成する。先後、序中終盤、持駒、成駒、玉位置を網羅する。
- progress loader、特徴量、SFNN scalar全再計算を実装し、参照fixtureに照合する。
- 欠損/切断/余剰/非有限値/誤arch等の拒否テストを実装する。
- WEBが利用できるモデル資産とprovenanceの所在、取得・hash検証コマンドを記録する。大きなbinのGit格納はrepo方針に従い、所在だけで再取得不能にならないようにする。

成果物: scalar実装、provenance、正解fixtureと生成手順、p1-engine.md。

終了条件: 実microの全fixtureでprogress和/bucketと評価値が参照値に一致。異常入力テスト合格。ライセンス/参照commit/両binのhashが確定し、WEBが同じ資産を取得できる。APIや探索統合が未実装でも本フェーズは終了できる。

### P2 / ENGINE: 差分評価・探索・Wasm API

参照仕様: H2の差分更新、H3、H4のREADME/テスト。H4のpackage新オプションはP5へ繰り延べる。

作業:

- 差分accumulatorとprogress更新を実装し、全再計算との一致を確認する。
- 通常将棋の探索へ統合し、グローバル/UsiEngine双方に契約どおりのload_sfnnを公開する。
- モデルの一括交換、失敗時の旧モデル保持、交換時TT消去、変則ビルドでの拒否を検証する。
- 非SIMDのweb target Wasmを生成し、実モデルでnative/Wasmの評価一致と合法手生成を確認する。評価照合には必要に応じテスト専用経路を用意する。
- 既存HalfKAv2と代表変則featureの関連テストを実行し、WEBへ成果物と再生成手順を渡す。

成果物: エンジン差分、Wasm/glue/型定義、再生成コマンド、p2-engine.md。

終了条件: 必須の全再計算/差分/native/Wasm比較と既存回帰が合格。契約APIが実Wasmに存在し、実microで探索できる。正解検証がskipされていない。WEBが指定checkoutから同じ成果物を作れる。

### P3 / WEB: 将棋ったーでの配信と準備処理

参照仕様: S1〜S4。S4のSIMD選択はP6へ繰り延べる。

作業:

- P0のcontractに従って2ファイルdescriptorとruntimeを実装する。P2未完了中はモックでAPI接続を検証する。
- P1の実資産を自サイト配信へ配置し、長さ/hashを固定する。同期元指定、wrapperテンプレートと型定義を更新する。
- Worker/画面の準備状態、準備待ちと探索時間の分離、再試行、キャンセル、世代管理を実装する。
- HTTP失敗、片方失敗、hash不一致、export不足、同時初期化、再試行、終了後の応答破棄をテストする。
- トップページ/perft/変則ルールでモデルを取得しないこと、再同期でwrapper変更が消えないことを確認する。

成果物: shogitter-3の変更、配信モデル/provenance、関連Vitest・型検査結果、p3-web.md。

終了条件: 実モデル資産と配信設定が確定し、runtime/Worker/UIの関連テストとpnpm typecheck:appが合格。P2待ちでもモックによる本フェーズ完了は可能だが、実Wasmの統合検証はP4未完了と明記する。

### P4 / LEAD: 実Wasm統合と初回導入判定

参照仕様: 検証1〜8の初回対象（micro/scalar/バンドル方式）。

作業:

- P2/P3の変更を互換な状態に揃え、commit/モデルhashを記録する。モックを用いず同期スクリプトから実Wasmを生成して将棋ったーへ接続する。
- ブラウザE2Eで準備完了→実SFNN探索→合法手適用を確認する。SFNNが使用された証拠をモデルID/arch等で残し、合法手が返るだけで合格にしない。
- Network/Workerの記録で自サイトからの2ファイル取得とブラウザ探索を確認する。通信失敗/再試行、stop後の再作成、ルール変更、perftも確認する。
- Chrome/Firefox、Safari/iOS、Androidの実機でP0の基準を測定し、cold/warm準備時間・メモリ・NPS・到達深さ・時間超過を記録する。利用できない実機は未検証と記録する。
- P0で固定した対局を実施し、棋力退行の判定と性能/安定性を評価する。必要な統合修正を両repoに加え、影響する検証を再実行する。

成果物: p4-lead.md（両repo revision、全コマンド、E2E結果、実機測定、対局記録、採否）。

終了条件: 実モデル/実WasmのE2E、該当回帰、対局、対象実機の基準が全て合格し、未検証の必須項目がない。未達項目があれば初回導入全体を完了としない。公開デプロイは行わない。

### P5 / ENGINE・WEB・LEAD: 任意のpackage v2

このフェーズは担当ごとに指定する。ENGINE完了→WEB完了→LEAD完了の順に進める。

- ENGINE: H4のSFNN CLI/xtaskオプションとS5のv2 writer、モデル検証、ライセンス同梱を実装。終了条件は正常archive生成と不足/誤arch等の拒否テスト合格。成果物はp5-engine.md。
- WEB: S5のv2 reader、展開/保存/削除/probe/Worker転送を実装。終了条件は関連テスト・型検査合格とv1互換。成果物はp5-web.md。
- LEAD: ENGINE生成archiveをWEBで取り込み、実モデルのprobe/探索/削除までE2E確認。終了条件はv2の往復とv1回帰合格。成果物はp5-lead.md。

### P6 / ENGINE・WEB・LEAD: 任意のmini/SIMD

割当時に対象をmini、SIMD、または両方と明記する。指定がなければ作業開始せず、依存待ちとして対象の指定を求める。

- ENGINE: 対象プロファイル/最適化を追加し、miniならP1相当のprovenance・参照一致、SIMDならscalarとの一致を検証する。終了条件は全比較と既存micro回帰合格。成果物はp6-engine.md。
- WEB: 対象成果物の配信/能力判定を追加。終了条件はSIMD非対応時のscalar動作とモデル別の読込テスト合格（該当するもの）。成果物はp6-web.md。
- LEAD: P4と同じ時間条件・端末で比較し、採用理由を記録する。終了条件は実機/E2E/棋力検証合格と採用プロファイルの確定。成果物はp6-lead.md。

## 目的と採用方針

haitaka-variantsの探索をWeb Worker内で実行し、訪問者のブラウザでつようら王11の評価関数を利用する。初回は通常将棋（ruleId=0）とmicroを対象とする。miniは同じ評価器の別プロファイルとして追加可能にするが、標準採用は同じ思考時間での対局と実機測定で決める。通常将棋向け学習モデルを変則将棋の既定モデルにはしない。

既存のHalfKAv2評価器を置換せず、別のSFNN評価器を追加する。binのリネーム、ヘッダ書換え、L1幅変更だけでは互換にならない。USI全面改修や探索アルゴリズム変更はこの作業に含めない。

## 配布元と確認できた仕様

配布記事: https://yaneurao.fanbox.cc/posts/12761272
本文はFANBOX公開API post.infoから読み取り確認した。

- micro: SFNN_ka2_2048_8_64_progress8。nn.bin約3.5MB、progress.binを合わせて約4.5MB。
- mini: SFNN_ka2_4096_8_64_progress8。nn.bin約7.5MB、合計約8.5MB。
- 記事はnn.binおよびprogress.binをMIT Licenseで配布すると明記。FV_SCALE推奨値は24。
- banner.png等へ同じ許諾を推定適用しない。初期導入に画像は不要。
- 記事の棋力比較をhaitakaへそのまま適用しない。探索が異なるため独自評価する。

micro配布物:
https://storage.yaneu.com/yaneuraou/TuyouraOu11micro-SFNN_ka2_2048_8_64_progress8-epoch0010-20261001.7z
mini配布物:
https://storage.yaneu.com/yaneuraou/TuyouraOu11mini-SFNN_ka2_4096_8_64_progress8-epoch0010-20261003.7z

公式仕様:
https://github.com/yaneurao/YaneuraOu/blob/master/docs/progress-format.md
https://github.com/yaneurao/YaneuraOu/blob/master/source/eval/nnue/architectures/sfnn_network.h
https://github.com/yaneurao/YaneuraOu/blob/master/source/eval/nnue/features/half_ka2.h

公式progress仕様では、81玉位置×1548 BonaPiece、ヘッダなしlittle-endian f64を125,388個（1,003,104 bytes）格納する。読込時にQ16へ丸め、局面和は64bit整数。progress8境界は -127527, -71999, -33477, 0, 33477, 71999, 127527 で、境界との一致は上側bucket。nn.bin内にProgressセクションはない。SFNN構造hashは0x3C203B32だが、hashだけで幅やstack配置まで検証したことにはならない。

公式masterは可変参照である。実装開始時に対応するcommitを固定し、配布binと一致することを検証する。P1でmicroを展開し、正確なbytes/SHA-256/ヘッダをprovenance.jsonへ固定した。配布物にライセンス文書はなく、記事の明示許諾とtag.jsonの作者表記をMODEL-LICENSE.txtへ保存した。miniは未取得。

## 現状のコードと差分

haitaka_wasm/src/nnue.rs はversion 0x7AF32F20、HalfKAv2系特徴量、L1=512または256、PSQT=8、8 stacks、hidden 16/32を前提としている。SFNN専用の特徴量、活性化、進行度、層配置が必要。

haitaka_wasm/src/lib.rs のload_nnue(bytes)はグローバル評価器を切り替え、TTを消去する。UsiSession::load_nnueはセッション固有モデルとTTを更新する。両経路の所有権を維持する。

将棋ったーには既に以下の経路がある。

- app/bot/bot.worker.ts → haitakaPlayer.ts → vendors/haitaka/index.ts → standard/index.js。
- app/engines/wasmBindgenRuntime.ts はnnueUrlを遅延fetchしてload_nnueへ渡す。
- standard/index.jsではNNUE設定がライセンス未確認としてコメントアウトされている。
- scripts/sync-haitaka-wasm.mjs がwrapperを再生成するため、生成物だけの修正は消える。
- 同スクリプトのソース既定パスは ../engine/haitaka。今回の ../haitaka-variants と一致しない。
- Engine Package v1はprofile.nnueの単一path/formatを扱う。progress.binを伴う構成の表現はない。
- bot.workerのreadyはplayerのimport完了時点。モデルの準備完了を意味しない。

## haitaka-variants実装計画

### H1: 配布物と参照実装を固定

1. micro/miniの配布アーカイブを取得・展開し、nn.bin、progress.bin、tag.json、ライセンス情報を確認。
2. provenance JSONにモデルID、arch、元URL、配布日、各ファイルのbytes/SHA-256、FV_SCALE=24、参照実装commitを記録する。
3. 配布物と同じarchの参照エンジンを用意し、評価値・progress bucketの正解データを採取する。公開masterとの不一致があれば対応revisionを解決するまで実装を確定しない。
4. モデルのMIT表記と実装コードのライセンスを別に扱う。既存リポジトリのライセンスを確認し、上流コードの移植がある場合はそのライセンスと出典を保持する。

### H2: SFNN評価器

追加候補: haitaka_wasm/src/sfnn/{mod.rs,format.rs,features.rs,progress.rs,accumulator.rs,kernels.rs}。

- archを明示した2048/4096プロファイル。loaderでヘッダ、section、次元、配列長、余剰bytesを検証。
- half_ka2/BonaPieceの盤座標、先後反転、駒種、成駒、持駒、玉の扱いを参照実装へ一致させる。既存HalfKAv2のindexを流用できると仮定しない。
- transformer、活性化（平方活性化等を含む）、後段層、stack順序、量子化、飽和、丸め、評価値の符号とFV_SCALEを参照実装に合わせる。
- progress.binを別データとして読み、非有限値、欠損、余剰データを拒否。f64→Q16の丸め規則も一致させる。
- まずscalarの全再計算で一致させ、その後差分accumulatorを実装。指す・取る・打つ・成る・玉移動・持駒変化・先後交代で再計算との一致を確認。
- 進行度差分更新とbucket切替を実装する。玉移動でどの状態が無効化されるかを参照実装と照合。
- 最初は単一スレッドscalar対応。SIMD128はscalarとの一致を確認してから追加し、非SIMDブラウザ用成果物を残す。

### H3: 探索統合と公開API

既存HalfKAv2とSFNNを評価器enum等で保持し、探索の評価・局面状態更新経路へ接続する。大きなモデルはArcで共有し、ノードごとに重みを複製しない。

追加API案（実装前に双方で固定）:

    load_sfnn(nn_bytes: Uint8Array, progress_bytes: Uint8Array, arch: string): string

対応archは記事の2つの完全な識別子だけを許可。戻り値はモデル記述、異常は例外とする。既存load_nnue(bytes)は既存形式のまま維持する。同等のUsiEngineメソッドとnative内部APIを追加する。

2ファイルの検証完了後にモデルを一括交換しTTを消去する。途中失敗で片方だけ更新しない。初期導入では通常将棋ビルドのみ受理し、変則将棋では明示エラーにする。モデルなし時の従来評価は維持するが、要求したSFNNのロード失敗を無通知で成功扱いしない。

### H4: 配布・検証

- haitaka_cli packageとxtaskは既存--nnueを維持。SFNN用に--sfnn-model、--progress、--sfnn-arch等を追加し、必須の組合せと実体をpackage生成前に検証する。
- READMEとdocs/shogitter-package.mdへ2ファイル方式、通常将棋限定、出典、ビルド方法を追記。
- 自動テストは小さな合成fixtureでloader拒否・境界値を検証。実モデル検証はモデルを明示指定した専用ジョブとし、未配置で黙ってskipして合格させない。

## 将棋ったー側変更仕様（実装担当者向け）

### S1: バンドルモデルと再現可能な同期

対象: scripts/sync-haitaka-wasm.mjs、app/bot/vendors/haitaka/standard/index.js、同型定義。

- HAITAKA_ENGINE_DIR等の引数/環境変数でエンジンルートを指定可能にし、このcheckoutからビルドできるようにする。使用commitを記録。
- 通常将棋wrapperのテンプレートへmodel descriptorを追加。nn.bin/progress.binを別のURL assetとして参照し、両方のSHA-256とarchを指定する。
- モデルは自サイト配信。閲覧者がFANBOXや上流storageへ直接アクセスする構成にしない。
- 配置案: public/engines/haitaka/tuyouraou11micro/<model-id>/{nn.bin,progress.bin,provenance.json,LICENSE.txt}。URLはモデル版で固定し、既存URLの内容を上書きしない。
- CIは事前取得したモデル資産のhashを検証。同期スクリプトで別ライセンスの旧shogi.nnueを暗黙に選ばない。既存のモデルコピー処理も確認して更新する。
- 変則将棋wrapperにはこのモデルを指定しない。micro/mini切替UIは初回導入では不要。

### S2: runtime初期化契約

対象: app/engines/wasmBindgenRuntime.ts とそのテスト。

- WasmBindgenModuleへload_sfnnを追加。BundledRuntimeOptionsに形式を識別できるmodel descriptorを追加し、既存nnueUrl利用との互換を保つ。
- descriptor例: { kind: 'sfnn-progress', id, arch, nnUrl, progressUrl, nnSha256, progressSha256 }。
- 順序: Wasm初期化 → 2ファイル取得 → HTTP/長さ/hash検証 → load_sfnn → 準備完了。
- 指定モデルに必要なexportがなければエラー。現行の「load_nnueがなければfalse」の挙動を必須モデルでは使用しない。
- 同一Workerの同時要求は1つの準備Promiseに集約。失敗Promiseは明示再試行可能にする。fetchにAbortControllerとタイムアウトを付け、dispose後の応答を破棄。
- 2ファイルは独立に取得可能だが、両方の検証完了前にエンジンへ反映しない。
- ダウンロード後の不要なArrayBufferを保持し続けない。複数Workerの重み共有は初回範囲外。
- perftはモデル不要であるため、従来どおりモデル取得を強制しない。

### S3: Workerと画面の準備状態

対象: app/bot/{bot.worker.ts,WebWorkerBot.ts,WorkerBot.ts,haitakaPlayer.ts} と呼出元。

- 現行initにはruleIdがない。ルールが分かるready(shogi)からprepare(ruleId)要求を送るか、initへruleIdを渡す経路を追加する。前者を推奨。
- 状態は loading-wasm / loading-model / ready / error。readyは指定ルールのモデルまで準備できた時点で返す。
- 最初の探索要求から思考時間を数える前に準備をawaitする。準備失敗を時間切れ・投了として処理しない。
- 画面には「思考エンジンを準備中」「評価モデルを読み込み中」と再試行導線を表示。失敗は説明して再試行できるようにし、手動の従来評価切替は必要なら別操作として提供する。
- 同期Wasm探索中に通常のWorkerメッセージだけで即時stopできるとは扱わない。現行のWorker終了・再作成を維持し、再作成時にモデルも再ロードされる前提でキャッシュを利用する。
- dispose/再初期化/ルール変更で古いready、error、bestmoveを採用しない。既存requestIdとWorker世代の管理を踏襲する。

### S4: HTTP配信

- バイナリはapplication/octet-stream、モデル版固定URLに長期Cache-Control（immutable）を設定。Wasmはapplication/wasm。
- 初回取得は通常将棋AI選択時まで遅延。全訪問者のトップページ表示でモデルを取得しない。
- 初回はブラウザHTTPキャッシュを利用し、IndexedDB/Cache Storageの独自管理は計測後に判断。
- SIMDは能力判定して対応成果物を選択する。単一スレッド版のためにSharedArrayBuffer/COOP/COEPを必須にしない。
- 配信容量とWasmメモリは別々に測定する。JS受信buffer、Wasmへのコピー、重み展開、progress Q16、TT、accumulatorの合計ピークを記録する。

### S5: Engine Package v1を使う場合

訪問者用のバンドル導入はS1〜S4で完結し、ユーザーによるアップロードを必須にしない。package配布も同時に対応する場合のみ以下を実施する。

- 既存profile.nnueの単一ファイル表現をSFNNに流用しない。schemaVersion=2を追加し、profile.modelを判別unionにする。
- 案: { kind: 'sfnn-progress', arch, nn: { path, sha256 }, progress: { path, sha256 }, fvScale: 24 }。
- v1 readerは維持し、v2 readerとpackage writerを同時提供。古いreaderがv2を明示拒否することを確認する。
- 対象: app/engines/{manifestTypes.ts,untarball.ts,engineStore.ts,types.ts,profileUtils.ts,engineWorkerScript.ts,user側初期化処理}、app/bot/userEngine.worker.ts、docs/shogitter-engine-protocol.md。
- 2ファイルの展開・保存・削除・Worker転送・probe/実行の両経路を対応し、不足ファイルや誤archを検出する。manifest上の相対path制約は既存方針を維持。
- archiveへモデルのライセンス・provenanceも同梱。SFNNをv1として出力しない。

## 検証と完了条件

1. 参照エンジンとの一致: 序盤・中盤・終盤、先後、持駒、成駒、玉移動の固定SFEN集合でprogress和/bucketと整数評価値を比較。丸め差を許容する場合は理由と上限を固定する。
2. 差分更新: 合法手列について毎手の全再計算と差分状態が一致。bucket境界と捕獲/打ち/成りを必ず含める。
3. loader: 欠損progress、NaN/Inf、切断、余剰、誤arch、幅違い、誤hashを拒否し、失敗時に旧モデルを保持する。
4. Wasm: 同じ入力でnative/scalar/SIMDの一致を確認。micro/miniおよび既存HalfKAv2・代表変則ビルドの回帰を確認。
5. 将棋ったー: 遅延取得、同時要求の集約、2ファイル片方失敗、再試行、export不足、Worker終了/再作成、ルール切替を単体テスト。
6. ブラウザE2E: 通常将棋でモデル読込済みが確認でき合法手を返す。サーバーへ探索要求を送らず、画面操作が止まらない。perftと変則将棋がこのモデルを取得しない。
7. 実機: Chrome/FirefoxとSafari/iOS、Androidでcold/warm準備時間、ピークメモリ、探索時間超過、NPS、到達深さを測定。micro対従来評価を同一movetime・先後交換で対局し、棋力退行とクラッシュがないことを確認する。
8. CI: cargo test -p haitaka_wasm と該当feature、CLI/packageテスト、将棋ったーの関連Vitest、pnpm typecheck:app、関連Playwrightを実行する。実モデル検証の結果を別に記録する。

## 初回完了の判定

担当と実行順序は冒頭のP0〜P6を正とする。H/S節は実装仕様であり、任意フェーズの機能を初回へ混ぜない。初回はP0〜P4が完了した時点で終了する。

最優先の判断材料は実binと参照実装の一致である。ロード成功や合法手生成だけではSFNN実装の正しさを判定しない。
