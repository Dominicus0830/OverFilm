//! Connecting the MCP server to Claude Desktop and Claude Code: one `"overfilm-subs"` entry is
//! added under `"mcpServers"` in their config file, everything else in the file is kept, and the
//! file as it was before is left beside it as `.bak`.
//!
//! Claude Desktop reads `claude_desktop_config.json` (Windows `%APPDATA%\Claude`, or the
//! Microsoft Store copy under `%LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude`;
//! macOS `~/Library/Application Support/Claude`; Linux `~/.config/Claude`). Claude Code keeps
//! its user-wide servers in `~/.claude.json` (what `claude mcp add --scope user` writes).

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// The entry's name in `mcpServers`.
pub const SERVER: &str = "overfilm-subs";
/// Config files larger than this are not read (`~/.claude.json` is usually well under 10 MB).
const MAX_CONFIG: u64 = 64 << 20;

/// Which Claude program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
    Desktop,
    Code,
}

impl Client {
    pub const ALL: [Client; 2] = [Client::Desktop, Client::Code];

    pub fn label(self) -> &'static str {
        match self {
            Client::Desktop => "Claude Desktop",
            Client::Code => "Claude Code",
        }
    }
}

/// Where a client stands with this program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// The program does not seem to be installed (no config folder / file).
    Missing,
    /// Installed, no `overfilm-subs` entry.
    NotConnected,
    /// The entry points at this program.
    Connected,
    /// The entry points somewhere else (the program was moved or is another copy): the path.
    Elsewhere(String),
    /// The file is there but is not a JSON object (left alone): why.
    Broken(String),
}

/// The config file a client uses on this computer, and whether the client looks installed.
pub fn config_path(client: Client) -> (PathBuf, bool) {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let home = env("USERPROFILE").filter(|_| cfg!(windows)).or_else(|| env("HOME")).unwrap_or_else(|| PathBuf::from("."));
    match client {
        Client::Code => {
            let p = home.join(".claude.json");
            let installed = p.is_file() || home.join(".claude").is_dir();
            (p, installed)
        }
        Client::Desktop => {
            let mut dirs = Vec::new();
            if cfg!(windows) {
                if let Some(a) = env("APPDATA") {
                    dirs.push(a.join("Claude"));
                }
                if let Some(l) = env("LOCALAPPDATA") {
                    dirs.extend(store_dirs(&l.join("Packages")));
                }
            } else if cfg!(target_os = "macos") {
                dirs.push(home.join("Library").join("Application Support").join("Claude"));
            } else {
                dirs.push(env("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")).join("Claude"));
            }
            let file = |d: &PathBuf| d.join("claude_desktop_config.json");
            // the folder that already has a config, else the first that exists, else the usual one
            let chosen = dirs.iter().find(|d| file(d).is_file()).or_else(|| dirs.iter().find(|d| d.is_dir())).or(dirs.first()).cloned();
            match chosen {
                Some(d) => {
                    let installed = d.is_dir();
                    (file(&d), installed)
                }
                None => (home.join("claude_desktop_config.json"), false),
            }
        }
    }
}

/// `…\Packages\Claude_*\LocalCache\Roaming\Claude` folders (the Microsoft Store install).
fn store_dirs(packages: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(packages) else { return Vec::new() };
    rd.filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("Claude_"))
        .map(|e| e.path().join("LocalCache").join("Roaming").join("Claude"))
        .filter(|d| d.is_dir())
        .take(4)
        .collect()
}

/// The entry this program wants: `command` run with `mcp`.
pub fn entry(client: Client, command: &Path) -> Value {
    let command = command.to_string_lossy();
    match client {
        Client::Desktop => json!({"command": command, "args": ["mcp"]}),
        Client::Code => json!({"type": "stdio", "command": command, "args": ["mcp"], "env": {}}),
    }
}

/// The JSON to paste by hand (the `mcpServers` block with only this entry).
pub fn snippet(client: Client, command: &Path) -> String {
    serde_json::to_string_pretty(&json!({"mcpServers": {SERVER: entry(client, command)}})).unwrap_or_default()
}

fn read(path: &Path) -> Result<Option<Map<String, Value>>, String> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if meta.len() > MAX_CONFIG {
        return Err(format!("{}: 설정 파일이 너무 큽니다 ({}MB)", path.display(), meta.len() >> 20));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = text.trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        return Ok(Some(Map::new()));
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(m)) => Ok(Some(m)),
        Ok(_) => Err("설정 파일이 JSON 객체가 아닙니다".into()),
        Err(e) => Err(format!("설정 파일의 JSON이 깨져 있습니다 ({e})")),
    }
}

/// How `path` stands with `command`; `installed` is whether the client looks installed.
pub fn state_of(path: &Path, installed: bool, command: &Path) -> State {
    match read(path) {
        Err(e) => State::Broken(e),
        Ok(None) if !installed => State::Missing,
        Ok(None) => State::NotConnected,
        Ok(Some(m)) => match m.get("mcpServers").and_then(|s| s.get(SERVER)) {
            None => State::NotConnected,
            Some(e) => {
                let cmd = e.get("command").and_then(Value::as_str).unwrap_or_default();
                if same_path(cmd, command) { State::Connected } else { State::Elsewhere(cmd.to_string()) }
            }
        },
    }
}

