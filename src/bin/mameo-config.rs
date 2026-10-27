//! mameo-config: 設定 GUI for mameo (voice input daemon)
//!
//! config.toml / llm-*.md を読み込み、Iced GUI で編集して保存する。
//! タブ: 「設定」（トリガーキー / 言語 / API キー / 貼り付けモード / アプリ別ルール）
//!       「プロンプト」（指示ファイルの選択・新規作成・編集）

#![windows_subsystem = "windows"]

use iced::widget::{
    button, checkbox, column, container, pick_list, row, scrollable, text, text_editor, text_input,
};
use iced::{Element, Length, Task, Theme};

#[path = "../config_ui/single_instance.rs"]
mod single_instance;
#[path = "../config_ui/theme.rs"]
mod theme;

use mameo::{AppRule, Config};
use theme::{ACCENT, BORDER};

/// 起動引数で設定またはプロンプトのタブを選ぶ。
fn parse_start_tab() -> Option<Tab> {
    let mut args = std::env::args().skip(1);
    let mut selected = None;
    while let Some(arg) = args.next() {
        let value = if arg == "--tab" || arg == "--open" {
            args.next().unwrap_or_default()
        } else {
            arg.strip_prefix("--tab=")
                .or_else(|| arg.strip_prefix("--open="))
                .unwrap_or(&arg)
                .to_string()
        };
        match value.to_ascii_lowercase().as_str() {
            "settings" => selected = Some(Tab::Settings),
            "prompt" | "prompts" => selected = Some(Tab::Prompt),
            _ => {}
        }
    }
    selected
}

fn main() -> iced::Result {
    let _instance = match single_instance::SingleInstance::acquire() {
        Ok(Some(instance)) => instance,
        Ok(None) => return Ok(()),
        Err(error) => {
            eprintln!("Cannot acquire config instance: {error}");
            return Ok(());
        }
    };
    iced::application(State::boot, State::update, State::view)
        .title(window_title)
        .window_size(iced::Size::new(560.0, 720.0))
        .resizable(true)
        .centered()
        .theme(aqua_theme)
        .subscription(|_| iced::Subscription::run(refresh_events))
        .run()
}

fn window_title(_state: &State) -> String {
    "mameo 設定".to_string()
}

fn aqua_theme(_state: &State) -> Theme {
    Theme::custom("AquaDark", theme::aqua_dark())
}

// ---------------------------------------------------------------
// 状態
// ---------------------------------------------------------------

/// タブ種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Settings,
    Prompt,
}

impl Tab {
    const ALL: [Tab; 2] = [Tab::Settings, Tab::Prompt];

    fn label(&self) -> &'static str {
        match self {
            Tab::Settings => "設定",
            Tab::Prompt => "プロンプト",
        }
    }
}

struct State {
    base_dir: std::path::PathBuf,
    config_path: std::path::PathBuf,

    active_tab: Tab,

    // --- 設定タブ: 編集中の値（確定時に config.toml へ書き出す） ---
    trigger_key: TriggerKey,
    language: Language,
    transcription_provider: TranscriptionProvider,
    groq_model: String,
    gemini_model: String,
    soniox_model: String,
    test_status: String,
    test_log_path: Option<std::path::PathBuf>,
    restore_clipboard: bool,
    default_paste_mode: PasteMode,
    app_rules: Vec<RuleRow>,

    prompt_files: Vec<String>,
    selected_prompt: Option<String>,
    prompt_editor: text_editor::Content,
    prompt_dirty: bool,
    new_prompt_name: String,

    status: String,
    dirty: bool,
}

/// pick_list 用トリガーキー
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // config.toml の値名と一致させるため接頭辞 Right を維持
enum TriggerKey {
    RightAlt,
    RightControl,
    RightShift,
}

impl TriggerKey {
    const ALL: [TriggerKey; 3] = [
        TriggerKey::RightAlt,
        TriggerKey::RightControl,
        TriggerKey::RightShift,
    ];

