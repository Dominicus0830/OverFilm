//! The user's Claude Code review settings, a small JSON file (`claude-review.json` in the
//! OverFilm data folder, shared by the editor and OverFilmSubs).

use std::path::Path;

use serde::{Deserialize, Serialize};

/// File name inside the OverFilm data folder.
pub const FILE: &str = "claude-review.json";

/// Longest time limit a setting may ask for (seconds).
pub const MAX_TIMEOUT: u64 = 1800;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// The switch: nothing is ever sent to Claude Code while this is off.
    pub enabled: bool,
    /// Ask on its own after Auto Cut / Suggestions mark cuts and after OverFilm Captions places
    /// captions (off: only when the user runs a review command).
    pub auto: bool,
    /// Path of `claude` (`claude.exe`, `claude.cmd`); empty: look on PATH and in the usual
    /// install folders.
    pub cli_path: String,
    /// Model alias or name passed as `--model` (empty: Claude Code's own default).
    pub model: String,
    /// Time limit of one question (seconds).
    pub timeout_secs: u64,
    /// Items asked about in one question (fewer, longer calls use the subscription better).
    pub batch: usize,
    /// Most items one review run asks about.
    pub max_items: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self { enabled: false, auto: false, cli_path: String::new(), model: String::new(), timeout_secs: 180, batch: 6, max_items: 60 }
    }
}

impl Settings {
    /// Read `path`; a missing or unreadable file gives the defaults (review off).
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Settings>(&t).ok()).map(Settings::clamped).unwrap_or_default()
    }

    /// Write `path` (creating its folder).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&self.clone().clamped()).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    /// Values kept in range (hand-edited files are hostile too).
    pub fn clamped(mut self) -> Self {
        self.timeout_secs = self.timeout_secs.clamp(10, MAX_TIMEOUT);
        self.batch = self.batch.clamp(1, 20);
        self.max_items = self.max_items.clamp(1, 500);
        self.cli_path = self.cli_path.trim().chars().take(1024).collect();
        self.model = self.model.trim().chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '[' | ']')).take(80).collect();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_off() {
        let s = Settings::load(Path::new("/nonexistent/overfilm/claude-review.json"));
        assert!(!s.enabled);
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn round_trip_and_clamp() {
        let dir = std::env::temp_dir().join(format!("overfilm-review-settings-{}", std::process::id()));
        let path = dir.join(FILE);
        let s = Settings { enabled: true, auto: true, timeout_secs: 5, batch: 999, model: "sonnet; rm -rf".into(), ..Settings::default() };
        s.save(&path).unwrap();
        let back = Settings::load(&path);
        assert!(back.enabled && back.auto);
        assert_eq!(back.timeout_secs, 10);
        assert_eq!(back.batch, 20);
        assert_eq!(back.model, "sonnetrm-rf");
        std::fs::write(&path, "{\"enabled\": true, \"timeoutSecs\": -3}").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        std::fs::write(&path, "{\"enabled\": true}").unwrap();
        assert!(Settings::load(&path).enabled);
        let _ = std::fs::remove_dir_all(dir);
    }
}
