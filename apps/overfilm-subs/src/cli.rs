//! `overfilm-subs`: the command line and MCP server.
//!
//! ```text
//! overfilm-subs extract <video or audio> [-o out.srt] [--mode general|commentary]
//!               [--terms "이름, 용어"] [--no-mask] [--glossary <folder>] [--line-chars 16]
//! overfilm-subs review <file.srt>
//! overfilm-subs models            download the models (about 6.5 GB) if missing
//! overfilm-subs mcp               MCP server on stdin/stdout (Claude Desktop and others)
//! ```

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use overfilm_subs::{job, mcp, pack, review};

const USAGE: &str = "사용법:
  overfilm-subs extract <영상/오디오> [-o 저장.srt] [--mode general|commentary] [--terms \"이름, 용어\"] [--no-mask] [--glossary 폴더] [--line-chars 16]
  overfilm-subs review <자막.srt>
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
    println!("소리 {}초를 {}초 걸려 처리, 단어집 교정 {}곳", done.audio_seconds, done.seconds_taken, done.corrections.len());
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("extract") => extract(args.get(1..).unwrap_or_default()),
        Some("review") => review_cmd(args.get(1..).unwrap_or_default()),
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
