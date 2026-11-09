//! OverFilm: the user's own glossary from the editor (OverFilm panel ▸ Glossary…, or Sequence ▸
//! OverFilm ▸ Glossary…). The words are the same `glossary/user.tsv` OverFilmSubs keeps
//! ([`overfilm_pipeline::usergloss`]); OverFilm Captions and Claude Code review read it with the
//! built-in pack, from the next run on.
//!
//! - `overfilm.glossary {query?, limit?, file?}`: the user's words (all) and the built-in words
//!   whose spellings contain `query` (the first `limit`, 300 by default), with how many there are.
//! - `overfilm.glossary.add {ko, en?, aliases?, old?, misheard?, mask?, inword?, note?, id?, file?}`:
//!   save a word. The list fields take an array or one text split at `,` and `;`. With `id` that
//!   word is replaced; without, it is added, or the word with the same `ko` gets the new spellings.
//! - `overfilm.glossary.remove {id, file?}`: remove one of the user's words.
//!
//! `file` is another table to use instead of the user's (tests, a shared word list).

use std::path::PathBuf;

use overfilm_pipeline::usergloss::{self, Entry, Form};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, str_p, u64_p};
use crate::{Result, Session};

/// Most built-in words answered at once.
const MAX_LIMIT: u64 = 2_000;
/// Most user words answered (the file is capped at 4 MiB anyway).
const MAX_USER: usize = 20_000;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "overfilm.glossary",
            label: "Glossary (OverFilm)…",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"query":str?,"limit":1..2000?,"file":path?}"#,
            enabled: crate::commands::always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "overfilm.glossary.add",
            label: "Add to Glossary (OverFilm)",
            menu: &[],
            shortcut: None,
            params: r#"{"ko":str,"en":str?,"aliases":[str]?,"old":[str]?,"misheard":[str]?,"mask":bool?,"inword":bool?,"note":str?,"id":str?,"file":path?}"#,
            enabled: crate::commands::always,
            run: add,
            journal: false,
        },
        CommandSpec {
            id: "overfilm.glossary.remove",
            label: "Remove from Glossary (OverFilm)",
            menu: &[],
            shortcut: None,
            params: r#"{"id":str,"file":path?}"#,
            enabled: crate::commands::always,
            run: remove,
            journal: false,
        },
    ]
}

fn file_of(p: &Value) -> PathBuf {
    match str_p(p, "file").map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => PathBuf::from(f),
        None => usergloss::file(),
    }
}

/// One word as the commands answer it.
pub fn entry_json(e: &Entry) -> Value {
    json!({
        "id": e.id, "category": e.category, "ko": e.ko, "en": e.en, "aliases": e.aliases, "old": e.old,
        "misheard": e.misheard, "flags": e.flags, "note": e.note,
    })
}

fn list_p(p: &Value, k: &str) -> Vec<String> {
    match p.get(k) {
        Some(Value::String(s)) => usergloss::split_list(s),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).take(500).flat_map(usergloss::split_list).collect(),
        _ => Vec::new(),
    }
}

fn list(_: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.glossary";
    let path = file_of(p);
    let user = usergloss::load_from(&path).map_err(|e| bad(CMD, e))?;
    let builtin = overfilm_pipeline::pack::glossary(None, true).map_err(|e| bad(CMD, e))?;
    let limit = u64_p(p, "limit").unwrap_or(300).clamp(1, MAX_LIMIT) as usize;
    let query = str_p(p, "query").unwrap_or_default().chars().take(200).collect::<String>();
    let hits: Vec<&Entry> = if query.trim().is_empty() { builtin.entries.iter().collect() } else { usergloss::search(&builtin, &query, usize::MAX) };
    Ok(json!({
        "file": path,
        "user": user.iter().take(MAX_USER).map(entry_json).collect::<Vec<_>>(),
        "builtin": hits.iter().take(limit).map(|e| entry_json(e)).collect::<Vec<_>>(),
        "builtinMatches": hits.len(),
        "builtinCount": builtin.entries.len(),
    }))
}

fn add(_: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.glossary.add";
    let text = |k: &str| str_p(p, k).unwrap_or_default().chars().take(1_000).collect::<String>();
    let f = Form {
        id: str_p(p, "id").map(str::to_string).filter(|i| !i.trim().is_empty()),
        ko: text("ko"),
        en: text("en"),
        aliases: list_p(p, "aliases"),
        old: list_p(p, "old"),
        misheard: list_p(p, "misheard"),
        mask: bool_p(p, "mask").unwrap_or(false),
        inword: bool_p(p, "inword").unwrap_or(false),
        note: text("note"),
    };
    let path = file_of(p);
    let e = usergloss::put(&path, &f).map_err(|e| bad(CMD, e))?;
    Ok(json!({"saved": path, "entry": entry_json(&e)}))
}

fn remove(_: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.glossary.remove";
    let id = str_p(p, "id").map(str::trim).filter(|i| !i.is_empty()).ok_or_else(|| bad(CMD, "`id` is the word's id (user1, user2, …)"))?;
    let path = file_of(p);
    usergloss::remove_from(&path, id).map_err(|e| bad(CMD, e))?;
    Ok(json!({"removed": id, "saved": path}))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("overfilm-glossary-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn words_are_added_changed_listed_and_removed() {
        let d = temp("words");
        let file = d.join("user.tsv");
        let f = file.to_string_lossy().to_string();
        let mut s = Session::default();
        let v = s.execute("overfilm.glossary", json!({"file": f, "query": "키리코", "limit": 5})).unwrap();
        assert_eq!(v["user"], json!([]));
        assert!(v["builtinMatches"].as_u64().unwrap() >= 1);
        assert!(v["builtin"].as_array().unwrap().len() <= 5);
        assert!(v["builtinCount"].as_u64().unwrap() > 100);
        let v =
            s.execute("overfilm.glossary.add", json!({"file": f, "ko": "정커퀸", "misheard": "전국퀸, 정커 킹", "aliases": ["정퀸"], "note": "탱커"})).unwrap();
        assert_eq!(v["entry"]["id"], "user1");
        assert_eq!(v["entry"]["misheard"], json!(["전국퀸", "정커 킹"]));
        // change it by id
        let v = s.execute("overfilm.glossary.add", json!({"file": f, "id": "user1", "ko": "정커퀸", "misheard": ["전국퀸"], "mask": false})).unwrap();
        assert_eq!(v["entry"]["aliases"], json!([]));
        let v = s.execute("overfilm.glossary", json!({"file": f})).unwrap();
        assert_eq!(v["user"].as_array().unwrap().len(), 1);
        assert_eq!(v["user"][0]["misheard"], json!(["전국퀸"]));
        assert!(s.execute("overfilm.glossary.add", json!({"file": f, "ko": "  "})).is_err());
        assert!(s.execute("overfilm.glossary.remove", json!({"file": f, "id": "user7"})).is_err());
        s.execute("overfilm.glossary.remove", json!({"file": f, "id": "user1"})).unwrap();
        assert_eq!(s.execute("overfilm.glossary", json!({"file": f})).unwrap()["user"], json!([]));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_broken_table_is_an_error_not_a_crash() {
        let d = temp("broken");
        let file = d.join("user.tsv");
        std::fs::write(&file, "아무거나\n").unwrap();
        let f = file.to_string_lossy().to_string();
        let mut s = Session::default();
        assert!(s.execute("overfilm.glossary", json!({"file": f})).is_err());
        assert!(s.execute("overfilm.glossary.add", json!({"file": f, "ko": "아나"})).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "아무거나\n", "a table that does not read is not overwritten");
        let _ = std::fs::remove_dir_all(&d);
    }
}
