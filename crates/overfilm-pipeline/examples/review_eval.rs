//! Claude Code caption review on a recognised video, for measuring it: lays the transcript of
//! `recognise_dump` out as the editor does, asks the installed Claude Code about the captions
//! the editor would ask about (or all of them), applies the answers like the editor, and writes
//! the captions before and after as SRT plus a log of every question and answer.
//!
//! `cargo run -p overfilm-pipeline --example review_eval -- rec.json out_dir [all] [terms] [commentary] [context.txt]`
//! then `cargo run -p overfilm-eval -- ref.srt out_dir/before.srt` (and `after.srt`).

use overfilm_review::captions::{self as caps, CapWord, CaptionDecision, CaptionIn, CaptionQuestion, Limits, RawChunk};
use overfilm_review::{ClaudeCode, Reviewer, Settings};

fn srt_time(ms: i64) -> String {
    let ms = ms.max(0);
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

fn srt(caps: &[(i64, i64, String)]) -> String {
    caps.iter().enumerate().map(|(i, (a, b, t))| format!("{}\n{} --> {}\n{}\n\n", i + 1, srt_time(*a), srt_time(*b), t)).collect()
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let (Some(input), Some(out)) = (args.get(1), args.get(2)) else { return Err("usage: rec.json out_dir [all] [terms] [commentary] [context.txt]".into()) };
    let all = args.iter().any(|a| a == "all");
    let with_terms = args.iter().any(|a| a == "terms");
    let video = if args.iter().any(|a| a == "commentary") { "대회 해설 중계" } else { "일반 게임 영상" };
    let g = overfilm_pipeline::pack::glossary(None, true)?;
    let terms: Vec<caps::Term> = g
        .entries
        .iter()
        .filter(|e| !e.ko.is_empty() && e.category != "profanity")
        .map(|e| caps::Term {
            ko: e.ko.clone(),
            en: e.en.clone(),
            kind: e.category.clone(),
            aliases: e.aliases.iter().chain(&e.old).cloned().collect(),
            misheard: e.misheard.clone(),
            note: e.note.clone(),
        })
        .collect();
    let context = args.iter().skip(3).find(|a| a.ends_with(".txt")).map(|p| std::fs::read_to_string(p).unwrap_or_default()).unwrap_or_default();
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(input).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let t: overfilm_asr::Transcript = serde_json::from_value(v["transcript"].clone()).map_err(|e| e.to_string())?;
    let raw: Vec<String> = serde_json::from_value(v["raw"].clone()).unwrap_or_default();
    // the editor's layout and its record of the pure recognition
    let lim = Limits::default();
    let layout = overfilm_captions::Layout { max_line: lim.line_width, max_lines: lim.max_lines, ..Default::default() };
    let cs = overfilm_pipeline::speakers::captions(&t, overfilm_pipeline::speakers::GameVoice::Drop, &layout);
    let words: Vec<&overfilm_asr::Word> = t.words().collect();
    let mut chunks = Vec::new();
    let mut first = 0usize;
    for (i, c) in t.chunks.iter().enumerate() {
        chunks.push(RawChunk { first_word: first, words: c.words.len(), raw: raw.get(i).cloned().unwrap_or_default(), text: c.text.clone() });
        first += c.words.len();
    }
    let raw_of = |idx: &[usize]| -> Option<String> {
        let mut parts = Vec::new();
        for ch in &chunks {
            let inside: Vec<usize> = idx.iter().copied().filter(|i| (ch.first_word..ch.first_word + ch.words).contains(i)).collect();
            let (Some(&a), Some(&b)) = (inside.iter().min(), inside.iter().max()) else { continue };
            let texts: Vec<&str> = words.get(ch.first_word..ch.first_word + ch.words)?.iter().map(|w| w.text.as_str()).collect();
            parts.push(ch.raw_for(a, b + 1, &texts)?);
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    };
    let infos: Vec<CaptionIn> = cs
        .iter()
        .enumerate()
        .map(|(n, c)| {
            let idx: Vec<usize> = (0..words.len())
                .filter(|&i| {
                    let w = words[i];
                    let mid = w.start_ms + (w.end_ms - w.start_ms) / 2;
                    !w.game && mid >= c.start_ms && mid < c.end_ms
                })
                .collect();
            let ws: Vec<CapWord> = idx
                .iter()
                .map(|&i| CapWord {
                    text: words[i].text.clone(),
                    start_ms: words[i].start_ms,
                    end_ms: words[i].end_ms,
                    confidence: words[i].conf.unwrap_or(1.0),
                })
                .collect();
            let corrected = ws.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            let q = CaptionQuestion {
                id: n.to_string(),
                start_ms: c.start_ms,
                end_ms: c.end_ms,
                text: c.text(),
                raw: raw_of(&idx),
                corrected,
                ..Default::default()
            };
            CaptionIn { q, words: ws }
        })
        .collect();
    let before: Vec<(i64, i64, String)> = infos.iter().map(|i| (i.q.start_ms, i.q.end_ms, i.q.text.clone())).collect();
    std::fs::write(format!("{out}/before.srt"), srt(&before)).map_err(|e| e.to_string())?;
    let cases: Vec<CaptionQuestion> = caps::select(&infos, &lim, all, |_| false, usize::MAX).into_iter().map(|(_, c)| c).collect();
    eprintln!("{} captions, {} to ask", infos.len(), cases.len());
    let cc = ClaudeCode::new(&Settings { enabled: true, timeout_secs: 600, ..Settings::default() }).map_err(|e| e.to_string())?;
    let stop = std::sync::atomic::AtomicBool::new(false);
    let mut answers = Vec::new();
    let mut log = Vec::new();
    for (k, b) in cases.chunks(6).enumerate() {
        let texts: Vec<&str> = b
            .iter()
            .flat_map(|c| {
                [c.text.as_str(), c.corrected.as_str(), c.raw.as_deref().unwrap_or("")]
                    .into_iter()
                    .chain(c.before.iter().chain(&c.after).map(|n| n.text.as_str()))
            })
            .collect();
        let bg = caps::Background {
            video: if with_terms { video.into() } else { String::new() },
            on_screen: Vec::new(),
            terms: if with_terms { caps::relevant_terms(&terms, &texts, 60) } else { Vec::new() },
        };
        let mut q = caps::question_with(b, &lim, &bg);
        if !context.is_empty() {
            q.prompt = format!("{context}\n\n{}", q.prompt);
        }
        let t0 = std::time::Instant::now();
        match cc.ask(&q, &stop) {
            Ok(v) => {
                let a = caps::parse(&v, b, &lim);
                eprintln!("batch {k}: {} answers in {:.0}s", a.len(), t0.elapsed().as_secs_f64());
                log.push(serde_json::json!({"cases": b, "reply": v, "kept": a}));
                answers.extend(a);
            }
            Err(e) => {
                eprintln!("batch {k}: {e}");
                log.push(serde_json::json!({"cases": b, "error": e.to_string()}));
            }
        }
    }
    std::fs::write(format!("{out}/log.json"), serde_json::to_string_pretty(&log).unwrap_or_default()).map_err(|e| e.to_string())?;
    let mut after = Vec::new();
    for (n, info) in infos.iter().enumerate() {
        let a = answers.iter().find(|a| a.id == n.to_string());
        match a.map(|a| (a.decision, a)) {
            Some((CaptionDecision::Fix, a)) => after.push((info.q.start_ms, info.q.end_ms, a.text.clone())),
            Some((CaptionDecision::Split, a)) => {
                for (p, (x, y)) in a.parts.iter().zip(caps::split_times(info.q.start_ms, info.q.end_ms, &info.words, &a.parts)) {
                    after.push((x, y, p.clone()));
                }
            }
            _ => after.push((info.q.start_ms, info.q.end_ms, info.q.text.clone())),
        }
    }
    std::fs::write(format!("{out}/after.srt"), srt(&after)).map_err(|e| e.to_string())?;
    let fixes = answers.iter().filter(|a| a.decision != CaptionDecision::Keep).count();
    eprintln!("asked {}, answered {}, changed {}", cases.len(), answers.len(), fixes);
    Ok(())
}
