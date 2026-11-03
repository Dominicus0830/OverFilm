//! Caption files for other tools: SubRip (Premiere), WebVTT (YouTube, web players), ASS (styled,
//! one colour per speaker) and a plain transcript, with optional snapping to video frames and a
//! minimum gap between captions, as editors expect.

use std::path::Path;

use overfilm_captions::Caption;
use serde::{Deserialize, Serialize};

use crate::edit::caption_speakers;

/// File kinds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[default]
    Srt,
    Vtt,
    Ass,
    Txt,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Srt, Format::Vtt, Format::Ass, Format::Txt];

    pub fn parse(s: &str) -> Option<Format> {
        match s.trim().trim_start_matches('.').to_ascii_lowercase().as_str() {
            "srt" => Some(Format::Srt),
            "vtt" | "webvtt" => Some(Format::Vtt),
            "ass" | "ssa" => Some(Format::Ass),
            "txt" | "text" => Some(Format::Txt),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Srt => "srt",
            Format::Vtt => "vtt",
            Format::Ass => "ass",
            Format::Txt => "txt",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Srt => "SRT (Premiere, 대부분의 편집기)",
            Format::Vtt => "WebVTT (유튜브, 웹 플레이어)",
            Format::Ass => "ASS (글꼴·색, 화자마다 색)",
            Format::Txt => "대본 TXT (시간, 화자, 말)",
        }
    }
}

/// Frame rates offered, as (numerator, denominator) frames per second.
pub const FRAME_RATES: [(u32, u32); 8] = [(24000, 1001), (24, 1), (25, 1), (30000, 1001), (30, 1), (50, 1), (60000, 1001), (60, 1)];

pub fn fps_label(fps: (u32, u32)) -> String {
    let v = f64::from(fps.0) / f64::from(fps.1.max(1));
    if fps.1 == 1 {
        format!("{v:.0}fps")
    } else {
        let t = format!("{v:.3}");
        format!("{}fps", t.trim_end_matches('0').trim_end_matches('.'))
    }
}

/// `29.97`, `29.97fps`, `30` … as one of [`FRAME_RATES`].
pub fn parse_fps(s: &str) -> Option<(u32, u32)> {
    let s = s.trim().trim_end_matches("fps").trim();
    FRAME_RATES.iter().copied().find(|&f| fps_label(f).trim_end_matches("fps") == s || (s == "23.98" && f == (24000, 1001)))
}

/// How to write the file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Export {
    pub format: Format,
    /// Snap times to this frame rate (numerator, denominator).
    #[serde(default)]
    pub fps: Option<(u32, u32)>,
    /// Frames kept empty between two captions (0: captions may touch). Needs `fps`.
    #[serde(default)]
    pub gap_frames: u32,
}

fn frame_of(ms: i64, (num, den): (u32, u32)) -> i64 {
    // nearest frame: ms * num / (den * 1000), rounded
    let n = i128::from(ms) * i128::from(num);
    let d = i128::from(den.max(1)) * 1000;
    i64::try_from((n * 2 + d) / (d * 2)).unwrap_or(i64::MAX)
}

fn ms_of_frame(f: i64, (num, den): (u32, u32)) -> i64 {
    let n = i128::from(f) * i128::from(den.max(1)) * 1000;
    let d = i128::from(num.max(1));
    i64::try_from((n * 2 + d) / (d * 2)).unwrap_or(i64::MAX)
}

/// Times on frame boundaries, at least one frame long, with `gap` frames between captions.
pub fn snap(caps: &[Caption], fps: (u32, u32), gap: u32) -> Vec<Caption> {
    if fps.0 == 0 || fps.1 == 0 {
        return caps.to_vec();
    }
    let mut frames: Vec<(i64, i64)> = caps
        .iter()
        .map(|c| {
            let a = frame_of(c.start_ms.max(0), fps);
            (a, frame_of(c.end_ms.max(0), fps).max(a + 1))
        })
        .collect();
    let gap = i64::from(gap.min(30));
    for i in 0..frames.len() {
        let next = frames.get(i + 1).map(|f| f.0);
        if let (Some(f), Some(ns)) = (frames.get_mut(i), next)
            && f.1 > ns - gap
        {
            // shorten this caption to leave the gap, but never below one frame
            f.1 = (ns - gap).max(f.0 + 1);
        }
    }
    caps.iter().zip(frames).map(|(c, (a, b))| Caption { start_ms: ms_of_frame(a, fps), end_ms: ms_of_frame(b, fps), lines: c.lines.clone() }).collect()
}

