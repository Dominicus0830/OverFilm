//! The user's own glossary (`glossary/user.tsv` beside the models, see [`crate::user_glossary_dir`]):
//! spellings the recogniser gets wrong for this user. OverFilmSubs (단어집 page, find and replace,
//! learned edits, MCP) and the editor (OverFilm panel ▸ Glossary…) both read and write it, and
//! [`crate::pack::glossary`] reads it with the built-in pack on every run.

use std::path::{Path, PathBuf};

pub use overfilm_glossary::Entry;
use overfilm_glossary::{COLUMNS, Glossary};

/// Largest user table read or written.
pub const MAX_FILE: u64 = 4 << 20;
/// The flags the form sets; any other flag on an entry is kept as it is.
const FORM_FLAGS: [&str; 2] = ["mask", "inword"];

/// The table the user's words are saved to.
pub fn file() -> PathBuf {
    crate::user_glossary_dir().join("user.tsv")
}

fn clean(x: &str) -> String {
    x.replace(['\t', '\n', '\r', ';', '|'], " ").split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect()
}

fn clean_list(v: &[String], not: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for x in v.iter().map(|x| clean(x)) {
        if !x.is_empty() && x != not && !out.contains(&x) {
            out.push(x);
        }
    }
    out
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

/// The TSV text of `entries` (header first).
pub fn to_tsv(entries: &[Entry]) -> String {
    let list = |v: &[String], sep: &str| v.iter().map(|x| clean(x)).collect::<Vec<_>>().join(sep);
    let mut s = COLUMNS.join("\t");
    s.push('\n');
    for e in entries {
        let row = [
            clean(&e.id),
            clean(&e.ko),
            clean(&e.en),
            list(&e.aliases, "; "),
            list(&e.old, "; "),
            list(&e.misheard, "; "),
            list(&e.flags, " "),
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

/// An id not used yet (`user1`, `user2`…).
fn next_id(entries: &[Entry]) -> String {
    let n = entries.iter().filter_map(|e| e.id.strip_prefix("user").and_then(|n| n.parse::<u64>().ok())).max().unwrap_or(0);
    format!("user{}", n.saturating_add(1))
}

/// Add "these misheard spellings are really `ko`" to `path`. An entry with the same `ko` gets the
/// new misheard spellings; a misheard spelling already listed is not added twice. Returns the
/// entry as saved.
pub fn add_to(path: &Path, ko: &str, misheard: &[String], note: &str) -> Result<Entry, String> {
    put(path, &Form { ko: ko.into(), misheard: misheard.to_vec(), note: note.into(), ..Form::default() })
}

/// One word as the glossary form fills it in: the table's columns (see [`overfilm_glossary`])
/// with the id given by the program and the flags as ticks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Form {
    /// The entry to change; `None` adds one (or adds to the entry with the same `ko`).
    pub id: Option<String>,
    pub ko: String,
    pub en: String,
    pub aliases: Vec<String>,
    pub old: Vec<String>,
    pub misheard: Vec<String>,
    /// Profanity: captions write `ko`'s initial consonants for every misheard form.
    pub mask: bool,
    /// Also replace a misheard form inside a longer word.
    pub inword: bool,
    pub note: String,
}

impl Form {
    /// The form filled in from `e` (to change it).
    pub fn of(e: &Entry) -> Self {
        Form {
            id: Some(e.id.clone()),
            ko: e.ko.clone(),
            en: e.en.clone(),
            aliases: e.aliases.clone(),
            old: e.old.clone(),
            misheard: e.misheard.clone(),
            mask: e.has_flag("mask"),
            inword: e.has_flag("inword"),
            note: e.note.clone(),
        }
    }
}

fn set_flags(flags: &mut Vec<String>, f: &Form) {
    flags.retain(|x| !FORM_FLAGS.contains(&x.as_str()));
    for (on, name) in [(f.mask, "mask"), (f.inword, "inword")] {
        if on {
            flags.push(name.into());
        }
    }
}

fn merge(into: &mut Vec<String>, more: Vec<String>) {
    for m in more {
        if !into.contains(&m) {
            into.push(m);
        }
    }
}

/// Save the form to `path`: with an id, that entry is replaced; without, a new entry is added,
/// or the entry with the same `ko` gets the new spellings (empty fields filled, ticks added).
/// Returns the entry as saved.
pub fn put(path: &Path, f: &Form) -> Result<Entry, String> {
    let ko = clean(&f.ko);
    if ko.is_empty() {
        return Err("맞는 표기가 비어 있습니다".into());
    }
    let (en, note) = (clean(&f.en), clean(&f.note));
    let (aliases, old, misheard) = (clean_list(&f.aliases, &ko), clean_list(&f.old, &ko), clean_list(&f.misheard, &ko));
    let mut entries = load_from(path)?;
    let entry = if let Some(id) = f.id.as_deref().map(str::trim).filter(|i| !i.is_empty()) {
        let Some(e) = entries.iter_mut().find(|e| e.id == id) else { return Err("그런 단어가 없습니다".into()) };
        e.ko = ko;
        e.en = en;
        e.aliases = aliases;
        e.old = old;
        e.misheard = misheard;
        e.note = note;
        set_flags(&mut e.flags, f);
        e.clone()
    } else if let Some(e) = entries.iter_mut().find(|e| e.ko == ko) {
        merge(&mut e.aliases, aliases);
        merge(&mut e.old, old);
        merge(&mut e.misheard, misheard);
        if e.en.is_empty() {
            e.en = en;
        }
        if e.note.is_empty() {
            e.note = note;
        }
        let keep = Form { mask: f.mask || e.has_flag("mask"), inword: f.inword || e.has_flag("inword"), ..Form::default() };
        set_flags(&mut e.flags, &keep);
        e.clone()
    } else {
        let mut e = Entry { category: "user".into(), id: next_id(&entries), ko, en, aliases, old, misheard, note, ..Entry::default() };
        set_flags(&mut e.flags, f);
        entries.push(e.clone());
        e
    };
    save_to(path, &entries)?;
    Ok(entry)
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

/// `"a; b, c"` → `[a, b, c]` (a list field of the form).
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
    fn the_whole_form_saves_and_changes() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("user.tsv");
        let f = Form {
            ko: "제트팩 캣".into(),
            en: "Jetpack Cat".into(),
            aliases: split_list("제트캣, 제트팩 캣"),
            old: vec![],
            misheard: split_list("제트팩et; 제트백 캣"),
            inword: true,
            note: "2026 영웅".into(),
            ..Form::default()
        };
        let e = put(&p, &f).unwrap();
        assert_eq!(e.aliases, vec!["제트캣"], "the right spelling itself is not an alias");
        assert_eq!(e.flags, vec!["inword"]);
        // change it: the id keeps, the ticks follow the form, other flags stay
        let mut entries = load_from(&p).unwrap();
        entries[0].flags.push("verify".into());
        save_to(&p, &entries).unwrap();
        let mut g = Form::of(&load_from(&p).unwrap()[0]);
        assert!(g.inword && !g.mask);
        g.inword = false;
        g.mask = true;
        g.misheard = vec!["제트백 캣".into()];
        let e = put(&p, &g).unwrap();
        assert_eq!(e.id, "user1");
        assert_eq!(e.misheard, vec!["제트백 캣"]);
        assert_eq!(e.flags, vec!["verify", "mask"]);
        assert!(put(&p, &Form { id: Some("user9".into()), ko: "아나".into(), ..Form::default() }).is_err());
        // the file is the table the recogniser reads
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("id\tko\ten\taliases\told\tmisheard\tflags\tnote\n"));
        assert!(text.contains("user1\t제트팩 캣\tJetpack Cat\t제트캣\t\t제트백 캣\tverify mask\t2026 영웅"));
        assert!(crate::pack::glossary(Some(d.path()), true).is_ok());
    }

    #[test]
    fn search_and_lists() {
        let g = crate::pack::glossary(None, true).unwrap();
        assert!(!search(&g, "키리코", 10).is_empty());
        assert!(search(&g, " ", 10).is_empty());
        assert_eq!(split_list("가, 나; 다\n"), vec!["가", "나", "다"]);
    }
}
