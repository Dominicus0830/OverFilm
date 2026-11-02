//! Time loading Qwen3-ASR on the CPU: `asr_load <asr dir>` (set OVERFILM_ASR_TRACE=1 for detail).

fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let t0 = std::time::Instant::now();
    let a = overfilm_asr::Asr::load(dir.as_ref(), true).unwrap();
    eprintln!("loaded in {:.1}s (gpu {})", t0.elapsed().as_secs_f32(), a.on_gpu());
    // let a first load finish writing the 8-bit cache
    std::thread::sleep(std::time::Duration::from_secs(30));
}
