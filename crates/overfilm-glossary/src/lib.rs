//! OverFilm glossary and language packs (milestone M2).
//!
//! Planned: a glossary format keyed by shared term ids (one id per hero, ability or map, with
//! per-language spellings, known misrecognitions and pronunciations), loading of the Overwatch
//! Korean pack in `packs/overwatch/ko/`, `transcript.applyGlossary` post-correction and jamo
//! similarity matching. See `OVERFILM.md`.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