fn same_path(a: &str, b: &Path) -> bool {
    let b = b.to_string_lossy();
    if cfg!(windows) { a.eq_ignore_ascii_case(&b) } else { a == b }
}

/// Add (or point again) this program's entry in `path`, keeping everything else. A file that is
/// not a JSON object is left untouched and an error comes back. The previous file is kept as
/// `<name>.bak`.
pub fn connect(path: &Path, client: Client, command: &Path) -> Result<(), String> {
    let mut root = read(path)?.unwrap_or_default();
    let servers = root.entry("mcpServers").or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(servers) = servers else {
        return Err("설정 파일의 \"mcpServers\"가 객체가 아니라 손대지 않았습니다".into());
    };
    servers.insert(SERVER.to_string(), entry(client, command));
    write(path, &root)
}

/// Remove this program's entry from `path` (nothing else changes).
pub fn disconnect(path: &Path) -> Result<(), String> {
    let Some(mut root) = read(path)? else { return Ok(()) };
    let removed = root.get_mut("mcpServers").and_then(Value::as_object_mut).and_then(|s| s.remove(SERVER)).is_some();
    if removed { write(path, &root) } else { Ok(()) }
}

fn write(path: &Path, root: &Map<String, Value>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(root).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if path.is_file() {
        std::fs::copy(path, bak(path)).map_err(|e| format!("백업을 만들지 못해 바꾸지 않았습니다: {e}"))?;
    }
    // write beside it, then swap, so a crash never leaves half a file
    let tmp = path.with_extension("json.overfilm-tmp");
    std::fs::write(&tmp, text + "\n").map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// `<file>.bak` beside `path`.
pub fn bak(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".bak");
    path.with_file_name(name)
}

/// The command-line program next to the running window (`overfilm-subs(.exe)`).
pub fn cli_path() -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join(if cfg!(windows) { "overfilm-subs.exe" } else { "overfilm-subs" }))).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd() -> PathBuf {
        PathBuf::from(if cfg!(windows) { r"C:\OverFilm\overfilm-subs.exe" } else { "/opt/overfilm/overfilm-subs" })
    }

    #[test]
    fn adds_the_entry_and_keeps_the_rest() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("claude_desktop_config.json");
        let before = r#"{"mcpServers":{"other":{"command":"x","args":["y"]}},"globalShortcut":"Ctrl+Space"}"#;
        std::fs::write(&p, before).unwrap();
        assert_eq!(state_of(&p, true, &cmd()), State::NotConnected);
        connect(&p, Client::Desktop, &cmd()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["globalShortcut"], "Ctrl+Space");
        assert_eq!(v["mcpServers"]["other"]["args"][0], "y");
        assert_eq!(v["mcpServers"][SERVER]["args"][0], "mcp");
        assert_eq!(std::fs::read_to_string(bak(&p)).unwrap(), before);
        assert_eq!(state_of(&p, true, &cmd()), State::Connected);
        // again is harmless
        connect(&p, Client::Desktop, &cmd()).unwrap();
        assert_eq!(state_of(&p, true, &cmd()), State::Connected);
        disconnect(&p).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(v["mcpServers"].get(SERVER).is_none());
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
    }

    #[test]
    fn creates_a_missing_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("Claude").join("claude_desktop_config.json");
        assert_eq!(state_of(&p, false, &cmd()), State::Missing);
        assert_eq!(state_of(&p, true, &cmd()), State::NotConnected);
        connect(&p, Client::Desktop, &cmd()).unwrap();
        assert_eq!(state_of(&p, true, &cmd()), State::Connected);
        assert!(!bak(&p).exists());
        // an empty file (as Claude Desktop sometimes leaves) counts as {}
        std::fs::write(&p, "\u{feff}\n").unwrap();
        connect(&p, Client::Code, &cmd()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["mcpServers"][SERVER]["type"], "stdio");
    }

    #[test]
    fn leaves_a_broken_file_alone() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.json");
        for bad in ["{\"mcpServers\": {", "[1,2]", "{\"mcpServers\": 3}"] {
            std::fs::write(&p, bad).unwrap();
            assert!(connect(&p, Client::Desktop, &cmd()).is_err(), "{bad}");
            assert_eq!(std::fs::read_to_string(&p).unwrap(), bad);
        }
        std::fs::write(&p, "[1,2]").unwrap();
        assert!(matches!(state_of(&p, true, &cmd()), State::Broken(_)));
    }

    #[test]
    fn notices_an_entry_for_another_copy() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.json");
        std::fs::write(&p, r#"{"mcpServers":{"overfilm-subs":{"command":"D:\\old\\overfilm-subs.exe","args":["mcp"]}}}"#).unwrap();
        assert_eq!(state_of(&p, true, &cmd()), State::Elsewhere(r"D:\old\overfilm-subs.exe".into()));
        connect(&p, Client::Desktop, &cmd()).unwrap();
        assert_eq!(state_of(&p, true, &cmd()), State::Connected);
    }
}
