//! mameo core library
//!
//! core daemon (bin) と設定 GUI (mameo-config) で共有する型・ロジック。
//! Config 構造体の単一真相源（single source of truth）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
pub mod transcription;

pub const GROQ_API_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
pub const DEFAULT_LLM_PROMPT: &str = include_str!("../llm-jp-normal.md");
pub const DEFAULT_ENGLISH_LLM_PROMPT: &str = include_str!("../llm-en-normal.md");

/// デジタル無音（値が厳密に 0）の前後だけを除去する。
/// 小声を切らないように音量しきい値は使わない。全て無音なら空を返す。
pub fn trim_digital_silence(samples: &[f32]) -> &[f32] {
    match samples.iter().position(|&sample| sample != 0.0) {
        Some(start) => {
            let end = samples.iter().rposition(|&sample| sample != 0.0).unwrap() + 1;
            &samples[start..end]
        }
        None => &samples[..0],
    }
}

/// 既存の定型ノイズを、末尾の句読点・空白の有無にかかわらず判定する。
pub fn is_noise_transcription(text: &str) -> bool {
    let text = text.trim().trim_end_matches(|c: char| {
        c.is_whitespace() || matches!(c, '。' | '、' | '.' | ',' | '!' | '！' | '?' | '？')
    });
    matches!(
        text,
        "ご視聴ありがとうございました" | "ありがとうございました" | "チャンネル登録お願いします"
    )
}

fn default_llm_prompt_file() -> String {
    "llm-jp-normal.md".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct AppRule {
    pub process_name: String,
    pub paste_mode: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub trigger_key: String,
    pub language: String,
    #[serde(default = "default_llm_prompt_file")]
    pub llm_prompt_file: String,
    pub groq_api_key: String,
    pub transcription_provider: String,
    pub groq_model: String,
    pub gemini_model: String,
    pub soniox_model: String,
    pub restore_clipboard: bool,
    /// 原因調査用。録音と認識結果を設定フォルダの diagnostics に保存する。
    pub debug_transcription: bool,
    pub default_paste_mode: String,
    #[serde(default)]
    pub app_rules: Vec<AppRule>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            trigger_key: "RightAlt".to_string(),
            language: "ja".to_string(),
            llm_prompt_file: default_llm_prompt_file(),
            groq_api_key: String::new(),
            transcription_provider: "groq".to_string(),
            groq_model: "whisper-large-v3-turbo".to_string(),
            gemini_model: "gemini-3.8-flash".to_string(),
            soniox_model: "stt-rt-v5".to_string(),
            restore_clipboard: true,
            debug_transcription: false,
            default_paste_mode: "auto".to_string(),
            app_rules: vec![AppRule {
                process_name: "emacs.exe".to_string(),
                paste_mode: "copy_only".to_string(),
            }],
        }
    }
}

/// 指示本文を送信プロンプトとして読み込む。空のファイル名は指示を無効化する。
/// ファイル I/O があるため録音後のワーカースレッドから呼ぶ。
pub fn load_transcription_prompt(base_dir: &Path, filename: &str) -> String {
    let filename = filename.trim();
    let instructions = if filename.is_empty() {
        String::new()
    } else {
        let path = base_dir.join(filename);
        match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                if error.kind() == std::io::ErrorKind::NotFound {
                    // create_new で既存ファイルを上書きしない。
                    use std::io::Write;
                    if let Ok(mut file) = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                    {
                        let _ = file.write_all(DEFAULT_LLM_PROMPT.as_bytes());
                    }
                }
                eprintln!(
                    "Prompt file {}: {error}; using default instructions",
                    path.display()
                );
                DEFAULT_LLM_PROMPT.to_string()
            }
        }
    };

    instructions.trim().to_string()
}

impl Config {
    /// config.toml を読み込む。無い場合だけ生成し、既存ファイルは変更しない。
    pub fn load_or_create(base_dir: &Path) -> Self {
        let path = base_dir.join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str::<Config>(&content) {
                Ok(cfg) => return cfg,
                Err(error) => {
                    eprintln!(
                        "Cannot parse {}: {error}; keeping existing file",
                        path.display()
                    );
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let default_cfg = Config::default();
                if let Ok(toml_str) = toml::to_string_pretty(&default_cfg) {
                    use std::io::Write;
                    // 読み込み後に別の処理が作成した場合も上書きしない。
                    match std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                    {
                        Ok(mut file) => {
                            if let Err(error) = file.write_all(toml_str.as_bytes()) {
                                eprintln!("Cannot write {}: {error}", path.display());
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                            return Self::load_or_default(base_dir);
                        }
                        Err(error) => eprintln!("Cannot create {}: {error}", path.display()),
                    }
                }
                return default_cfg;
            }
            Err(error) => eprintln!(
                "Cannot read {}: {error}; keeping existing file",
                path.display()
            ),
        }
        Config::default()
    }

    /// config.toml を読み込む。無くてもファイルは生成しない（GUI 初回表示用）。
    pub fn load_or_default(base_dir: &Path) -> Self {
        let path = base_dir.join("config.toml");
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = toml::from_str::<Config>(&content) {
                return cfg;
            }
        }
        Self::default()
    }

    /// config.toml を保存する。
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let toml_str = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, toml_str).map_err(|e| e.to_string())
    }
}

/// 設定・指示ファイルの共通保存先。exe 隣接のフラグで判定する。
pub fn get_base_dir() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| std::io::Error::other("Executable has no parent directory"))?;
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    resolve_base_dir(exe_dir, appdata.as_deref())
}

