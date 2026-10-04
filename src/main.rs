#![windows_subsystem = "windows"]

use arboard::Clipboard;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use mameo::{get_base_dir, load_dictionary, Config, GROQ_API_URL};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use rdev::{listen, Event, EventType, Key as RKey};
use reqwest::blocking::multipart;
use serde_json::Value;
use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use tray_icon::{Icon, TrayIconBuilder};

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::HWND;
#[cfg(windows)]
use windows::Win32::System::Threading::{
    CreateMutexW, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, TranslateMessage,
    MSG,
};

fn load_or_create_config(base_dir: &Path) -> Config {
    Config::load_or_create(base_dir)
}

#[cfg(windows)]
fn get_active_process_name() -> Option<String> {
    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        if hwnd.is_invalid() || hwnd.0.is_null() {
            return None;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }

        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;

        let res = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = windows::Win32::Foundation::CloseHandle(handle);

        if res.is_ok() && size > 0 {
            let full_path = String::from_utf16_lossy(&buf[..size as usize]);
            Path::new(&full_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
fn get_active_process_name() -> Option<String> {
    None
}

// 16x16 のミニマムな赤丸アイコンをインメモリ生成
fn create_default_icon() -> Icon {
    let width = 16;
    let height = 16;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 - 7.5;
            let dy = y as f32 - 7.5;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq <= 36.0 {
                // 赤色ドット（マイク録音・常駐のシンボル）
                rgba.extend_from_slice(&[220, 50, 50, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Icon::from_rgba(rgba, width, height).expect("Failed to create tray icon")
}

// 関連付けられた既定エディタで開く
fn open_file(path: &Path) {
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", "", &path.to_string_lossy()])
        .spawn();
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Windows 多重起動防止ガード
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Foundation::GetLastError;
        let mutex_name: Vec<u16> = "Global\\mameo_single_instance_mutex\0"
            .encode_utf16()
            .collect();
        let _ = CreateMutexW(None, true, PCWSTR(mutex_name.as_ptr()));
        if GetLastError().0 == 183 {
            // ERROR_ALREADY_EXISTS
            return Ok(());
        }
    }

    let base_dir = get_base_dir();
    let config = Arc::new(RwLock::new(load_or_create_config(&base_dir)));
    let dict = Arc::new(RwLock::new(load_dictionary(&base_dir)));

    // --- タスクトレイメニューの構築 ---
    let tray_menu = Menu::new();
    let item_open_config = MenuItem::new("設定を開く", true, None);
    let item_open_dict = MenuItem::new("辞書を開く (DICT.csv)", true, None);
    let item_reload = MenuItem::new("設定・辞書の再読み込み", true, None);
    let item_exit = MenuItem::new("終了 (Exit)", true, None);

    let _ = tray_menu.append(&item_open_config);
    let _ = tray_menu.append(&item_open_dict);
    let _ = tray_menu.append(&item_reload);
    let _ = tray_menu.append(&PredefinedMenuItem::separator());
    let _ = tray_menu.append(&item_exit);

    let _tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("mameo - 音声入力")
        .with_icon(create_default_icon())
        .build()?;

    // --- 音声入力ストリーム初期化 ---
    let host = cpal::default_host();
    let device = host.default_input_device().expect("No input device found");
    let audio_cfg = device
        .default_input_config()
        .expect("Failed to get audio config");
    let sample_rate = audio_cfg.sample_rate().0;
    let channels = audio_cfg.channels();

    let is_recording = Arc::new(AtomicBool::new(false));
    let audio_buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));

    let audio_buf_clone = Arc::clone(&audio_buffer);
    let is_rec_clone = Arc::clone(&is_recording);

    let stream = device.build_input_stream(
        &audio_cfg.into(),
        move |data: &[f32], _: &_| {
            if is_rec_clone.load(Ordering::SeqCst) {
                let mut buf = audio_buf_clone.lock().unwrap();
                for chunk in data.chunks(channels as usize) {
                    let mono = chunk.iter().sum::<f32>() / channels as f32;
                    buf.push(mono);
                }
            }
        },
        |err| eprintln!("Audio stream error: {err}"),
        None,
    )?;
    stream.play()?;

    // --- キーボード監視スレッド ---
    let is_rec_key = Arc::clone(&is_recording);
    let audio_buf_key = Arc::clone(&audio_buffer);
    let config_key = Arc::clone(&config);
    let dict_key = Arc::clone(&dict);

    thread::spawn(move || {
        let _ = listen(move |event: Event| {
            let (target_key, lang, api_key_cfg) = {
                let cfg = config_key.read().unwrap();
                let k = match cfg.trigger_key.to_lowercase().as_str() {
                    "rightcontrol" | "rcontrol" => RKey::ControlRight,
                    "rightshift" | "rshift" => RKey::ShiftRight,
                    _ => RKey::AltGr,
                };
                (k, cfg.language.clone(), cfg.groq_api_key.clone())
            };

            let api_key = if !api_key_cfg.is_empty() {
                api_key_cfg
            } else {
                std::env::var("GROQ_API_KEY").unwrap_or_default()
            };

            if event.event_type == EventType::KeyPress(target_key) {
                if !is_rec_key.load(Ordering::SeqCst) {
                    audio_buf_key.lock().unwrap().clear();
                    is_rec_key.store(true, Ordering::SeqCst);
                }
            } else if event.event_type == EventType::KeyRelease(target_key)
                && is_rec_key.load(Ordering::SeqCst)
            {
                is_rec_key.store(false, Ordering::SeqCst);

                let samples = audio_buf_key.lock().unwrap().clone();
                if samples.len() < (sample_rate as usize / 5) {
                    return; // 0.2秒未満はスキップ
                }

                let config_clone = Arc::clone(&config_key);
                let dict_clone = Arc::clone(&dict_key);

                thread::spawn(move || {
                    if let Ok(wav_bytes) = samples_to_wav(samples, sample_rate) {
                        let (prompt, local_dict) = {
                            let d = dict_clone.read().unwrap();
                            let p = d.values().cloned().collect::<Vec<_>>().join(", ");
                            (p, d.clone())
                        };

                        if let Ok(mut text) = call_groq_whisper(&api_key, wav_bytes, &lang, &prompt)
                        {
                            let trimmed = text.trim();
                            if trimmed == "ご視聴ありがとうございました"
                                || trimmed == "ありがとうございました"
                                || trimmed == "チャンネル登録お願いします"
                            {
                                return;
                            }

                            for (wrong, correct) in local_dict.iter() {
                                text = text.replace(wrong, correct);
                            }

                            let clean_text = text.trim();
                            if !clean_text.is_empty() {
                                let cfg = config_clone.read().unwrap();
                                dispatch_paste(clean_text, &cfg);
                            }
                        }
                    }
                });
            }
        });
    });

    // --- メインスレッド（Windows メッセージループ & トレイメニュー処理） ---
    let menu_channel = MenuEvent::receiver();

    #[cfg(windows)]
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);

            if let Ok(event) = menu_channel.try_recv() {
                if event.id == item_open_config.id() {
                    // GUI (mameo-config.exe) が core と同じディレクトリにあれば起動、
                    // 無ければ config.toml を既定エディタで開く（フォールバック）。
                    let gui_exe = base_dir.join("mameo-config.exe");
                    if gui_exe.exists() {
                        let _ = std::process::Command::new(&gui_exe).spawn();
                    } else {
                        open_file(&base_dir.join("config.toml"));
                    }
                } else if event.id == item_open_dict.id() {
                    open_file(&base_dir.join("DICT.csv"));
                } else if event.id == item_reload.id() {
                    let mut c = config.write().unwrap();
                    *c = load_or_create_config(&base_dir);
                    let mut d = dict.write().unwrap();
                    *d = load_dictionary(&base_dir);
                } else if event.id == item_exit.id() {
                    std::process::exit(0);
                }
            }
        }
    }

    Ok(())
}

