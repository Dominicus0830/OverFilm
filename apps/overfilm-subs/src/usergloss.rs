//! The user's own glossary (`glossary/user.tsv` beside the models): spellings the recogniser gets
//! wrong for this user, added from the window, from find and replace, from learned edits or over
//! MCP, and read with the built-in pack on every run. Shared with the editor
//! ([`overfilm_pipeline::usergloss`]).

use overfilm_glossary::Entry;

pub use overfilm_pipeline::usergloss::{Form, add_to, file, load_from, put, remove_from, save_to, search, split_list, to_tsv};

pub fn load() -> Result<Vec<Entry>, String> {
    load_from(&file())
}

pub fn save(entries: &[Entry]) -> Result<(), String> {
    save_to(&file(), entries)
}

pub fn add(ko: &str, misheard: &[String], note: &str) -> Result<Entry, String> {
    add_to(&file(), ko, misheard, note)
}
