# Haitaka Variants のネイティブ USI 対応と最大探索深さの設定

## 初回リリースの範囲

Windows x64 と Android arm64 で、標準入出力を使う USI エンジンを配布する。本将棋、安南将棋、安北将棋、５五将棋を最初の配布対象とする。Windows では本将棋の対局を既存 GUI で確認し、Android では端末上での起動と USI の入出力を確認する。変則ルールの動作は、対応 GUI の有無にかかわらず、ルールを理解するテスト用クライアントで検証する。

将棋ったーの内蔵 Haitaka には「最大探索深さ」を追加する。「最大思考時間」と同時に指定でき、先に達した上限で探索を打ち切る。Web 側の作業はネイティブ版の配布と並行して進められる。

## 既存の実装

- `haitaka_cli usi` は本将棋と SFEN に対応するビルドで起動できる。`go depth N`、`go movetime N depth D`、`go nodes N` を処理する。現状は `go` の完了まで標準入力の読み取りが止まり、残り時間・秒読み・加算からの時間配分は未実装。CLI と WASM が探索と `UsiSession` を共有している。[CLI](../haitaka_cli/src/main.rs) · [USI セッション](../haitaka_wasm/src/lib.rs)
- ルールは Cargo feature で選ぶ。`extended-json` のルールは、この計画のベースではバージョン付き JSON API を使う。別ブランチ `feat/search-pvs-lmr` の `1122b52` には、中将棋の Lishogi 向け USI アダプターとテストがある。[ルールの選択](../haitaka/src/lib.rs) · [拡張ルールの CLI](../haitaka_cli/src/extended.rs)
- 将棋ったーの内蔵 Haitaka は、レジストリで最大深さを `6` に固定している。`haitakaPlayer.ts` 以降では SFEN/USI と拡張 JSON の両方に深さと時間を渡せる。画面から Worker までの設定経路を追加すれば、既存の WASM API を使って設定を適用できる。

## 1. ネイティブ USI を対局ソフトから使えるようにする

