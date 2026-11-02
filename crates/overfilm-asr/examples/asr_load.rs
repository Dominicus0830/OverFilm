//! Time loading Qwen3-ASR on the CPU, and with a WAV (16 kHz mono 16-bit) the encoder and the
//! prompt pass of two 60-second pieces: `asr_load <asr dir> [wav]` (OVERFILM_ASR_TRACE=1 for detail).

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let t0 = std::time::Instant::now();
    let mut asr = overfilm_asr::Asr::load(a[1].as_ref(), true).unwrap();
    eprintln!("loaded in {:.1}s (gpu {})", t0.elapsed().as_secs_f32(), asr.on_gpu());
    if let Some(w) = a.get(2) {
        let b = std::fs::read(w).unwrap();
        let data = b.windows(4).position(|w| w == b"data").unwrap() + 8;
        let audio: Vec<f32> = b[data..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect();
        let piece = &audio[..audio.len().min(16000 * 60)];
        // stop at the first decoding step (after the two encodes)
        let mut n = 0;
        let _ = asr.transcribe_many(&[piece, piece], "", Some("Korean"), &mut || {
            n += 1;
            n > 2
        });
        let c = asr.clock();
        eprintln!("encoder {:.1}s, prefill {:.1}s", c.encode, c.prefill);
    } else {
        // let a first load finish writing the 8-bit cache
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
}
