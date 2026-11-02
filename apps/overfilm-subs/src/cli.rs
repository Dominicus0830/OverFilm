//! `overfilm-subs`: the command line and MCP server.
//!
//! ```text
//! overfilm-subs extract <video or audio> [-o out.srt] [--mode general|commentary]
//!               [--terms "이름, 용어"] [--no-mask] [--no-digits] [--glossary <folder>] [--line-chars 16]
//!               [--speakers auto|off|N] [--game-voice drop|label|keep] [--per-speaker] [--keep-fillers]
//!               [--audio-track N]
//! overfilm-subs review <file.srt>
//! overfilm-subs fix <file.srt>          apply the glossary (misrecognitions, profanity initials)
//! overfilm-subs relayout <file.overfilm.json> [-o out.srt] [--line-chars 16] [--one-line]
//!               [--game-voice drop|label|keep] [--rename 화자1=이름]…
//!                                       captions again from the saved word times
//! overfilm-subs redo <file.srt> --from <time> --to <time> [--input <video>] [--terms "…"]
//!                                       recognise one part again, keeping hand fixes elsewhere
//! overfilm-subs export <file.srt> [--format srt|vtt|ass|txt] [--fps 29.97] [--gap 2] [-o out]
//!                                       other formats, times on frames, a gap between captions
//! overfilm-subs voice add <name> <video or audio> [--from 1:23] [--to 1:45]
//! overfilm-subs voice list | voice remove <name>
//!                                 enrolled voices (named in captions, --only-voices keeps them)
//! overfilm-subs models            download the models (about 6.5 GB) if missing
//! overfilm-subs mcp               MCP server on stdin/stdout (Claude Desktop and others)
//! overfilm-subs gpu-check [--quick] GPU kernels and the real model against the CPU
//! ```

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use overfilm_subs::{export, job, mcp, pack, review, speakers, voices};