fn clock(ms: i64, sep: char) -> String {
    let ms = ms.max(0);
    format!("{:02}:{:02}:{:02}{sep}{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

pub fn to_vtt(caps: &[Caption]) -> String {
    let mut s = String::from("WEBVTT\n\n");
    for c in caps {
        // "-->" inside a cue would end it early
        let text = c.text().replace("-->", "→");
        s.push_str(&format!("{} --> {}\n{text}\n\n", clock(c.start_ms, '.'), clock(c.end_ms, '.')));
    }
    s
}

/// ASS colours (`&H00BBGGRR`) for speakers in order of appearance.
const ASS_COLOURS: [&str; 6] = ["&H00FFFFFF", "&H0066E6FF", "&H00FFD27A", "&H009BF59B", "&H00C8A0FF", "&H0080B4FF"];

fn ass_time(ms: i64) -> String {
    let cs = ms.max(0) / 10;
    format!("{}:{:02}:{:02}.{:02}", cs / 360_000, cs / 6000 % 60, cs / 100 % 60, cs % 100)
}

fn ass_text(s: &str) -> String {
    s.replace('\\', "＼").replace('{', "(").replace('}', ")").replace('\n', "\\N")
}

fn ass_name(s: &str) -> String {
    s.replace([',', '\n', '\r'], " ").chars().take(40).collect()
}

/// ASS with one style per speaker: the `(이름)` labels become the speaker's colour and name field.
pub fn to_ass(caps: &[Caption], font: &str) -> String {
    let who = caption_speakers(caps);
    let mut speakers: Vec<String> = Vec::new();
    for (w, _) in &who {
        if let Some(w) = w
            && !speakers.contains(w)
            && speakers.len() < 40
        {
            speakers.push(w.clone());
        }
    }
    let font = ass_name(font);
    let style =
        |name: &str, colour: &str| format!("Style: {name},{font},64,{colour},&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,3,1,2,60,60,60,1\n");
    let mut s = String::from(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\nWrapStyle: 2\nScaledBorderAndShadow: yes\n\n[V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n",
    );
    s.push_str(&style("Default", ASS_COLOURS[0]));
    for k in 0..speakers.len() {
        s.push_str(&style(&format!("S{}", k + 1), ASS_COLOURS.get(k % ASS_COLOURS.len()).copied().unwrap_or("&H00FFFFFF")));
    }
    s.push_str("\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");
    for (c, (w, text)) in caps.iter().zip(&who) {
        let (style, name) = match w.as_ref().and_then(|w| speakers.iter().position(|x| x == w)) {
            Some(k) => (format!("S{}", k + 1), w.clone().map(|w| ass_name(&w)).unwrap_or_default()),
            None => ("Default".to_string(), String::new()),
        };
        s.push_str(&format!("Dialogue: 0,{},{},{style},{name},0,0,0,,{}\n", ass_time(c.start_ms), ass_time(c.end_ms), ass_text(text)));
    }
    s
}

/// A readable transcript: `[00:01:02] 화자1: 말`, a line per caption.
pub fn to_txt(caps: &[Caption]) -> String {
    let mut s = String::new();
    for (c, (w, text)) in caps.iter().zip(caption_speakers(caps)) {
        let t = clock(c.start_ms, ',');
        let t = t.split(',').next().unwrap_or_default();
        let text = text.replace('\n', " ");
        match w {
            Some(w) => s.push_str(&format!("[{t}] {w}: {text}\n")),
            None => s.push_str(&format!("[{t}] {text}\n")),
        }
    }
    s
}

/// The file text for `e`.
pub fn render(caps: &[Caption], e: &Export) -> String {
    let caps = match e.fps {
        Some(fps) => snap(caps, fps, e.gap_frames),
        None => caps.to_vec(),
    };
    match e.format {
        Format::Srt => overfilm_captions::to_srt(&caps),
        Format::Vtt => to_vtt(&caps),
        Format::Ass => to_ass(&caps, "Malgun Gothic"),
        Format::Txt => to_txt(&caps),
    }
}

/// Write `caps` to `path` as `e` says (UTF-8 with BOM for SRT, TXT and ASS, which Windows tools
/// need to see Korean; WebVTT must start with its signature, so none there).
pub fn write(path: &Path, caps: &[Caption], e: &Export) -> Result<(), String> {
    let mut s = if e.format == Format::Vtt { String::new() } else { String::from("\u{feff}") };
    s.push_str(&render(caps, e));
    std::fs::write(path, s).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(s: i64, e: i64, t: &str) -> Caption {
        Caption { start_ms: s, end_ms: e, lines: t.lines().map(String::from).collect() }
    }

    #[test]
    fn formats() {
        let caps = vec![cap(1000, 2500, "(화자1) 안녕\n하세요"), cap(2500, 4000, "반가워요 {x}"), cap(4100, 5000, "(아이치) 네")];
        let v = to_vtt(&caps);
        assert!(v.starts_with("WEBVTT\n\n00:00:01.000 --> 00:00:02.500\n(화자1) 안녕\n하세요\n\n"), "{v}");
        let a = to_ass(&caps, "Malgun Gothic");
        assert!(a.contains("Style: S1,Malgun Gothic,64,&H00FFFFFF"), "{a}");
        assert!(a.contains("Dialogue: 0,0:00:01.00,0:00:02.50,S1,화자1,0,0,0,,안녕\\N하세요\n"), "{a}");
        assert!(a.contains("Dialogue: 0,0:00:02.50,0:00:04.00,S1,화자1,0,0,0,,반가워요 (x)\n"), "label carries over: {a}");
        assert!(a.contains(",S2,아이치,"));
        let t = to_txt(&caps);
        assert_eq!(t.lines().next(), Some("[00:00:01] 화자1: 안녕 하세요"));
        assert_eq!(Format::parse(".VTT"), Some(Format::Vtt));
        assert_eq!(Format::parse("doc"), None);
        assert_eq!(fps_label((30000, 1001)), "29.97fps");
        assert_eq!(fps_label((24000, 1001)), "23.976fps");
        assert_eq!(fps_label((60, 1)), "60fps");
        assert_eq!(parse_fps("29.97"), Some((30000, 1001)));
        assert_eq!(parse_fps("60fps"), Some((60, 1)));
        assert_eq!(parse_fps("23.98"), Some((24000, 1001)));
        assert_eq!(parse_fps("31"), None);
    }

    #[test]
    fn frames_and_gaps() {
        let caps = vec![cap(1001, 2000, "가"), cap(2010, 3000, "나"), cap(3000, 3001, "다")];
        let s = snap(&caps, (30000, 1001), 2);
        // 1001 ms at 29.97 fps is frame 30 = 1001 ms; 2010 ms is frame 60 = 2002 ms
        assert_eq!(s[0].start_ms, 1001);
        assert_eq!(s[1].start_ms, 2002);
        assert_eq!(s[0].end_ms, ms_of_frame(58, (30000, 1001)), "two frames before the next");
        assert!(s.iter().all(|c| c.end_ms > c.start_ms), "{s:?}");
        let same = snap(&caps, (0, 1), 2);
        assert_eq!(same, caps);
        // hostile numbers do not overflow
        let _ = snap(&[cap(i64::MAX, i64::MAX, "x"), cap(-5, -1, "y")], (60000, 1001), 99);
    }

    #[test]
    fn writes_with_bom_except_vtt() {
        let d = tempfile::tempdir().unwrap();
        let caps = vec![cap(0, 900, "가")];
        let p = d.path().join("a.vtt");
        write(&p, &caps, &Export { format: Format::Vtt, ..Default::default() }).unwrap();
        assert!(std::fs::read(&p).unwrap().starts_with(b"WEBVTT"));
        let p = d.path().join("a.srt");
        write(&p, &caps, &Export { format: Format::Srt, fps: Some((25, 1)), gap_frames: 0 }).unwrap();
        assert_eq!(crate::job::read_srt(&p).unwrap()[0].end_ms, 920, "900 ms is frame 22.5, rounded up");
    }
}
