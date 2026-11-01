//! Learning from the user's own fixes: each time a caption's text is corrected by hand, the words
//! that changed are counted (`전국퀸` → `정커퀸`). Once the same fix has been made [`ASK_AT`]
//! times, the window (or the assistant) offers to put it in the user glossary, so the next
//! videos come out right. Counts are kept in `learned.json` beside the models.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Times the same fix is made before it is offered for the glossary.
pub const ASK_AT: u32 = 3;
/// Longest fix learned, in words on either side.
const MAX_WORDS: usize = 3;
/// Most words of a caption compared (longer texts are not diffed).
const MAX_TOKENS: usize = 200;
/// Most fixes remembered.
const MAX_PAIRS: usize = 2000;
const MAX_FILE: u64 = 4 << 20;

/// One fix and how often it was made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pair {
    pub from: String,
    pub to: String,
    pub count: u32,
    /// The user said not to offer it again.
    #[serde(default)]
    pub dismissed: bool,
    /// Already put in the glossary.
    #[serde(default)]
    pub added: bool,
}

impl Pair {
    pub fn open(&self) -> bool {
        !self.dismissed && !self.added
    }
}

pub fn file() -> PathBuf {
    if cfg!(test) { std::env::temp_dir().join("overfilm-test-learned.json") } else { crate::edit::data_dir().join("learned.json") }
}

fn strip(s: &str) -> &str {
    s.trim_matches(|c: char| !c.is_alphanumeric())
}

