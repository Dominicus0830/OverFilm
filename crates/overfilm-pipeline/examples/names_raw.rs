//! Try naming kill feed players and heroes from the name plates without the editor (evaluation
//! tool; the editor's `overfilm.kills.scan` does the same on the frames it decodes).
//!
//! ```sh
//! ffmpeg -i vod.mp4 -vf fps=2 -f rawvideo -pix_fmt rgba - | \
//!   cargo run -p overfilm-pipeline --features ocr --release --example names_raw -- 1280 720 2 [start_s] [out_dir]
//! ```
//!
//! Prints which team is blue in the feed, each feed player with the plate and name it was linked
//! to, each feed hero with its plate hero, and how many kills were paired. With `out_dir`, each
//! feed hero's and plate hero's picture is written there as PPM (`feed-hero-N.ppm`,
//! `plate-hero-N.ppm`) to check the links by eye.

use std::io::Read;

use overfilm_vision::{DeathTracker, FeedReader, HeroNames, HeroTracker, LineupReader, Luma, Rgb, TemplateSet, heroes, lineup, link};

fn clock(ms: i64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn ppm(path: &str, img: &Rgb) {
    let mut f = format!("P6\n{} {}\n255\n", img.width, img.height).into_bytes();
    f.extend(&img.px);
    let _ = std::fs::write(path, f);
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let get = |i: usize| a.get(i).cloned().unwrap_or_default();
    let (w, h): (usize, usize) = (get(1).parse().unwrap_or(0), get(2).parse().unwrap_or(0));
    let fps: f64 = get(3).parse().unwrap_or(2.0);
    let start: f64 = get(4).parse().unwrap_or(0.0);
    let out_dir = get(5);
    let Ok(set) = TemplateSet::from_json(overfilm_vision::builtin::OWCS_KOREA_2026) else { return eprintln!("templates") };
    let Some(feed) = set.killfeed.clone() else { return eprintln!("no kill feed in the templates") };
    let dir = overfilm_pipeline::models_dir();
    if let Err(e) = overfilm_pipeline::ocr::ensure(&dir, &mut |_, _| true) {
        return eprintln!("{e}");
    }
    let Ok(ocr) = overfilm_pipeline::ocr::Ocr::load(&dir) else { return eprintln!("ocr load") };
    let roster = overfilm_pipeline::game::roster();
    let confirm = (fps / 2.0).ceil().max(2.0) as u8;
    let mut deaths = DeathTracker::new(set.bars.clone(), confirm, 8_000);
    let mut reader = FeedReader::new(set.feed_names.clone(), confirm, (feed.row > 0.0).then_some(feed.row));
    let mut hero_tracker = HeroTracker::new(set.bars.clone(), 12_000);
    let mut plates = LineupReader::new(&set.bars);
    // the first frame each plate hero print was seen on, to dump its picture
    let mut portraits: Vec<(heroes::HeroPrint, Rgb)> = Vec::new();
    let mut feed_rows: Vec<(i64, Rgb)> = Vec::new();
    let mut buf = vec![0u8; w * h * 4];
    let mut stdin = std::io::stdin().lock();
    let (mut k, mut last) = (0u64, i64::MIN);
    let t0 = std::time::Instant::now();
    while stdin.read_exact(&mut buf).is_ok() {
        let at = ((start + k as f64 / fps) * 1000.0) as i64;
        k += 1;
        let Some(l) = Luma::from_rgba(&buf, w, h, 640) else { continue };
        let live = set.live(&l);
        let health: Vec<Vec<bool>> =
            set.bars.iter().map(|b| Rgb::crop_rgba(&buf, w, h, b.under_rect(), 1024).map(|r| b.health(&r)).unwrap_or_default()).collect();
        deaths.push_with(at, &l, live, &health);
        if let Some(img) = Rgb::crop_rgba(&buf, w, h, feed.rect, 1024) {
            reader.push(at, &img, live);
            if !out_dir.is_empty() && live && k % 4 == 0 {
                feed_rows.push((at, img));
                if feed_rows.len() > 4_000 {
                    feed_rows.remove(0);
                }
            }
        }
        hero_tracker.push(at, &buf, w, h, &l, live);
        if !out_dir.is_empty() && live {
            for b in &set.bars {
                for p in 0..b.players {
                    let r = heroes::portrait(b, p);
                    if let Some(pr) = heroes::HeroPrint::read(&buf, w, h, r)
                        && !portraits.iter().any(|(q, _)| q.diff(&pr) < heroes::SAME)
                        && let Some(img) = Rgb::crop_rgba(&buf, w, h, r, 256)
                    {
                        portraits.push((pr, img));
                    }
                }
            }
        }
        plates.sample(at, live);
        if live && at.saturating_sub(last) >= if plates.settled(at) { 30_000 } else { 2_000 } {
            for b in &set.bars {
                let words = ocr.read(&buf, w, h, lineup::read_rect(b, None));
                let (team, names) = lineup::sort_words(b, None, &words);
                if let Some(t) = team {
                    plates.team(at, b.team, &t);
                }
                for (p, n) in names {
                    plates.name(at, b.team, p, &n);
                }
            }
            last = at;
        }
    }
    let d = deaths.into_deaths();
    let mut scan = reader.finish();
    let info = plates.finish(&roster);
    let mut hs = hero_tracker.finish(&HeroNames::default());
    let links = link::link(&scan, &d, &hs, &info);
    let pl = |p: usize| roster.players.get(p).map(|p| p.handle.clone());
    let filled = link::fill_names(&mut scan, &mut hs, &links, &pl);
    println!("frames {k}, deaths {}, feed kills {}, plate heroes {}, {:.0} s", d.len(), scan.events.len(), hs.prints.len(), t0.elapsed().as_secs_f64());
    println!("blue team: {:?}, names filled: {filled}", links.blue_team);
    let paired = links.victims.iter().filter(|v| v.is_some()).count();
    let killers = links.killers.iter().filter(|v| v.is_some()).count();
    println!("victims paired {paired}/{}, killers placed {killers}/{}", scan.events.len(), scan.events.iter().filter(|e| e.killer.is_some()).count());
    for (i, c) in scan.players.iter().enumerate() {
        println!("player {:>2}: {:<12} seen {:>3} plate {:?} side {:?}", i + 1, scan.player(i), c.seen, links.player_plates.get(i).copied().flatten(), c.side);
    }
    for (i, c) in scan.heroes.iter().enumerate() {
        println!("feed hero {:>2}: seen {:>3} → plate hero {:?}", i + 1, c.seen, links.heroes.get(i).copied().flatten().map(|h| h + 1));
    }
    // per kill: the victim's name as the plate says
    let plate_name = |at: i64, p: Option<(u8, u8)>| p.and_then(|(t, n)| info.stretch_at(at).and_then(|s| s.player(t, n))).and_then(pl);
    for (e, (v, kp)) in scan.events.iter().zip(links.victims.iter().zip(&links.killers)).take(400) {
        println!(
            "{} {} | plates: {} → {}",
            clock(e.at),
            scan.line(e),
            plate_name(e.at, *kp).unwrap_or_else(|| "?".into()),
            plate_name(e.at, *v).unwrap_or_else(|| "?".into())
        );
    }
    if std::env::var_os("TRACE").is_some() {
        for (e, v) in scan.events.iter().zip(&links.victims) {
            if v.is_some() {
                continue;
            }
            let near: Vec<String> =
                d.iter().filter(|x| (x.at - e.at).abs() <= 6_000).map(|x| format!("{}팀{}번 {:+}", x.team, x.player, x.at - e.at)).collect();
            eprintln!("unpaired {} {:?}: {}", clock(e.at), e.victim.side, near.join(", "));
        }
    }
    if !out_dir.is_empty() {
        let _ = std::fs::create_dir_all(&out_dir);
        for (i, p) in hs.prints.iter().enumerate() {
            if let Some((_, img)) = portraits.iter().min_by(|a, b| a.0.diff(p).total_cmp(&b.0.diff(p))) {
                ppm(&format!("{out_dir}/plate-hero-{}.ppm", i + 1), img);
            }
        }
        // each feed hero: the feed on a sample shortly before one of its rows left
        for (i, _) in scan.heroes.iter().enumerate() {
            let e = scan.events.iter().find(|e| e.victim.hero == Some(i) || e.killer.and_then(|k| k.hero) == Some(i));
            if let Some(e) = e
                && let Some((_, img)) = feed_rows.iter().filter(|(t, _)| *t <= e.seen).max_by_key(|(t, _)| *t)
            {
                ppm(&format!("{out_dir}/feed-hero-{}.ppm", i + 1), img);
            }
        }
    }
}