fn dispatch_paste(text: &str, config: &Config) {
    let active_proc = get_active_process_name().unwrap_or_default().to_lowercase();
    let mut paste_mode = config.default_paste_mode.as_str();

    for rule in &config.app_rules {
        if rule.process_name.to_lowercase() == active_proc {
            paste_mode = &rule.paste_mode;
            break;
        }
    }

    let mut clipboard = match Clipboard::new() {
        Ok(c) => c,
        Err(_) => return,
    };
    let prev_clip = clipboard.get_text().unwrap_or_default();
    let _ = clipboard.set_text(text);

    match paste_mode {
        "copy_only" => {
            // Emacs などの場合はクリップボードに格納したまま保持
        }
        "ctrl_y" => {
            if let Ok(mut enigo) = Enigo::new(&Settings::default()) {
                let _ = enigo.key(Key::Control, Direction::Press);
                let _ = enigo.key(Key::Unicode('y'), Direction::Click);
                let _ = enigo.key(Key::Control, Direction::Release);
            }
            if config.restore_clipboard {
                thread::sleep(std::time::Duration::from_millis(200));
                let _ = clipboard.set_text(prev_clip);
            }
        }
        _ => {
            if let Ok(mut enigo) = Enigo::new(&Settings::default()) {
                let _ = enigo.key(Key::Control, Direction::Press);
                let _ = enigo.key(Key::Unicode('v'), Direction::Click);
                let _ = enigo.key(Key::Control, Direction::Release);
            }
            if config.restore_clipboard {
                thread::sleep(std::time::Duration::from_millis(200));
                let _ = clipboard.set_text(prev_clip);
            }
        }
    }
}

fn samples_to_wav(
    samples: Vec<f32>,
    sample_rate: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut cursor = Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
    for sample in samples {
        let val = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer.write_sample(val)?;
    }
    writer.finalize()?;
    Ok(cursor.into_inner())
}

fn call_groq_whisper(
    api_key: &str,
    wav_data: Vec<u8>,
    lang: &str,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::blocking::Client::new();
    let part = multipart::Part::bytes(wav_data)
        .file_name("audio.wav")
        .mime_str("audio/wav")?;

    let mut form = multipart::Form::new()
        .part("file", part)
        .text("model", "whisper-large-v3-turbo")
        .text("response_format", "json");

    if !lang.is_empty() && lang != "auto" {
        form = form.text("language", lang.to_string());
    }
    if !prompt.is_empty() {
        form = form.text("prompt", prompt.to_string());
    }

    let res = client
        .post(GROQ_API_URL)
        .header("Authorization", format!("Bearer {api_key}"))
        .multipart(form)
        .send()?
        .json::<Value>()?;

    let text = res["text"].as_str().unwrap_or("").to_string();
    Ok(text)
}