    fn as_str(&self) -> &'static str {
        match self {
            TriggerKey::RightAlt => "RightAlt",
            TriggerKey::RightControl => "RightControl",
            TriggerKey::RightShift => "RightShift",
        }
    }

    fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "rightcontrol" | "rcontrol" => TriggerKey::RightControl,
            "rightshift" | "rshift" => TriggerKey::RightShift,
            _ => TriggerKey::RightAlt,
        }
    }
}

impl std::fmt::Display for TriggerKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// pick_list 用言語
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Ja,
    Auto,
    En,
}

impl Language {
    const ALL: [Language; 3] = [Language::Ja, Language::Auto, Language::En];

    fn as_str(&self) -> &'static str {
        match self {
            Language::Ja => "ja",
            Language::Auto => "auto",
            Language::En => "en",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "auto" => Language::Auto,
            "en" => Language::En,
            _ => Language::Ja,
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// pick_list 用文字起こしプロバイダー
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranscriptionProvider {
    Groq,
    Gemini,
    Soniox,
}

impl TranscriptionProvider {
    const ALL: [Self; 3] = [Self::Groq, Self::Gemini, Self::Soniox];

    fn as_str(self) -> &'static str {
        match self {
            Self::Groq => "groq",
            Self::Gemini => "gemini",
            Self::Soniox => "soniox",
        }
    }

    fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "gemini" => Self::Gemini,
            "soniox" => Self::Soniox,
            _ => Self::Groq,
        }
    }
}

impl std::fmt::Display for TranscriptionProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Groq => "Groq",
            Self::Gemini => "Gemini",
            Self::Soniox => "Soniox",
        })
    }
}

/// pick_list 用貼り付けモード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasteMode {
    Auto,
    CopyOnly,
    CtrlY,
}

impl PasteMode {
    const ALL: [PasteMode; 3] = [PasteMode::Auto, PasteMode::CopyOnly, PasteMode::CtrlY];

    fn as_str(&self) -> &'static str {
        match self {
            PasteMode::Auto => "auto",
            PasteMode::CopyOnly => "copy_only",
            PasteMode::CtrlY => "ctrl_y",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "copy_only" => PasteMode::CopyOnly,
            "ctrl_y" => PasteMode::CtrlY,
            _ => PasteMode::Auto,
        }
    }
}

impl std::fmt::Display for PasteMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// アプリ別ルール 1 行分の編集状態
struct RuleRow {
    process_name: String,
    paste_mode: PasteMode,
}

impl RuleRow {
    fn new(process_name: String, paste_mode: PasteMode) -> Self {
        Self {
            process_name,
            paste_mode,
        }
    }
}

// ---------------------------------------------------------------
// ロード / セーブ
// ---------------------------------------------------------------

impl State {
    /// iced の boot 関数（引数なし）。
    /// core (mameo.exe) と同じ場所の config.toml を探し、無ければ %APPDATA%\mameo。
    fn boot() -> Self {
        let start_tab = parse_start_tab();
        Self::load(
            mameo::get_base_dir().expect("設定フォルダを解決できません"),
            start_tab,
        )
    }

