# P0共通契約 v1

状態: P0確定。2026-10-30。変更時はLEADが変更理由とENGINE/WEBへの影響を記録する。
親計画: ../tuyouraou11-browser-nnue.md。本書の初回スコープと型をH/S節の提案例より優先する。

## スコープと作業ブランチ

- 両repoとも `feat/tuyouraou11-p0` から開始済み。開始点はhaitaka-variants `d0c29c0c7835f9dfed36323b59aceac06265f603`、shogitter-3 `c94a8d630ba879b59a098719e420e561128e7d49`。
- 初回はruleId=0、micro、scalar、単一Worker、バンドル配信。mini/SIMD/package v2は対象外。
- P1/P2 ENGINEはhaitaka側、P3 WEBはshogitter側を実装。後続agentがさらにブランチを分ける場合はこの作業ブランチを起点にし、引継ぎにbranch/baseを残す。
- master/rebootの作業へ戻して実装しない。既存未追跡ファイルをまとめて追加しない。

## エンジン公開API

```ts
type SfnnArch = 'SFNN_ka2_2048_8_64_progress8';
// wasm-bindgenのトップレベルexportおよびUsiEngineインスタンスの両方
function load_sfnn(
  nn_bytes: Uint8Array,
  progress_bytes: Uint8Array,
  arch: string,
): string;
```

同期API。戻り値は受理したarch文字列そのもの。モデルID/hashはWEBの検証済みdescriptorから得る。説明文の解析で成功判定しない。native内部APIは同じ3引数を参照で受けResult<String, String>を返す。

失敗はwasm-bindgen Resultの例外とし、文字列を `<CODE>: <detail>` の形にする。WEBはString(error)から既知prefixを抽出し、detailを制御フローに使わない。

| CODE | 条件 |
|---|---|
| SFNN_UNSUPPORTED_ARCH | 完全一致しないarch（miniを含む） |
| SFNN_UNSUPPORTED_RULESET | 通常将棋以外のfeatureビルド |
| SFNN_INVALID_NN | nn.binのヘッダ/section/次元/長さ/余剰等の不正 |
| SFNN_INVALID_PROGRESS | progress.binの長さ/非有限値等の不正 |

検証順序はarch、build、nn、progress。両方を検証後に一括交換し、その評価器に属するTTを消去する。失敗時は旧評価器とTTを保持する。トップレベルとUsiEngineは従来どおり別所有で、一方のロードで他方を変更しない。既存load_nnueのABIを維持する。要求したSFNN失敗をhandcraftedへ自動フォールバックしない。

重みのSHA-256照合はWEB/資産検証側の責任。エンジンも内容構造を必ず検証する。FV_SCALEは24固定で公開引数を増やさない。終局値やmate scoreと静的評価値を区別する。

## モデルdescriptorとprovenance

```ts
type ModelFile = {
  url: string;       // 同一originの版固定絶対パス（/engines/...）
  bytes: number;     // 展開後binの正確な長さ。正のsafe integer
  sha256: string;    // lowercase hex 64文字
};
type SfnnModelDescriptor = {
  kind: 'sfnn-progress';
  id: string;
  arch: 'SFNN_ka2_2048_8_64_progress8';
  fvScale: 24;
  nn: ModelFile;
  progress: ModelFile;
};
```

既存nnueUrlとmodelは同時指定不可。未指定は既存挙動。初回はlegacy nnueUrlへの暗黙フォールバックなし。

P1が `provenance.json` を確定する。必須項目はschemaVersion=1、id、name、arch、fvScale、sourcePageUrl、archiveUrl、archiveSha256、publishedDate（記事日）、artifactDate（配布名の日付）、license=MIT、licenseEvidence、reference.repository/commit、files.nn/files.progress の path/bytes/sha256。reference.commitは40桁の固定commit。モデルhashと取り違えない。

モデルIDは `tuyouraou11micro-20261001-epoch0010-<nnSha256の先頭12桁>-<progressSha256の先頭12桁>`。URLルートは `/engines/haitaka/tuyouraou11micro/<id>/`。配置はshogitterのpublic配下に対応させる。両binとprovenance.json、LICENSE.txtを含める。同じIDの内容を更新しない。著作権表記は配布元情報を転記し、推測で作らない。配布元の表記が不足する場合はP1で記録して解決する。

P1以前のモックでは明確にtest専用descriptorを用い、本番設定へ仮hash/概算bytesを置かない。実モデルの正確なnn bytes/hashと上流commitはP1の成果物でありP0で捏造しない。progress bytesは公式仕様上1,003,104だが実配布物とも照合する。

