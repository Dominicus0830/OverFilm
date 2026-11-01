//! Running the Claude Code command line.
//!
//! One question is one `claude -p` run: the prompt goes in on stdin (no command-line length or
//! quoting limits for Korean text), the answer comes back as `--output-format json` with the
//! answer object checked against `--json-schema` (`structured_output`). The run:
//!
//! - signs in the way the user did in Claude Code (their Claude subscription): `ANTHROPIC_API_KEY`
//!   and `ANTHROPIC_AUTH_TOKEN` are removed from its environment so a key set for something else
//!   is never billed;
//! - has no tools (`--tools ""`), no MCP servers, no saved session, and runs in an empty folder,
//!   so it only reads what it is sent and cannot touch files;
//! - is killed at the time limit or when the stop flag is set;
//! - opens no console window on Windows.
//!
//! An npm install on Windows is a `claude.cmd`; batch files cannot take JSON arguments safely,
//! so the script it wraps (`node …\cli.js`) is run instead, and when that is not found the
//! schema and instructions go in the prompt and the answer is read out of the reply text.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::text::{cap, extract_json};
use crate::{ReviewError, Settings};

/// Most output read from one run.
const MAX_OUTPUT: u64 = 8 << 20;
/// Time limit of the version and sign-in checks.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// One question for the reviewer.
#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    /// What the reviewer is and how to answer (the system prompt).
    pub system: String,
    /// The cases, as text.
    pub prompt: String,
    /// JSON schema of the answer object.
    pub schema: Value,
}

/// Something that answers [`Question`]s: Claude Code, or a stand-in in tests.
pub trait Reviewer: Send + Sync {
    /// The answer object, or why there is none. Must return soon after `stop` is set.
    fn ask(&self, q: &Question, stop: &AtomicBool) -> Result<Value, ReviewError>;
}

#[derive(Clone, Debug, PartialEq)]
enum Launch {
    /// A program (`claude.exe`, the `claude` launcher on macOS/Linux).
    Exe(PathBuf),
    /// The JavaScript CLI behind an npm `claude.cmd`.
    Node { node: PathBuf, script: PathBuf },
    /// A batch file whose script was not found: simple arguments only.
    Batch(PathBuf),
}

/// The Claude Code command line, found and ready to ask.
#[derive(Clone, Debug)]
pub struct ClaudeCode {
    launch: Launch,
    model: String,
    timeout: Duration,
}

/// What [`ClaudeCode::status`] found.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub installed: bool,
    pub path: String,
    pub version: String,
    pub logged_in: bool,
    /// As Claude Code reports it (`claude.ai`, `oauth_token`, …).
    pub auth_method: String,
    /// Signed in with a Claude account (subscription), not an API key.
    pub subscription: bool,
    /// One line for the user.
    pub message: String,
}

impl ClaudeCode {
    /// Find Claude Code as the settings say (their path, else PATH and the usual folders).
    pub fn new(settings: &Settings) -> Result<Self, ReviewError> {
        let launch = locate(&settings.cli_path).ok_or(ReviewError::NotInstalled)?;
        Ok(Self { launch, model: settings.model.clone(), timeout: Duration::from_secs(settings.timeout_secs.max(1)) })
    }

    /// The program that runs.
    pub fn path(&self) -> &Path {
        match &self.launch {
            Launch::Exe(p) | Launch::Batch(p) => p,
            Launch::Node { script, .. } => script,
        }
    }

