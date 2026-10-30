//! OverFilm editing assistants (milestones M6, M7 and M9).
//!
//! Planned: engine commands reachable over MCP that propose and, once approved, apply edits:
//! caption review, silence marks, cut, highlight and restructure suggestions, caption styles from
//! user assets, screen recognition (kill feed, eliminations, ultimates) and editing-style learning
//! (`style.learn/inspect/suggest/forget`). Every change stays undoable. See `OVERFILM.md`.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