1. 引数なしで起動できる `haitaka-usi` バイナリを設ける。`haitaka_cli usi` と起動処理を共有し、既存の CLI 引数、自己対局アーカイブ、WASM の `UsiEngine.send()` 呼び出しもサポートする。標準出力は USI 応答に使い、診断ログは標準エラーに出す。`id name` にリリース版とルール名を含める。
2. ネイティブではコマンドの受信と探索を分ける。探索ワーカーに局面・棋譜履歴・上限を渡し、通常探索、静止探索、DFPN が停止要求を確認できるようにする。通常完了時と `stop` では、有効な探索要求に対して `bestmove` を一度返す。`stop` では最後に完了した深さの手を採用し、完了した反復がなければ合法手を返す。`quit`・EOF・対局終了・局面更新では探索を破棄し、古い局面の応答を後から出さない。
3. `position startpos` と `position sfen ... moves ...` の合法手検証、局面履歴、各ルールの終局判定を再利用する。`go btime ... wtime ...` は手番側の残り時間の一部を使い、秒読みまたは加算を加味する。その手で使える時間から通信と処理の余裕を引いて期限を決め、時間が実質ゼロなら直ちに合法手を選ぶ。`go infinite` では反復ごとに `info depth` などを出し、深さ上限に達した場合も `stop` を受けてから `bestmove` を返す。[将棋所の USI 仕様](https://shogidokoro2.stars.ne.jp/usi.html)
4. GUI 互換の `USI_Hash` と既存の `Hash` を同じ設定へ結び付ける。先読みは初回リリースでは無効とし、GUI からの `USI_Ponder` 設定を安全に受理する。`go mate` には `checkmate notimplemented` を返す。`gameover` で探索と対局状態を片付け、次の対局を開始できるようにする。[将棋所の USI 仕様](https://shogidokoro2.stars.ne.jp/usi.html)

## 2. Windows と Android にルール別の実行ファイルを配布する

| 対象 | ビルド・配布 | 動作確認 |
| --- | --- | --- |
| Windows x64 | `x86_64-pc-windows-msvc` で `.exe` を作り、ZIP に同梱 | パイプでの自動テストに加え、将棋所で本将棋エンジンを登録し、対局・検討・中断を確認する。空白や日本語を含むパスでも起動する。 |
| Android arm64 | `aarch64-linux-android` を NDK の Clang でリンクし、実行ファイルとして配布 | `adb` で arm64 端末に転送し、USI の対話、対局相当の着手列、中断と終了を確認する。動作を確認した API レベルと起動方法を同梱する。 |

ルールはビルド単位で選び、実行ファイル名にも含める。配布物にルール名、初期局面、局面と指し手の表記、終局の扱いを記載する。初回は追加の評価ファイルなしで起動できる手作り評価を既定とする。CI のビルド行列でルールごとに Cargo feature を指定し、配布対象すべてで起動と合法手応答を検証する。Linux はプロトコル回帰テストに使う。[Rust の Windows ターゲット](https://doc.rust-lang.org/nightly/rustc/platform-support/windows-msvc.html) · [Android NDK のビルド手順](https://developer.android.com/ndk/guides/other_build_systems)

Android 10 以降をターゲットとする一般アプリでは、書き込み可能なアプリ内ディレクトリから実行ファイルを起動する際に制約がある。対局アプリへの組み込みは、アプリ側が対応する起動方式を確認できた段階で、アプリ同梱や JNI 連携を検討する。初回の Android 配布は端末上で動く実行ファイルまでとする。[Android の実行制約](https://developer.android.com/about/versions/10/behavior-changes-10?hl=ja#execute-permission)

安南・安北・５五将棋では、本将棋と同じ形の USI コマンドを使い、各ビルドの着手・終局規則で応答する。`startpos` はそのルールの初期局面とする。後続では既存の SFEN/USI で表せるルールを順次追加する。中将棋は `1122b52` の Lishogi 向け SFEN と複数マスを通る指し手の変換を取り込み、共通の起動・停止のテストと組み合わせる。独自の駒、持駒、履歴などを標準表記で表せないルールは、既存 JSON API を提供し、対応クライアントと表記・終局の取り決めができたものから USI アダプターを追加する。

## 3. 探索深さを利用者が指定できるようにする

| 利用経路 | 設定と優先順位 |
| --- | --- |
| ネイティブ USI | `setoption name MaxDepth value N` を公開する。既定は現行の時間探索と同じ `64` とし、値は整数 `1〜100` を初期案とする。対局時計・`go movetime`・`go infinite` で明示深さを省略した場合に適用する。`go depth N` はその探索の明示指定として優先し、`go movetime N depth D` では指定された時間・深さの両方を使う。`go nodes` は既存の計測契約を使う。 |
| 将棋ったー | 内蔵 Haitaka の設定に「最大探索深さ」を表示し、既定を現行の `6` とする。入力範囲は `1〜100` を初期案とし、既存の思考時間と同じくブラウザのセッションに保存する。時間設定の表示は「最大思考時間」とする。 |

Web では `SelfBotShogi.tsx` の設定と `bot-states.ts` の保存値を、`Shogi.tsx` の `useBotUpdater` と `SelfBotShogiService` から `WebWorkerBot` の探索リクエストへ渡す。`bot.worker.ts` が未指定時に `playerRegistry.ts` の既定値を補い、`haitakaPlayer.ts` を通して、既存の WASM の時間・深さ指定へ渡す。設定変更時は進行中の探索を取り消し、同じ局面で新しい上限を適用する。上限を下げるときは、深い設定の置換表の評価を新しい探索に流用しないよう、探索セッションを作り直すか置換表をクリアする。アップロード型エンジンへの表示は、時間と深さの併用を能力として確認できるものまで後続で広げる。

深さは反復深化の上限を指す。静止探索や詰み探索で、その先の手順を読む場合がある。同じ値でも端末・局面・ルールによって到達する深さや棋力は変わるため、画面には設定値を深さとして表示する。[初心者向け棋力調整の計画](beginner-bot-strength-control.md)

## 完了条件

- **プロトコル** `usi`、`isready`、局面と履歴の入力、持ち時間・秒読み・加算、`go depth`、`go movetime`、`go nodes`、`go infinite`、`stop`、`gameover`、`quit` をパイプ越しにテストする。両手番の時間配分と残り時間ゼロをテストする。探索開始直後と完了直前の中断、連続対局、入力終了でハングや重複応答が起きないことを確認する。
- **変則ルールと配布** 配布する各 OS・ルールの組み合わせで、そのルールでのみ意味を持つ着手または初期局面を使い、応答の合法性を検証する。中将棋を追加する際は、獅子の二段移動、成り、棋譜履歴の再現を Lishogi 表記のテストに含める。
- **設定** USI では `MaxDepth` の範囲と、明示深さや時間との優先順位を確認する。将棋ったーでは深さ `1`、既定の `6`、短い時間上限を組み合わせ、実際にエンジンへ届く値と完了深さを検証する。SFEN/USI のルールと拡張 JSON のルールで、先に達した上限で探索が終わり、合法手が返ることを確認する。再読み込み後の設定復元、設定変更後に古い探索結果が指されないこと、未設定時の既定値もテストする。
