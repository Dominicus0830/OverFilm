//! OverFilm review by Claude Code: the editor hands the calls it is unsure of to the Claude Code
//! command line (`claude -p`, signed in with the user's Claude subscription, never an API key) and
//! gets a structured answer back. Off unless the user turns it on ([`Settings::enabled`]).
//!
//! - [`cli`]: find `claude` (Windows `claude.exe`, an npm `claude.cmd`, macOS/Linux), check it is
//!   signed in, run one question with a JSON schema, a time limit and a stop flag. Any failure
//!   (not installed, not signed in, usage limit, timeout, an answer that does not parse) is a
//!   [`ReviewError`]; callers leave their own result as it was.
//! - [`cuts`]: an "OverFilm 컷" range the rules are unsure of (speech at its edge, a highlight
//!   next to it, game voice inside): cut it, keep it, or keep exact parts of it.
//! - [`captions`]: a caption that needs judgment (unsure words, a glossary correction, an awkward
//!   line break): the pure recognition, the corrected text and the captions around it go out; a
//!   keep, a corrected text or a split into several captions comes back.
//! - [`Settings`]: the user's switch and options, a small JSON file in the OverFilm data folder.
//!
//! Times are integer milliseconds of sequence time in and out (no floating-point seconds).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod captions;
pub mod cli;
pub mod cuts;
pub mod settings;
mod text;

pub use cli::{ClaudeCode, Reviewer, Status};
pub use settings::Settings;
pub use text::{bare, clock, display_width, extract_json};

/// Why a review gave no answer.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ReviewError {
    #[error("Claude Code 검토가 꺼져 있습니다 (설정에서 켜 주세요)")]
    Disabled,
    #[error("Claude Code(claude)를 찾지 못했습니다. 설치한 뒤 다시 해 보세요: https://claude.com/claude-code")]
    NotInstalled,
    #[error("Claude Code에 로그인되어 있지 않습니다. 명령 창에서 `claude`를 실행해 구독 계정으로 로그인해 주세요")]
    NotLoggedIn,
    #[error("Claude 사용 한도에 걸렸습니다: {0}")]
    Limit(String),
    #[error("Claude Code가 {0}초 안에 답하지 않았습니다")]
    Timeout(u64),
    #[error("멈춤")]
    Cancelled,
    #[error("Claude Code 실행 실패: {0}")]
    Failed(String),
    #[error("Claude Code의 답을 읽지 못했습니다: {0}")]
    BadAnswer(String),
}
