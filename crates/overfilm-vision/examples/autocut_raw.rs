//! Try templates and the cut plan on a real broadcast without the editor (evaluation tool).
//!
//! ```sh
//! # learn: one frame as 8-bit gray, rectangles in frame-relative units
//! ffmpeg -ss 2340 -i vod.mp4 -frames:v 1 -f rawvideo -pix_fmt gray f.gray
//! cargo run -p overfilm-vision --example autocut_raw -- learn screens.json 1280 720 f.gray standby "break" 0.0,0.0,0.25,0.8 [ignore=x,y,w,h]
//! # run: gray frames at a fixed rate on stdin
//! ffmpeg -i vod.mp4 -vf fps=2 -f rawvideo -pix_fmt gray - | cargo run -p overfilm-vision --example autocut_raw -- run screens.json 1280 720 2
//! ```

use std::io::Read;

use filmcraft_time::Tick;
use overfilm_vision::{Analyzer, CutParams, Luma, NRect, Screen, Template, TemplateSet, plan_cuts, segments};

fn rect(s: &str) -> Option<NRect> {
    let v: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    match v.as_slice() {
        [x, y, w, h] => NRect::new(*x, *y, *w, *h).clamped(),
        _ => None,
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let get = |i: usize| a.get(i).cloned().unwrap_or_default();
    let (w, h): (usize, usize) = (get(3).parse().unwrap_or(0), get(4).parse().unwrap_or(0));
    let path = get(2);
    let mut set = std::fs::read_to_string(&path).ok().and_then(|t| TemplateSet::from_json(&t).ok()).unwrap_or_default();
    match get(1).as_str() {
        "learn" => {
            let px = std::fs::read(get(5)).unwrap_or_default();
            let Some(f) = Luma::from_gray(px, w, h) else { return eprintln!("bad frame") };
            let Some(screen) = Screen::parse(&get(6)) else { return eprintln!("bad screen") };
            let (mut rects, mut ignore) = (Vec::new(), Vec::new());
            for s in a.iter().skip(8) {
                match s.strip_prefix("ignore=") {
                    Some(r) => ignore.extend(rect(r)),
                    None => rects.extend(rect(s)),
                }
            }
            match Template::learn(&get(7), screen, &f, &rects, &ignore) {
                Ok(t) => {
                    println!("{}: self score {:.3}", t.name, t.score(&f));
                    set.insert(t);
                    let _ = std::fs::write(&path, set.to_json());
                }
                Err(e) => eprintln!("{e}"),
            }
        }
        "run" => {
            let fps: f64 = get(5).parse().unwrap_or(2.0);
            let verbose = get(6) == "-v";
            let mut an = Analyzer::new(set.clone());
            let mut buf = vec![0u8; w * h];
            let mut stdin = std::io::stdin().lock();
            let mut k = 0u64;
            while stdin.read_exact(&mut buf).is_ok() {
                let t = Tick::from_seconds_f64(k as f64 / fps);
                if let Some(f) = Luma::from_gray(buf.clone(), w, h)
                    && let Some(s) = an.push(t, f)
                    && verbose
                {
                    let scores: Vec<String> = set
                        .templates
                        .iter()
                        .map(|tp| format!("{:.2}", tp.score(&Luma::from_gray(buf.clone(), w, h).unwrap_or(Luma { width: 1, height: 1, px: vec![0] }))))
                        .collect();
                    println!("{} {:?} still {:.2} [{}]", overfilm_vision::plan::clock(t), s.screen, s.still, scores.join(" "));
                }
                k += 1;
            }
            let p = CutParams { frozen_needs_live: set.templates.iter().any(|t| t.screen == Screen::Live), ..CutParams::default() };
            let samples = an.into_samples();
            for g in segments(&samples, &p) {
                println!(
                    "segment {:?} {} – {} ({:?})",
                    g.screen,
                    overfilm_vision::plan::clock(g.range.start),
                    overfilm_vision::plan::clock(g.range.end()),
                    g.how
                );
            }
            let cuts = plan_cuts(&segments(&samples, &p), &p);
            for c in &cuts {
                println!(
                    "CUT {:?} {} – {} dissolve {}",
                    c.reason,
                    overfilm_vision::plan::clock(c.range.start),
                    overfilm_vision::plan::clock(c.range.end()),
                    c.dissolve
                );
            }
            println!(
                "{} cuts, {} removed of {}",
                cuts.len(),
                overfilm_vision::plan::clock(overfilm_vision::plan::removed(&cuts)),
                overfilm_vision::plan::clock(Tick::from_seconds_f64(k as f64 / fps))
            );
        }
        _ => eprintln!("usage: learn|run …"),
    }
}
