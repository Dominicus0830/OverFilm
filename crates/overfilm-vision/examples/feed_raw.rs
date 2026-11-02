//! Try reading who killed whom off a kill feed without the editor (evaluation tool).
//!
//! ```sh
//! # one picture: the rows found on it
//! ffmpeg -i shot.png -f rawvideo -pix_fmt rgba shot.rgba
//! cargo run -p overfilm-vision --example feed_raw -- rows 430 331 shot.rgba 0,0,1,1
//! # a video: RGBA frames at a fixed rate on stdin, the feed's rectangle; prints the kills
//! ffmpeg -i game.mp4 -vf fps=2 -f rawvideo -pix_fmt rgba - | cargo run -p overfilm-vision --example feed_raw -- run 1920 1080 2 0.7,0.05,0.3,0.3
//! ```

use std::io::Read;

use filmcraft_time::{TICKS_PER_SECOND, Tick};
use overfilm_vision::feedrows::read_rows;
use overfilm_vision::{FeedNames, FeedReader, NRect, Rgb};

fn clock(ms: i64) -> String {
    overfilm_vision::plan::clock(Tick(ms.saturating_mul(TICKS_PER_SECOND / 1000)))
}

fn rect(s: &str) -> Option<NRect> {
    let v: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    match v.as_slice() {
        [x, y, w, h] => Some(NRect::new(*x, *y, *w, *h)),
        _ => None,
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let get = |i: usize| a.get(i).cloned().unwrap_or_default();
    let (w, h): (usize, usize) = (get(2).parse().unwrap_or(0), get(3).parse().unwrap_or(0));
    match get(1).as_str() {
        "rows" => {
            let px = std::fs::read(get(4)).unwrap_or_default();
            let Some(r) = rect(&get(5)) else { return eprintln!("bad rect") };
            let Some(img) = Rgb::crop_rgba(&px, w, h, r, 1024) else { return eprintln!("bad frame") };
            let rows = read_rows(&img, None);
            println!("row height {:?}", overfilm_vision::feedrows::row_height(&img));
            let mut reader = FeedReader::new(FeedNames::default(), 1, None);
            reader.push_rows(0, &rows);
            let scan = reader.finish();
            for (r, e) in rows.iter().zip(&scan.events) {
                let k = r.killer.as_ref().map_or("-", |k| k.side.ko());
                println!("y={:.3} {k} → {} ult={} : {}", r.y, r.victim.side.ko(), r.ult, scan.line(e));
            }
            println!("{} row(s), {} player(s), {} hero(es)", rows.len(), scan.players.len(), scan.heroes.len());
        }
        "pairs" => {
            // how alike each two names and pictures on the picture read (for tuning)
            let px = std::fs::read(get(4)).unwrap_or_default();
            let Some(r) = rect(&get(5)) else { return eprintln!("bad rect") };
            let Some(img) = Rgb::crop_rgba(&px, w, h, r, 1024) else { return eprintln!("bad frame") };
            let rows = read_rows(&img, None);
            let sides: Vec<(String, &overfilm_vision::feedrows::FeedSide)> = rows
                .iter()
                .enumerate()
                .flat_map(|(i, r)| r.killer.iter().map(move |k| (format!("{i}k"), k)).chain(std::iter::once((format!("{i}v"), &r.victim))))
                .collect();
            for (i, (a, sa)) in sides.iter().enumerate() {
                for (b, sb) in sides.iter().skip(i + 1) {
                    let hero = sa.hero.as_ref().zip(sb.hero.as_ref()).map_or(0.0, |(p, q)| p.similarity(q));
                    println!("{a} {b} name {:.2} hero {hero:.2}", sa.name.similarity(&sb.name));
                }
            }
        }
        "run" => {
            let fps: f64 = get(4).parse().unwrap_or(2.0);
            let Some(r) = rect(&get(5)) else { return eprintln!("bad rect") };
            let mut reader = FeedReader::new(FeedNames::default(), 2, None);
            let mut buf = vec![0u8; w * h * 4];
            let mut stdin = std::io::stdin().lock();
            let mut n = 0u64;
            while stdin.read_exact(&mut buf).is_ok() {
                let at = (n as f64 * 1000.0 / fps) as i64;
                if let Some(img) = Rgb::crop_rgba(&buf, w, h, r, 1024) {
                    reader.push(at, &img, true);
                }
                n += 1;
            }
            let scan = reader.finish();
            for e in &scan.events {
                println!("{} {}{}", clock(e.at), scan.line(e), if e.ult { " [궁극기]" } else { "" });
            }
            println!("{} kill(s) in {n} frames", scan.events.len());
        }
        _ => eprintln!("usage: feed_raw rows W H file.rgba x,y,w,h | run W H fps x,y,w,h"),
    }
}
