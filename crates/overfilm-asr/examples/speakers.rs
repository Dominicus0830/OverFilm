//! Speaker labelling from the command line, for checking the port and tuning.
//!
//! ```sh
//! cargo run --release -p overfilm-asr --features qwen --example speakers -- fbank audio.f32 > feats.f32
//! cargo run --release -p overfilm-asr --features qwen --example speakers -- embed MODEL_DIR audio.f32
//! cargo run --release -p overfilm-asr --features qwen --example speakers -- diarize MODEL_DIR audio.f32 words.json [threshold…] [--speakers N] [--window MS] [--step MS] [--overlap SEG_MODEL.onnx]
//! ```
//! `audio.f32` is raw 16 kHz mono little-endian f32 (`ffmpeg -i x -ac 1 -ar 16000 -f f32le`);
//! `words.json` is a list of `{"text","start_ms","end_ms"}`. `diarize` prints one JSON line per
//! threshold: `{"threshold", "speakers", "labels": [per word]}`.

use std::io::Write;
use std::path::Path;

use overfilm_asr::Word;
use overfilm_asr::speakers::{self, Diarizer, Params, fbank};

fn read_f32(p: &str) -> Vec<f32> {
    let b = std::fs::read(p).expect("audio");
    b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("fbank") => {
            let audio = read_f32(&a[1]);
            let f = fbank::Fbank::new().compute(&audio);
            let mut out = std::io::stdout().lock();
            for v in f {
                out.write_all(&v.to_le_bytes()).unwrap();
            }
        }
        Some("embed") => {
            let mut d = Diarizer::load(Path::new(&a[1])).expect("model");
            if std::env::var("SPK_GPU").is_ok() {
                eprintln!("{}", d.use_gpu());
            }
            let audio = read_f32(&a[2]);
            let t = std::time::Instant::now();
            let v = d.embed(&audio).expect("embed").unwrap_or_default();
            eprintln!("{:.3} s", t.elapsed().as_secs_f32());
            println!("{}", serde_json::to_string(&v).unwrap());
        }
        Some("diarize") => {
            let mut d = Diarizer::load(Path::new(&a[1])).expect("model");
            if std::env::var("SPK_GPU").is_ok() {
                eprintln!("{}", d.use_gpu());
            }
            let audio = read_f32(&a[2]);
            let words: Vec<Word> = serde_json::from_str(&std::fs::read_to_string(&a[3]).unwrap()).unwrap();
            let mut p = Params::default();
            let mut thresholds = Vec::new();
            let mut dump = None;
            let mut load: Option<String> = None;
            let mut seg_net = None;
            let mut op = speakers::overlap::OverlapParams::default();
            let mut i = 4;
            while i < a.len() {
                match a[i].as_str() {
                    "--speakers" => {
                        p.speakers = a[i + 1].parse().ok();
                        i += 1;
                    }
                    "--window" => {
                        p.window_ms = a[i + 1].parse().unwrap();
                        i += 1;
                    }
                    "--step" => {
                        p.step_ms = a[i + 1].parse().unwrap();
                        i += 1;
                    }
                    "--known" => {
                        let (name, file) = a[i + 1].split_once('=').unwrap();
                        let v = d.profile(&read_f32(file)).unwrap().expect("speech in sample");
                        p.known.push(speakers::KnownVoice { name: name.into(), vector: v });
                        i += 1;
                    }
                    "--load" => {
                        load = Some(a[i + 1].clone());
                        i += 1;
                    }
                    "--switch" => {
                        p.switch_cost = a[i + 1].parse().unwrap();
                        i += 1;
                    }
                    "--overlap" => {
                        seg_net = Some(speakers::seg::SegNet::load(Path::new(&a[i + 1])).expect("overlap model"));
                        i += 1;
                    }
                    "--ov-share" => {
                        op.word_share = a[i + 1].parse().unwrap();
                        i += 1;
                    }
                    "--ov-sim" => {
                        op.min_similarity = a[i + 1].parse().unwrap();
                        i += 1;
                    }
                    "--ov-step" => {
                        op.step_ms = a[i + 1].parse().unwrap();
                        i += 1;
                    }
                    "--dump" => {
                        dump = Some(a[i + 1].clone());
                        i += 1;
                    }
                    "--pause" => {
                        p.pause_ms = a[i + 1].parse().unwrap();
                        i += 1;
                    }
                    "--min-seconds" => {
                        p.min_speaker_seconds = a[i + 1].parse().unwrap();
                        i += 1;
                    }
                    t => thresholds.push(t.parse::<f32>().unwrap()),
                }
                i += 1;
            }
            if thresholds.is_empty() {
                thresholds.push(p.threshold);
            }
            let t0 = std::time::Instant::now();
            let (wins, vecs): (Vec<(i64, i64)>, Vec<Option<Vec<f32>>>) = if let Some(l) = &load {
                let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(l).unwrap()).unwrap();
                (serde_json::from_value(v["windows"].clone()).unwrap(), serde_json::from_value(v["vectors"].clone()).unwrap())
            } else {
                let wins = speakers::windows(&speakers::speech_regions(&words, p.pause_ms), p.window_ms, p.step_ms);
                let vecs = speakers::window_vectors(&audio, &wins, &mut |b| d.frame_map(b), &|m, x, y| d.pool(m, x, y), &mut |_, _| true).unwrap();
                (wins, vecs)
            };
            eprintln!("{} windows, {:.1} s", wins.len(), t0.elapsed().as_secs_f32());
            if let Some(d) = dump {
                std::fs::write(d, serde_json::json!({"windows": wins, "vectors": vecs}).to_string()).unwrap();
            }
            for th in thresholds {
                let p = Params { threshold: th, ..p.clone() };
                let c = speakers::cluster_windows(&vecs, &wins, &p);
                let l = speakers::label_words(&words, &wins, &vecs, &c, &p);
                let also = match &seg_net {
                    Some(n) => {
                        let t1 = std::time::Instant::now();
                        let o = d.overlaps(n, &audio, &words, &l.words, &op, &mut |_| true).unwrap();
                        eprintln!("overlap {:.1} s", t1.elapsed().as_secs_f32());
                        o
                    }
                    None => Vec::new(),
                };
                println!("{}", serde_json::json!({"threshold": th, "speakers": l.speakers.len(), "labels": l.words, "also": also, "stats": l.speakers}));
            }
        }
        Some("seg") => {
            let net = speakers::seg::SegNet::load(Path::new(&a[1])).expect("model");
            let audio = read_f32(&a[2]);
            let t = std::time::Instant::now();
            let out = net.forward(&audio).expect("forward");
            eprintln!("{:.3} s", t.elapsed().as_secs_f32());
            println!("{}", serde_json::to_string(&out).unwrap());
        }
        _ => eprintln!("usage: speakers fbank|embed|diarize …"),
    }
}
