//! OverFilm 자막 추출: the subtitle extractor split out of OverFilm as its own program. A video or
//! audio file goes in, a Korean SRT comes out (Qwen3-ASR with the Overwatch glossary as context,
//! word times from Qwen3-ForcedAligner, glossary correction and profanity initials, at most two
//! lines per caption). Three front ends share this library: the window (`OverFilmSubs`), the
//! command line and the MCP server for caption review (`overfilm-subs`).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod job;
pub mod mcp;
pub mod media;
pub mod pack;
pub mod review;
pub mod speakers;

/// File extensions offered in the open dialog.
pub const MEDIA_EXTENSIONS: &[&str] =
    &["mp4", "mov", "m4v", "mkv", "webm", "avi", "mxf", "ts", "mts", "m2ts", "mpg", "mpeg", "mp3", "m4a", "aac", "wav", "flac", "ogg", "opus"];