    fn command(&self, args: &[String]) -> Command {
        let mut cmd = match &self.launch {
            Launch::Exe(p) | Launch::Batch(p) => Command::new(p),
            Launch::Node { node, script } => {
                let mut c = Command::new(node);
                c.arg(script);
                c
            }
        };
        cmd.args(args);
        cmd.env_remove("ANTHROPIC_API_KEY").env_remove("ANTHROPIC_AUTH_TOKEN");
        let dir = std::env::temp_dir().join("overfilm-claude-review");
        if std::fs::create_dir_all(&dir).is_ok() {
            cmd.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd
    }

    /// Installed, its version, signed in and how.
    pub fn status(&self) -> Status {
        let never = AtomicBool::new(false);
        let mut st = Status { installed: true, path: self.path().display().to_string(), ..Status::default() };
        if let Ok((_, out, _)) = run(self.command(&["--version".into()]), "", CHECK_TIMEOUT, &never) {
            // "2.1.316 (Claude Code)" → "2.1.316"
            st.version = out.split_whitespace().next().unwrap_or_default().chars().take(40).collect();
        }
        match run(self.command(&["auth".into(), "status".into(), "--json".into()]), "", CHECK_TIMEOUT, &never) {
            Ok((_, out, err)) => match extract_json(&out) {
                Some(v) => {
                    st.logged_in = v["loggedIn"].as_bool().unwrap_or(false);
                    st.auth_method = v["authMethod"].as_str().unwrap_or_default().chars().take(40).collect();
                    st.subscription = st.logged_in && !st.auth_method.to_ascii_lowercase().contains("api");
                }
                None => st.message = format!("로그인 상태를 읽지 못했습니다: {}", cap(err.trim(), 200)),
            },
            Err(e) => st.message = e.to_string(),
        }
        if st.message.is_empty() {
            st.message = match (st.logged_in, st.subscription) {
                (true, true) => format!("Claude Code {} 사용 준비됨 (구독 계정으로 로그인됨)", st.version),
                (true, false) => {
                    "로그인 방식이 API 키입니다. 구독 계정으로 쓰려면 명령 창에서 `claude` 실행 후 /login 으로 Claude 계정에 로그인해 주세요".into()
                }
                _ => ReviewError::NotLoggedIn.to_string(),
            };
        }
        st
    }

    fn args(&self, q: &Question, structured: bool) -> Vec<String> {
        let mut a: Vec<String> = ["-p", "--output-format", "json", "--no-session-persistence", "--strict-mcp-config"].iter().map(|s| s.to_string()).collect();
        if structured {
            a.extend(["--json-schema".into(), q.schema.to_string(), "--system-prompt".into(), q.system.clone(), "--tools".into(), String::new()]);
        }
        if !self.model.is_empty() {
            a.extend(["--model".into(), self.model.clone()]);
        }
        a
    }
}

impl Reviewer for ClaudeCode {
    fn ask(&self, q: &Question, stop: &AtomicBool) -> Result<Value, ReviewError> {
        let structured = !matches!(self.launch, Launch::Batch(_));
        let input = if structured {
            q.prompt.clone()
        } else {
            format!("{}\n\n{}\n\n답은 아래 JSON 스키마를 따르는 JSON 객체 하나만 쓰세요. 다른 글은 쓰지 마세요.\n{}", q.system, q.prompt, q.schema)
        };
        let (code, out, err) = run(self.command(&self.args(q, structured)), &input, self.timeout, stop)?;
        read_answer(code, &out, &err)
    }
}

/// The answer object out of a `--output-format json` run.
pub fn read_answer(code: Option<i32>, stdout: &str, stderr: &str) -> Result<Value, ReviewError> {
    let Some(v) = extract_json(stdout) else {
        let why = if stderr.trim().is_empty() { stdout } else { stderr };
        return Err(classify(if why.trim().is_empty() { "출력 없음" } else { why }, code));
    };
    let failed = v["is_error"].as_bool().unwrap_or(false) || v["subtype"].as_str().is_some_and(|s| s != "success");
    if failed {
        let text = format!("{} {} {}", v["result"].as_str().unwrap_or_default(), v["subtype"].as_str().unwrap_or_default(), stderr);
        return Err(classify(&text, code));
    }
    if let Some(o) = v.get("structured_output").filter(|o| o.is_object()) {
        return Ok(o.clone());
    }
    if v.get("type").is_none() && v.get("result").is_none() {
        // not Claude Code's envelope: the answer itself
        return Ok(v);
    }
    let text = v["result"].as_str().unwrap_or_default();
    extract_json(text).ok_or_else(|| ReviewError::BadAnswer(cap(text.trim(), 200)))
}

/// Which failure an error text describes.
fn classify(text: &str, code: Option<i32>) -> ReviewError {
    let t = text.to_lowercase();
    let any = |keys: &[&str]| keys.iter().any(|k| t.contains(k));
    if any(&["not logged in", "/login", "log in", "login", "invalid api key", "authentication", "unauthorized", "oauth"]) {
        ReviewError::NotLoggedIn
    } else if any(&["usage limit", "limit reached", "rate limit", "rate_limit", "429", "overloaded", "quota"]) {
        ReviewError::Limit(cap(text.trim(), 200))
    } else {
        let code = code.map(|c| format!(" (종료 코드 {c})")).unwrap_or_default();
        ReviewError::Failed(format!("{}{code}", cap(text.trim(), 300)))
    }
}

/// Run `cmd` with `input` on stdin; (exit code, stdout, stderr). Killed at `timeout` or `stop`.
fn run(mut cmd: Command, input: &str, timeout: Duration, stop: &AtomicBool) -> Result<(Option<i32>, String, String), ReviewError> {
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ReviewError::NotInstalled,
        _ => ReviewError::Failed(e.to_string()),
    })?;
    let stdin = child.stdin.take();
    let bytes = input.as_bytes().to_vec();
    let writer = std::thread::Builder::new().name("claude-stdin".into()).spawn(move || {
        if let Some(mut w) = stdin {
            let _ = w.write_all(&bytes);
        }
    });
    let out = child.stdout.take().and_then(|r| std::thread::Builder::new().name("claude-stdout".into()).spawn(move || read_capped(r)).ok());
    let err = child.stderr.take().and_then(|r| std::thread::Builder::new().name("claude-stderr".into()).spawn(move || read_capped(r)).ok());
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                return Err(ReviewError::Failed(e.to_string()));
            }
        }
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ReviewError::Cancelled);
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ReviewError::Timeout(timeout.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if let Ok(w) = writer {
        let _ = w.join();
    }
    let stdout = out.and_then(|h| h.join().ok()).unwrap_or_default();
    let stderr = err.and_then(|h| h.join().ok()).unwrap_or_default();
    Ok((status.code(), stdout, stderr))
}

