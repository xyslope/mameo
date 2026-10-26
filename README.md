# mameo

Windows 向けの超軽量・低遅延な Push-to-Talk 型 音声入力ツールです。  
常駐メモリは約 15MB、単一プロセス・単一バイナリで静かに動作し、Groq Whisper API（`whisper-large-v3-turbo`）による爆速の文字起こしを提供します。

---

## 主な特徴

- **超軽量・単一プロセス**: 常駐メモリ約 15MB。Electron や重い GUI フレームワークを一切使わず、単一バイナリのみで完結。
- **低遅延 (Sub-second)**: キーを離してから約 0.3 秒で自動入力（Groq Whisper Turbo 利用）。
- **Push-to-Talk 方式**: 指定キー（デフォルト: `Right Alt`）を押している間だけ録音し、離すと即座に入力。
- **エディタ・アプリ個別設定**: フォーカス中のアプリに応じて動作を切り替え可能（例: 通常アプリは `Ctrl+V` で即時貼り付け、Emacs や端末では誤爆を防ぐためクリップボード格納のみ）。
- **プロンプト & ハルシネーション抑制**: Markdownで文字起こしの指示や専門用語を指定し、Whisper 特有の無音ノイズ（「ありがとうございました」等）の自動破棄。
- **タスクトレイ常駐**: システムトレイアイコンから設定の呼び出し、再読み込み、終了が可能。
- **多重起動防止**: Named Mutex による安全な単一インスタンス動作。

---

## 導入方法

### 方法 A: リリースバイナリを利用する（推奨）
1. Releases から最新の `mameo-vX.X.X-windows-x64.zip` をダウンロードして解凍します。
2. 任意のフォルダ（例: `C:\Tools\mameo`）に配置します。
3. ZIP 配布には `portable.flag` を同梱し、`mameo.exe` と同じフォルダに配置してください。

### 方法 B: ソースからビルドする
```powershell
git clone https://github.com/xyslope/mameo.git
cd mameo
cargo build --release
```
生成された `target\release\mameo.exe` を使用します。
ポータブル版として使う場合は、リポジトリの `portable.flag` を `target\release` にコピーしてください。`cargo build` はフラグを自動コピーしません。

---

## 初期設定

