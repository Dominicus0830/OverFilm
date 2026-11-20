//! OverFilm: a backup of the whole project before each big OverFilm edit, and going back to it in
//! one step (see `OVERFILM.md`).
//!
//! A One Go run, applying the cut markers, or applying a Claude Code review makes many changes,
//! often in several undo steps with the editor's own edits in between, and a long run can push the
//! start out of the undo history. Before such an edit OverFilm keeps the project as it was (a
//! "checkpoint"): in memory for this session, and, for a saved project, as a file in the
//! `OverFilm-Backups` folder beside it (the last [`MAX_FILES`]). Going back is one undo step, so it
//! can itself be undone.
//!
//! This is not the undo history (`edit.undo` goes back one step at a time) and not Auto Save or the
//! crash-recovery journal (`autosave.rs`, on a timer and after crashes): a checkpoint is taken only
//! before a big OverFilm edit, and named after it. A One Go run takes one at its start and none for
//! its steps, so going back undoes the whole run.
//!
//! - `overfilm.checkpoint.save {label?}` takes one now.
//! - `overfilm.checkpoint.list` the checkpoints of this session, newest first, and the backup
//!   files of the saved project.
//! - `overfilm.checkpoint.restore {id?, file?}` goes back to checkpoint `id` (the newest when
//!   neither is given) or to a backup file listed by `list`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use filmcraft_project::Project;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, str_p, u64_p};
use crate::{EngineError, Result, Session};

/// Checkpoints kept in memory (each holds a whole project).
pub const MAX_KEPT: usize = 10;
/// Backup files kept beside a saved project.
pub const MAX_FILES: usize = 5;
/// The folder beside the project file.
pub const DIR: &str = "OverFilm-Backups";
/// Longest label (letters).
const MAX_LABEL: usize = 80;
/// A backup file bigger than this is not read back.
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// A project as it was before a big OverFilm edit.
#[derive(Clone)]
pub struct Checkpoint {
    pub id: u64,
    /// What was about to happen ("한 번에 흐름 시작 전").
    pub label: String,
    /// Local time it was taken, `YYYY-MM-DD HH:MM:SS`.
    pub at: String,
    pub project: Arc<Project>,
    /// The backup file written for it (saved projects only).
    pub file: Option<PathBuf>,
    /// The document it belongs to ([`Session::document_epoch`]): another project opened since
    /// makes it stale.
    epoch: u64,
}

/// Checkpoints of this session, oldest first.
#[derive(Clone, Default)]
pub struct CheckpointState {
    pub list: Vec<Checkpoint>,
    next: u64,
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "overfilm.checkpoint.save",
            label: "Back Up the Project Now (OverFilm)",
            menu: &[],
            shortcut: None,
            params: r#"{"label":str?}"#,
            enabled: crate::commands::always,
            run: save_cmd,
            journal: false,
        },
        CommandSpec {
            id: "overfilm.checkpoint.list",
            label: "OverFilm Backups",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: crate::commands::always,
            run: list_cmd,
            journal: false,
        },
        CommandSpec {
            id: "overfilm.checkpoint.restore",
            label: "Undo the Last Big OverFilm Edit (Back to Backup)",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"id":n?,"file":path?}"#,
            enabled: has_checkpoint,
            run: restore_cmd,
            journal: false,
        },
    ]
}

/// The checkpoints of the open project, oldest first.
fn current(s: &Session) -> impl DoubleEndedIterator<Item = &Checkpoint> {
    let epoch = s.document_epoch;
    s.overfilm_checkpoints.list.iter().filter(move |c| c.epoch == epoch)
}

fn has_checkpoint(s: &Session) -> std::result::Result<(), String> {
    if current(s).next().is_none() && backup_files(s).is_empty() {
        Err("no OverFilm backup yet (one is taken before each big OverFilm edit)".into())
    } else {
        Ok(())
    }
}

fn now(s: &Session) -> (i64, String) {
    let unix = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let offset = s.persistence.as_ref().map_or(0, |p| (p.local_offset)(unix));
    (unix, filmcraft_format::autosave::display_time(unix, offset))
}

fn project_path(s: &Session) -> Option<PathBuf> {
    s.path.as_deref().filter(|p| !p.is_empty()).map(PathBuf::from)
}

fn backup_dir(project: &Path) -> PathBuf {
    project.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")).join(DIR)
}