## runtimeの準備とライフサイクル

WEB実装の外部契約:

```ts
type Prepared = {
  ruleId: number;
  evaluator: 'sfnn' | 'handcrafted';
  modelId: string | null;
  arch: SfnnArch | null;
};
type PrepareState = 'loading-wasm' | 'loading-model' | 'ready' | 'error';
// Haitaka vendor経路に追加。共有のPlayer interface拡張は必須ではない。
prepareHaitaka(ruleId: number): Promise<Prepared>;
```

1. ruleId=0ならWasm初期化→export存在確認→2ファイルfetch→HTTP/bytes/SHA-256検証→load_sfnn→返り値arch確認→ready。
2. 各fetchは開始から30,000msでabort。HTTP成功でもHTML/空データ等は長さ/hashで拒否。2ファイル取得は並列可、一方失敗なら残りもabort。
3. 準備の全体上限は60,000ms。上限超過はWorkerを終了して準備Promiseをrejectする。同期Wasmを同じWorkerのtimerで中断できるとは扱わない。
4. 同一Worker/同一ruleIdの同時prepareは同一処理へ集約。成功後は再ロードしない。失敗後はerrorを保持し、明示再試行で新Workerを作る。無限自動retryを行わない。
5. runtimeは不要bufferを成功/失敗時に解放可能にする。disposeは世代を無効化しfetchをabort。外側のWorker終了も必ずpending Promiseをsettleする。
6. perftはWasmのみを初期化し、モデルを取得しない。別ルールに備えてrule0の準備を先回りしない。

WEBの準備エラーcodeは WASM_INIT_FAILED / MODEL_EXPORT_MISSING / MODEL_HTTP_ERROR / MODEL_SIZE_MISMATCH / MODEL_HASH_MISMATCH / MODEL_LOAD_FAILED / PREPARE_TIMEOUT / ABORTED とする。UIにはmessageとretryableを渡し、投了へ変換しない。HTTP/hash等は再試行操作可、dispose/ルール変更のABORTEDは通常のエラー通知を出さない。

## ruleIdとWorker通信

standardのruleIdsには0以外もある（packages/ai/src/haitakaEngineConfig.json）。standard wrapperのsearchへ無条件ensureNnueLoadedを入れない。モデル指定が有効なのはruleId=0だけ。トップレベル評価器はmodule内で共有されるため、ルール変更時には同じstandardモジュールへの切替でもWorkerを終了・再作成する。モデルをunloadする新APIは初回不要。

既存initの `["ready"]` はworkerのplayer import完了だけを表し、モデル準備とは区別する。既存random/possibilityist/UserEngineBotの通信は維持する。Haitakaでは以下を追加:

```ts
// main → worker（generationとprepareIdはmainが単調増加させる）
['prepare', { generation: number, prepareId: number, ruleId: number }]
// worker → main
['prepare-state', {
  generation: number, prepareId: number, ruleId: number,
  state: PrepareState,
  prepared?: Prepared,  // state=readyで必須
  error?: { code: string, message: string, retryable: boolean }, // errorで必須
}]
```

mainはWorkerオブジェクトの同一性、generation、prepareId、ruleIdが全て現在の要求と一致する応答だけを受理する。新Workerごとのinit完了後にprepareを送る。go受信時にも準備済みruleIdとの一致を検証し、不一致は要求に紐づくerrorで返す。goのrequestId契約は維持する。

WorkerBot.ready(shogi)はHaitakaだけprepare成功まで待つ。getLoadState()はその間loadingのままとし、既存loaded表示を早く返さない。詳細準備状態の購読をWebWorkerBot→SelfBotShogiへ接続し、「思考エンジンを準備中」「評価モデルを読み込み中」と再試行を表示する。購読の具体的実装はWEBが選べるが、上記メッセージ/準備条件は固定。

探索時間は準備完了後、workerが同期探索を呼ぶ直前から測る。mainのthinking表示も準備完了後。準備待ちはmovetimeを消費しない。WorkerBot.goのbusy restart経路も新Workerのprepareまでawaitしてからgoを送る。stopは準備中にも機能させ、pending prepareをABORTEDでsettleしてWorkerを終了する。再起動後に必要なprepareを再実行する。dispose後にreadyへ復帰しない。

## 正解fixture契約