    fn load(base_dir: std::path::PathBuf, start_tab: Option<Tab>) -> Self {
        let config_path = base_dir.join("config.toml");
        let cfg = Config::load_or_default(&base_dir);
        let mut prompt_status = String::new();
        if let Err(error) = ensure_builtin_prompts(&base_dir) {
            prompt_status = format!("標準プロンプト作成エラー: {error}");
        }
        let _ = mameo::load_transcription_prompt(&base_dir, &cfg.llm_prompt_file);
        let prompt_files = list_prompts(&base_dir, &cfg.llm_prompt_file);
        let selected_prompt =
            (!cfg.llm_prompt_file.is_empty()).then(|| cfg.llm_prompt_file.clone());
        let prompt_text = selected_prompt
            .as_ref()
            .and_then(|name| std::fs::read_to_string(base_dir.join(name)).ok())
            .unwrap_or_default();
        Self {
            prompt_files,
            selected_prompt,
            prompt_editor: text_editor::Content::with_text(&prompt_text),
            prompt_dirty: false,
            new_prompt_name: String::new(),
            base_dir,
            config_path,
            active_tab: start_tab.unwrap_or(Tab::Settings),
            trigger_key: TriggerKey::parse(&cfg.trigger_key),
            language: Language::parse(&cfg.language),
            transcription_provider: TranscriptionProvider::parse(&cfg.transcription_provider),
            groq_model: cfg.groq_model,
            gemini_model: cfg.gemini_model,
            soniox_model: cfg.soniox_model,
            test_status: "未検証".into(),
            test_log_path: None,
            restore_clipboard: cfg.restore_clipboard,
            default_paste_mode: PasteMode::parse(&cfg.default_paste_mode),
            app_rules: cfg
                .app_rules
                .into_iter()
                .map(|r| RuleRow::new(r.process_name, PasteMode::parse(&r.paste_mode)))
                .collect(),
            status: prompt_status,
            dirty: false,
        }
    }

    fn to_config(&self) -> Config {
        Config {
            trigger_key: self.trigger_key.as_str().to_string(),
            language: self.language.as_str().to_string(),
            transcription_provider: self.transcription_provider.as_str().to_string(),
            groq_model: self.groq_model.clone(),
            gemini_model: self.gemini_model.clone(),
            soniox_model: self.soniox_model.clone(),
            // API キーは環境変数から取得し、設定ファイルへ保存しない。
            groq_api_key: String::new(),
            restore_clipboard: self.restore_clipboard,
            default_paste_mode: self.default_paste_mode.as_str().to_string(),
            app_rules: self
                .app_rules
                .iter()
                .map(|r| AppRule {
                    process_name: r.process_name.trim().to_string(),
                    paste_mode: r.paste_mode.as_str().to_string(),
                })
                .collect(),
            ..Config::load_or_default(&self.base_dir)
        }
    }

    fn current_model_mut(&mut self) -> &mut String {
        match self.transcription_provider {
            TranscriptionProvider::Groq => &mut self.groq_model,
            TranscriptionProvider::Gemini => &mut self.gemini_model,
            TranscriptionProvider::Soniox => &mut self.soniox_model,
        }
    }

    fn current_model(&self) -> &str {
        match self.transcription_provider {
            TranscriptionProvider::Groq => &self.groq_model,
            TranscriptionProvider::Gemini => &self.gemini_model,
            TranscriptionProvider::Soniox => &self.soniox_model,
        }
    }

    fn save(&mut self) {
        let cfg = self.to_config();
        match cfg.save(&self.config_path) {
            Ok(()) => {
                self.status = format!("保存しました: {}", self.config_path.display());
                self.dirty = false;
            }
            Err(e) => {
                self.status = format!("保存エラー: {e}");
            }
        }
    }
}

// ---------------------------------------------------------------
// メッセージ / update
// ---------------------------------------------------------------

#[derive(Debug, Clone)]
enum Message {
    TabSelected(Tab),

    TriggerKeySelected(TriggerKey),
    LanguageSelected(Language),
    TranscriptionProviderSelected(TranscriptionProvider),
    ModelChanged(String),
    TestConnection,
    TestFinished(Result<(String, std::path::PathBuf), String>),
    RestoreClipboardToggled(bool),
    PasteModeSelected(PasteMode),

    AddRule,
    RemoveRule(usize),
    RuleProcessChanged(usize, String),
    RulePasteModeSelected(usize, PasteMode),

    Save,
    RefreshPrompts,
    PromptSelected(String),
    PromptEdited(text_editor::Action),
    NewPromptNameChanged(String),
    CreatePrompt,
    SavePrompt,
}