fn stem(project: &Path) -> String {
    project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Backup files of the saved project, oldest first.
fn backup_files(s: &Session) -> Vec<PathBuf> {
    if cfg!(target_arch = "wasm32") {
        return Vec::new();
    }
    let Some(p) = project_path(s) else { return Vec::new() };
    filmcraft_format::autosave::list_auto_saves(&backup_dir(&p), &stem(&p))
}

/// Keep the project as it is now, before `label` happens. Nothing is taken while a One Go run is
/// going (its own checkpoint, from its start, covers every step) or when the project has not
/// changed since the last one. Never fails: a backup file that cannot be written is left out.
pub fn take(s: &mut Session, label: &str) -> Option<u64> {
    if crate::overfilm_flow::active(s) {
        return None;
    }
    take_now(s, label)
}

/// [`take`] even while a One Go run is going (the run's own checkpoint).
pub(crate) fn take_now(s: &mut Session, label: &str) -> Option<u64> {
    let project = s.project.clone();
    let epoch = s.document_epoch;
    // those of a project closed since are of no use
    s.overfilm_checkpoints.list.retain(|c| c.epoch == epoch);
    if let Some(last) = s.overfilm_checkpoints.list.last()
        && Arc::ptr_eq(&last.project, &project)
    {
        return Some(last.id);
    }
    let label: String = label.trim().chars().take(MAX_LABEL).collect();
    let (unix, at) = now(s);
    let file = if cfg!(target_arch = "wasm32") {
        None
    } else {
        project_path(s).and_then(|p| {
            let offset = s.persistence.as_ref().map_or(0, |x| (x.local_offset)(unix));
            let data = filmcraft_format::encode(&project, false);
            filmcraft_format::autosave::write_auto_save(&backup_dir(&p), &stem(&p), unix, offset, &data, MAX_FILES).ok()
        })
    };
    let st = &mut s.overfilm_checkpoints;
    st.next = st.next.saturating_add(1);
    let id = st.next;
    st.list.push(Checkpoint { id, label, at, project, file, epoch });
    if st.list.len() > MAX_KEPT {
        let extra = st.list.len() - MAX_KEPT;
        st.list.drain(..extra);
    }
    Some(id)
}

fn save_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let label = str_p(p, "label").unwrap_or("직접 백업");
    let id = take_now(s, label);
    Ok(json!({"id": id}))
}

fn list(s: &Session) -> Value {
    let mem: Vec<Value> =
        current(s).rev().map(|c| json!({"id": c.id, "label": c.label, "at": c.at, "file": c.file.as_ref().map(|f| f.display().to_string())})).collect();
    let files: Vec<String> = backup_files(s).iter().rev().map(|f| f.display().to_string()).collect();
    json!({"checkpoints": mem, "files": files})
}

fn list_cmd(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(list(s))
}

fn restore_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.checkpoint.restore";
    let (project, label) = if let Some(f) = str_p(p, "file") {
        // only the files listed beside this project (not any path)
        let path = backup_files(s)
            .into_iter()
            .find(|x| x.display().to_string() == f)
            .ok_or_else(|| bad(CMD, format!("not an OverFilm backup of this project: {f}")))?;
        let len = std::fs::metadata(&path).map(|m| m.len()).map_err(|e| EngineError::Other(format!("{f}: {e}")))?;
        if len > MAX_FILE_BYTES {
            return Err(bad(CMD, format!("{f}: too big")));
        }
        let bytes = std::fs::read(&path).map_err(|e| EngineError::Other(format!("{f}: {e}")))?;
        let loaded = filmcraft_format::decode(&bytes).map_err(|e| EngineError::Other(format!("{f}: {e}")))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        (Arc::new(loaded.project), name)
    } else {
        let c = match u64_p(p, "id") {
            Some(id) => current(s).find(|c| c.id == id).ok_or_else(|| bad(CMD, format!("no checkpoint {id}")))?,
            None => current(s).next_back().ok_or_else(|| bad(CMD, "no OverFilm backup in this session"))?,
        };
        (c.project.clone(), format!("{} ({})", c.label, c.at))
    };
    restore(s, project, &label)
}

/// Put `project` back as one undo step.
fn restore(s: &mut Session, project: Arc<Project>, label: &str) -> Result<Value> {
    if Arc::ptr_eq(&s.project, &project) {
        return Ok(json!({"restored": label, "changed": false}));
    }
    s.edit(&format!("OverFilm 백업으로 되돌리기: {label}"), |pr, _| {
        *pr = (*project).clone();
        Ok(())
    })?;
    // sequences or items the edits made are gone again
    s.fix_state();
    s.toast(format!("OverFilm 백업으로 되돌렸습니다: {label} (실행 취소로 다시 되돌릴 수 있습니다)"));
    Ok(json!({"restored": label, "changed": true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rename(s: &mut Session, name: &str) {
        let name = name.to_string();
        s.edit("rename", move |pr, _| {
            pr.name = name;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn restore_goes_back_in_one_undo_step() {
        let mut s = Session::default();
        rename(&mut s, "A");
        assert!(take(&mut s, "큰 편집 전").is_some());
        // the same project again: no second checkpoint
        assert_eq!(take(&mut s, "또"), s.overfilm_checkpoints.list.last().map(|c| c.id));
        assert_eq!(s.overfilm_checkpoints.list.len(), 1);
        for n in 0..3 {
            rename(&mut s, &format!("B{n}"));
        }
        let undo = s.history.undo.len();
        let v = s.execute("overfilm.checkpoint.restore", json!({})).unwrap();
        assert_eq!(v["changed"], true);
        assert_eq!(s.project.name, "A");
        assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project.name, "B2", "the restore can be undone");
        assert!(s.execute("overfilm.checkpoint.restore", json!({"id": 99})).is_err());
        assert!(s.execute("overfilm.checkpoint.restore", json!({"file": "/etc/passwd"})).is_err());
    }

    #[test]
    fn keeps_the_last_few() {
        let mut s = Session::default();
        for n in 0..(MAX_KEPT + 3) {
            rename(&mut s, &format!("S{n}"));
            take(&mut s, &"가".repeat(500));
        }
        assert_eq!(s.overfilm_checkpoints.list.len(), MAX_KEPT);
        assert!(s.overfilm_checkpoints.list.iter().all(|c| c.label.chars().count() == MAX_LABEL));
        assert_eq!(list(&s)["checkpoints"].as_array().map(Vec::len), Some(MAX_KEPT));
    }
}
