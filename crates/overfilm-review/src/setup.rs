//! Getting Claude Code ready: how to install it, signing in, and the models to choose from.
//!
//! Claude Code has no command that lists models, so the list is built from what the installed
//! version says: the aliases its `--help` names for `--model` (they change with new versions),
//! the usual aliases, and the model the user picked in Claude Code itself
//! (`~/.claude/settings.json`). The first entry is always Claude Code's own default (no
//! `--model`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Aliases every recent Claude Code takes, kept when `--help` names fewer.
const USUAL: [&str; 3] = ["opus", "sonnet", "haiku"];

/// One model to choose.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    /// What goes to `--model` (empty: Claude Code's default).
    pub id: String,
    pub label: String,
}

/// The aliases a `claude --help` text names in its `--model` entry (`'fable', 'opus', or 'sonnet'`).
pub fn help_aliases(help: &str) -> Vec<String> {
    let Some(start) = help.find("--model") else { return Vec::new() };
    let rest = help.get(start + "--model".len()..).unwrap_or_default();
    // the entry ends where the next option starts
    let end = rest.find("\n  -").unwrap_or(rest.len());
    let entry = rest.get(..end).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for (i, part) in entry.split('\'').enumerate() {
        let a = part.trim();
        let ok = i % 2 == 1 && (1..=40).contains(&a.len()) && a.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '[' | ']'));
        if ok && !out.iter().any(|x| x == a) {
            out.push(a.to_string());
        }
    }
    out
}

/// The model the user set in Claude Code (`"model"` in `~/.claude/settings.json`), if any.
pub fn user_default(settings_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(settings_json).ok()?;
    let m = v.get("model")?.as_str()?.trim();
    let ok = (1..=80).contains(&m.len()) && m.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '[' | ']'));
    ok.then(|| m.to_string())
}

/// `~/.claude/settings.json`.
pub fn user_settings_path() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(|h| PathBuf::from(h).join(".claude").join("settings.json"))
}

fn label(id: &str) -> String {
    let what = match id {
        "opus" => "가장 똑똑함, 사용량 많이 씀",
        "sonnet" => "빠르고 정확함, 추천",
        "haiku" => "가장 빠름, 사용량 적게 씀",
        "fable" => "최신 모델",
        "opusplan" => "계획은 opus, 실행은 sonnet",
        _ if id.contains("[1m]") => "긴 문맥",
        _ => "",
    };
    if what.is_empty() { id.to_string() } else { format!("{id} ({what})") }
}

/// The models to offer: Claude Code's default first, then the aliases `--help` names, the usual
/// ones, the user's Claude Code default and the one saved in OverFilm (`saved`), each once.
pub fn models(help: &str, user: Option<&str>, saved: &str) -> Vec<Model> {
    let mut ids: Vec<String> = help_aliases(help);
    for u in USUAL {
        if !ids.iter().any(|x| x == u) {
            ids.push(u.to_string());
        }
    }
    for extra in [user, Some(saved)].into_iter().flatten() {
        let e = extra.trim();
        if !e.is_empty() && !ids.iter().any(|x| x == e) {
            ids.push(e.to_string());
        }
    }
    let default = match user.map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => format!("Claude Code 기본값 (지금: {u})"),
        None => "Claude Code 기본값".to_string(),
    };
    std::iter::once(Model { id: String::new(), label: default }).chain(ids.into_iter().map(|id| Model { label: label(&id), id })).collect()
}

/// One step of installing Claude Code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub text: String,
    /// A command to type (copied with a button), if the step has one.
    pub command: String,
}

/// Where the install guide lives.
pub const INSTALL_DOCS: &str = "https://code.claude.com/docs/en/setup";

fn step(text: &str, command: &str) -> Step {
    Step { text: text.into(), command: command.into() }
}

/// How to install Claude Code and sign in, for this computer's system.
pub fn install_steps() -> Vec<Step> {
    let (open, install) = if cfg!(windows) {
        ("시작 메뉴에서 'PowerShell'을 열고 아래 명령을 붙여 넣어 실행합니다.", "irm https://claude.ai/install.ps1 | iex")
    } else {
        ("터미널을 열고 아래 명령을 붙여 넣어 실행합니다.", "curl -fsSL https://claude.ai/install.sh | bash")
    };
    vec![
        step(open, install),
        step("Node.js(npm)가 있다면 이 명령으로 설치해도 됩니다.", "npm install -g @anthropic-ai/claude-code"),
        step(
            "설치가 끝나면 창을 닫았다가 새로 열고, 아래 명령으로 Claude 계정(Pro·Max 구독)에 로그인합니다. 브라우저가 열리면 로그인하고 허용을 누릅니다.",
            "claude auth login --claudeai",
        ),
        step("OverFilm으로 돌아와 '다시 확인'을 누릅니다. 설치한 곳을 못 찾으면 설정에서 claude 프로그램 경로를 직접 지정합니다.", ""),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELP: &str = "Options:\n  --mcp-config <configs...>  Load MCP servers\n  --model <model>  Model for the current session. Provide\n                   an alias for the latest model (e.g.\n                   'fable', 'opus', or 'sonnet') or a\n                   model's full name.\n  -n, --name <name>  Set a display name ('x y')\n";

    #[test]
    fn aliases_from_help() {
        assert_eq!(help_aliases(HELP), ["fable", "opus", "sonnet"]);
        assert!(help_aliases("no such option").is_empty());
        assert!(help_aliases("--model 'rm -rf /'").is_empty());
    }

    #[test]
    fn model_list() {
        let m = models(HELP, Some("opus[1m]"), "claude-sonnet-5-5");
        let ids: Vec<&str> = m.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["", "fable", "opus", "sonnet", "haiku", "opus[1m]", "claude-sonnet-5-5"]);
        assert!(m[0].label.contains("opus[1m]"));
        assert_eq!(models("", None, "").len(), 4);
        assert_eq!(models("", None, "sonnet").len(), 4);
    }

    #[test]
    fn user_model_read_safely() {
        assert_eq!(user_default(r#"{"model": "opus"}"#).as_deref(), Some("opus"));
        assert_eq!(user_default(r#"{"model": "opus; rm"}"#), None);
        assert_eq!(user_default("not json"), None);
        assert_eq!(user_default("{}"), None);
    }

    #[test]
    fn install_guide() {
        let s = install_steps();
        assert!(s.len() >= 3);
        assert!(s.iter().any(|x| x.command.contains("claude auth login")));
    }
}