const USAGE: &str = "사용법:
  overfilm-subs extract <영상/오디오> [-o 저장.srt] [--mode general|commentary] [--terms \"이름, 용어\"] [--no-mask] [--no-digits] [--glossary 폴더] [--line-chars 16] [--cpu]
                [--speakers auto|off|숫자] [--game-voice drop|label|keep] [--per-speaker]
      --speakers     목소리 구별: auto(기본, 사람 수 자동), off(끔), 숫자(사람 수를 알 때, 1이면 구별을 건너뛰어 빠름)
      --audio-track  인식할 오디오 트랙 번호 (1부터, 기본 1). OBS에서 마이크를 따로 녹음한 트랙 등
      --game-voice   영웅 대사·아나운서 같은 게임 음성: drop(자막에서 빼기, 기본), label((게임) 표시), keep(그대로)
      --per-speaker  사람마다 SRT를 따로 저장 (이름.화자1.srt …)
      --only-voices  등록한 목소리(voice add)의 말만 자막으로 (스트리머 말만 등)
      --no-voices    등록한 목소리로 이름 붙이지 않기
      --keep-fillers 추임새(어, 음)와 말 더듬기(그 그게)를 자막에 남기기 (기본은 뺌, 단어 시간 파일에는 늘 남음)
      --from 1:23 --to 4:56   이 구간만 인식 (자막 시간은 원본 기준)
  overfilm-subs review <자막.srt>
  overfilm-subs fix <자막.srt>      단어집 교정 적용 (원본은 .srt.bak)
  overfilm-subs relayout <이름.overfilm.json> [-o 저장.srt] [--line-chars 16] [--one-line] [--keep-fillers] [--game-voice …] [--rename 화자1=이름]   인식 다시 안 하고 자막만 다시 나누기
  overfilm-subs redo <자막.srt> --from 00:01:23,000 --to 00:01:40,000 [--input 영상] [--terms \"이름\"]   이 부분만 다시 인식 (다른 곳 고친 글자는 그대로)
  overfilm-subs export <자막.srt> [--format srt|vtt|ass|txt] [--fps 29.97] [--gap 2] [-o 저장]   다른 형식으로 (프레임 맞춤, 자막 사이 빈 프레임)
  overfilm-subs voice add <이름> <영상/오디오> [--from 1:23] [--to 1:45]   목소리 등록 (10~30초 권장, 그 사람만 말하는 구간)
  overfilm-subs voice list | voice remove <이름>
  overfilm-subs models     모델 받기 (약 6.5GB, 처음 한 번)
  overfilm-subs mcp        MCP 서버 (Claude Desktop 등에서 연결)
  overfilm-subs gpu-check  그래픽카드 계산이 CPU와 같은지 확인 (--quick: 모델 비교 생략)";

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
            "--no-digits" => o.digits = false,
            "--glossary" => o.glossary_dir = Some(PathBuf::from(val()?)),
            "--line-chars" => o.line_chars = val()?.parse().map_err(|_| "--line-chars는 숫자")?,
            "--one-line" => o.max_lines = 1,
            "--cpu" => o.gpu = false,
            "--speakers" => match val()?.as_str() {
                "auto" | "자동" => (o.speakers, o.num_speakers) = (true, None),
                "off" | "끔" => o.speakers = false,
                n => {
                    (o.speakers, o.num_speakers) =
                        (true, Some(n.parse::<usize>().ok().filter(|&n| (1..=20).contains(&n)).ok_or("--speakers는 auto, off 또는 1~20")?))
                }
            },
            "--audio-track" => {
                o.audio_track = val()?.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).filter(|&n| n < 64).ok_or("--audio-track은 1~64")?;
            }
            "--game-voice" => o.game_voice = speakers::GameVoice::parse(&val()?).ok_or("--game-voice는 drop, label 또는 keep")?,
            "--per-speaker" => o.per_speaker = true,
            "--only-voices" => o.only_voices = true,
            "--no-voices" => o.use_voices = false,
            "--clean" => o.clean_fillers = true,
            "--keep-fillers" => o.clean_fillers = false,
            "--from" | "--to" => {
                let v = val()?;
                let t = voices::parse_time(&v).and_then(|t| i64::try_from(t).ok()).ok_or_else(|| format!("{a} 뒤에 시간 (예: 1:23)"))?;
                let (f, e) = o.range.unwrap_or((0, i64::MAX / 4));
                o.range = Some(if a == "--from" { (t, e) } else { (f, t) });
            }
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
    let who = job::speaker_summary(&done);
    if !who.is_empty() {
        println!("{who}");
    }
    for p in &done.speaker_srts {
        println!("화자별 자막 → {}", p.display());
    }
    Ok(())
}

