//! Transcribe a 16 kHz mono f32 WAV with Qwen3-ASR (+ aligner): `qwen_asr <asr dir> <aligner dir> <wav> [context file] [q8] [gpu]`.
//! Prints the transcript JSON.

fn read_wav(p: &str) -> Vec<f32> {
    let b = std::fs::read(p).unwrap();
    // minimal reader for 16-bit PCM mono WAV
    let data = b.windows(4).position(|w| w == b"data").unwrap() + 8;
    b[data..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let audio = read_wav(&a[3]);
    let ctx = a.get(4).filter(|s| !s.is_empty()).map(|p| std::fs::read_to_string(p).unwrap()).unwrap_or_default();
    let quantize = a.get(5).is_some_and(|s| s == "q8");
    let s = overfilm_asr::Settings {
        asr_dir: a[1].as_ref(),
        aligner_dir: a[2].as_ref(),
        quantize,
        context: ctx.trim(),
        language: Some("Korean"),
        correct: &|t: &str, _, _| t.to_string(),
        gpu: a.iter().any(|s| s == "gpu"),
        skip_silence: a.iter().any(|s| s == "skip-silence"),
        live: None,
    };
    let t0 = std::time::Instant::now();
    let t = overfilm_asr::transcribe(&audio, &s, &mut |f, m| {
        eprintln!("[{:>6.1}s] {:>5.1}% {m}", t0.elapsed().as_secs_f32(), f * 100.0);
        true
    })
    .unwrap();
    eprintln!("device {}", t.device);
    println!("{}", serde_json::to_string_pretty(&t).unwrap());
}
