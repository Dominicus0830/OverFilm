//! Try the assistants on a recording without the editor:
//! `ffmpeg -i video.mp4 -f f32le -ac 1 -ar 16000 - | cargo run -p overfilm-ai --release --example assist_raw`
//! prints the suggested silences and highlights (audio only, no transcript).

use std::io::Read;

fn clock(ms: i64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn main() -> std::io::Result<()> {
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes)?;
    let samples: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    let levels = overfilm_ai::levels(&samples, 16_000);
    let quiet = overfilm_ai::silences(&levels, &[], &overfilm_ai::SilenceParams::default());
    let total: i64 = quiet.iter().map(|s| s.len()).sum();
    println!("{} silences, {} in all", quiet.len(), clock(total));
    for s in quiet.iter().take(20) {
        println!("  quiet {} – {}", clock(s.start_ms), clock(s.end_ms));
    }
    for h in overfilm_ai::highlights(&levels, &[], &overfilm_ai::HighlightParams { count: 15, ..Default::default() }) {
        println!("highlight {} – {}  peak {}  score {:.1}  +{:.1} dB", clock(h.span.start_ms), clock(h.span.end_ms), clock(h.peak_ms), h.score, h.louder_db);
    }
    Ok(())
}
