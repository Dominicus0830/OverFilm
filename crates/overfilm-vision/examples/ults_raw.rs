//! Try ultimate detection on a real broadcast without the editor (evaluation tool).
//!
//! ```sh
//! # learn a team's ready-ultimate icon: one RGBA frame where that player's ultimate is ready
//! ffmpeg -ss 2080 -i vod.mp4 -frames:v 1 -f rawvideo -pix_fmt rgba f.rgba
//! cargo run -p overfilm-vision --example ults_raw -- learn screens.json 1280 720 f.rgba 1 x,y,w,h 1
//! # run: RGBA frames at a fixed rate on stdin; prints deaths, ultimates and ultimate kills
//! ffmpeg -i vod.mp4 -vf fps=2 -f rawvideo -pix_fmt rgba - | cargo run -p overfilm-vision --example ults_raw -- run screens.json 1280 720 2
//! ```

use std::io::Read;

use filmcraft_time::{TICKS_PER_SECOND, Tick};
use overfilm_vision::{DeathTracker, Luma, NRect, TemplateSet, UltIcon, UltTracker, ults};

fn clock(ms: i64) -> String {
    overfilm_vision::plan::clock(Tick(ms.saturating_mul(TICKS_PER_SECOND / 1000)))
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
            let Some(f) = Luma::from_rgba(&px, w, h, w) else { return eprintln!("bad frame") };
            let team: u8 = get(6).parse().unwrap_or(0);
            let v: Vec<f32> = get(7).split(',').filter_map(|x| x.trim().parse().ok()).collect();
            let [x, y, rw, rh] = v.as_slice() else { return eprintln!("bad rect") };
            let player: u8 = get(8).parse().unwrap_or(1);
            let Some(bar) = set.bars.iter_mut().find(|b| b.team == team) else { return eprintln!("no bar for team {team}") };
            match UltIcon::learn(&f, bar, NRect::new(*x, *y, *rw, *rh), player) {
                Ok(icon) => {
                    bar.ult = Some(icon);
                    println!("team {team}: ready {:?}", bar.ult_ready(&f));
                    let _ = std::fs::write(&path, set.to_json());
                }
                Err(e) => eprintln!("{e}"),
            }
        }
        "feed" => {
            // feed <file> <W> <H> <frame.rgba> <rect x,y,w,h> <icon x,y,w,h>: learn and list icons
            let px = std::fs::read(get(5)).unwrap_or_default();
            let Some(f) = Luma::from_rgba(&px, w, h, w) else { return eprintln!("bad frame") };
            let r = |s: String| -> Option<NRect> {
                let v: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                match v.as_slice() {
                    [x, y, w, h] => Some(NRect::new(*x, *y, *w, *h)),
                    _ => None,
                }
            };
            let (Some(rect), Some(icon)) = (r(get(6)), r(get(7))) else { return eprintln!("bad rects") };
            match overfilm_vision::KillFeed::learn(&f, rect, icon) {
                Ok(feed) => {
                    println!("radius {:.4}", feed.radius);
                    for i in feed.icons(&f) {
                        println!("icon at {:.0},{:.0}", i.x * w as f32, i.y * h as f32);
                    }
                }
                Err(e) => eprintln!("{e}"),
            }
        }
        "probe" => {
            let px = std::fs::read(get(5)).unwrap_or_default();
            let Some(f) = Luma::from_rgba(&px, w, h, w) else { return eprintln!("bad frame") };
            for b in &set.bars {
                let Some(icon) = &b.ult else { continue };
                let r: Vec<String> = (0..b.players).map(|k| icon.read(&f, b, k)).map(|(s, fat)| format!("{s:.2}/{fat:.3}")).collect();
                println!("team {} (learnt fat {:.3}): {}", b.team, icon.fat, r.join(" "));
            }
        }
        "run" => {
            let fps: f64 = get(5).parse().unwrap_or(2.0);
            let start: f64 = get(6).parse().unwrap_or(0.0);
            let confirm = (fps / 2.0).ceil().max(2.0) as u8;
            let mut deaths = DeathTracker::new(set.bars.clone(), confirm, 8_000);
            let mut uses = UltTracker::new(set.bars.clone(), confirm);
            let has_live = set.templates.iter().any(|t| t.screen == overfilm_vision::Screen::Live);
            let mut buf = vec![0u8; w * h * 4];
            let mut stdin = std::io::stdin().lock();
            let mut k = 0u64;
            while stdin.read_exact(&mut buf).is_ok() {
                let at = ((start + k as f64 / fps) * 1000.0) as i64;
                if let Some(f) = Luma::from_rgba(&buf, w, h, w) {
                    let live = !has_live || set.live(&f);
                    deaths.push(at, &f, live);
                    uses.push(at, &f, live);
                }
                k += 1;
            }
            let d = deaths.into_deaths();
            let u = ults::without_deaths(&uses.into_uses(), &d, 1_500);
            for x in &u {
                println!("ULT  {} {}팀 {}번", clock(x.at), x.team, x.player);
            }
            for x in ults::ult_kills(&u, &d, 6_000) {
                println!("UKILL {} {}팀 {}번 <- {}팀 {}번 ({})", clock(x.death.at), x.death.team, x.death.player, x.by.team, x.by.player, clock(x.by.at));
            }
            println!("deaths {} ultimates {}", d.len(), u.len());
        }
        _ => eprintln!("usage: see the file's doc comment"),
    }
}
