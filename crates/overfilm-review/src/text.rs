//! Small text helpers shared by the question builders.

use serde_json::Value;

/// `m:ss.d` (or `h:mm:ss.d`) of a millisecond time, for people and for the prompt.
pub fn clock(ms: i64) -> String {
    let ms = ms.max(0);
    let tenths = (ms / 100) % 10;
    let secs = ms / 1000;
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}.{tenths}") } else { format!("{m}:{s:02}.{tenths}") }
}

/// Display width in half-cells: Hangul, CJK and other wide letters 2, ASCII and narrow ones 1
/// (a Korean caption line of 16 letters is 32).
pub fn display_width(s: &str) -> u32 {
    s.chars().map(|c| if is_wide(c) { 2u32 } else { 1 }).fold(0u32, u32::saturating_add)
}

fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xA960..=0xA97F | 0xAC00..=0xD7A3 | 0xD7B0..=0xD7FF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6)
}

/// Letters and digits only, lowercased: two texts that differ only in spacing and punctuation
/// compare equal.
pub fn bare(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// At most `n` characters of `s` (an ellipsis when cut).
pub fn cap(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut t: String = s.chars().take(n).collect();
    t.push('…');
    t
}

/// The JSON object in a reply that may wrap it in prose or a ```json fence: the whole text when
/// it parses, else the first `{` to its matching `}`.
pub fn extract_json(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t)
        && v.is_object()
    {
        return Some(v);
    }
    let start = t.find('{')?;
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, c) in t.char_indices().skip_while(|(i, _)| *i < start) {
        if in_str {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth = depth.saturating_add(1),
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return t.get(start..i + c.len_utf8()).and_then(|s| serde_json::from_str::<Value>(s).ok()).filter(Value::is_object);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks() {
        assert_eq!(clock(0), "0:00.0");
        assert_eq!(clock(61_250), "1:01.2");
        assert_eq!(clock(3_725_900), "1:02:05.9");
        assert_eq!(clock(-5), "0:00.0");
        assert!(!clock(i64::MAX).is_empty());
    }

    #[test]
    fn widths() {
        assert_eq!(display_width("겐지 ok"), 7);
        assert_eq!(display_width(""), 0);
    }

    #[test]
    fn json_in_prose() {
        assert_eq!(extract_json(r#"{"a":1}"#).unwrap()["a"], 1);
        let v = extract_json("답입니다:\n```json\n{\"a\": \"}{\", \"b\": {\"c\": 2}}\n```\n끝").unwrap();
        assert_eq!(v["b"]["c"], 2);
        assert_eq!(v["a"], "}{");
        assert!(extract_json("no json here").is_none());
        assert!(extract_json("{ broken").is_none());
        assert!(extract_json("[1,2]").is_none());
    }

    #[test]
    fn bare_and_cap() {
        assert_eq!(bare("겐지, 나이스!"), "겐지나이스");
        assert_eq!(cap("가나다라", 2), "가나…");
        assert_eq!(cap("가나", 2), "가나");
    }
}
