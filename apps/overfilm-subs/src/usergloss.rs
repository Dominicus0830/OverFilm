//! The user's own glossary (`glossary/user.tsv` beside the models): spellings the recogniser gets
//! wrong for this user, added from the window, from find and replace, from learned edits or over
//! MCP, and read with the built-in pack on every run.

use std::path::{Path, PathBuf};

use overfilm_glossary::{COLUMNS, Entry, Glossary};

/// Largest user table read or written.
const MAX_FILE: u64 = 4 << 20;

pub fn file() -> PathBuf {
    crate::mcp::user_glossary_dir().join("user.tsv")
}

fn clean(x: &str) -> String {
    x.replace(['\t', '\n', '\r', ';', '|'], " ").split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect()
}

/// The entries of `path` (none when it does not exist yet).
pub fn load_from(path: &Path) -> Result<Vec<Entry>, String> {
    let Ok(meta) = std::fs::metadata(path) else { return Ok(Vec::new()) };
    if meta.len() > MAX_FILE {
        return Err(format!("{}: 파일이 너무 큽니다", path.display()));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut g = Glossary::default();
    g.add_table("user", &text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(g.entries)
}

pub fn load() -> Result<Vec<Entry>, String> {
    load_from(&file())
}

/// The TSV text of `entries` (header first).
pub fn to_tsv(entries: &[Entry]) -> String {
    let mut s = COLUMNS.join("\t");
    s.push('\n');
    for e in entries {
        let row = [
            clean(&e.id),
            clean(&e.ko),
            clean(&e.en),
            e.aliases.iter().map(|x| clean(x)).collect::<Vec<_>>().join("; "),
            e.old.iter().map(|x| clean(x)).collect::<Vec<_>>().join("; "),
            e.misheard.iter().map(|x| clean(x)).collect::<Vec<_>>().join("; "),
            e.flags.iter().map(|x| clean(x)).collect::<Vec<_>>().join(" "),
            clean(&e.note),
        ];
        s.push_str(&row.join("\t"));
        s.push('\n');
    }
    s
}

/// Write `entries` to `path`, after checking the table reads back.
pub fn save_to(path: &Path, entries: &[Entry]) -> Result<(), String> {
    let text = to_tsv(entries);
    if text.len() as u64 > MAX_FILE {
        return Err("단어집이 너무 큽니다".into());
    }
    Glossary::default().add_table("user", &text).map_err(|e| e.to_string())?;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn save(entries: &[Entry]) -> Result<(), String> {
    save_to(&file(), entries)
}

/// An id not used yet (`user1`, `user2`…).
fn next_id(entries: &[Entry]) -> String {
    let n = entries.iter().filter_map(|e| e.id.strip_prefix("user").and_then(|n| n.parse::<u64>().ok())).max().unwrap_or(0);
    format!("user{}", n.saturating_add(1))
}

/// Add "these misheard spellings are really `ko`" to `path`. An entry with the same `ko` gets the
/// new misheard spellings; a misheard spelling already listed is not added twice. Returns the
/// entry as saved.
pub fn add_to(path: &Path, ko: &str, misheard: &[String], note: &str) -> Result<Entry, String> {
    let ko = clean(ko);
    if ko.is_empty() {
        return Err("맞는 표기가 비어 있습니다".into());
    }
    let misheard: Vec<String> = misheard.iter().map(|m| clean(m)).filter(|m| !m.is_empty() && *m != ko).collect();
    let mut entries = load_from(path)?;
    let entry = match entries.iter_mut().find(|e| e.ko == ko) {
        Some(e) => {
            for m in misheard {
                if !e.misheard.contains(&m) {
                    e.misheard.push(m);
                }
            }
            if e.note.is_empty() {
                e.note = clean(note);
            }
            e.clone()
        }
        None => {
            let e = Entry { category: "user".into(), id: next_id(&entries), ko, misheard, note: clean(note), ..Entry::default() };
            entries.push(e.clone());
            e
        }
    };
    save_to(path, &entries)?;
    Ok(entry)
}

pub fn add(ko: &str, misheard: &[String], note: &str) -> Result<Entry, String> {
    add_to(&file(), ko, misheard, note)
}

/// Remove the entry with `id`.
pub fn remove_from(path: &Path, id: &str) -> Result<(), String> {
    let mut entries = load_from(path)?;
    let before = entries.len();
    entries.retain(|e| e.id != id);
    if entries.len() == before {
        return Err("그런 단어가 없습니다".into());
    }
    save_to(path, &entries)
}

/// `"a; b, c"` → `[a, b, c]` (the window's misheard field).
pub fn split_list(s: &str) -> Vec<String> {
    s.split([';', ',', '\n']).map(clean).filter(|x| !x.is_empty()).collect()
}

/// Built-in and user entries whose spellings contain `q` (for "is it there already?").
pub fn search<'a>(g: &'a Glossary, q: &str, max: usize) -> Vec<&'a Entry> {
    let q = q.trim();
    if q.is_empty() {
        return Vec::new();
    }
    g.entries
        .iter()
        .filter(|e| std::iter::once(&e.ko).chain(&e.aliases).chain(&e.old).chain(&e.misheard).chain(std::iter::once(&e.en)).any(|s| s.contains(q)))
        .take(max)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_merge_remove_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("g").join("user.tsv");
        assert!(load_from(&p).unwrap().is_empty());
        let e = add_to(&p, "정커퀸", &["전국퀸".into()], "메모\t탭").unwrap();
        assert_eq!(e.id, "user1");
        let e2 = add_to(&p, " 정커퀸 ", &["전국퀸".into(), "정커 퀸;".into(), "정커퀸".into()], "").unwrap();
        assert_eq!(e2.misheard, vec!["전국퀸", "정커 퀸"]);
        assert_eq!(e2.note, "메모 탭");
        add_to(&p, "키리코", &["끼룩코".into()], "").unwrap();
        let all = load_from(&p).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].id, "user2");
        remove_from(&p, "user1").unwrap();
        assert!(remove_from(&p, "user1").is_err());
        let e3 = add_to(&p, "아나", &[], "").unwrap();
        assert_eq!(e3.id, "user3", "ids are never reused");
        assert!(add_to(&p, "  ", &[], "").is_err());
        // the saved table is what the extractor reads
        let g = crate::pack::glossary(Some(p.parent().unwrap()), true).unwrap();
        assert!(crate::pack::corrector(&g).apply("끼룩코 왔다").text.starts_with("키리코"));
    }

    #[test]
    fn search_and_lists() {
        let g = crate::pack::glossary(None, true).unwrap();
        assert!(!search(&g, "키리코", 10).is_empty());
        assert!(search(&g, " ", 10).is_empty());
        assert_eq!(split_list("가, 나; 다\n"), vec!["가", "나", "다"]);
    }
}