fn voice(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("add") => {
            let name = args.get(1).ok_or(USAGE)?;
            let file = args.get(2).map(PathBuf::from).ok_or(USAGE)?;
            let (mut from, mut to) = (None, None);
            let mut it = args.iter().skip(3);
            while let Some(a) = it.next() {
                let t = it.next().and_then(|v| voices::parse_time(v)).ok_or_else(|| format!("{a} 뒤에 시간 (예: 1:23)"))?;
                match a.as_str() {
                    "--from" => from = Some(t),
                    "--to" => to = Some(t),
                    s => return Err(format!("모르는 옵션: {s}")),
                }
            }
            let v = job::guarded(|| voices::enroll(name, &file, from, to))?;
            println!("목소리 '{}' 등록 ({:.0}초 샘플) → {}", v.name, v.seconds, voices::dir().display());
            Ok(())
        }
        Some("list") => {
            let l = voices::list();
            if l.is_empty() {
                println!("등록한 목소리가 없습니다. overfilm-subs voice add <이름> <파일> [--from 1:23] [--to 1:45]");
            }
            for v in l {
                println!("{} ({:.0}초 샘플, {})", v.name, v.seconds, v.source);
            }
            Ok(())
        }
        Some("remove") => {
            let name = args.get(1).ok_or(USAGE)?;
            voices::remove_in(&voices::dir(), name)?;
            println!("목소리 '{name}' 지움");
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
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

/// `redo 자막.srt --from 1:23 --to 1:40 [--input 영상] [--terms …]`: recognise a part again.
fn redo_cmd(args: &[String]) -> Result<(), String> {
    let mut it = args.iter();
    let (mut srt, mut input, mut from, mut to) = (None, None, None, None);
    let mut o = job::Options::new(PathBuf::new());
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or_else(|| format!("{a} 뒤에 값이 필요합니다"));
        match a.as_str() {
            "--input" => input = Some(PathBuf::from(val()?)),
            "--terms" => o.extra_terms = val()?,
            "--from" => from = Some(review::parse_clock(&val()?).ok_or("--from 시간 형식 (예: 00:01:23,000 또는 1:23.0)")?),
            "--to" => to = Some(review::parse_clock(&val()?).ok_or("--to 시간 형식")?),
            "--cpu" => o.gpu = false,
            "--clean" => o.clean_fillers = true,
            "--keep-fillers" => o.clean_fillers = false,
            s if s.starts_with('-') => return Err(format!("모르는 옵션: {s}")),
            s => srt = Some(PathBuf::from(s)),
        }
    }
    let srt = srt.ok_or(USAGE)?;
    let (from, to) = (from.ok_or("--from이 필요합니다")?, to.ok_or("--to가 필요합니다")?);
    o.input = match input {
        Some(p) => p,
        None => overfilm_subs::MEDIA_EXTENSIONS
            .iter()
            .map(|e| srt.with_extension(e))
            .find(|p| p.is_file())
            .ok_or("원본 영상을 찾지 못했습니다. --input으로 알려 주세요.")?,
    };
    let d = mcp::user_glossary_dir();
    o.glossary_dir = d.is_dir().then_some(d);
    let mut p = progress_bar();
    let words = job::guarded(|| job::recognise_span(&o, from, to, &mut p))?;
    eprintln!();
    let mut opened = overfilm_subs::edit::Opened::load(&srt)?;
    let n = opened.apply_redo(&o, from, to, words)?;
    println!("{}–{} 다시 인식: 자막 {n}개 → {}", review::clock(from), review::clock(to), srt.display());
    Ok(())
}

fn export_cmd(args: &[String]) -> Result<(), String> {
    let mut it = args.iter();
    let mut input = None;
    let mut out = None;
    let mut e = export::Export::default();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or_else(|| format!("{a} 뒤에 값이 필요합니다"));
        match a.as_str() {
            "-o" | "--output" => out = Some(PathBuf::from(val()?)),
            "--format" | "-f" => e.format = export::Format::parse(&val()?).ok_or("--format은 srt, vtt, ass 또는 txt")?,
            "--fps" => e.fps = Some(export::parse_fps(&val()?).ok_or("--fps는 23.976, 24, 25, 29.97, 30, 50, 59.94, 60 중 하나")?),
            "--gap" => e.gap_frames = val()?.parse().map_err(|_| "--gap은 프레임 수")?,
            s if s.starts_with('-') => return Err(format!("모르는 옵션: {s}")),
            s => input = Some(PathBuf::from(s)),
        }
    }
    let input = input.ok_or(USAGE)?;
    if e.gap_frames > 0 && e.fps.is_none() {
        return Err("--gap에는 --fps가 필요합니다".into());
    }
    let caps = job::read_srt(&input)?;
    let out = out.unwrap_or_else(|| input.with_extension(e.format.extension()));
    if out == input {
        return Err("원본과 같은 파일에는 내보내지 않습니다. -o로 다른 이름을 주세요.".into());
    }
    export::write(&out, &caps, &e)?;
    println!("자막 {}개 → {}", caps.len(), out.display());
    Ok(())
}

