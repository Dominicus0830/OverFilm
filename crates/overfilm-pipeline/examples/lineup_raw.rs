//! Read teams, players and hero swaps off a real broadcast without the editor (evaluation tool;
//! the editor's `overfilm.kills.scan` does the same on the frames it decodes).
//!
//! ```sh
//! ffmpeg -i vod.mp4 -vf fps=2 -f rawvideo -pix_fmt rgba - | \
//!   cargo run -p overfilm-pipeline --features ocr --release --example lineup_raw -- 1280 720 2 [start_s]
//! ```

use std::io::Read;

use overfilm_vision::{HeroNames, HeroTracker, LineupReader, Luma, TemplateSet, lineup};

fn clock(ms: i64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let get = |i: usize| a.get(i).cloned().unwrap_or_default();
    let (w, h): (usize, usize) = (get(1).parse().unwrap_or(0), get(2).parse().unwrap_or(0));
    let fps: f64 = get(3).parse().unwrap_or(2.0);
    let start: f64 = get(4).parse().unwrap_or(0.0);
    let Ok(set) = TemplateSet::from_json(overfilm_vision::builtin::OWCS_KOREA_2026) else { return eprintln!("templates") };
    let dir = overfilm_pipeline::models_dir();
    if let Err(e) = overfilm_pipeline::ocr::ensure(&dir, &mut |_, _| true) {
        return eprintln!("{e}");
    }
    let Ok(ocr) = overfilm_pipeline::ocr::Ocr::load(&dir) else { return eprintln!("ocr load") };
    let roster = overfilm_pipeline::game::roster();
    let mut heroes = HeroTracker::new(set.bars.clone(), 12_000);
    let mut plates = LineupReader::new(&set.bars);
    let mut buf = vec![0u8; w * h * 4];
    let mut stdin = std::io::stdin().lock();
    let (mut k, mut reads, mut last) = (0u64, 0u32, i64::MIN);
    let t0 = std::time::Instant::now();
    let mut ocr_time = std::time::Duration::ZERO;
    while stdin.read_exact(&mut buf).is_ok() {
        let at = ((start + k as f64 / fps) * 1000.0) as i64;
        k += 1;
        let Some(l) = Luma::from_rgba(&buf, w, h, 640) else { continue };
        let live = set.live(&l);
        heroes.push(at, &buf, w, h, &l, live);
        plates.sample(at, live);
        if live && at.saturating_sub(last) >= if plates.settled(at) { 30_000 } else { 2_000 } {
            let t = std::time::Instant::now();
            for b in &set.bars {
                let words = ocr.read(&buf, w, h, lineup::read_rect(b, None));
                let (team, names) = lineup::sort_words(b, None, &words);
                if std::env::var("TRACE").is_ok() {
                    eprintln!("{} {}팀 {:?} {:?}", clock(at), b.team, team, names);
                }
                if let Some(t) = team {
                    plates.team(at, b.team, &t);
                }
                for (p, n) in names {
                    plates.name(at, b.team, p, &n);
                }
            }
            ocr_time += t.elapsed();
            reads += 1;
            last = at;
        }
    }
    let info = plates.finish(&roster);
    let hs = heroes.finish(&HeroNames::default());
    let team = |i: Option<usize>| i.and_then(|i| roster.teams.get(i)).map(|t| t.name.clone()).unwrap_or("?".into());
    println!("teams: {} vs {}", team(info.teams[0]), team(info.teams[1]));
    let pl = |p: Option<usize>| p.and_then(|i| roster.players.get(i)).map(|p| p.handle.clone()).unwrap_or("?".into());
    for s in &info.stretches {
        println!(
            "stretch {}–{}: {} | {}",
            clock(s.start),
            clock(s.end),
            s.plates[0].iter().map(|p| pl(*p)).collect::<Vec<_>>().join(","),
            s.plates[1].iter().map(|p| pl(*p)).collect::<Vec<_>>().join(",")
        );
    }
    for ph in lineup::attribute(&hs, &info) {
        let e = &ph.event;
        let who = match ph.player {
            Some(p) => pl(Some(p)),
            None => format!("{}팀 {}번", e.team, e.player),
        };
        match e.from {
            Some(f) => println!("SWAP {} {who}: {} -> {}", clock(e.at), hs.label(f), hs.label(e.to)),
            None => println!("PICK {} {who}: {}", clock(e.at), hs.label(e.to)),
        }
    }
    println!("frames {k}, OCR reads {reads} ({:.0} s), total {:.0} s", ocr_time.as_secs_f64(), t0.elapsed().as_secs_f64());
}
