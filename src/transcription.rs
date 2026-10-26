//! 音声認識の送信先を切り替える境界。録音・貼り付けには依存しない。
use crate::{Config, GROQ_API_URL};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::blocking::{multipart, Client, Response};
use serde_json::{json, Value};
use std::time::Duration;
pub mod soniox;

pub fn transcribe(config: &Config, wav: Vec<u8>, prompt: &str) -> Result<String, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    match config
        .transcription_provider
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "groq" => groq(&client, config, wav, prompt),
        "gemini" => gemini(&client, config, &wav, prompt),
        "soniox" => soniox::transcribe(config, &wav, prompt),
        _ => Err("transcription_provider must be groq, gemini or soniox".into()),
    }
}

fn env_key(name: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| format!("Environment variable {name} is missing"))
}

fn response_json(response: Response) -> Result<Value, String> {
    let status = response.status();
    let value: Value = response
        .json()
        .map_err(|_| format!("API returned {status} with an invalid JSON response"))?;
    if !status.is_success() {
        let message = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("request failed");
        return Err(format!("API returned {status}: {message}"));
    }
    Ok(value)
}

fn groq(client: &Client, config: &Config, wav: Vec<u8>, prompt: &str) -> Result<String, String> {
    let key = if config.groq_api_key.trim().is_empty() {
        env_key("GROQ_API_KEY")?
    } else {
        config.groq_api_key.clone()
    };
    if config.groq_model.trim().is_empty() {
        return Err("groq_model is empty".into());
    }
    let part = multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
    let mut form = multipart::Form::new()
        .part("file", part)
        .text("model", config.groq_model.clone())
        .text("response_format", "json");
    if !config.language.is_empty() && config.language != "auto" {
        form = form.text("language", config.language.clone());
    }
    if !prompt.is_empty() {
        form = form.text("prompt", prompt.to_string());
    }
    let response = client
        .post(GROQ_API_URL)
        .bearer_auth(key)
        .multipart(form)
        .send()
        .map_err(|e| e.to_string())?;
    response_json(response)?["text"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "Groq response has no transcription text".into())
}

fn gemini_request(config: &Config, wav: &[u8], prompt: &str) -> Result<Value, String> {
    if config.gemini_model.trim().is_empty() {
        return Err("gemini_model is empty".into());
    }
    // base64 は元の音声の約4/3倍。巨大な録音の追加割り当てを避ける。
    if wav.len() > 14_000_000 {
        return Err("Gemini inline audio is too large; use a shorter recording".into());
    }
    let audio = json!({"type": "audio", "mime_type": "audio/wav", "data": STANDARD.encode(wav)});
    if config.gemini_model == "gemini-3.5-transcribe" {
        let language_codes = match config.language.as_str() {
            "" | "auto" => vec![],
            "ja" => vec!["ja-JP"],
            "en" => vec!["en-US"],
            language => vec![language],
        };
        // 専用モデルは自由文プロンプトを受け付けないため言語ヒントを使う。
        Ok(json!({
            "model": config.gemini_model,
            "input": [audio],
            "store": false,
            "generation_config": {"transcription_config": {
                "language_codes": language_codes,
                "mode": {"type": "verbatim"}
            }}
        }))
    } else {
        let mut instruction = String::from(
            "Transcribe only the speech in this audio. Return only the transcript, without commentary, markdown or timestamps. Preserve the spoken languages and the actual number of repetitions. Do not invent words or answer questions in the audio. If there is no speech, return an empty string.\n"
        );
        if !config.language.is_empty() && config.language != "auto" {
            instruction.push_str(&format!(
                "Primary language hint: {}. Preserve other languages when spoken.\n",
                config.language
            ));
        }
        if !prompt.is_empty() {
            instruction.push_str("Transcription context and vocabulary:\n");
            instruction.push_str(prompt);
        }
        Ok(json!({"model": config.gemini_model, "store": false,
            "input": [{"type": "text", "text": instruction}, audio]}))
    }
}

fn gemini(client: &Client, config: &Config, wav: &[u8], prompt: &str) -> Result<String, String> {
    let key = env_key("GEMINI_API_KEY")?;
    let body =
        serde_json::to_vec(&gemini_request(config, wav, prompt)?).map_err(|e| e.to_string())?;
    if body.len() >= 20_000_000 {
        return Err(
            "Gemini request exceeds the inline audio limit; use a shorter recording or prompt"
                .into(),
        );
    }
    let response = client
        .post("https://generativelanguage.googleapis.com/v1beta/interactions")
        .header("x-goog-api-key", key)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .map_err(|e| e.to_string())?;
    gemini_text(&response_json(response)?)
}

fn gemini_text(value: &Value) -> Result<String, String> {
    if value["status"] != "completed" {
        return Err("Gemini interaction did not complete successfully".into());
    }
    let mut text = String::new();
    let mut found = false;
    if let Some(steps) = value["steps"].as_array() {
        for step in steps.iter().filter(|step| step["type"] == "model_output") {
            if let Some(content) = step["content"].as_array() {
                for part in content.iter().filter(|part| part["type"] == "text") {
                    if let Some(fragment) = part["text"].as_str() {
                        found = true;
                        text.push_str(fragment);
                    }
                }
            }
        }
    }
    if found {
        Ok(text)
    } else {
        Err("Gemini response has no transcription text".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_requests_preserve_prompt_or_use_transcription_hints() {
        let mut config = Config::default();
        let request = gemini_request(&config, b"wav", "日本語と英語が混在").unwrap();
        assert!(request["input"][0]["text"]
            .as_str()
            .unwrap()
            .contains("日本語と英語が混在"));
        assert_eq!(request["input"][1]["data"], "d2F2");
        config.gemini_model = "gemini-3.5-transcribe".into();
        config.language = "auto".into();
        let request = gemini_request(&config, b"wav", "ignored").unwrap();
        assert_eq!(request["input"].as_array().unwrap().len(), 1);
        assert_eq!(
            request["generation_config"]["transcription_config"]["language_codes"],
            json!([])
        );
        assert!(request["generation_config"]["transcription_config"]
            .get("custom_vocabulary")
            .is_none());
    }

    #[test]
    fn gemini_response_excludes_thoughts_and_rejects_failed_results() {
        let value = json!({"status": "completed", "steps": [
            {"type": "model_output", "content": [
                {"type": "thought", "text": "do not paste"},
                {"type": "text", "text": "よしよし、"},
                {"type": "text", "text": "carbon pricing。"}
            ]}
        ]});
        assert_eq!(gemini_text(&value).unwrap(), "よしよし、carbon pricing。");
        assert!(gemini_text(&json!({"status": "failed"})).is_err());
        assert!(gemini_text(&json!({"status": "completed", "steps": []})).is_err());
    }
}
