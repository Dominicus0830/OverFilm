//! OverFilm Korean caption layout (milestone M4).
//!
//! Planned: at most two lines per caption, line length by display width (Hangul 1, Latin 0.5),
//! penalty-based line breaking that keeps glossary terms and names whole, shot-change snapping and
//! reading-speed checks. It replaces the greedy caption blocking at the call site in `edit`.
//! See `OVERFILM.md`.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
