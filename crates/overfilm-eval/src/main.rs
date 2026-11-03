//! `overfilm-eval [--terms FILE] [--json] <reference> <hypothesis>`: score a transcript (or a
//! directory of transcripts) against a reference. See the library docs and `OVERFILM.md`.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::path::PathBuf;
use std::process::ExitCode;

use overfilm_eval::{Report, evaluate_corrected, load_glossary, load_terms};
use overfilm_glossary::Corrector;

const USAGE: &str = "usage: overfilm-eval [--terms PATH] [--correct DIR] [--json] <reference> <hypothesis>

  <reference> <hypothesis>  two transcript files (.srt .vtt .ttml .dfxp or plain .txt), or two
                            directories whose files pair up by name without extension
  --terms PATH              game-term list, one term per line (`canonical | other spelling`),
                            or a glossary directory of .tsv tables (packs/overwatch/ko/glossary)
  --correct DIR             correct each hypothesis with the glossary in DIR before scoring
                            (misrecognitions to glossary spellings, profanity to initials)
  --json                    print the full report as JSON";

struct Args {
    reference: PathBuf,
    hyp: PathBuf,
    terms: Option<PathBuf>,
    correct: Option<PathBuf>,
    json: bool,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut paths = Vec::new();
    let mut terms = None;
    let mut correct = None;
    let mut json = false;
    let mut args = args;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--terms" => terms = Some(PathBuf::from(args.next().ok_or("--terms needs a file")?)),
            "--correct" => correct = Some(PathBuf::from(args.next().ok_or("--correct needs a glossary directory")?)),
            "--json" => json = true,
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ => paths.push(PathBuf::from(a)),
        }
    }
    let mut paths = paths.into_iter();
    match (paths.next(), paths.next(), paths.next()) {
        (Some(reference), Some(hyp), None) => Ok(Args { reference, hyp, terms, correct, json }),
        _ => Err("expected a reference and a hypothesis".into()),
    }
}

fn pct(x: f64) -> String {
    format!("{:6.2}%", x * 100.0)
}

fn print_table(r: &Report) {
    let terms = r.term_accuracy.is_some();
    println!("{:<24} {:>8} {:>8} {:>7} {:>9}", "file", "CER", "WER", "chars", if terms { "terms" } else { "" });
    for f in &r.files {
        let t = f.term_accuracy.map(pct).unwrap_or_default();
        println!("{:<24} {:>8} {:>8} {:>7} {:>9}", f.name, pct(f.cer), pct(f.wer), f.score.chars.reference, t);
    }
    let t = r.term_accuracy.map(pct).unwrap_or_default();
    println!("{:<24} {:>8} {:>8} {:>7} {:>9}", "TOTAL", pct(r.cer), pct(r.wer), r.total.chars.reference, t);
    let c = &r.total.chars;
    println!("\nchars: {} substitutions, {} deletions, {} insertions", c.substitutions, c.deletions, c.insertions);
    if let Some(t) = &r.total.terms {
        println!("terms: {} of {} recognized ({} in hypothesis)", t.matched, t.reference, t.hypothesis);
        for m in &t.mismatched {
            println!("  {}: reference {}, hypothesis {}", m.term, m.reference, m.hypothesis);
        }
    }
    if !r.missing.is_empty() {
        println!("no hypothesis for: {}", r.missing.join(", "));
    }
}

fn run(a: &Args) -> Result<(), String> {
    let terms = a.terms.as_deref().map(load_terms).transpose().map_err(|e| e.to_string())?;
    let corrector = a.correct.as_deref().map(load_glossary).transpose().map_err(|e| e.to_string())?.map(|g| Corrector::new(&g).fuzzy(&g).korean());
    let report = evaluate_corrected(&a.reference, &a.hyp, terms.as_ref(), corrector.as_ref()).map_err(|e| e.to_string())?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
    } else {
        print_table(&report);
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("overfilm-eval: {e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("overfilm-eval: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Result<Args, String> {
        parse_args(s.iter().map(|a| a.to_string()))
    }

    #[test]
    fn parses_options_anywhere() {
        let a = args(&["--json", "r.srt", "--terms", "t.txt", "h.srt", "--correct", "g"]).unwrap();
        assert_eq!(a.correct, Some(PathBuf::from("g")));
        assert!(a.json);
        assert_eq!(a.terms, Some(PathBuf::from("t.txt")));
        assert_eq!((a.reference, a.hyp), (PathBuf::from("r.srt"), PathBuf::from("h.srt")));
    }

    #[test]
    fn rejects_bad_args() {
        assert!(args(&["r.srt"]).is_err());
        assert!(args(&["a", "b", "c"]).is_err());
        assert!(args(&["a", "b", "--terms"]).is_err());
        assert!(args(&["a", "b", "--correct"]).is_err());
        assert!(args(&["--bogus", "a", "b"]).is_err());
    }
}
