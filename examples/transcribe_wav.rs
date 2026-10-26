//! 保存した録音で送信先を比較する。設定保存・貼り付けはしない。
use mameo::{load_transcription_prompt, Config};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        return Err(
            "Usage: transcribe_wav <groq|gemini|soniox|soniox-stream> <model> <wav> <settings-directory>".into(),
        );
    }
    let base = std::path::Path::new(&args[4]);
    let mut config = Config::load_or_default(base);
    config.transcription_provider = if args[1] == "soniox-stream" {
        "soniox".into()
    } else {
        args[1].clone()
    };
    config.groq_model = args[2].clone();
    config.gemini_model = args[2].clone();
    config.soniox_model = args[2].clone();
    let prompt = load_transcription_prompt(base, &config.llm_prompt_file);
    let start = std::time::Instant::now();
    let text = if args[1] == "soniox-stream" {
        use std::sync::Arc;
        let mut wav = hound::WavReader::open(&args[3])?;
        let spec = wav.spec();
        if spec.channels != 1
            || spec.bits_per_sample != 16
            || spec.sample_format != hound::SampleFormat::Int
        {
            return Err("Streaming comparison expects a mono 16-bit PCM WAV".into());
        }
        let samples: Vec<f32> = wav
            .samples::<i16>()
            .map(|sample| sample.map(|sample| sample as f32 / i16::MAX as f32))
            .collect::<Result<_, _>>()?;
        let (sender, receiver) = std::sync::mpsc::sync_channel(512);
        let control = mameo::transcription::soniox::StreamControl::default();
        let released = Arc::clone(&control.released_at);
        let release_marker = Arc::clone(&released);
        let producer = std::thread::spawn(move || {
            let began = std::time::Instant::now();
            let chunk_len = (spec.sample_rate as usize / 50).max(1);
            for (index, chunk) in samples.chunks(chunk_len).enumerate() {
                if sender.send(chunk.to_vec()).is_err() {
                    return;
                }
                let due = began + std::time::Duration::from_millis((index as u64 + 1) * 20);
                if let Some(delay) = due.checked_duration_since(std::time::Instant::now()) {
                    std::thread::sleep(delay);
                }
            }
            *release_marker.lock().unwrap() = Some(std::time::Instant::now());
            drop(sender);
        });
        let result = mameo::transcription::soniox::transcribe_stream(
            &config,
            spec.sample_rate,
            receiver,
            control,
            &prompt,
        );
        producer.join().map_err(|_| "Audio simulation failed")?;
        let result = result?;
        if let Some(at) = *released.lock().unwrap() {
            eprintln!(
                "After simulated key release: {:.2}s",
                at.elapsed().as_secs_f64()
            );
        }
        eprintln!("Finalization: {}ms", result.finalize_ms);
        eprintln!(
            "Connection: {}ms; release to finalize request: {:?}ms",
            result.connection_ms, result.release_to_finalize_request_ms
        );
        result.text
    } else {
        mameo::transcription::transcribe(&config, std::fs::read(&args[3])?, &prompt)?
    };
    println!("{text}");
    eprintln!("Elapsed: {:.2}s", start.elapsed().as_secs_f64());
    Ok(())
}
