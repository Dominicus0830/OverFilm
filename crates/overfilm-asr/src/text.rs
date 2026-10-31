//! Text handling around the models: the ASR output's `language X<asr_text>` header, the
//! repetition guard, the aligner's word units, and the aligner's monotonic timestamp repair.
//! Ported from the reference wrapper (`qwen_asr/inference/utils.py` and
//! `qwen3_forced_aligner.py`, Apache-2.0).

/// Runs of one character longer than this collapse to one character; a pattern repeated this many
/// times is kept once.
const REPEAT_THRESHOLD: usize = 20;
/// Longest repeated pattern looked for.
const REPEAT_MAX_LEN: usize = 20;

/// The transcription inside a raw ASR output (header removed when present, repetitions fixed).
pub fn parse_output(raw: &str) -> String {
    let s = fix_repetitions(raw.trim());
    match s.split_once("<asr_text>") {
        Some((meta, text)) if meta.to_lowercase().contains("language none") && text.trim().is_empty() => String::new(),
        Some((_, text)) => text.trim().to_string(),
        None => s.trim().to_string(),
    }
}

fn fix_char_repeats(s: &[char]) -> Vec<char> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while let Some(&c) = s.get(i) {
        let run = s.get(i..).map_or(0, |r| r.iter().take_while(|&&x| x == c).count()).max(1);
        if run > REPEAT_THRESHOLD {
            out.push(c);
        } else {
            out.extend(std::iter::repeat_n(c, run));
        }
        i += run;
    }
    out
}

/// A pattern of `k` chars at `i` repeated at least the threshold times: the end of its repeats.
fn repeat_end(s: &[char], i: usize, k: usize) -> Option<usize> {
    let pat = s.get(i..i + k)?;
    let mut end = i + k;
    let mut reps = 1;
    while s.get(end..end + k) == Some(pat) {
        end += k;
        reps += 1;
    }
    (reps >= REPEAT_THRESHOLD).then_some(end)
}

fn fix_pattern_repeats(s: &[char]) -> Vec<char> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let found = (1..=REPEAT_MAX_LEN).take_while(|k| i + k * REPEAT_THRESHOLD <= s.len()).find_map(|k| repeat_end(s, i, k).map(|e| (k, e)));
        match found {
            Some((k, end)) => {
                out.extend_from_slice(s.get(i..i + k).unwrap_or(&[]));
                i = end;
            }
            None => {
                if let Some(&c) = s.get(i) {
                    out.push(c);
                }
                i += 1;
            }
        }
    }
    out
}

/// Collapse runaway repetitions (a decoder stuck on one syllable or phrase).
pub fn fix_repetitions(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    fix_pattern_repeats(&fix_char_repeats(&chars)).into_iter().collect()
}

fn kept(c: char) -> bool {
    c == '\'' || c.is_alphanumeric()
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x20000..=0x2A6DF | 0x2A700..=0x2CEAF | 0xF900..=0xFAFF)
}