/// The word spans that differ between `old` and `new`, as `(old words, new words)`: runs of at
/// most [`MAX_WORDS`] words on each side, both sides non-empty, punctuation-only edits left out.
pub fn diff(old: &str, new: &str) -> Vec<(String, String)> {
    let a: Vec<&str> = old.split_whitespace().collect();
    let b: Vec<&str> = new.split_whitespace().collect();
    if a.is_empty() || b.is_empty() || a.len() > MAX_TOKENS || b.len() > MAX_TOKENS {
        return Vec::new();
    }
    // longest common subsequence table, from the ends
    let (n, m) = (a.len(), b.len());
    let mut l = vec![0u16; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let v = if a.get(i) == b.get(j) {
                l.get(at(i + 1, j + 1)).copied().unwrap_or(0).saturating_add(1)
            } else {
                l.get(at(i + 1, j)).copied().unwrap_or(0).max(l.get(at(i, j + 1)).copied().unwrap_or(0))
            };
            if let Some(x) = l.get_mut(at(i, j)) {
                *x = v;
            }
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    let (mut ra, mut rb): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    let mut flush = |ra: &mut Vec<&str>, rb: &mut Vec<&str>| {
        if !ra.is_empty() && !rb.is_empty() && ra.len() <= MAX_WORDS && rb.len() <= MAX_WORDS {
            let (x, y) = (ra.join(" "), rb.join(" "));
            let (sx, sy) = (strip(&x), strip(&y));
            if sx != sy && !sx.is_empty() && !sy.is_empty() && sx.chars().count() <= 30 && sy.chars().count() <= 30 {
                out.push((sx.to_string(), sy.to_string()));
            }
        }
        ra.clear();
        rb.clear();
    };
    while i < n || j < m {
        if i < n && j < m && a.get(i) == b.get(j) {
            flush(&mut ra, &mut rb);
            i += 1;
            j += 1;
        } else if j < m && (i >= n || l.get(at(i, j + 1)) >= l.get(at(i + 1, j))) {
            rb.extend(b.get(j));
            j += 1;
        } else {
            ra.extend(a.get(i));
            i += 1;
        }
    }
    flush(&mut ra, &mut rb);
    out
}

pub fn load_from(path: &Path) -> Vec<Pair> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() <= MAX_FILE => std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn save_to(path: &Path, pairs: &mut Vec<Pair>) -> Result<(), String> {
    if pairs.len() > MAX_PAIRS {
        // forget the rarest fixes first
        pairs.sort_by_key(|p| std::cmp::Reverse(p.count));
        pairs.truncate(MAX_PAIRS);
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let json = serde_json::to_string_pretty(pairs).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("{}: {e}", path.display()))
}

/// Count the fixes between `old` and `new`; returns the ones that just reached [`ASK_AT`] and
/// are still open.
pub fn record_in(path: &Path, old: &str, new: &str) -> Vec<Pair> {
    let fixes = diff(old, new);
    if fixes.is_empty() {
        return Vec::new();
    }
    let mut pairs = load_from(path);
    let mut ready = Vec::new();
    for (from, to) in fixes {
        let p = match pairs.iter_mut().find(|p| p.from == from && p.to == to) {
            Some(p) => {
                p.count = p.count.saturating_add(1);
                p.clone()
            }
            None => {
                let p = Pair { from, to, count: 1, dismissed: false, added: false };
                pairs.push(p.clone());
                p
            }
        };
        if p.count == ASK_AT && p.open() {
            ready.push(p);
        }
    }
    let _ = save_to(path, &mut pairs);
    ready
}

pub fn record(old: &str, new: &str) -> Vec<Pair> {
    record_in(&file(), old, new)
}

/// Fixes made at least `min` times and still open, most frequent first.
pub fn open_in(path: &Path, min: u32) -> Vec<Pair> {
    let mut v: Vec<Pair> = load_from(path).into_iter().filter(|p| p.count >= min && p.open()).collect();
    v.sort_by_key(|p| std::cmp::Reverse(p.count));
    v
}

pub fn open(min: u32) -> Vec<Pair> {
    open_in(&file(), min)
}

/// Mark a fix as put in the glossary (`added`) or not to be offered again.
pub fn resolve_in(path: &Path, from: &str, to: &str, added: bool) -> Result<(), String> {
    let mut pairs = load_from(path);
    let p = pairs.iter_mut().find(|p| p.from == from && p.to == to).ok_or("그런 고침 기록이 없습니다")?;
    if added {
        p.added = true;
    } else {
        p.dismissed = true;
    }
    save_to(path, &mut pairs)
}

pub fn resolve(from: &str, to: &str, added: bool) -> Result<(), String> {
    resolve_in(&file(), from, to, added)
}

/// Put a learned fix in the user glossary and mark it done.
pub fn accept(p: &Pair) -> Result<(), String> {
    crate::usergloss::add(&p.to, std::slice::from_ref(&p.from), "직접 고친 내용에서")?;
    resolve(&p.from, &p.to, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffs_find_word_fixes() {
        assert_eq!(diff("(화자1) 전국퀸 나왔다", "(화자1) 정커퀸 나왔다"), vec![("전국퀸".into(), "정커퀸".into())]);
        assert_eq!(diff("할수 있다 진짜로", "할 수 있다 진짜로"), vec![("할수".into(), "할 수".into())]);
        assert_eq!(diff("키리코가 왔다.", "키리코가 왔다!"), vec![], "punctuation only");
        assert_eq!(diff("가 나 다", "가 나 다"), vec![]);
        // a whole sentence rewritten is not a word fix
        assert_eq!(diff("하나 둘 셋 넷 다섯", "여섯 일곱 여덟 아홉 열"), vec![]);
        // a word added or removed is not a fix either
        assert_eq!(diff("가 다", "가 나 다"), vec![]);
        assert_eq!(diff("", "가"), vec![]);
    }

    #[test]
    fn third_time_asks_once() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("learned.json");
        assert!(record_in(&p, "전국퀸 왔다", "정커퀸 왔다").is_empty());
        assert!(record_in(&p, "또 전국퀸", "또 정커퀸").is_empty());
        let ready = record_in(&p, "전국퀸이다", "정커퀸이다");
        assert!(ready.is_empty(), "a different word form is a different fix");
        let ready = record_in(&p, "전국퀸 등장", "정커퀸 등장");
        assert_eq!(ready.len(), 1);
        assert_eq!((ready[0].from.as_str(), ready[0].to.as_str(), ready[0].count), ("전국퀸", "정커퀸", 3));
        assert!(record_in(&p, "전국퀸 등장", "정커퀸 등장").is_empty(), "asked once");
        assert_eq!(open_in(&p, 3).len(), 1);
        resolve_in(&p, "전국퀸", "정커퀸", false).unwrap();
        assert!(open_in(&p, 1).iter().all(|x| x.from != "전국퀸" || x.to != "정커퀸"));
        assert!(resolve_in(&p, "없음", "x", true).is_err());
        std::fs::write(&p, "garbage").unwrap();
        assert!(load_from(&p).is_empty());
    }
}
