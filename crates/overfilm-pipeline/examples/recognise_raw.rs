//! Recognise a sound file with or without a match prior (evaluation tool): prints each chunk's
//! time, pure recognition and corrected text as JSON lines.
//!
//! ```sh
//! ffmpeg -i vod.mp4 -ss 1700 -t 600 -ac 1 -ar 16000 -f f32le clip.f32
//! cargo run -p overfilm-pipeline --features qwen --release --example recognise_raw -- clip.f32 [prior.json]
//! ```

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let Ok(bytes) = std::fs::read(a.get(1).cloned().unwrap_or_default()) else { return eprintln!("sound file") };
    let audio: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let prior = a.get(2).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok());
    let mut r = overfilm_pipeline::recognise::Request::new(overfilm_pipeline::pack::Mode::Commentary);
    r.speakers = false;
    r.gpu = false;
    r.prior = prior;
    let models = overfilm_pipeline::models_dir();
    let t0 = std::time::Instant::now();
    if let Err(e) = overfilm_pipeline::recognise::ensure_models(&models, &mut |_, m| {
        eprintln!("{m}");
        true
    }) {
        return eprintln!("{e}");
    }
    let rec = match overfilm_pipeline::recognise::recognise(&audio, &r, &models, &mut |f, m| {
        eprintln!("[{:>6.0}s] {:>5.1}% {m}", t0.elapsed().as_secs_f32(), f * 100.0);
        true
    }) {
        Ok(x) => x,
        Err(e) => return eprintln!("{e}"),
    };
    for (c, raw) in rec.transcript.chunks.iter().zip(&rec.raw_texts) {
        println!("{}", serde_json::json!({"start": c.start_ms, "end": c.end_ms, "raw": raw, "text": c.text}));
    }
}
