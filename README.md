# mameo

Windows 向けの超軽量・低遅延な Push-to-Talk 型 音声入力ツールです。  
常駐メモリは約 15MB、単一プロセス・単一バイナリで静かに動作し、Groq Whisper API（`whisper-large-v3-turbo`）による爆速の文字起こしを提供します。

---

## 主な特徴

- **超軽量・単一プロセス**: 常駐メモリ約 15MB。Electron や重い GUI フレームワークを一切使わず、単一バイナリのみで完結。
- **低遅延 (Sub-second)**: キーを離してから約 0.3 秒で自動入力（Groq Whisper Turbo 利用）。
- **Push-to-Talk 方式**: 指定キー（デフォルト: `Right Alt`）を押している間だけ録音し、離すと即座に入力。
- **エディタ・アプリ個別設定**: フォーカス中のアプリに応じて動作を切り替え可能（例: 通常アプリは `Ctrl+V` で即時貼り付け、Emacs や端末では誤爆を防ぐためクリップボード格納のみ）。
- **ユーザー辞書 & ハルシネーション抑制**: `DICT.csv` による固有名詞・専門用語の表記揺れ自動補正と、Whisper 特有の無音ノイズ（「ありがとうございました」等）の自動破棄。
- **タスクトレイ常駐**: システムトレイアイコンから設定や辞書の呼び出し、再読み込み、終了が可能。
- **多重起動防止**: Named Mutex による安全な単一インスタンス動作。

---

## 導入方法

### 方法 A: リリースバイナリを利用する（推奨）
1. Releases から最新の `mameo-vX.X.X-windows-x64.zip` をダウンロードして解凍します。
2. 任意のフォルダ（例: `C:\Tools\mameo`）に配置します。

### 方法 B: ソースからビルドする
```powershell
git clone https://github.com/xyslope/mameo.git
cd mameo
cargo build --release
```
生成された `target\release\mameo.exe` を使用します。

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
バイナリと同じ階層に配置されます。

```toml
trigger_key = "RightAlt"        # 録音キー: "RightAlt", "RightControl", "RightShift"
language = "ja"                 # 言語: "ja", "en", "auto"
groq_api_key = ""               # 空欄の場合は環境変数 GROQ_API_KEY を参照
restore_clipboard = true        # ペースト後にクリップボード履歴を復元するか
default_paste_mode = "auto"     # "auto" (Ctrl+V) または "copy_only"

# アプリ個別設定（プロセス名で判定）
[[app_rules]]
process_name = "emacs.exe"
paste_mode = "copy_only"        # Emacs では直接貼り付けずクリップボード格納のみ（p や C-y で貼り付け）
```

### ユーザー辞書 (`DICT.csv`)
CSVのテーブル形式で、専門用語や誤変換しやすい単語の置換ルールを管理できます。  
設定内容は Whisper の推論プロンプトへの事前注入と、テキスト受信後の事後置換の両方に適用されます。

```markdown
# 単語置換テーブル
誤認識・読み, 置換後
スライドブイ,Slidev
クオート, Quarto
イーマックス, emacs
エルパカ, Elpaca
```

> **Note**: `config.toml` や `DICT.csv` を編集した後は、トレイアイコンを右クリックして **「設定・辞書の再読み込み」** を実行すると、アプリを再起動することなく変更が反映されます。

---

## Windows 起動時に自動起動させる場合

1. `mameo.exe` のショートカットを作成します。
2. `Win + R` を押し、`shell:startup` と入力してスタートアップフォルダを開きます。
3. 作成したショートカットをそのフォルダに配置します。

---

## ライセンス

MIT License