impl State {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TabSelected(t) => {
                self.active_tab = t;
            }
            Message::TriggerKeySelected(k) => {
                self.trigger_key = k;
                self.dirty = true;
            }
            Message::LanguageSelected(l) => {
                self.language = l;
                self.dirty = true;
            }
            Message::TranscriptionProviderSelected(provider) => {
                self.transcription_provider = provider;
                self.test_status = "未検証".into();
                self.dirty = true;
            }
            Message::ModelChanged(model) => {
                self.current_model_mut().clone_from(&model);
                self.test_status = "未検証".into();
                self.dirty = true;
            }
            Message::TestConnection => {
                let mut config = self.to_config();
                config.llm_prompt_file.clear();
                let base = self.base_dir.clone();
                self.test_status = "テスト中…".into();
                return Task::perform(
                    async move {
                        let model = match config.transcription_provider.as_str() {
                            "groq" => &config.groq_model,
                            "gemini" => &config.gemini_model,
                            _ => &config.soniox_model,
                        }
                        .clone();
                        let wav = include_bytes!("../../test-data/transcriptiontest.wav").to_vec();
                        let result = mameo::transcription::transcribe(&config, wav, "");
                        let success = result.is_ok();
                        let log = format!(
                            "provider={}\nmodel={}\nresult={:?}\n",
                            config.transcription_provider, model, result
                        );
                        let dir = base.join("diagnostics");
                        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                        let stamp = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        let path = dir.join(format!(
                            "api-test-{}-{stamp}.log",
                            config.transcription_provider
                        ));
                        std::fs::write(&path, log).map_err(|e| e.to_string())?;
                        if success {
                            Ok(("成功".into(), path))
                        } else {
                            Ok(("失敗".into(), path))
                        }
                    },
                    Message::TestFinished,
                );
            }
            Message::TestFinished(result) => match result {
                Ok((status, path)) => {
                    self.test_status = status;
                    self.test_log_path = Some(path);
                }
                Err(error) => {
                    self.test_status = format!("失敗: {error}");
                }
            },
            Message::RestoreClipboardToggled(v) => {
                self.restore_clipboard = v;
                self.dirty = true;
            }
            Message::PasteModeSelected(m) => {
                self.default_paste_mode = m;
                self.dirty = true;
            }
            Message::AddRule => {
                self.app_rules
                    .push(RuleRow::new(String::new(), PasteMode::Auto));
                self.dirty = true;
            }
            Message::RemoveRule(i) => {
                if i < self.app_rules.len() {
                    self.app_rules.remove(i);
                    self.dirty = true;
                }
            }
            Message::RuleProcessChanged(i, s) => {
                if let Some(r) = self.app_rules.get_mut(i) {
                    r.process_name = s;
                    self.dirty = true;
                }
            }
            Message::RulePasteModeSelected(i, m) => {
                if let Some(r) = self.app_rules.get_mut(i) {
                    r.paste_mode = m;
                    self.dirty = true;
                }
            }
            Message::PromptSelected(name) => {
                if self.prompt_dirty {
                    self.status = "先にプロンプトを保存してください。".into();
                } else {
                    match std::fs::read_to_string(self.base_dir.join(&name)) {
                        Ok(body) => {
                            let mut cfg = Config::load_or_default(&self.base_dir);
                            cfg.llm_prompt_file = name.clone();
                            match cfg.save(&self.config_path) {
                                Ok(()) => {
                                    self.selected_prompt = Some(name);
                                    self.prompt_editor = text_editor::Content::with_text(&body);
                                    self.status = "使用するプロンプトを保存しました。次回の録音から反映されます。".into();
                                }
                                Err(e) => self.status = format!("設定保存エラー: {e}"),
                            }
                        }
                        Err(e) => self.status = format!("読み込みエラー: {e}"),
                    }
                }
            }
            Message::PromptEdited(action) => {
                if self.selected_prompt.is_some() {
                    self.prompt_dirty |= action.is_edit();
                    self.prompt_editor.perform(action);
                }
            }
            Message::NewPromptNameChanged(name) => self.new_prompt_name = name,
            Message::CreatePrompt => {
                let name = self.new_prompt_name.trim().to_string();
                if self.prompt_dirty {
                    self.status = "先にプロンプトを保存してください。".into();
                } else if !valid_prompt_name(&name) {
                    self.status = "llm-en-normal.md のようなファイル名を入力してください。".into();
                } else {
                    use std::io::Write;
                    let result = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(self.base_dir.join(&name))
                        .and_then(|mut file| {
                            file.write_all(if name.starts_with("llm-en-") {
                                mameo::DEFAULT_ENGLISH_LLM_PROMPT.as_bytes()
                            } else {
                                mameo::DEFAULT_LLM_PROMPT.as_bytes()
                            })
                        });
                    match result {
                        Ok(()) => {
                            self.prompt_files = list_prompts(&self.base_dir, &name);
                            self.new_prompt_name.clear();
                            return self.update(Message::PromptSelected(name));
                        }
                        Err(e) => self.status = format!("新規作成エラー: {e}"),
                    }
                }
            }
            Message::SavePrompt => {
                if let Some(name) = &self.selected_prompt {
                    match std::fs::write(self.base_dir.join(name), self.prompt_editor.text()) {
                        Ok(()) => {
                            self.prompt_dirty = false;
                            self.status =
                                "プロンプトを保存しました。次回の録音から反映されます。".into();
                        }
                        Err(e) => self.status = format!("プロンプト保存エラー: {e}"),
                    }
                }
            }
            Message::Save => {
                self.save();
            }
            Message::RefreshPrompts => {
                self.prompt_files = list_prompts(&self.base_dir, "");
            }
        }
        Task::none()
    }

    // ---------------------------------------------------------------
    // view
    // ---------------------------------------------------------------

    fn view(&self) -> Element<'_, Message> {
        // --- タブバー ---
        let mut tab_bar = row![].spacing(6);
        for t in Tab::ALL {
            let label = text(t.label()).size(14);
            let btn = if t == self.active_tab {
                // アクティブタブは水色で強調
                button(label)
                    .padding([6, 18])
                    .style(|theme: &Theme, status| button::Style {
                        background: Some(iced::Background::Color(ACCENT)),
                        text_color: theme.palette().background,
                        ..button::primary(theme, status)
                    })
            } else {
                button(label)
                    .on_press(Message::TabSelected(t))
                    .padding([6, 18])
            };
            tab_bar = tab_bar.push(btn);
        }

        let header = row![text("mameo").size(20), tab_bar,]
            .spacing(20)
            .align_y(iced::Alignment::Center);

        let content = match self.active_tab {
            Tab::Settings => self.view_settings(),
            Tab::Prompt => self.view_prompt(),
        };

        // 下部ステータス行
        let status_line = text(if self.dirty || self.prompt_dirty {
            format!("● 未保存の変更があります　{}", self.status)
        } else {
            self.status.clone()
        })
        .size(12)
        .style(|_| text::Style {
            color: Some(ACCENT),
        });

        let body = column![header, content, status_line]
            .spacing(14)
            .padding(20);

        scrollable(container(body).width(Length::Fill).center_x(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    // --- 設定タブ ---
    fn view_settings(&self) -> Element<'_, Message> {
        let trigger_row = labeled_row(
            "トリガーキー",
            pick_list(
                TriggerKey::ALL,
                Some(self.trigger_key),
                Message::TriggerKeySelected,
            )
            .width(180),
        );

        let language_row = labeled_row(
            "言語",
            pick_list(
                Language::ALL,
                Some(self.language),
                Message::LanguageSelected,
            )
            .width(180),
        );

        let provider_row = labeled_row(
            "文字起こしモデル",
            pick_list(
                TranscriptionProvider::ALL,
                Some(self.transcription_provider),
                Message::TranscriptionProviderSelected,
            )
            .width(180),
        );
        let model_row = labeled_row(
            "モデル名",
            text_input("モデル名", self.current_model())
                .on_input(Message::ModelChanged)
                .width(240),
        );
        let test_row = row![
            button("接続テスト").on_press(Message::TestConnection),
            text(if let Some(path) = &self.test_log_path {
                format!("{} — {}", self.test_status, path.display())
            } else {
                self.test_status.clone()
            })
            .size(12),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center);
        let provider_note = text(
            "API キーは GROQ_API_KEY / GEMINI_API_KEY / SONIOX_API_KEY 環境変数で設定します。接続テストでは test-data/transcriptiontest.wav を送信します。",
        ).size(11);

        let clipboard_row = row![checkbox(self.restore_clipboard)
            .label("音声入力後にクリップボードの内容を復元する")
            .on_toggle(Message::RestoreClipboardToggled)]
        .spacing(8);

        let paste_mode_row = labeled_row(
            "既定の貼り付けモード",
            pick_list(
                PasteMode::ALL,
                Some(self.default_paste_mode),
                Message::PasteModeSelected,
            )
            .width(180),
        );

        // --- アプリ別ルール ---
        let mut rules_col = column![text("アプリ別ルール").size(16)].spacing(8);

        for (i, rule) in self.app_rules.iter().enumerate() {
            rules_col = rules_col.push(
                row![
                    text_input("process.exe", &rule.process_name)
                        .on_input(move |s| Message::RuleProcessChanged(i, s))
                        .width(240),
                    pick_list(PasteMode::ALL, Some(rule.paste_mode), move |m| {
                        Message::RulePasteModeSelected(i, m)
                    })
                    .width(150),
                    button(text("削除").size(12))
                        .on_press(Message::RemoveRule(i))
                        .padding(6),
                ]
                .spacing(10)
                .align_y(iced::Alignment::Center),
            );
        }

        rules_col =
            rules_col.push(button(text("+ ルールを追加").size(13)).on_press(Message::AddRule));

        let rules_box = container(rules_col.spacing(10))
            .padding(14)
            .style(|theme: &Theme| {
                container::Style::default()
                    .background(theme.palette().background)
                    .border(iced::Border::default().rounded(8).width(1).color(BORDER))
            });

        // --- 保存ボタン ---
        let save_btn = button(text("保存").size(15))
            .on_press(Message::Save)
            .padding([8, 28])
            .style(|theme: &Theme, status| button::primary(theme, status));

        column![
            trigger_row,
            language_row,
            provider_row,
            model_row,
            test_row,
            provider_note,
            clipboard_row,
            paste_mode_row,
            rules_box,
            save_btn,
        ]
        .spacing(16)
        .into()
    }

    fn view_prompt(&self) -> Element<'_, Message> {
        column![
            text("文字起こしのプロンプト").size(16),
            text(format!("保存先: {}", self.base_dir.display())).size(12),
            text("選択は自動保存され、起動時と次回の録音に使用されます。本文は編集後に保存してください。").size(12),
            pick_list(self.prompt_files.as_slice(), self.selected_prompt.as_ref(), Message::PromptSelected)
                .placeholder("プロンプトを選択").width(Length::Fill),
            row![
                text_input("llm-en-lecture.md", &self.new_prompt_name)
                    .on_input(Message::NewPromptNameChanged).width(Length::Fill),
                button("新規作成").on_press(Message::CreatePrompt),
            ].spacing(10),
            text("ファイル名では認識言語は変わりません。「設定」の言語を指定してください。").size(12),
            text_editor(&self.prompt_editor).on_action(Message::PromptEdited)
                .height(360).placeholder("文字起こしの指示や専門用語を入力してください"),
            button("プロンプトを保存").on_press(Message::SavePrompt).padding([8, 28]),
        ].spacing(14).into()
    }
}

