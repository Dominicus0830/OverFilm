//! Transcribe many 16 kHz mono PCM16 WAVs with one loaded Qwen3-ASR (q8, CPU):
//! `asr_many <asr dir> <list file: one wav path per line>`. Prints `path\ttext` per line.
fn read_wav(p: &str) -> Vec<f32> {
    let b = std::fs::read(p).unwrap();
    let data = b.windows(4).position(|w| w == b"data").unwrap() + 8;
    b[data..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect()
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut asr = overfilm_asr::Asr::load(a[1].as_ref(), true).unwrap();
    for p in std::fs::read_to_string(&a[2]).unwrap().lines().filter(|l| !l.is_empty()) {
        let t = asr.transcribe(&read_wav(p), "", Some("Korean"), &mut || false).unwrap();
        println!("{p}\t{}", t.replace(['\n', '\t'], " "));
    }
}