fn relayout(args: &[String]) -> Result<(), String> {
    let mut it = args.iter();
    let mut input = None;
    let mut out = None;
    let mut o = job::Options::new(PathBuf::new());
    let mut renames: Vec<(String, String)> = Vec::new();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--output" => out = it.next().map(PathBuf::from),
            "--game-voice" => {
                o.game_voice = it.next().and_then(|v| speakers::GameVoice::parse(v)).ok_or("--game-voice는 drop, label 또는 keep")?;
            }
            "--rename" => {
                let v = it.next().ok_or("--rename 화자1=이름")?;
                let (from, to) = v.split_once('=').ok_or("--rename 화자1=이름")?;
                renames.push((from.trim().to_string(), to.trim().to_string()));
            }
            "--line-chars" => o.line_chars = it.next().and_then(|v| v.parse().ok()).ok_or("--line-chars는 숫자")?,
            "--one-line" => o.max_lines = 1,
            "--cpu" => o.gpu = false,
            "--clean" => o.clean_fillers = true,
            "--keep-fillers" => o.clean_fillers = false,
            s => input = Some(PathBuf::from(s)),
        }
    }
    let input = input.ok_or(USAGE)?;
    if std::fs::metadata(&input).map(|m| m.len()).unwrap_or(0) > 256 << 20 {
        return Err("파일이 너무 큽니다".into());
    }
    let text = std::fs::read_to_string(&input).map_err(|e| format!("{}: {e}", input.display()))?;
    let mut t: overfilm_asr::Transcript = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", input.display()))?;
    if !renames.is_empty() {
        for (from, to) in &renames {
            let s = t.speakers.iter_mut().find(|s| &s.label == from).ok_or_else(|| format!("{from}: 그런 화자가 없습니다"))?;
            s.label = to.chars().take(20).collect();
        }
        let json = serde_json::to_string_pretty(&t).map_err(|e| e.to_string())?;
        std::fs::write(&input, json).map_err(|e| format!("{}: {e}", input.display()))?;
    }
    let caps = speakers::captions_keep(&t, o.game_voice, speakers::Keep::All, &o.layout(), o.clean_fillers);
    let name = input.file_name().map(|n| n.to_string_lossy().replace(".overfilm.json", ".srt")).unwrap_or_else(|| "out.srt".into());
    let out = out.unwrap_or_else(|| input.with_file_name(name));
    job::write_srt(&out, &caps)?;
    println!("자막 {}개 → {}", caps.len(), out.display());
    Ok(())
}

/// Check the GPU against the CPU: every kernel, tiny networks, then (unless `--quick`) the real
/// recogniser.
fn gpu_check(args: &[String]) -> Result<(), String> {
    let ctx = overfilm_asr::gpu::Ctx::with(args.iter().any(|a| a == "--software")).map_err(|e| e.to_string())?;
    println!("GPU: {}", ctx.name);
    if !ctx.tiled() {
        println!("참고: 이 그래픽카드에서는 타일 곱셈이 틀려서 줄 단위 곱셈으로 대신합니다.");
    }
    let mut all_ok = true;
    for r in overfilm_asr::gpu::check::report(&ctx) {
        all_ok &= r.ok();
        println!("{r}");
    }
    let models = job::models_dir();
    if !args.iter().any(|a| a == "--quick") && overfilm_asr::catalogue::ASR.installed(&models) {
        println!("실제 모델 비교 중 (CPU와 GPU에 차례로 불러옵니다, 몇 분 걸립니다)...");
        let dir = overfilm_asr::catalogue::ASR.dir(&models);
        let reports = job::guarded(|| overfilm_asr::gpu_model_check(&dir, true, &ctx).map_err(|e| e.to_string()))?;
        for r in reports {
            all_ok &= r.ok();
            println!("{r}");
        }
    }
    if all_ok {
        println!("모두 통과: GPU를 써도 됩니다.");
        Ok(())
    } else {
        Err("GPU 결과가 CPU와 다릅니다. 이 화면을 그대로 보내 주세요. (자막 추출은 자동으로 CPU를 씁니다)".into())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("extract") => extract(args.get(1..).unwrap_or_default()),
        Some("review") => review_cmd(args.get(1..).unwrap_or_default()),
        Some("fix") => fix(args.get(1..).unwrap_or_default()),
        Some("relayout") => relayout(args.get(1..).unwrap_or_default()),
        Some("export") => export_cmd(args.get(1..).unwrap_or_default()),
        Some("redo") => redo_cmd(args.get(1..).unwrap_or_default()),
        Some("voice") => voice(args.get(1..).unwrap_or_default()),
        Some("models") => {
            let mut p = progress_bar();
            let r = job::ensure_models(&job::models_dir(), &mut p);
            eprintln!();
            r.map(|()| println!("모델 준비됨: {}", job::models_dir().display()))
        }
        Some("mcp") => mcp::serve(),
        Some("gpu-check") => gpu_check(args.get(1..).unwrap_or_default()),
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
