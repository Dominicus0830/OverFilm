//! Recognise a 16 kHz mono 16-bit WAV with the editor's pipeline and write the transcript, the
//! pure recognition of each chunk and the corrections as JSON (evaluation input for the Claude
//! Code caption review, `overfilm-review`'s `review_eval` example).
//!
//! `cargo run --release -p overfilm-pipeline --features qwen --example recognise_dump -- in.wav out.json [general|commentary]`

fn read_wav(path: &str) -> Result<Vec<f32>, String> {
    let b = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    // find the "data" chunk of a plain PCM16 WAV
    let mut i = 12usize;
    while i + 8 <= b.len() {
        let id = b.get(i..i + 4).unwrap_or_default();
        let n = b.get(i + 4..i + 8).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize).unwrap_or(0);
        if id == b"data" {
            let data = b.get(i + 8..(i + 8 + n).min(b.len())).unwrap_or_default();
            return Ok(data.chunks_exact(2).map(|s| f32::from(i16::from_le_bytes([s[0], s[1]])) / 32768.0).collect());
        }
        i += 8 + n + (n & 1);
    }
    Err("no data chunk".into())
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let (Some(input), Some(output)) = (args.get(1), args.get(2)) else { return Err("usage: in.wav out.json [general|commentary]".into()) };
    let mode = args.get(3).and_then(|m| overfilm_pipeline::pack::Mode::parse(m)).unwrap_or(overfilm_pipeline::pack::Mode::General);
    let audio = read_wav(input)?;
    let models = overfilm_pipeline::models_dir();
    overfilm_pipeline::recognise::ensure_models(&models, &mut |f, m| {
        eprintln!("{:.0}% {m}", f * 100.0);
        true
    })?;
    let mut req = overfilm_pipeline::recognise::Request::new(mode);
    req.gpu = false;
    let t0 = std::time::Instant::now();
    let rec = overfilm_pipeline::recognise::recognise(&audio, &req, &models, &mut |f, m| {
        eprintln!("{:.0}% {m}", f * 100.0);
        true
    })?;
    let v = serde_json::json!({"transcript": rec.transcript, "raw": rec.raw_texts, "corrections": rec.corrections, "seconds": t0.elapsed().as_secs()});
    std::fs::write(output, serde_json::to_string(&v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    eprintln!("done in {}s", t0.elapsed().as_secs());
    Ok(())
}
