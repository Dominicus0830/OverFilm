//! The OverFilm caption pipeline shared by the subtitle extractor (`apps/overfilm-subs`) and the
//! editor (`filmcraft-engine`'s `overfilm.captions.generate`): the Overwatch Korean pack built in
//! ([`pack`]), recognition with glossary correction, speaker labels and game voice
//! ([`recognise`], feature `qwen`), caption turns ([`speakers`]), filler clean-up ([`clean`]) and
//! enrolled voices ([`voices`]). Both programs keep their models, voices and user glossary in the
//! same folders ([`models_dir`]), so a model downloaded once serves both.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod clean;
pub mod pack;
#[cfg(feature = "qwen")]
pub mod recognise;
pub mod speakers;
pub mod voices;

use std::path::PathBuf;

/// Where the models live: `models` beside the program if that folder exists (portable copy),
/// else `OVERFILM_MODELS`, else the user's local data folder (`…/OverFilm/models`).
pub fn models_dir() -> PathBuf {
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("models")))
        && dir.is_dir()
    {
        return dir;
    }
    if let Some(d) = std::env::var_os("OVERFILM_MODELS") {
        return PathBuf::from(d);
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("OverFilm").join("models")
}

/// The folder that holds `models`, `voices` and `glossary`.
pub fn data_dir() -> PathBuf {
    models_dir().parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// The user's own glossary tables (words added in review, learned corrections).
pub fn user_glossary_dir() -> PathBuf {
    data_dir().join("glossary")
}
