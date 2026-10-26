#![windows_subsystem = "windows"]

use arboard::Clipboard;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use mameo::{
    get_base_dir, is_noise_transcription, load_transcription_prompt, trim_digital_silence, Config,
};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use rdev::{listen, Event, EventType, Key as RKey};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use tray_icon::{Icon, TrayIconBuilder};

struct LiveRecording {
    sender: SyncSender<Vec<f32>>,
    control: mameo::transcription::soniox::StreamControl,
}

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

/// ファイルの mtime を取得（存在しない場合は None）。
fn file_mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// GUI (mameo-config.exe) を起動する。tab は設定タブ指定。
/// exe が無い場合はフォールバック (config.toml を既定エディタで開く)。
///
/// GUI は自己 exe (mameo.exe) と同じディレクトリを優先して探す。
fn launch_gui_or_editor(base_dir: &Path, tab: Option<&str>, fallback: &Path) {
    let mut gui_dirs: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            gui_dirs.push(dir.to_path_buf());
        }
    }
    gui_dirs.push(base_dir.to_path_buf());

    for dir in &gui_dirs {
        let gui_exe = dir.join("mameo-config.exe");
        if gui_exe.exists() {
            let mut cmd = std::process::Command::new(&gui_exe);
            if let Some(t) = tab {
                cmd.arg(t);
            }
            let _ = cmd.spawn();
            return;
        }
    }
    open_file(fallback);
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

    let base_dir = get_base_dir()?;
    let config = Arc::new(RwLock::new(load_or_create_config(&base_dir)));
    // 初回起動時に指示ファイルを用意する。読み取り不可なら既定の本文を使う。
    let _ = load_transcription_prompt(&base_dir, &config.read().unwrap().llm_prompt_file);

    // --- ファイル監視ポーラ（config.toml の外部変更を自動反映） ---
    // GUI からの保存を Reload 無しで受け取る。mtime 変化検出のみの軽量実装。
    {
        let config_watch = Arc::clone(&config);
        let base = base_dir.clone();
        thread::spawn(move || {
            let mut last_cfg_m = file_mtime(&base.join("config.toml"));
            loop {
                thread::sleep(std::time::Duration::from_millis(1500));
                let cfg_m = file_mtime(&base.join("config.toml"));
                if cfg_m != last_cfg_m {
                    last_cfg_m = cfg_m;
                    let c = Config::load_or_default(&base);
                    if let Ok(mut w) = config_watch.try_write() {
                        *w = c;
                    }
                }
            }
        });
    }

    // --- タスクトレイメニューの構築 ---
    let tray_menu = Menu::new();
    let item_open_config = MenuItem::new("設定を開く", true, None);
    let item_open_folder = MenuItem::new("設定フォルダを開く", true, None);
    let item_reload = MenuItem::new("設定の再読み込み", true, None);
    let item_exit = MenuItem::new("終了 (Exit)", true, None);

    let _ = tray_menu.append(&item_open_config);
    let _ = tray_menu.append(&item_open_folder);
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
    let live_recording: Arc<Mutex<Option<LiveRecording>>> = Arc::new(Mutex::new(None));

    let audio_buf_clone = Arc::clone(&audio_buffer);
    let is_rec_clone = Arc::clone(&is_recording);
    let live_audio = Arc::clone(&live_recording);

    let stream = device.build_input_stream(
        &audio_cfg.into(),
        move |data: &[f32], _: &_| {
            let live = live_audio.lock().unwrap();
            if is_rec_clone.load(Ordering::SeqCst) {
                if let Some(recording) = live.as_ref() {
                    let mono = data
                        .chunks(channels as usize)
                        .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
                        .collect();
                    if let Err(TrySendError::Full(_)) = recording.sender.try_send(mono) {
                        recording.control.overflowed.store(true, Ordering::Relaxed);
                    }
                    return;
                }
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
    let prompt_base_dir = base_dir.clone();
    let live_key = Arc::clone(&live_recording);

    thread::spawn(move || {
        let _ = listen(move |event: Event| {
            let target_key = {
                let cfg = config_key.read().unwrap();
                let k = match cfg.trigger_key.to_lowercase().as_str() {
                    "rightcontrol" | "rcontrol" => RKey::ControlRight,
                    "rightshift" | "rshift" => RKey::ShiftRight,
                    _ => RKey::AltGr,
                };
                k
            };

            if event.event_type == EventType::KeyPress(target_key) {
                if !is_rec_key.load(Ordering::SeqCst) {
                    let request_config = config_key.read().unwrap().clone();
                    let mut live = live_key.lock().unwrap();
                    audio_buf_key.lock().unwrap().clear();
                    if request_config
                        .transcription_provider
                        .trim()
                        .eq_ignore_ascii_case("soniox")
                    {
                        // 上限付きチャネル。音声コールバックでは try_send のみ。
                        let (sender, receiver) = sync_channel(512);
                        let control = mameo::transcription::soniox::StreamControl::default();
                        *live = Some(LiveRecording {
                            sender,
                            control: control.clone(),
                        });
                        let paste_config = Arc::clone(&config_key);
                        let base = prompt_base_dir.clone();
                        thread::spawn(move || {
                            let prompt =
                                load_transcription_prompt(&base, &request_config.llm_prompt_file);
                            match mameo::transcription::soniox::transcribe_stream(
                                &request_config,
                                sample_rate,
                                receiver,
                                control,
                                &prompt,
                            ) {
                                Ok(result) => {
                                    eprintln!("Soniox finalization: {}ms", result.finalize_ms);
                                    let timing = serde_json::json!({
                                        "connection_ms": result.connection_ms,
                                        "release_to_finalize_request_ms": result.release_to_finalize_request_ms,
                                        "finalize_ms": result.finalize_ms,
                                        "release_to_result_ms": result.release_to_result_ms,
                                    });
                                    let diagnostic = if request_config.debug_transcription
                                        && !result.diagnostic_samples.is_empty()
                                    {
                                        samples_to_wav(&result.diagnostic_samples, sample_rate)
                                            .ok()
                                            .and_then(|wav| save_diagnostic_audio(&base, &wav))
                                    } else {
                                        None
                                    };
                                    deliver_transcription(
                                        result.text,
                                        &request_config,
                                        &paste_config,
                                        diagnostic.as_deref(),
                                        Some(&timing),
                                    );
                                }
                                Err(error) => eprintln!("Streaming transcription failed: {error}"),
                            }
                        });
                    }
                    is_rec_key.store(true, Ordering::SeqCst);
                }
            } else if event.event_type == EventType::KeyRelease(target_key)
                && is_rec_key.load(Ordering::SeqCst)
            {
                {
                    let mut live = live_key.lock().unwrap();
                    is_rec_key.store(false, Ordering::SeqCst);
                    // Senderを破棄して確定を通知。次の録音とは別のチャネルを使う。
                    if let Some(recording) = live.take() {
                        *recording.control.released_at.lock().unwrap() =
                            Some(std::time::Instant::now());
                        drop(recording);
                        return;
                    }
                }

                let samples = audio_buf_key.lock().unwrap().clone();
                if samples.len() < (sample_rate as usize / 5) {
                    return; // 0.2秒未満はスキップ
                }

                let config_clone = Arc::clone(&config_key);
                let prompt_base = prompt_base_dir.clone();

                thread::spawn(move || {
                    let samples = trim_digital_silence(&samples);
                    if samples.is_empty() {
                        return; // 完全な無音は API に送らない。
                    }
                    if let Ok(wav_bytes) = samples_to_wav(samples, sample_rate) {
                        let request_config = config_clone.read().unwrap().clone();
                        let prompt = load_transcription_prompt(
                            &prompt_base,
                            &request_config.llm_prompt_file,
                        );
                        let diagnostic_path = if request_config.debug_transcription {
                            save_diagnostic_audio(&prompt_base, &wav_bytes)
                        } else {
                            None
                        };

                        match mameo::transcription::transcribe(&request_config, wav_bytes, &prompt)
                        {
                            Ok(text) => deliver_transcription(
                                text,
                                &request_config,
                                &config_clone,
                                diagnostic_path.as_deref(),
                                None,
                            ),
                            Err(error) => eprintln!("Transcription failed: {error}"),
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
                    launch_gui_or_editor(&base_dir, None, &base_dir.join("config.toml"));
                } else if event.id == item_open_folder.id() {
                    open_file(&base_dir);
                } else if event.id == item_reload.id() {
                    let mut c = config.write().unwrap();
                    *c = load_or_create_config(&base_dir);
                } else if event.id == item_exit.id() {
                    std::process::exit(0);
                }
            }
        }
    }

    Ok(())
}

/// 明示的に有効化した場合だけ保存。API キーやプロンプトは記録しない。
fn save_diagnostic_audio(base: &Path, wav: &[u8]) -> Option<PathBuf> {
    let result = (|| -> std::io::Result<PathBuf> {
        let dir = base.join("diagnostics");
        std::fs::create_dir_all(&dir)?;
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(std::io::Error::other)?
            .as_nanos();
        let path = dir.join(format!("{}-{id}.wav", std::process::id()));
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?
            .write_all(wav)?;
        Ok(path)
    })();
    match result {
        Ok(path) => Some(path),
        Err(error) => {
            eprintln!("Cannot save diagnostic audio: {error}");
            None
        }
    }
}

fn save_diagnostic_text(
    path: &Path,
    raw: &str,
    config: &Config,
    discarded: bool,
    timing: Option<&serde_json::Value>,
) {
    let data = serde_json::json!({
        "raw_text": raw,
        "language": config.language,
        "provider": config.transcription_provider,
        "model": match config.transcription_provider.trim().to_ascii_lowercase().as_str() {
            "gemini" => &config.gemini_model,
            "soniox" => &config.soniox_model,
            _ => &config.groq_model,
        },
        "discarded": discarded,
        "timing": timing,
    });
    let result = serde_json::to_vec_pretty(&data)
        .map_err(std::io::Error::other)
        .and_then(|bytes| std::fs::write(path.with_extension("json"), bytes));
    if let Err(error) = result {
        eprintln!("Cannot save diagnostic text: {error}");
    }
}

fn deliver_transcription(
    text: String,
    request_config: &Config,
    paste_config: &RwLock<Config>,
    diagnostic: Option<&Path>,
    timing: Option<&serde_json::Value>,
) {
    if is_noise_transcription(&text) {
        if let Some(path) = diagnostic {
            save_diagnostic_text(path, &text, request_config, true, timing);
        }
        return;
    }
    if let Some(path) = diagnostic {
        save_diagnostic_text(path, &text, request_config, false, timing);
    }
    if !text.trim().is_empty() {
        let config = paste_config.read().unwrap().clone();
        dispatch_paste(text.trim(), &config);
    }
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
    samples: &[f32],
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