`reference-fixture.schema.json` をJSON Schemaとして用いる。値は参照実装で採取し、手書き期待値は禁止。P1は生成script、参照commit、ビルドflags、計測コマンドを添付する。JSONの64bit整数精度を保つためQ16和は10進文字列。bucketは0始まり。

progress.sumsQ16は参照実装が加算に用いる各視点の整数和を名前付きで全て記録し、selectedSumQ16はbucket選択直前の整数値を記録する。各名前と算出方法（先後視点の合成等）はgeneratorのREADMEに対応ソース行付きで記す。P1で実装と異なる想定が判明したらfixtureを偽装せず契約差分としてLEADへ引き継ぐ。

静的評価は手番側視点、FV_SCALE適用後、探索補正/TT/探索由来のmate値を含めない参照の整数値。P1は最低64局面、先後/序中終盤/持駒/成駒/玉位置をタグ付けして含める。P2はseed=1の合法手列を計1,000局面以上比較し、捕獲/打ち/成り/玉移動の出現数を記録する。合成progress境界の直前/一致/直後も全7境界で確認。整数評価値、bucket、Q16和は完全一致とする。

## P4対局・性能判定

対局は同一commit/同一探索設定、A=micro、B=handcrafted。各100局（50開始局面×先後2）の200msと1,000ms、計200局を実施。開始局面はseed=1、通常将棋初期局面から合法手を8手生成した50個を先にファイルへ固定しhashを記録。最大256手で引分、千日手等は現行合法ルールに従う。開幕生成用と対局用のseed/設定を記録し、途中で局面や条件を変えない。

既存CLIにはSFNN用の評価指定がない。P2 ENGINEはpackage v2を実装せずに、テストharnessまたはnative評価選択経路を用意してP4で同じ探索へ両評価器を指定できるようにする。既存の--nnueだけでSFNNが動くと仮定しない。P4は同じ局面の実Wasmでの探索スモークも別途記録する。

スコア率=(勝ち+引分/2)/局数。95%区間は開始局面ペアを単位に層別（時間条件別）bootstrap 10,000回、seed=1、percentile 2.5/97.5%。総合区間の上端<0.5なら退行で不合格。上端>=0.5でも向上/非劣性が証明されたとは記述しない。各時間条件の結果も残す。クラッシュ/不正着手は通常の敗戦に混ぜて隠さず不合格とし、修正後同じ全200局を再測定する。

対象matrixはdesktop Chrome/Firefox、iOS Safari実機、Android Chrome実機。端末型番/OS/browser版/省電力状態を結果へ固定。P0では端末を利用できておらず測定値は未確定。

- cold/warm準備: 各10回。coldはHTTP cacheなし、warmはHTTP cacheあり・新Worker。Wasm/モデル準備時間を分離して中央値と最大を記録。
- 探索: 固定20 SFEN×各3回、200ms/1,000ms/既定3,000ms。engine elapsedとmain側往復時間、NPS/深さ、合法手、クラッシュを記録。
- メモリ: Wasm memory.buffer.byteLengthのピーク、JS binary buffer確保量、可能ならprocess/JS heapを分離。Safari等で取得不能な値を0として扱わない。
- UI: 準備/探索中の入力操作と表示更新をE2E確認。長時間UI停止とWorkerクラッシュは不合格。

数値性能閾値は未決（P0で実測根拠なし）。LEADがP4開始時に同じ端末/通信条件で従来版を計測し、配信先制約も調べて `performance-budget.md` に端末ごとの準備時間/メモリ/時間超過上限と根拠を固定する。WEBはS4の配信制約を提供する。SFNNの採否結果を見る前に予算を凍結し、都合のよい事後変更をしない。実機がない/測定不能項目の代替根拠がない場合はP4未完了。この未決事項はP1/P2/P3の開始を妨げない。

## P0の検証範囲

本書は実装契約であって検証結果ではない。実bin/正解fixture/測定結果は未生成。公式masterの参照を正解として固定したとは扱わず、P1で対応commitとbinの一致を証明する。P4までの未決項目はstatus.mdへ明示する。

## P1の確認結果（契約の明確化）

P1で実microと固定参照commitを検証済み。provenance.json、reference-fixture.json、p1-engine.mdを参照。progress.sumsQ16のキーはblack/white、その合計がselectedSumQ16で確定。有限の範囲外progress重みは参照どおりQ16変換後int32へ飽和し、NaN/Infinityは拒否する（旧表の「変換範囲等の不正」はこの挙動へ明確化）。API/Worker通信の変更なし。236局面の全整数値が完全一致。エンジンの評価クリップは参照のMAX_PLY=246で±31753。