fn resolve_base_dir(exe_dir: &Path, appdata: Option<&Path>) -> std::io::Result<PathBuf> {
    if exe_dir.join("portable.flag").is_file() {
        return Ok(exe_dir.to_path_buf());
    }
    let appdata = appdata
        .filter(|path| path.is_absolute())
        .ok_or_else(|| std::io::Error::other("APPDATA must be an absolute directory"))?;
    let base = appdata.join("mameo");
    std::fs::create_dir_all(&base)?;
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_trimming_preserves_quiet_speech_and_internal_silence() {
        assert!(trim_digital_silence(&[]).is_empty());
        assert!(trim_digital_silence(&[0.0, -0.0, 0.0]).is_empty());
        assert_eq!(
            trim_digital_silence(&[0.0, 0.000001, 0.0, -0.000001, 0.0]),
            &[0.000001, 0.0, -0.000001]
        );
    }

    #[test]
    fn noise_filter_handles_punctuation_without_removing_longer_sentences() {
        for text in [
            "ご視聴ありがとうございました。",
            " ありがとうございました！ \n",
            "チャンネル登録お願いします.",
        ] {
            assert!(is_noise_transcription(text));
        }
        assert!(!is_noise_transcription("今日はありがとうございました。"));
        assert!(!is_noise_transcription(
            "よしよしよしよし、こんな感じかな。"
        ));
    }

    #[test]
    fn portable_flag_alone_selects_the_settings_directory() {
        let base = std::env::temp_dir().join(format!(
            "mameo-directory-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let exe_dir = base.join("bin");
        let appdata = base.join("Roaming");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::write(exe_dir.join("config.toml"), "language = \"en\"\n").unwrap();
        let installed_dir = resolve_base_dir(&exe_dir, Some(&appdata)).unwrap();
        assert_eq!(installed_dir, appdata.join("mameo"));
        assert!(installed_dir.is_dir());
        std::fs::write(installed_dir.join("config.toml"), "language = \"ja\"\n").unwrap();
        std::fs::write(exe_dir.join("portable.flag"), "").unwrap();
        let portable_dir = resolve_base_dir(&exe_dir, Some(&appdata)).unwrap();
        assert_eq!(portable_dir, exe_dir);
        assert_eq!(Config::load_or_create(&portable_dir).language, "en");
        assert_eq!(Config::load_or_create(&installed_dir).language, "ja");
        assert_eq!(resolve_base_dir(&exe_dir, None).unwrap(), exe_dir);
        std::fs::remove_file(exe_dir.join("portable.flag")).unwrap();
        assert_eq!(
            resolve_base_dir(&exe_dir, Some(&appdata)).unwrap(),
            installed_dir
        );
        assert!(resolve_base_dir(&exe_dir, None).is_err());
        assert!(resolve_base_dir(&exe_dir, Some(Path::new("relative"))).is_err());
        std::fs::remove_file(exe_dir.join("config.toml")).unwrap();
        std::fs::remove_file(installed_dir.join("config.toml")).unwrap();
        std::fs::remove_dir(exe_dir).unwrap();
        std::fs::remove_dir(installed_dir).unwrap();
        std::fs::remove_dir(appdata).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[test]
    fn config_is_only_created_when_missing_and_never_reset_on_reload() {
        let base = std::env::temp_dir().join(format!(
            "mameo-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&base).unwrap();
        let path = base.join("config.toml");
        Config::load_or_create(&base);
        assert!(toml::from_str::<Config>(&std::fs::read_to_string(&path).unwrap()).is_ok());

        let custom = "# keep my settings\nlanguage = \"en\"\ngroq_api_key = \"test-key\"\n";
        std::fs::write(&path, custom).unwrap();
        for _ in 0..2 {
            let config = Config::load_or_create(&base);
            assert_eq!(config.language, "en");
            assert_eq!(config.groq_api_key, "test-key");
            assert_eq!(config.trigger_key, "RightAlt");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), custom);
        }
        for invalid in ["language = [", "restore_clipboard = \"invalid\""] {
            std::fs::write(&path, invalid).unwrap();
            for _ in 0..2 {
                Config::load_or_create(&base);
                assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
            }
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        Config::load_or_create(&base);
        assert!(path.is_dir());
        std::fs::remove_dir(&path).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[test]
    fn old_config_defaults_to_japanese_prompt() {
        let mut value = toml::Value::try_from(Config::default()).unwrap();
        value.as_table_mut().unwrap().remove("llm_prompt_file");
        let config: Config = value.try_into().unwrap();
        assert_eq!(config.llm_prompt_file, "llm-jp-normal.md");
    }

    #[test]
    fn prompt_file_is_created_and_edits_are_read() {
        let base = std::env::temp_dir().join(format!(
            "mameo-prompt-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&base).unwrap();
        let file = "llm-jp-lecture.md";
        let initial = load_transcription_prompt(&base, file);
        assert!(initial.starts_with(DEFAULT_LLM_PROMPT.trim()));
        assert_eq!(
            std::fs::read_to_string(base.join(file)).unwrap(),
            DEFAULT_LLM_PROMPT
        );
        std::fs::write(base.join(file), "大学で環境経済学を教えています。\n").unwrap();
        assert_eq!(
            load_transcription_prompt(&base, file),
            "大学で環境経済学を教えています。"
        );
        assert_eq!(load_transcription_prompt(&base, ""), "");
        std::fs::write(base.join(file), "").unwrap();
        assert_eq!(load_transcription_prompt(&base, file), "");
        std::fs::remove_file(base.join(file)).unwrap();
        std::fs::remove_dir(base).unwrap();
    }
}
