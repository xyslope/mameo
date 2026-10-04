# Implementation Plan (PLAN.md)

mameo の機能拡張、設定・辞書管理、タスクトレイ常駐、および設定 GUI アプリ開発に向けた実装ロードマップ。

---

## Milestone 4: 設定用 GUI アプリ（Iced）の追加 — 次に作業する内容

### リポジトリ構成（決定: 案 A — 別リポジトリ分離）
- GUI 版は **新規 private リポジトリ `mameo-config`** で開発（Microsoft Store 配布向け GUI + 設定 UI のみ）。
- 公開リポジトリ `mameo` は GUI 無しコア版のみ。core 改良は git dependency で `mameo-config` 側に取り込む（開発中は `[patch]` で path 参照に差し替え）。
- これにより、公開リポジトリへの機密コード混入リスクと長期ブランチの merge 衝突を構造的に回避する。


### 設計方針
- GUI は core daemon から独立した単体アプリ（例: `mameo-gui.exe`）。core daemon 自体は GUI の有無に依存せず常に動作する
- フレームワーク: **Iced**（Rust ネイティブの宣言型 GUI。Tauri / Electron / Qt 等の大型フレームワークは使わない）
- デザイン: **水色ベースのクールなテーマ**（薄い水色背景 + 水色アクセントの選択肢・ボタン、ダーク寄りの落ち着いた配色）

### 起動フロー（タスクトレイとの連携）
トレイの「設定を開く」を選択したとき、core daemon は次の挙動を行う:
1. 実行ファイルと同じディレクトリ（config.toml と同層）に `mameo-gui.exe` が存在するか確認
2. **存在する場合** → GUI アプリを spawn して開く
3. **存在しない場合** → これまでどおり `config.toml` を既定のエディタで開く（フォールバック。動作は現状維持）

### タスクリスト
- [ ] Workspace 化: `crates/core`（本体 daemon）+ `crates/gui`（Iced アプリ）構成に整理
- [x] mameo-config (別リポジトリ) に Iced GUI 実装: 水色ダークテーマ、トリガーキー / 言語 / API キー / クリップボード復元 / 貼り付けモード / アプリ別ルール編集、config.toml 保存（実機検証済み）
- [x] config 配置ルール統一: exe 隣接に config.toml があればポータブル、無ければ `%APPDATA%\mameo` (Roaming)。core と GUI で同一ロジック（MSIX/Store 対応）
- [x] core 側（トレイメニュー処理）に GUI 検出 conditional を実装（上記起動フロー）
- [x] GUI 側での `config.toml` 保存処理（lib.rs 経由で `Config` を共有、構造体二重定義解消済み）
- [ ] GUI 辞書タブ実装（DICT.csv 編集） ※実装済み。残: core の DICT.csv 行順保持へ
  （core は現在ハッシュマップで列順が変わる可能性 → `load_dictionary_rows` 使用へ統一）
- [ ] GUI 保存後に core へ設定変更を反映（Reload 不要の即時反映、まずは保存→トレイ側再読み込みで暫定対応でも可）

### CLI から GUI を開けるようにする（併設）
- [ ] GUI アプリ単体起動引数の追加（`mameo-gui.exe --open settings` 等。トレイ以外からも開けるように）

## Milestone 5: AI プロバイダ管理（BYOK・マルチプロバイダ対応）
- [ ] 統一 trait 背後でのプロバイダ抽象化（Groq / OpenAI / Gemini / Ollama）
- [ ] `config.toml` でプロバイダ選択（GUI のドロップダウンから選べるように）
- [ ] 既定は Groq（whisper-large-v3-turbo）を維持、現行動作を壊さない

## Milestone 6: クリップボード履歴ポップアップ（将来構想）
- [ ] core 側にスレッドセーフなリングバッファ（テキストのみ・画像/ファイルはスキップ）
- [ ] 音声挿入前に直前のクリップボード内容を履歴 index 0 として確保
- [ ] ポップアップ UI（Iced・フレームレス浮動ウィンドウ・カーソル近傍に表示）
- [ ] A–J の直接選択ペースト、n/p・矢印キーで移動、Enter 確定、Esc 閉じる
- [ ] トリガー: RightShift ダブルタップ（内部フック、config.toml で無効化可能）＋外部スクリプト用 CLI サブコマンド（AHK 連携用）

## 配布・ブランチ戦略（GUI 無し Public 版と GUI 付き版の分離）
- 公開 `main` ブランチ: GUI 無しコア版のみ（CLI daemon + トレイ + テキストによる config 編集）。Workspace Cargo.toml は `crates/core` のみ参照
- 非公開 GUI 版（Store 版）: `crates/core` + `crates/gui` を含む別ブランチで管理
  - 公開 `main` からの merge で core の改良を随時取り込む
  - GitHub Releases で core 版を公開、GUI 版は別パイプラインでパッケージ化

---

# 完了済み Milestone（アーカイブ）

## Milestone 1: 外部設定と辞書の分離 ✅
- [x] 設定ファイル `config.toml` の設計と読み込み実装
  - 実行ファイル（exe）と同じディレクトリを探索
  - トリガーキーの指定（`RightAlt`, `RightControl`, `RightShift` 等）
  - 言語指定（`ja`, `auto` 等）
  - APIキーのロード（`config.toml` 優先、環境変数 `GROQ_API_KEY` をフォールバック）
  - クリップボード復元設定（`restore_clipboard = true/false`）
- [x] 辞書ファイルの読み込み実装
  - ※実装は `DICT.md` ではなく **`DICT.csv`**（カンマ区切りテキスト形式）で `PLAN` 記載から変更済み
  - 起動時にメモリ上にハッシュマップとしてキャッシュ
  - Groq Whisper の `prompt` 引数へ渡す固有名詞リストの抽出（事前誘導）
  - API レスポンス文字列に対するローカル置換処理（事後確定）

## Milestone 2: タスクトレイ常駐 & プロセス管理（UX改善） ✅
- [x] 軽量クレート（`tray-icon` + `muda`）によるタスクトレイ常駐
- [x] 右クリックメニューの実装:
  - 「設定を開く (`config.toml`)」→ 関連付けられたエディタで開く
  - 「辞書を開く (`DICT.md`)」→ 関連付けられたエディタで開く
  - 「辞書・設定を再読み込み (Reload)」→ メモリ上のキャッシュを即時更新
  - 「終了 (Exit)」→ プロセスをクリーンに終了
- [x] コンソール非表示化（`#![windows_subsystem = "windows"]`）
- [x] 二重起動防止（Windows Named Mutex による多重起動ガード）
- ※注意: メニュー表示名は `DICT.md` だが、実体は `DICT.csv` → 表示・パスの不一致あり（別途修正予定）

## Milestone 3: 堅牢性と使い勝手のブラッシュアップ ✅（一部残タスクあり）
- [x] 無音・極小音声ガード（0.2 秒未満の録音は API 呼び出しをスキップ）
- [x] クリップボード復元ディレイ（200ms 固定で実装済み。今後調整の余地あり）
- [ ] 録音状態に応じたトレイアイコンの切り替え（待機中 / 録音中） ← 未実装、残タスク