/// The aligner units of one whitespace-separated word.
fn units_of(w: &str, out: &mut Vec<String>) {
    let mut buf = String::new();
    for c in w.chars().filter(|&c| kept(c)) {
        if is_cjk(c) {
            if !buf.is_empty() {
                out.push(std::mem::take(&mut buf));
            }
            out.push(c.to_string());
        } else {
            buf.push(c);
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
}

/// Each whitespace-separated word of `text` with its number of aligner units (0 for
/// punctuation-only words).
pub fn word_units(text: &str) -> Vec<(&str, usize)> {
    text.split_whitespace()
        .map(|w| {
            let mut v = Vec::new();
            units_of(w, &mut v);
            (w, v.len())
        })
        .collect()
}

/// Aligner units of a transcript: space-separated words (Korean eojeol) with punctuation removed,
/// CJK ideographs one per unit.
pub fn align_units(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for w in text.split_whitespace() {
        units_of(w, &mut out);
    }
    out
}

fn nearest_normal(v: &[f64], normal: &[bool], from: usize, back: bool) -> Option<f64> {
    if back {
        (0..from).rev().find(|&k| normal.get(k) == Some(&true)).and_then(|k| v.get(k).copied())
    } else {
        (from..v.len()).find(|&k| normal.get(k) == Some(&true)).and_then(|k| v.get(k).copied())
    }
}

/// Longest non-decreasing subsequence membership (O(n²), n is a few hundred per chunk).
fn lis_members(v: &[f64]) -> Vec<bool> {
    let n = v.len();
    let mut dp = vec![1usize; n];
    let mut parent = vec![usize::MAX; n];
    for i in 1..n {
        for j in 0..i {
            if v[j] <= v[i] && dp[j] + 1 > dp[i] {
                dp[i] = dp[j] + 1;
                parent[i] = j;
            }
        }
    }
    let mut normal = vec![false; n];
    let best = (0..n).fold(None, |b: Option<usize>, i| match b {
        Some(bi) if dp[bi] >= dp[i] => Some(bi),
        _ => Some(i),
    });
    let mut idx = best.unwrap_or(usize::MAX);
    while let Some(slot) = normal.get_mut(idx) {
        *slot = true;
        idx = parent.get(idx).copied().unwrap_or(usize::MAX);
    }
    normal
}

/// Repair aligner timestamps that go backwards: keep the longest non-decreasing run, fill short
/// gaps (≤ 2) from the nearer neighbour and longer ones by linear interpolation.
pub fn fix_timestamps(data: &[f64]) -> Vec<f64> {
    let n = data.len();
    let normal = lis_members(data);
    let mut out = data.to_vec();
    let mut i = 0;
    while i < n {
        if normal[i] {
            i += 1;
            continue;
        }
        let j = (i..n).find(|&k| normal[k]).unwrap_or(n);
        let left = nearest_normal(&out, &normal, i, true);
        let right = nearest_normal(&out, &normal, j, false);
        let count = j - i;
        for k in i..j {
            out[k] = match (left, right) {
                (Some(l), Some(r)) if count > 2 => l + (r - l) / (count + 1) as f64 * (k - i + 1) as f64,
                (Some(l), Some(r)) => {
                    if k + 1 - i <= j - k {
                        l
                    } else {
                        r
                    }
                }
                (Some(l), None) => l,
                (None, Some(r)) => r,
                (None, None) => out[k],
            };
        }
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header() {
        assert_eq!(parse_output("language Korean<asr_text>안녕하세요."), "안녕하세요.");
        assert_eq!(parse_output("language None<asr_text>"), "");
        assert_eq!(parse_output(" 그냥 텍스트 "), "그냥 텍스트");
    }

    #[test]
    fn collapses_repetitions() {
        let s = format!("앗{}끝", "아".repeat(40));
        assert_eq!(fix_repetitions(&s), "앗아끝");
        let s = format!("시작{}끝", "와우 ".repeat(30));
        assert_eq!(fix_repetitions(&s), "시작와우 끝");
        assert_eq!(fix_repetitions("정상 문장입니다"), "정상 문장입니다");
    }

    #[test]
    fn units() {
        assert_eq!(align_units("겐지가, 용검을 써요! D.Va 76"), vec!["겐지가", "용검을", "써요", "DVa", "76"]);
        assert_eq!(align_units("你好 world"), vec!["你", "好", "world"]);
        assert!(align_units("... !!").is_empty());
    }

    #[test]
    fn timestamps_repaired() {
        // reference behaviour: one dip filled from the nearer neighbour
        assert_eq!(fix_timestamps(&[0.0, 80.0, 40.0, 160.0, 240.0]), vec![0.0, 80.0, 80.0, 160.0, 240.0]);
        // a long anomaly is interpolated
        let v = fix_timestamps(&[0.0, 900.0, 900.0, 900.0, 100.0, 200.0, 300.0, 400.0]);
        assert!(v.windows(2).all(|w| w[0] <= w[1]), "{v:?}");
        assert!(fix_timestamps(&[]).is_empty());
    }
}
