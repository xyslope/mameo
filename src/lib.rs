//! mameo core library
//!
//! core daemon (bin) と設定 GUI (mameo-config) で共有する型・ロジック。
//! Config 構造体の単一真相源（single source of truth）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const GROQ_API_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct AppRule {
    pub process_name: String,
    pub paste_mode: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub trigger_key: String,
    pub language: String,
    pub groq_api_key: String,
    pub restore_clipboard: bool,
    pub default_paste_mode: String,
    #[serde(default)]
    pub app_rules: Vec<AppRule>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            trigger_key: "RightAlt".to_string(),
            language: "ja".to_string(),
            groq_api_key: String::new(),
            restore_clipboard: true,
            default_paste_mode: "auto".to_string(),
            app_rules: vec![AppRule {
                process_name: "emacs.exe".to_string(),
                paste_mode: "copy_only".to_string(),
            }],
        }
    }
}

impl Config {
    /// config.toml を読み込む。無ければデフォルト値でファイルを生成して返す。
    pub fn load_or_create(base_dir: &Path) -> Self {
        let path = base_dir.join("config.toml");
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(cfg) = toml::from_str::<Config>(&content) {
                    return cfg;
                }
            }
        }
        let default_cfg = Config::default();
        if let Ok(toml_str) = toml::to_string_pretty(&default_cfg) {
            let _ = std::fs::write(&path, toml_str);
        }
        default_cfg
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

/// DICT.csv を行順保持のリストで読み込む（読み方 -> 表記）。
///
/// 旧 `load_dictionary`（HashMap・順序不定）の後継。プロンプト誘導の語順が
/// ファイル順に固定され、ハッシュマップの非決定性を排す。
/// 同じ読みが複数行ある場合は最後の値を採用（HashMap 動作と一致）。
pub fn load_dictionary_rows(base_dir: &Path) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = Vec::new();

    let path = base_dir.join("DICT.csv");
    if !path.exists() {
        let template = "スリデブ,Slidev\nクオート,Quarto\nイーマックス,emacs\nエルパカ,Elpaca\n";
        let _ = std::fs::write(&path, template);
    }

    if let Ok(content) = std::fs::read_to_string(&path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.splitn(2, ',').map(|s| s.trim()).collect();
            if parts.len() == 2 && !parts[0].is_empty() {
                if let Some(existing) = rows.iter_mut().find(|(r, _)| r == parts[0]) {
                    existing.1 = parts[1].to_string(); // 後勝ちで値更新・行位置は維持
                } else {
                    rows.push((parts[0].to_string(), parts[1].to_string()));
                }
            }
        }
    }
    rows
}

/// 辞書行リストを DICT.csv に保存する。
pub fn save_dictionary_rows(base_dir: &Path, rows: &[(String, String)]) -> Result<(), String> {
    let path = base_dir.join("DICT.csv");
    let mut out = String::new();
    for (reading, word) in rows {
        let reading = reading.trim();
        let word = word.trim();
        if reading.is_empty() {
            continue; // 読みが空の行は保存しない
        }
        // 値にカンマや改行が入る場合は CSV 的に壊れるのでクォートする
        let needs_quote =
            |s: &str| s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r');
        let quote = |s: &str| {
            if needs_quote(s) {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.to_string()
            }
        };
        out.push_str(&format!("{},{}\n", quote(reading), quote(word)));
    }
    std::fs::write(&path, out).map_err(|e| e.to_string())
}

/// config.toml / DICT.csv の探索基準ディレクトリ。
///
/// 1. exe と同じディレクトリに config.toml がある → ポータブルモード
/// 2. %APPDATA%\mameo (Roaming)。Microsoft 標準のユーザー単位設定場所。
///    (Store/MSIX 版は exe 隣接に書き込めないため、インストール版はこちら)
pub fn get_base_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let dir = dir.to_path_buf();
            if dir.join("config.toml").exists() {
                return dir;
            }
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        let dir = PathBuf::from(appdata).join("mameo");
        if std::fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }
    PathBuf::from(".")
}
