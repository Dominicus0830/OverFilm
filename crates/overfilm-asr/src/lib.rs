//! OverFilm speech recognition (milestones M3 and M5).
//!
//! Planned: Qwen3-ASR-1.7B and Qwen3-ForcedAligner-0.6B ported to pure Rust on candle, exposed
//! as a `filmcraft_speech::Transcriber` with word times and confidences; model download; and the
//! streamer mode classifier that keeps the streamer's speech and drops in-game hero voice lines.
//! See `OVERFILM.md`.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
