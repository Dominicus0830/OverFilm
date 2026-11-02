//! OverFilm screen recognition for Overwatch esports broadcasts (milestone M7).
//!
//! Frames sampled from a broadcast (a few per second, shrunk to [`luma::MAX_WIDTH`]) are matched
//! against **templates** learnt from the same broadcast: the break screen's fixed graphics, the
//! "PREPARE TO ATTACK" banner, a pause notice ([`template`]). A frozen picture during a round
//! counts as a pause too. The labelled samples become **segments** and the segments a **cut plan**
//! ([`plan`]):
//!
//! - waiting time (break screens, round setup) is removed up to `keep_before` (10 s) before it
//!   ends, i.e. before the match or round starts;
//! - pauses are removed whole and the join gets a short dissolve;
//! - nothing else is ever cut: a sample no template matches is play.
//!
//! The crate does no decoding; the engine feeds it frames (`overfilm.autoCut`).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

/// Template files built in (see `data/README.md`).
pub mod builtin {
    /// OWCS Asia Korea 2026 broadcasts (720p and up): break screen, round setup banners, game HUD,
    /// both teams' name plates.
    pub const OWCS_KOREA_2026: &str = include_str!("../data/owcs-korea-2026.overfilm-screens.json");
}
pub mod killfeed;
pub mod kills;
pub mod luma;
pub mod plan;
pub mod template;
pub mod ults;

use serde::{Deserialize, Serialize};

pub use killfeed::{FeedTracker, KillFeed};
pub use kills::{Death, DeathTracker, Fight, PlayerBar};
pub use luma::{Luma, NRect};
pub use plan::{Analyzer, Cut, CutParams, CutReason, Detected, Look, Sample, Segment, plan_cuts, segments};
pub use template::{Part, Template, TemplateSet};
pub use ults::{FeedKill, UltIcon, UltKill, UltTracker, UltUse};

/// A broadcast state a template recognises.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Screen {
    /// Break / standby screen between matches or maps (big countdown, sponsor panel).
    Standby,
    /// Round setup: "PREPARE TO ATTACK/DEFEND" with its countdown; the round starts when it ends.
    Setup,
    /// The game is paused (a pause notice on screen).
    Pause,
    /// In-game HUD (score bar): play is on. Lets a frozen picture count as a pause outside rounds
    /// found by a setup banner.
    Live,
}

impl Screen {
    pub fn parse(s: &str) -> Option<Screen> {
        match s.trim().to_ascii_lowercase().as_str() {
            "standby" | "break" | "waiting" | "대기" => Some(Screen::Standby),
            "setup" | "prepare" | "준비" => Some(Screen::Setup),
            "pause" | "paused" | "퍼즈" => Some(Screen::Pause),
            "live" | "game" | "hud" | "경기" => Some(Screen::Live),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Screen::Standby => "standby",
            Screen::Setup => "setup",
            Screen::Pause => "pause",
            Screen::Live => "live",
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum VisionError {
    #[error("the rectangle is empty or outside the frame")]
    EmptyRect,
    #[error("the rectangle is a flat colour; pick a part with text or graphics")]
    Flat,
    #[error("template file: {0}")]
    Templates(String),
}
