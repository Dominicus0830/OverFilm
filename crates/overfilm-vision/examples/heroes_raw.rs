//! Try hero-swap reading on a real broadcast without the editor (evaluation tool).
//!
//! ```sh
//! # RGBA frames at a fixed rate on stdin; prints each plate's picks and swaps
//! ffmpeg -ss 1700 -i vod.mp4 -vf fps=2 -f rawvideo -pix_fmt rgba - | cargo run -p overfilm-vision --release --example heroes_raw -- screens.json 1280 720 2 1700
//! # dump the portraits of one frame as PPM files (to check where they are)
//! cargo run -p overfilm-vision --example heroes_raw -- screens.json 1280 720 dump f.rgba out_dir
//! ```

use std::io::Read;

use filmcraft_time::{TICKS_PER_SECOND, Tick};
use overfilm_vision::{HeroNames, HeroTracker, Luma, TemplateSet, heroes};

fn clock(ms: i64) -> String {
    overfilm_vision::plan::clock(Tick(ms.saturating_mul(TICKS_PER_SECOND / 1000)))
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let get = |i: usize| a.get(i).cloned().unwrap_or_default();
    let (w, h): (usize, usize) = (get(2).parse().unwrap_or(0), get(3).parse().unwrap_or(0));
    let text = std::fs::read_to_string(get(1)).unwrap_or_default();
    let set =
        if text.is_empty() { TemplateSet::from_json(overfilm_vision::builtin::OWCS_KOREA_2026) } else { TemplateSet::from_json(&text) }.unwrap_or_default();
    if get(4) == "dump" {
        let px = std::fs::read(get(5)).unwrap_or_default();
        let out = get(6);
        for b in &set.bars {
            for k in 0..b.players {
                let r = heroes::portrait(b, k);
                let Some(c) = overfilm_vision::Rgb::crop_rgba(&px, w, h, r, 256) else { continue };
                let mut f = format!("P6\n{} {}\n255\n", c.width, c.height).into_bytes();
                f.extend(&c.px);
                let _ = std::fs::write(format!("{out}/t{}p{}.ppm", b.team, k + 1), f);
            }
        }
        return;
    }
    let fps: f64 = get(4).parse().unwrap_or(2.0);
    let start: f64 = get(5).parse().unwrap_or(0.0);
    let hold: i64 = std::env::var("HOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(12_000);
    let mut t = HeroTracker::new(set.bars.clone(), hold);
    if let Some(s) = std::env::var("SAME").ok().and_then(|v| v.parse().ok()) {
        t.same = s;
    }
    let has_live = set.templates.iter().any(|t| t.screen == overfilm_vision::Screen::Live);
    let mut buf = vec![0u8; w * h * 4];
    let mut stdin = std::io::stdin().lock();
    let mut k = 0u64;
    let t0 = std::time::Instant::now();
    while stdin.read_exact(&mut buf).is_ok() {
        let at = ((start + k as f64 / fps) * 1000.0) as i64;
        if let Some(f) = Luma::from_rgba(&buf, w, h, 640) {
            let live = !has_live || set.live(&f);
            t.push(at, &buf, w, h, &f, live);
        }
        k += 1;
    }
    let el = t0.elapsed();
    let s = t.finish(&HeroNames::default());
    for e in &s.events {
        match e.from {
            Some(f) => println!("SWAP {} {}팀 {}번 {} -> {}", clock(e.at), e.team, e.player, s.label(f), s.label(e.to)),
            None => println!("PICK {} {}팀 {}번 {}", clock(e.at), e.team, e.player, s.label(e.to)),
        }
    }
    if let Ok(dir) = std::env::var("PRINTS") {
        for (i, p) in s.prints.iter().enumerate() {
            let g = heroes::GRID;
            let mut f = format!("P6\n{g} {g}\n255\n").into_bytes();
            f.extend(&p.0);
            let _ = std::fs::write(format!("{dir}/h{:02}.ppm", i + 1), f);
        }
    }
    println!("frames {k}, heroes {}, events {}, {:?}", s.prints.len(), s.events.len(), el);
}