fn labeled_row<'a>(
    label: &'a str,
    widget: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    row![text(label).size(14), widget.into()]
        .spacing(14)
        .align_y(iced::Alignment::Center)
        .into()
}

fn valid_prompt_name(name: &str) -> bool {
    name.starts_with("llm-")
        && name.ends_with(".md")
        && name.len() > 7
        && !name
            .chars()
            .any(|c| c.is_control() || "<>:\\/\"|?*".contains(c))
}

fn ensure_builtin_prompts(base: &std::path::Path) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(base)?;
    for (name, body) in [
        ("llm-jp-normal.md", mameo::DEFAULT_LLM_PROMPT),
        ("llm-en-normal.md", mameo::DEFAULT_ENGLISH_LLM_PROMPT),
    ] {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(base.join(name))
        {
            Ok(mut file) => file.write_all(body.as_bytes())?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_preserve_files_and_provider_settings() {
        let base = std::env::temp_dir().join(format!(
            "mameo-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let mut cfg = Config::default();
        cfg.transcription_provider = "soniox".into();
        cfg.soniox_model = "custom-model".into();
        cfg.debug_transcription = true;
        cfg.save(&base.join("config.toml")).unwrap();
        std::fs::write(base.join("llm-jp-normal.md"), "custom Japanese prompt").unwrap();
        let mut state = State::load(base.clone(), None);
        assert_eq!(
            std::fs::read_to_string(base.join("llm-jp-normal.md")).unwrap(),
            "custom Japanese prompt"
        );
        assert_eq!(
            std::fs::read_to_string(base.join("llm-en-normal.md")).unwrap(),
            mameo::DEFAULT_ENGLISH_LLM_PROMPT
        );
        let _ = state.update(Message::PromptSelected("llm-en-normal.md".into()));
        state.save();
        let saved = Config::load_or_default(&base);
        assert_eq!(saved.llm_prompt_file, "llm-en-normal.md");
        assert_eq!(saved.transcription_provider, "soniox");
        assert_eq!(saved.soniox_model, "custom-model");
        assert!(saved.debug_transcription);
        state.prompt_editor = text_editor::Content::with_text("edited English prompt");
        state.prompt_dirty = true;
        let _ = state.update(Message::PromptSelected("llm-jp-normal.md".into()));
        assert_eq!(state.selected_prompt.as_deref(), Some("llm-en-normal.md"));
        let _ = state.update(Message::SavePrompt);
        assert_eq!(
            std::fs::read_to_string(base.join("llm-en-normal.md")).unwrap(),
            "edited English prompt"
        );
        std::fs::write(base.join("llm-external.md"), "external").unwrap();
        let _ = state.update(Message::RefreshPrompts);
        assert!(state
            .prompt_files
            .iter()
            .any(|name| name == "llm-external.md"));
        std::fs::remove_file(base.join("llm-external.md")).unwrap();
        let _ = state.update(Message::RefreshPrompts);
        assert!(!state
            .prompt_files
            .iter()
            .any(|name| name == "llm-external.md"));
        assert_eq!(state.prompt_editor.text().trim(), "edited English prompt");
        state.new_prompt_name = "llm-en-lecture.md".into();
        let _ = state.update(Message::CreatePrompt);
        assert_eq!(
            Config::load_or_default(&base).llm_prompt_file,
            "llm-en-lecture.md"
        );
        state.new_prompt_name = "llm-en-lecture.md".into();
        let _ = state.update(Message::CreatePrompt);
        assert!(state.status.contains("エラー"));
        assert!(!valid_prompt_name("llm-../escape.md"));
        assert!(!valid_prompt_name("llm-en:stream.md"));
        std::fs::remove_dir_all(base).unwrap();
    }
}

fn list_prompts(base: &std::path::Path, _selected: &str) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(base)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| valid_prompt_name(name))
        .collect();
    files.sort();
    files
}

/// GUIを開いている間、外部から追加・削除したプロンプトも一覧に反映する。
fn refresh_events() -> impl iced::futures::Stream<Item = Message> {
    let (mut sender, receiver) = iced::futures::channel::mpsc::channel(1);
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        if sender.is_closed() {
            break;
        }
        let _ = sender.try_send(Message::RefreshPrompts);
    });
    receiver
}
