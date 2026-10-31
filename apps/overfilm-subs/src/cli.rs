//! `overfilm-subs`: the command line and MCP server.
//!
//! ```text
//! overfilm-subs extract <video or audio> [-o out.srt] [--mode general|commentary]
//!               [--terms "이름, 용어"] [--no-mask] [--glossary <folder>] [--line-chars 16]
//! overfilm-subs review <file.srt>
//! overfilm-subs fix <file.srt>          apply the glossary (misrecognitions, profanity initials)
//! overfilm-subs relayout <file.overfilm.json> [-o out.srt] [--line-chars 16] [--one-line]
//!                                       captions again from the saved word times
//! overfilm-subs models            download the models (about 6.5 GB) if missing
//! overfilm-subs mcp               MCP server on stdin/stdout (Claude Desktop and others)
//! ```

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use overfilm_subs::{job, mcp, pack, review};

const USAGE: &str = "사용법:
  overfilm-subs extract <영상/오디오> [-o 저장.srt] [--mode general|commentary] [--terms \"이름, 용어\"] [--no-mask] [--glossary 폴더] [--line-chars 16] [--cpu]
  overfilm-subs review <자막.srt>
  overfilm-subs fix <자막.srt>      단어집 교정 적용 (원본은 .srt.bak)
  overfilm-subs relayout <이름.overfilm.json> [-o 저장.srt] [--line-chars 16] [--one-line]   인식 다시 안 하고 자막만 다시 나누기
  overfilm-subs models     모델 받기 (약 6.5GB, 처음 한 번)
  overfilm-subs mcp        MCP 서버 (Claude Desktop 등에서 연결)";

fn progress_bar() -> impl FnMut(f32, &str) -> bool {
    let mut last = String::new();
    move |f, m| {
        let line = format!("{:>5.1}% {m}", f * 100.0);
        if line != last {
            eprint!("\r{line:<100}");
            let _ = std::io::stderr().flush();
            last = line;
        }
        true
    }
}

fn extract(args: &[String]) -> Result<(), String> {
    let mut it = args.iter();
    let mut input = None;
    let mut o = job::Options::new(PathBuf::new());
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or_else(|| format!("{a} 뒤에 값이 필요합니다"));
        match a.as_str() {
            "-o" | "--output" => o.output = Some(PathBuf::from(val()?)),
            "--mode" => o.mode = pack::Mode::parse(&val()?).ok_or("--mode는 general 또는 commentary")?,
            "--terms" => o.extra_terms = val()?,
            "--no-mask" => o.mask_profanity = false,
            "--glossary" => o.glossary_dir = Some(PathBuf::from(val()?)),
            "--line-chars" => o.line_chars = val()?.parse().map_err(|_| "--line-chars는 숫자")?,
            "--one-line" => o.max_lines = 1,
            "--cpu" => o.gpu = false,
            s if s.starts_with('-') => return Err(format!("모르는 옵션: {s}\n{USAGE}")),
            s => input = Some(PathBuf::from(s)),
        }
    }
    o.input = input.ok_or(USAGE)?;
    if o.glossary_dir.is_none() {
        let d = mcp::user_glossary_dir();
        o.glossary_dir = d.is_dir().then_some(d);
    }
    let mut p = progress_bar();
    let done = job::guarded(|| job::run(&o, &mut p))?;
    eprintln!();
    println!("자막 {}개 → {}", done.captions, done.srt.display());
    println!("단어 시간 → {}", done.transcript.display());
    println!("소리 {}초를 {}초 걸려 처리 ({}), 단어집 교정 {}곳", done.audio_seconds, done.seconds_taken, done.device, done.corrections.len());
    Ok(())
}

fn review_cmd(args: &[String]) -> Result<(), String> {
    let p = args.first().map(PathBuf::from).ok_or(USAGE)?;
    let caps = job::read_srt(&p)?;
    let g = pack::glossary(None, true)?;
    let issues = review::review(&caps, &overfilm_captions::Layout::default(), &pack::corrector(&g));
    for i in &issues {
        let t = caps.get(i.index.saturating_sub(1)).map(|c| c.text().replace('\n', " / ")).unwrap_or_default();
        println!("#{} [{}] {} | {t}", i.index, i.kind, i.message);
    }
    println!("자막 {}개, 검토할 곳 {}개", caps.len(), issues.len());
    Ok(())
}

fn fix(args: &[String]) -> Result<(), String> {
    let p = args.first().ok_or(USAGE)?;
    let server = mcp::Server::default();
    let r =
        server.handle(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "apply_glossary", "arguments": {"path": p}}}));
    let text = r.as_ref().and_then(|v| v["result"]["content"][0]["text"].as_str()).unwrap_or_default().to_string();
    if r.as_ref().is_some_and(|v| v["result"]["isError"] == true) {
        return Err(text);
    }
    println!("{text}");
    Ok(())
}

fn relayout(args: &[String]) -> Result<(), String> {
    let mut it = args.iter();
    let mut input = None;
    let mut out = None;
    let mut o = job::Options::new(PathBuf::new());
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--output" => out = it.next().map(PathBuf::from),
            "--line-chars" => o.line_chars = it.next().and_then(|v| v.parse().ok()).ok_or("--line-chars는 숫자")?,
            "--one-line" => o.max_lines = 1,
            "--cpu" => o.gpu = false,
            s => input = Some(PathBuf::from(s)),
        }
    }
    let input = input.ok_or(USAGE)?;
    if std::fs::metadata(&input).map(|m| m.len()).unwrap_or(0) > 256 << 20 {
        return Err("파일이 너무 큽니다".into());
    }
    let text = std::fs::read_to_string(&input).map_err(|e| format!("{}: {e}", input.display()))?;
    let t: overfilm_asr::Transcript = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", input.display()))?;
    let words: Vec<overfilm_captions::TimedWord> =
        t.words().map(|w| overfilm_captions::TimedWord { text: w.text.clone(), start_ms: w.start_ms, end_ms: w.end_ms }).collect();
    let caps = overfilm_captions::layout(&words, &o.layout());
    let name = input.file_name().map(|n| n.to_string_lossy().replace(".overfilm.json", ".srt")).unwrap_or_else(|| "out.srt".into());
    let out = out.unwrap_or_else(|| input.with_file_name(name));
    job::write_srt(&out, &caps)?;
    println!("자막 {}개 → {}", caps.len(), out.display());
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("extract") => extract(args.get(1..).unwrap_or_default()),
        Some("review") => review_cmd(args.get(1..).unwrap_or_default()),
        Some("fix") => fix(args.get(1..).unwrap_or_default()),
        Some("relayout") => relayout(args.get(1..).unwrap_or_default()),
        Some("models") => {
            let mut p = progress_bar();
            let r = job::ensure_models(&job::models_dir(), &mut p);
            eprintln!();
            r.map(|()| println!("모델 준비됨: {}", job::models_dir().display()))
        }
        Some("mcp") => mcp::serve(),
        _ => Err(USAGE.to_string()),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