### 1. Groq API キーの準備
[Groq Console](https://console.groq.com/) で無料の API キーを発行します。

以下のいずれかの方法で設定します。
- **方法 1（推奨）**: 初回起動時に自動生成される `config.toml` を開き、`groq_api_key = "gsk_xxxx..."` に記述する。
- **方法 2**: Windows の環境変数 `GROQ_API_KEY` にキーを登録しておく。

### 2. 起動
`mameo.exe` を実行します。コンソール画面は開かず、タスクバーの通知領域（タスクトレイ）に赤い丸アイコンが表示されます。

---

## 使い方

1. メモ帳、ブラウザ、エディタなど、入力したい場所をクリックしてカーソルを置きます。
2. **`Right Alt`（右 Alt キー）** を押しながらマイクに向かって話します。
3. 話し終えたらキーを離します。約 0.3 秒でカーソル位置に入力されます。

---

## 設定とカスタマイズ

タスクトレイのアイコンを右クリックすると、各種ファイルを直接開くことができます。

### 設定ファイル (`config.toml`)
保存先は起動時に決まり、`config.toml`・指示ファイルで共通です。

- exe の隣に `portable.flag` がある場合: exe と同じフォルダ（ZIP・ポータブル版）。フラグの本文は任意で、空ファイルでも構いません。
- フラグがない場合: `%APPDATA%\mameo`（通常インストール・Store版）。フォルダがなければ作成します。

Store版には `portable.flag` を同梱しません。exe の隣に `config.toml` があっても、フラグがなければそちらは使いません。保存先を切り替えても設定の自動移動や削除は行いません。必要な設定・指示ファイルは使用するフォルダへコピーしてください。フラグの追加・削除後はアプリを再起動してください。トレイの「設定フォルダを開く」で実際の保存先を確認できます。

ファイルが存在しない場合だけ初期設定を生成します。既存ファイルは起動・再読み込み時に上書きしません。省略した項目には既定値を使い、構文エラーや読み取り失敗の場合はファイルを保持したまま、その回は既定値で動作します。

```toml
trigger_key = "RightAlt"        # 録音キー: "RightAlt", "RightControl", "RightShift"
language = "ja"                 # 言語: "ja", "en", "auto"
llm_prompt_file = "llm-jp-normal.md" # API に渡す指示ファイル（空欄で指示を無効化）
groq_api_key = ""               # 空欄の場合は環境変数 GROQ_API_KEY を参照
transcription_provider = "groq" # "groq", "gemini", "soniox"
groq_model = "whisper-large-v3-turbo"
gemini_model = "gemini-3.8-flash" # 環境変数 GEMINI_API_KEY を使用
soniox_model = "stt-rt-v5"       # 環境変数 SONIOX_API_KEY を使用
restore_clipboard = true        # ペースト後にクリップボード履歴を復元するか
debug_transcription = false    # true で録音・認識結果を保存（原因調査用）
default_paste_mode = "auto"     # "auto" (Ctrl+V) または "copy_only"

# アプリ個別設定（プロセス名で判定）
[[app_rules]]
process_name = "emacs.exe"
paste_mode = "copy_only"        # Emacs では直接貼り付けずクリップボード格納のみ（p や C-y で貼り付け）
```

### 指示ファイル (`llm-言語-種別.md`)

使用中の設定フォルダに UTF-8 の Markdown ファイルを置き、`config.toml` の `llm_prompt_file` で指定します。例えば `llm-jp-normal.md`、`llm-jp-lecture.md`、`llm-en-normal.md` のように、言語と用途を名前に含めて切り替えられます。ファイル名から言語を自動判定することはなく、API の言語指定には `language` を使います（日本語は `ja`）。

別ソフトの `mameo-config` の「プロンプト」タブでも、設定フォルダにある `llm-*.md` の選択、新規作成、本文の編集ができます。選択はその場で `llm_prompt_file` に保存され、次回の録音と次回起動時に使われます。本文は「プロンプトを保存」で保存してください。未保存の本文がある場合は、保存するまで別ファイルへ切り替えられません。日本語・英語の標準プロンプトは初回表示時に存在しない場合だけ作成し、既存ファイルは上書きしません。英語の音声を入力する場合は、設定タブの言語も `en` にしてください。

`llm-jp-lecture.md` の本文例:

```markdown
基本的に日本語を話しています。
あいまいな場合は、一番近い日本語を返してください。
話者は大学で環境経済学を教えているので、その関係の専門用語をよく使います。
```

Groqには本文をWhisperの `prompt` として渡します。Geminiの汎用音声モデルには、録音と文字起こしの指示、本文をまとめて渡します。Whisperの結果を別のLLMで補正する二段階の構成ではありません。

指示ファイルは送信前に読み込むので、本文の編集は次の録音から反映されます。設定の変更は約1.5秒で自動反映、またはトレイの「設定の再読み込み」で反映できます。既存の設定に `llm_prompt_file` がない場合は `llm-jp-normal.md` を使います。指定ファイルがなければ既定の日本語の指示で自動生成し、生成・読み取りに失敗した場合も既定の指示にフォールバックします。空の本文なら追加の指示は送信しません。

### 専門用語・表記の指定

辞書機能は廃止しました。専門用語や希望する表記は、使用中のプロンプト本文に記載してください。例えば「スライドブイは Slidev、クオートは Quarto と表記してください」と指定できます。認識結果の文字列置換は行わず、指定が反映されるかは認識モデルによります。既存の `DICT.csv` は削除しませんが、読み込み・生成・監視は行いません。

設定アプリは「設定」「プロンプト」の2タブです。起動中にもう一度開くと、既存ウィンドウを前面に出します。設定フォルダの `llm-*.md` は起動時と起動中に1秒間隔で検出し、ドロップダウンへ反映します。トレイの「設定フォルダを開く」で保存先を確認できます。

---

## Groq / Gemini / Soniox の切り替え

設定は `[[app_rules]]` より前のトップレベルに記述してください。既存の設定に新しい項目がなくても、これまでどおり Groq Turbo を使います。ファイルは自動で書き換えません。

```toml
transcription_provider = "gemini"
gemini_model = "gemini-3.8-flash"
language = "auto"
```

Geminiは環境変数 `GEMINI_API_KEY` を使います。環境変数を追加した後はアプリを再起動してください。プロバイダーやモデルの設定変更は約1.5秒で自動反映し、次の録音から使います。音声入力をサポートするモデルを指定してください。モデル名やキーのエラーで別のプロバイダーに自動切り替えはしません。

`gemini_model = "gemini-3.5-transcribe"` も使用できます。この専用モデルは `verbatim` モードで発話の繰り返しを保持します。自由文のMarkdown指示は専用モデルには渡しません。自由文のカスタマイズを使いたい場合は汎用音声モデルを指定してください。専用モデルでは `ja` を `ja-JP`、`en` を `en-US` に変換し、`auto` では言語ヒントを省略します。

Geminiには録音を1回のリクエストで送ります。JSONとbase64音声を含めて20MB未満に制限し、WAVが14MBを超える場合は送信せずエラーにします。新しいSDKや常駐プロセスは追加していません。

保存済みWAVで比較する場合:

```powershell
cargo run --example transcribe_wav -- gemini gemini-3.8-flash "C:\path\recording.wav" "C:\path\mameo"
```

このコマンドは文字起こしと所要時間を表示し、貼り付けやconfigの保存はしません。診断JSONにも使用した `provider` と `model` を記録します。

### Sonioxを試す

[Soniox Console](https://console.soniox.com/) でAPIキーを発行し、環境変数 `SONIOX_API_KEY` に設定してアプリを再起動してください。

```toml
transcription_provider = "soniox"
soniox_model = "stt-rt-v5"
language = "auto"
```

SonioxではMarkdown本文を認識用コンテキストとして渡します。`auto` では言語ヒントを省略します。`ja` などを指定しても、それ以外の言語は制限しません。暫定結果は貼り付けず、確定トークンだけを連結します。

アプリではキーを押した時点でワーカーがSonioxへ接続し、録音中からモノラルPCM音声を送信・認識します。キーを離すと200ms分の無音を追加して手動確定を要求し、確定結果を貼り付けます。接続確立前の音声は上限付きキューに保持します。キューが満杯なら録音を中止し、欠けた音声の認識結果は貼り付けません。キー監視・音声キャプチャ上で通信やファイル読み込みを行いません。接続先は米国の標準エンドポイントで、接続は録音ごとに作成・終了します。

Sonioxでは録音中の無音も送信されますが、全てデジタル無音だった録音や0.2秒未満の録音の結果は貼り付けません。GroqとGeminiは引き続きキー解放後の録音送信方式です。

同じWAVで比較する場合:

```powershell
cargo run --example transcribe_wav -- soniox stt-rt-v5 "C:\path\recording.wav" "C:\path\mameo"
```

録音中の送信を模擬してキー解放後の待ち時間を測る場合は、`soniox` を `soniox-stream` に置き換えます。モノラル16bit PCMのWAVを20msずつ実時間で送り、`After simulated key release` に入力終了から確定までの時間を表示します。通常の `soniox` は録音全体をまとめて送る比較用です。

## 認識の繰り返し・誤変換の調査

`config.toml` のトップレベル（`[[app_rules]]` より前）に `debug_transcription = true` を設定すると、設定フォルダの `diagnostics` に録音ごとの WAV と JSON を保存します。WAV を聞いて実際の録音を確認し、JSON の `raw_text` でAPIの認識結果を確認できます。API に失敗した場合は WAV だけが残ります。

録音音声と文字起こし内容がディスクに保存されるため、調査が終わったら `false` に戻し、不要な診断ファイルを削除してください。自動削除は行いません。既定では無効です。

Sonioxの診断JSONの `timing` には `connection_ms`（接続時間）、`release_to_finalize_request_ms`（実際のキー解放から送信待ちの音声を送り切って確定要求するまで）、`finalize_ms`（確定要求から結果まで）、`release_to_result_ms`（実際のキー解放から結果まで）を記録します。これらには診断保存・貼り付け後の画面反映時間は含みません。

Groq・GeminiではAPIに送る前に前後の完全な無音（サンプル値が厳密に0）を除去します。全て無音なら送信せず、診断ファイルも生成しません。小声を保持するため音量しきい値は使わず、発話の途中の無音は残します。診断 WAV はこの処理後の送信音声です。Sonioxの診断WAVは録音中に送った音声で、確定用に追加した200msの無音は含めません。

## Windows 起動時に自動起動させる場合

1. `mameo.exe` のショートカットを作成します。
2. `Win + R` を押し、`shell:startup` と入力してスタートアップフォルダを開きます。
3. 作成したショートカットをそのフォルダに配置します。

---

## ライセンス

MIT License
