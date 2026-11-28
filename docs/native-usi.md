# Haitaka Variants のネイティブ USI エンジン

`haitaka-usi` は起動すると標準入出力で USI コマンドをやり取りします。Windows では、対局ソフトのエンジン登録画面で展開した `.exe` を選択します。Android では、arm64 向けの実行ファイルを USI クライアントから起動できます。下記の `adb` 手順で、対局アプリがなくても端末上で動作を確認できます。

GitHub Actions の `Native USI` ワークフローは、ルールと OS の組み合わせごとに ZIP または tar.gz を成果物として保存します。

## 対応ルール

| 配布物のルール名 | 将棋ルール | Cargo feature |
| --- | --- | --- |
| `standard` | 本将棋 | なし |
| `annan` | 安南将棋 | `annan` |
| `anhoku` | 安北将棋 | `anhoku` |
| `minishogi` | ５五将棋 | `minishogi` |

各実行ファイルの `position startpos` は、そのルールの平手初期局面を使います。`position sfen ... moves ...` で局面と棋譜を渡せます。５五将棋の座標は `1a` から `5e` までです。変則ルールで対局するには、クライアント側でも同じルールの盤面・着手・対局結果を扱える必要があります。終局を検出した場合は `info string gameover ...` が付く場合があり、USI の指し手の応答には終局を表す `bestmove resign` を使います。エンジンはルールに応じた初期局面と評価を内蔵しているため、追加の評価ファイルなしで起動できます。

## 深さと時間

- エンジンの設定画面で `MaxDepth` を選べます。範囲は 1〜100、既定値は 64 です。探索時間を指定したときは、時間か深さの上限に達した時点で着手します。深さは反復深化の上限で、静止探索や詰み探索では先の手も読みます。
- USI を直接操作する場合は `setoption name MaxDepth value 3` と指定します。`go depth 3` はその探索の深さを明示します。`go movetime 1000 depth 3` は時間 1000 ミリ秒と深さ 3 を同時に指定します。
- 対局時計の `btime`・`wtime` と、秒読み `byoyomi` または加算 `binc`・`winc` に対応します。検討用の `go infinite` は `stop` を受け取ると着手候補を返します。先読みは無効です。`go mate` には `checkmate notimplemented` を返します。

## Windows と Linux でのビルド

Rust と対象 OS のリンカーを用意し、リポジトリのルートから実行します。Windows x64 は MSVC ターゲットを使い、配布ビルドでは C ランタイムを静的にリンクします。

```powershell
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = "-C target-feature=+crt-static"
cargo build --release --target x86_64-pc-windows-msvc -p haitaka_cli --bin haitaka-usi
```

Linux で安南将棋をビルドする例です。別のルールは表の feature に置き換えます。

```sh
cargo build --release -p haitaka_cli --bin haitaka-usi --features annan
./target/release/haitaka-usi
```

`cargo run -p haitaka_cli --release -- usi` でも同じ USI エンジンを起動できます。

## Android arm64 でのビルドと起動

配布ビルドは Android NDK r30 と API 23 向けのリンカーを使います。Linux x64 の例では、`Haitaka_NDK` をインストール先に設定してビルドします。[NDK のビルド手順](https://developer.android.com/ndk/guides/other_build_systems)

```sh
rustup target add aarch64-linux-android
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$Haitaka_NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android23-clang"
cargo build --release --target aarch64-linux-android -p haitaka_cli --bin haitaka-usi
```

USB デバッグを利用できる arm64 端末で実行する例です。ビルド済みの配布物を使う場合は、`adb push` の転送元を展開した実行ファイルに置き換えます。

```sh
adb push target/aarch64-linux-android/release/haitaka-usi /data/local/tmp/haitaka-usi
adb shell chmod 755 /data/local/tmp/haitaka-usi
adb shell -T /data/local/tmp/haitaka-usi
```

起動したら `usi` に対する `usiok` と `isready` に対する `readyok` を確認します。`position startpos`、`go depth 1` と入力し、`bestmove` を受け取ってから `quit` で終了します。

Android 10 以降をターゲットにする一般アプリでは、書き込み可能なアプリ内ディレクトリから実行ファイルを直接起動することに制約があります。GUI からの利用は、各アプリが対応するエンジンの起動方法を確認してください。[Android の実行制約](https://developer.android.com/about/versions/10/behavior-changes-10?hl=ja#execute-permission)