fn read_capped(r: impl Read) -> String {
    let mut r = r;
    let mut buf = Vec::new();
    let _ = (&mut r).take(MAX_OUTPUT).read_to_end(&mut buf);
    // keep draining so the program never blocks on a full pipe
    let _ = std::io::copy(&mut r, &mut std::io::sink());
    String::from_utf8_lossy(&buf).into_owned()
}

/// Claude Code as the settings say: their path, else PATH, else the usual install folders.
fn locate(custom: &str) -> Option<Launch> {
    let custom = custom.trim().trim_matches('"');
    if !custom.is_empty() {
        let p = PathBuf::from(custom);
        return p.is_file().then(|| launch_for(p));
    }
    let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd", "claude.bat"] } else { &["claude"] };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    if let Some(h) = &home {
        dirs.push(h.join(".local").join("bin"));
        dirs.push(h.join(".claude").join("local"));
    }
    if let Some(a) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(a).join("npm"));
    }
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file()).map(launch_for)
}

fn is_batch(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
}

/// How to start the file at `p`: a batch file from npm runs the program or script it wraps.
fn launch_for(p: PathBuf) -> Launch {
    if !is_batch(&p) {
        return Launch::Exe(p);
    }
    let Some(dir) = p.parent() else { return Launch::Batch(p) };
    let pkg = dir.join("node_modules").join("@anthropic-ai").join("claude-code");
    for exe in [pkg.join("bin").join("claude.exe"), pkg.join("claude.exe")] {
        if exe.is_file() {
            return Launch::Exe(exe);
        }
    }
    let script = pkg.join("cli.js");
    let node = [dir.join("node.exe")]
        .into_iter()
        .chain(std::env::var_os("PATH").map(|v| std::env::split_paths(&v).map(|d| d.join("node.exe")).collect::<Vec<_>>()).unwrap_or_default())
        .find(|n| n.is_file());
    match node {
        Some(node) if script.is_file() => Launch::Node { node, script },
        _ => Launch::Batch(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_from_structured_output() {
        let out = r#"{"type":"result","subtype":"success","is_error":false,"result":"…","structured_output":{"decision":"cut"}}"#;
        assert_eq!(read_answer(Some(0), out, "").unwrap()["decision"], "cut");
    }

    #[test]
    fn answer_from_result_text() {
        let out = r#"{"type":"result","subtype":"success","is_error":false,"result":"네.\n```json\n{\"decision\": \"keep\"}\n```"}"#;
        assert_eq!(read_answer(Some(0), out, "").unwrap()["decision"], "keep");
        let bad = r#"{"type":"result","subtype":"success","is_error":false,"result":"잘 모르겠습니다"}"#;
        assert!(matches!(read_answer(Some(0), bad, ""), Err(ReviewError::BadAnswer(_))));
    }

    #[test]
    fn failures_are_told_apart() {
        let login = r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}"#;
        assert_eq!(read_answer(Some(1), login, ""), Err(ReviewError::NotLoggedIn));
        let limit = r#"{"type":"result","subtype":"success","is_error":true,"result":"Claude AI usage limit reached|1762000000"}"#;
        assert!(matches!(read_answer(Some(1), limit, ""), Err(ReviewError::Limit(_))));
        assert!(matches!(read_answer(Some(1), "", "segfault"), Err(ReviewError::Failed(_))));
        assert!(matches!(read_answer(None, "", ""), Err(ReviewError::Failed(_))));
        let turns = r#"{"type":"result","subtype":"error_max_turns","is_error":false}"#;
        assert!(matches!(read_answer(Some(0), turns, ""), Err(ReviewError::Failed(_))));
    }

    #[test]
    fn custom_path_missing_is_not_installed() {
        let s = Settings { cli_path: "/nonexistent/claude".into(), ..Settings::default() };
        assert!(matches!(ClaudeCode::new(&s), Err(ReviewError::NotInstalled)));
    }

    #[test]
    fn batch_files_are_unwrapped() {
        let dir = std::env::temp_dir().join(format!("overfilm-review-npm-{}", std::process::id()));
        let pkg = dir.join("node_modules").join("@anthropic-ai").join("claude-code");
        std::fs::create_dir_all(&pkg).unwrap();
        let cmd = dir.join("claude.cmd");
        std::fs::write(&cmd, "@echo off").unwrap();
        // no script: batch mode
        assert_eq!(launch_for(cmd.clone()), Launch::Batch(cmd.clone()));
        std::fs::write(pkg.join("cli.js"), "").unwrap();
        std::fs::write(dir.join("node.exe"), "").unwrap();
        assert_eq!(launch_for(cmd.clone()), Launch::Node { node: dir.join("node.exe"), script: pkg.join("cli.js") });
        std::fs::create_dir_all(pkg.join("bin")).unwrap();
        std::fs::write(pkg.join("bin").join("claude.exe"), "").unwrap();
        assert_eq!(launch_for(cmd), Launch::Exe(pkg.join("bin").join("claude.exe")));
        assert_eq!(launch_for(dir.join("claude")), Launch::Exe(dir.join("claude")));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A stand-in `claude` (a shell script) checks the arguments, stdin, the removed API key, the
    /// time limit and the stop flag.
    #[cfg(unix)]
    #[test]
    fn runs_a_fake_cli() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("overfilm-review-fake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("claude");
        std::fs::write(
            &fake,
            r#"#!/bin/sh
case "$1" in
  --version) echo "9.9.9 (Claude Code)"; exit 0;;
  auth) echo '{"loggedIn": true, "authMethod": "claude.ai"}'; exit 0;;
esac
input=$(cat)
case "$input" in *slow*) sleep 30;; esac
key=${ANTHROPIC_API_KEY:-none}
args=$(echo "$*" | tr '"' "'")
printf '{"type":"result","subtype":"success","is_error":false,"result":"","structured_output":{"key":"%s","args":"%s","input":"%s"}}' "$key" "$args" "$input" | tr '\n' ' '
"#,
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let s = Settings { cli_path: fake.display().to_string(), model: "sonnet".into(), timeout_secs: 2, ..Settings::default() };
        let cc = ClaudeCode::new(&s).unwrap();
        let st = cc.status();
        assert!(st.installed && st.logged_in && st.subscription, "{st:?}");
        assert_eq!(st.version, "9.9.9");
        let q = Question { system: "sys".into(), prompt: "질문".into(), schema: serde_json::json!({"type": "object"}) };
        let stop = AtomicBool::new(false);
        let v = cc.ask(&q, &stop).unwrap();
        assert_eq!(v["key"], "none");
        assert_eq!(v["input"], "질문");
        let args = v["args"].as_str().unwrap();
        assert!(args.contains("--json-schema") && args.contains("--model sonnet") && args.contains("--no-session-persistence"), "{args}");
        let slow = Question { prompt: "slow".into(), ..q.clone() };
        let t0 = Instant::now();
        assert_eq!(cc.ask(&slow, &stop), Err(ReviewError::Timeout(2)));
        assert!(t0.elapsed() < Duration::from_secs(10));
        stop.store(true, Ordering::Relaxed);
        assert_eq!(cc.ask(&slow, &stop), Err(ReviewError::Cancelled));
        let _ = std::fs::remove_dir_all(dir);
    }
}
