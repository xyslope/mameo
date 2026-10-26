//! 録音中のPCM送信・手動確定と、録音済みWAVの比較用バックエンド。
use super::env_key;
use crate::Config;
use serde_json::{json, Value};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{
    mpsc::{Receiver, RecvTimeoutError},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::Message;

const ENDPOINT: &str = "wss://stt-rt.soniox.com/transcribe-websocket";

type Socket = tungstenite::WebSocket<MaybeTlsStream<TcpStream>>;

pub struct StreamResult {
    pub text: String,
    pub diagnostic_samples: Vec<f32>,
    pub finalize_ms: u128,
    pub connection_ms: u128,
    pub release_to_finalize_request_ms: Option<u128>,
    pub release_to_result_ms: Option<u128>,
}

#[derive(Clone, Default)]
pub struct StreamControl {
    pub overflowed: Arc<AtomicBool>,
    pub released_at: Arc<Mutex<Option<Instant>>>,
}

/// キー押下時のワーカーから開始する。入力チャネルの切断がキー解放を表す。
pub fn transcribe_stream(
    config: &Config,
    sample_rate: u32,
    input: Receiver<Vec<f32>>,
    control: StreamControl,
    prompt: &str,
) -> Result<StreamResult, String> {
    let started = Instant::now();
    let socket = connect(config)?;
    let connection_ms = started.elapsed().as_millis();
    let mut result = run_stream(socket, config, sample_rate, input, control, prompt)?;
    result.connection_ms = connection_ms;
    Ok(result)
}

fn run_stream(
    mut socket: Socket,
    config: &Config,
    sample_rate: u32,
    input: Receiver<Vec<f32>>,
    control: StreamControl,
    prompt: &str,
) -> Result<StreamResult, String> {
    let mut start = start_request(config, prompt);
    start["audio_format"] = json!("pcm_s16le");
    start["sample_rate"] = json!(sample_rate);
    start["num_channels"] = json!(1);
    socket
        .send(Message::Text(start.to_string().into()))
        .map_err(|e| e.to_string())?;
    let mut text = String::new();
    let mut samples = Vec::new();
    let mut sample_count = 0;
    let mut has_audio = false;
    let mut released = None;
    let mut release_to_finalize_request_ms = None;
    loop {
        if control.overflowed.load(Ordering::Relaxed) {
            return Err(
                "Audio queue overflowed; recording cancelled to avoid missing words".into(),
            );
        }
        if released.is_none() {
            match input.recv_timeout(Duration::from_millis(10)) {
                Ok(mut chunk) => {
                    // 接続中や受信待ちで溜まった音声をまとめて送る。
                    // 小さいチャンクごとに read の待ち時間を支払わない。
                    for next in input.try_iter().take(31) {
                        chunk.extend(next);
                    }
                    sample_count += chunk.len();
                    has_audio |= chunk.iter().any(|&sample| sample != 0.0);
                    if config.debug_transcription {
                        samples.extend_from_slice(&chunk);
                    }
                    socket
                        .send(Message::Binary(pcm_bytes(&chunk).into()))
                        .map_err(|e| e.to_string())?;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    // 200ms の無音を音声として送り、発話の末尾を確定する。
                    socket
                        .send(Message::Binary(
                            vec![0; sample_rate as usize / 5 * 2].into(),
                        ))
                        .map_err(|e| e.to_string())?;
                    socket
                        .send(Message::Text(
                            json!({"type": "finalize"}).to_string().into(),
                        ))
                        .map_err(|e| e.to_string())?;
                    released = Some(Instant::now());
                    release_to_finalize_request_ms = control
                        .released_at
                        .lock()
                        .unwrap()
                        .map(|at| at.elapsed().as_millis());
                }
            }
        }
        let timeout = if let Some(at) = released {
            Duration::from_secs(60)
                .checked_sub(at.elapsed())
                .filter(|time| !time.is_zero())
                .ok_or("Soniox finalization timed out")?
                .min(Duration::from_millis(10))
        } else {
            Duration::from_millis(1)
        };
        set_read_timeout(&socket, timeout)?;
        // 暫定結果を録音中にも受信し、サーバーの送信バッファを詰まらせない。
        match socket.read() {
            Ok(Message::Text(message)) => {
                let value: Value = serde_json::from_str(&message).map_err(|e| e.to_string())?;
                let finished = append_final_tokens(&value, &mut text)?;
                let finalized = value["tokens"].as_array().is_some_and(|tokens| {
                    tokens
                        .iter()
                        .any(|token| token["is_final"] == true && token["text"] == "<fin>")
                });
                if let Some(at) = released {
                    if finalized || finished {
                        let finalize_ms = at.elapsed().as_millis();
                        let _ = socket.close(None);
                        return Ok(StreamResult {
                            text: if has_audio && sample_count >= sample_rate as usize / 5 {
                                text
                            } else {
                                String::new()
                            },
                            diagnostic_samples: samples,
                            finalize_ms,
                            connection_ms: 0,
                            release_to_finalize_request_ms,
                            release_to_result_ms: control
                                .released_at
                                .lock()
                                .unwrap()
                                .map(|at| at.elapsed().as_millis()),
                        });
                    }
                } else if finished {
                    return Err("Soniox finished before recording stopped".into());
                }
            }
            Ok(Message::Close(_)) => return Err("Soniox closed before finalization".into()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn pcm_bytes(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| ((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes())
        .collect()
}

fn set_read_timeout(socket: &Socket, timeout: Duration) -> Result<(), String> {
    let tcp = match socket.get_ref() {
        MaybeTlsStream::Plain(stream) => stream,
        MaybeTlsStream::NativeTls(stream) => stream.get_ref(),
        _ => return Err("Unsupported Soniox TLS transport".into()),
    };
    tcp.set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())
}

pub(super) fn transcribe(config: &Config, wav: &[u8], prompt: &str) -> Result<String, String> {
    let mut socket = connect(config)?;
    socket
        .send(Message::Text(
            start_request(config, prompt).to_string().into(),
        ))
        .map_err(|e| e.to_string())?;
    for chunk in wav.chunks(32 * 1024) {
        socket
            .send(Message::Binary(chunk.to_vec().into()))
            .map_err(|e| e.to_string())?;
    }
    socket
        .send(Message::Text("".into()))
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut result = String::new();
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| "Soniox transcription timed out".to_string())?;
        set_read_timeout(&socket, remaining.min(Duration::from_secs(20)))?;
        match socket.read().map_err(|e| e.to_string())? {
            Message::Text(message) => {
                let value: Value = serde_json::from_str(&message).map_err(|e| e.to_string())?;
                if append_final_tokens(&value, &mut result)? {
                    let _ = socket.close(None);
                    return Ok(result);
                }
            }
            Message::Close(_) => {
                return Err("Soniox closed before returning a finished transcript".into())
            }
            _ => {}
        }
    }
}

fn connect(config: &Config) -> Result<Socket, String> {
    let key = env_key("SONIOX_API_KEY")?;
    if config.soniox_model.trim().is_empty() {
        return Err("soniox_model is empty".into());
    }
    let mut request = ENDPOINT.into_client_request().map_err(|e| e.to_string())?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {key}")
            .parse()
            .map_err(|_| "Invalid SONIOX_API_KEY".to_string())?,
    );
    // 接続・TLSハンドシェイクはワーカー上で行う。
    let addresses = ("stt-rt.soniox.com", 443)
        .to_socket_addrs()
        .map_err(|e| e.to_string())?;
    let mut connection = None;
    for address in addresses {
        if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(10)) {
            connection = Some(stream);
            break;
        }
    }
    let tcp = connection.ok_or_else(|| "Cannot connect to Soniox".to_string())?;
    tcp.set_nodelay(true).map_err(|e| e.to_string())?;
    tcp.set_read_timeout(Some(Duration::from_secs(20)))
        .map_err(|e| e.to_string())?;
    tcp.set_write_timeout(Some(Duration::from_secs(20)))
        .map_err(|e| e.to_string())?;
    let (socket, _) = tungstenite::client_tls(request, tcp).map_err(|e| e.to_string())?;
    Ok(socket)
}

fn start_request(config: &Config, prompt: &str) -> Value {
    let hints: Vec<&str> = match config.language.trim() {
        "" | "auto" => vec![],
        language => vec![language],
    };
    json!({"model": config.soniox_model, "audio_format": "auto",
        "language_hints": hints, "language_hints_strict": false,
        "enable_endpoint_detection": false,
        "context": {"text": prompt}})
}

/// 暫定トークンは毎回更新されるため連結しない。確定分だけを蓄積する。
fn append_final_tokens(value: &Value, text: &mut String) -> Result<bool, String> {
    if value.get("error_code").is_some() || value.get("error_type").is_some() {
        return Err(format!(
            "Soniox: {}",
            value["error_message"].as_str().unwrap_or("request failed")
        ));
    }
    if let Some(tokens) = value["tokens"].as_array() {
        for token in tokens.iter().filter(|token| token["is_final"] == true) {
            if let Some(fragment) = token["text"].as_str() {
                if !matches!(fragment, "<end>" | "<fin>")
                    && token["translation_status"] != "translation"
                {
                    text.push_str(fragment);
                }
            }
        }
    }
    Ok(value["finished"] == true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_audio_is_sent_before_release_and_finalized_after_release() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sent, seen) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut socket = tungstenite::accept(tcp).unwrap();
            let start = socket.read().unwrap().into_text().unwrap();
            let start: Value = serde_json::from_str(&start).unwrap();
            assert_eq!(start["audio_format"], "pcm_s16le");
            assert_eq!(start["sample_rate"], 1000);
            assert_eq!(socket.read().unwrap().into_data().len(), 512);
            socket
                .send(Message::Text(
                    json!({"tokens": [
                        {"text": "誤った暫定結果", "is_final": false},
                        {"text": "よしよし", "is_final": true}
                    ]})
                    .to_string()
                    .into(),
                ))
                .unwrap();
            sent.send(()).unwrap(); // ここまで、入力Senderは生きている。
            assert_eq!(socket.read().unwrap().into_data(), vec![0u8; 400]);
            let finalize: Value =
                serde_json::from_str(&socket.read().unwrap().into_text().unwrap()).unwrap();
            assert_eq!(finalize["type"], "finalize");
            socket
                .send(Message::Text(
                    json!({"tokens": [
                        {"text": "、こんな感じ。", "is_final": true},
                        {"text": "<fin>", "is_final": true}
                    ]})
                    .to_string()
                    .into(),
                ))
                .unwrap();
        });
        let (sender, receiver) = std::sync::mpsc::sync_channel(64);
        // 接続確立中に短いコールバックが溜まった場合を模擬する。
        for _ in 0..32 {
            sender.send(vec![0.01; 8]).unwrap();
        }
        let control = StreamControl::default();
        let release_marker = Arc::clone(&control.released_at);
        let producer = std::thread::spawn(move || {
            seen.recv_timeout(Duration::from_secs(5)).unwrap();
            *release_marker.lock().unwrap() = Some(Instant::now());
            drop(sender); // 録音停止を模擬。
        });
        let tcp = TcpStream::connect(address).unwrap();
        let (socket, _) =
            tungstenite::client(format!("ws://{address}"), MaybeTlsStream::Plain(tcp)).unwrap();
        let config = Config {
            debug_transcription: true,
            ..Config::default()
        };
        let result = run_stream(socket, &config, 1000, receiver, control, "").unwrap();
        assert_eq!(result.text, "よしよし、こんな感じ。");
        assert_eq!(result.diagnostic_samples, vec![0.01; 256]);
        assert!(result.release_to_finalize_request_ms.is_some());
        assert!(result.release_to_result_ms.unwrap() >= result.finalize_ms);
        producer.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn pcm_conversion_clips_samples_and_uses_little_endian() {
        assert_eq!(
            pcm_bytes(&[-2.0, 0.0, 2.0]),
            [
                (-32767i16).to_le_bytes(),
                0i16.to_le_bytes(),
                32767i16.to_le_bytes()
            ]
            .concat()
        );
    }

    #[test]
    fn provisional_tokens_do_not_duplicate_final_transcription() {
        let mut text = String::new();
        assert!(!append_final_tokens(
            &json!({"tokens": [
            {"text": "よしよしよしよし", "is_final": false}]}),
            &mut text
        )
        .unwrap());
        assert!(!append_final_tokens(
            &json!({"tokens": [
            {"text": "よしよし", "is_final": true},
            {"text": "、こんな", "is_final": false}]}),
            &mut text
        )
        .unwrap());
        assert!(append_final_tokens(
            &json!({"finished": true, "tokens": [
            {"text": "、こんな感じかな。", "is_final": true},
            {"text": "<fin>", "is_final": true}]}),
            &mut text
        )
        .unwrap());
        assert_eq!(text, "よしよし、こんな感じかな。");
        assert!(append_final_tokens(
            &json!({"error_code": 401, "error_message": "Invalid key"}),
            &mut text
        )
        .is_err());
    }

    #[test]
    fn configuration_passes_context_without_restricting_mixed_languages() {
        let config = Config {
            language: "auto".into(),
            ..Config::default()
        };
        let value = start_request(&config, "環境経済学の講義");
        assert_eq!(value["language_hints"], json!([]));
        assert_eq!(value["language_hints_strict"], false);
        assert_eq!(value["context"]["text"], "環境経済学の講義");
        assert!(value["context"].get("terms").is_none());
        assert!(value.get("api_key").is_none());
    }
}
