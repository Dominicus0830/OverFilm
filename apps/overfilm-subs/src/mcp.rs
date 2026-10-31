//! MCP server (JSON-RPC 2.0, one message per line on stdin/stdout) for caption extraction and
//! review from an assistant such as Claude Desktop. Extraction runs in the background (it takes
//! minutes); everything else reads or edits an SRT file and returns at once.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use overfilm_captions::Caption;
use serde_json::{Value, json};

use crate::job::{self, Options};
use crate::pack::{self, Mode};
use crate::review;

/// Longest request line read (an SRT path and a few fields; caps a hostile client).
const MAX_LINE: usize = 1 << 20;
/// Most issues or captions returned at once.
const MAX_ITEMS: usize = 500;

#[derive(Clone, Default, serde::Serialize)]
struct JobState {
    id: u64,
    input: String,
    running: bool,
    progress: f32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<job::Done>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Default)]
pub struct Server {
    job: Arc<Mutex<JobState>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

fn tools() -> Value {
    let path = json!({"type": "string", "description": "SRT 파일 경로"});
    let index = json!({"type": "integer", "minimum": 1, "description": "자막 번호 (1부터)"});
    json!([
        {"name": "extract_subtitles", "description": "영상/오디오 파일에서 한국어 자막(SRT)을 뽑습니다. 몇 분 걸리므로 바로 돌아오고, job_status로 진행을 확인합니다. 결과는 입력 파일 옆 .srt와 .overfilm.json(단어 시간)입니다.",
         "inputSchema": {"type": "object", "required": ["input"], "properties": {
            "input": {"type": "string", "description": "영상 또는 오디오 파일 경로"},
            "output": {"type": "string", "description": "저장할 SRT 경로 (기본: 입력 이름.srt)"},
            "mode": {"type": "string", "enum": ["general", "commentary"], "description": "general: 일반 게임 영상, commentary: 대회 해설 (팀·선수 이름 포함)"},
            "extra_terms": {"type": "string", "description": "인식에 도움될 이름·용어, 쉼표로 구분"},
            "mask_profanity": {"type": "boolean", "description": "욕설을 초성으로 (기본 true)"},
            "gpu": {"type": "boolean", "description": "그래픽카드(GPU)가 있으면 사용 (기본 true, 없거나 오류면 CPU)"}}}},
        {"name": "job_status", "description": "자막 추출 작업의 진행률, 메시지, 끝났으면 결과 파일과 단어집 교정 목록.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "cancel_job", "description": "진행 중인 자막 추출을 멈춥니다.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "read_subtitles", "description": "SRT 자막을 번호·시간·글자로 읽습니다.",
         "inputSchema": {"type": "object", "required": ["path"], "properties": {"path": path, "from": index, "count": {"type": "integer", "minimum": 1, "maximum": MAX_ITEMS}}}},
        {"name": "review_subtitles", "description": "검토할 곳을 찾습니다: 단어집과 다른 표기(오인식·욕설, 고칠 글자 제안 포함), 읽기 속도, 줄 길이·줄 수, 너무 짧음, 겹침, 반복, 빈 자막.",
         "inputSchema": {"type": "object", "required": ["path"], "properties": {"path": path, "kinds": {"type": "array", "items": {"type": "string", "enum": ["glossary", "fast", "width", "lines", "short", "overlap", "repeat", "empty"]}}}}},
        {"name": "find_silences", "description": "자막 사이 말이 없는 구간(무음)을 찾습니다. 편집할 때 자를 후보로 표시만 하고 지우지는 않습니다.",
         "inputSchema": {"type": "object", "required": ["path"], "properties": {"path": path, "min_ms": {"type": "integer", "minimum": 200, "description": "이보다 긴 빈 구간 (기본 2000)"}}}},
        {"name": "edit_subtitle", "description": "자막 하나의 글자나 시간을 고칩니다. 줄바꿈은 \\n. 처음 고칠 때 .srt.bak 백업을 만듭니다.",
         "inputSchema": {"type": "object", "required": ["path", "index"], "properties": {"path": path, "index": index, "text": {"type": "string"},
            "start": {"type": "string", "description": "HH:MM:SS,mmm"}, "end": {"type": "string", "description": "HH:MM:SS,mmm"}}}},
        {"name": "split_subtitle", "description": "자막 하나를 두 개로 나눕니다. before_text로 끝나는 앞부분과 나머지로, 시간은 글자 수 비율로 나눕니다.",
         "inputSchema": {"type": "object", "required": ["path", "index", "before_text"], "properties": {"path": path, "index": index, "before_text": {"type": "string"}}}},
        {"name": "merge_subtitles", "description": "자막 index와 바로 다음 자막을 하나로 합칩니다.",
         "inputSchema": {"type": "object", "required": ["path", "index"], "properties": {"path": path, "index": index}}},
        {"name": "delete_subtitle", "description": "자막 하나를 지웁니다.",
         "inputSchema": {"type": "object", "required": ["path", "index"], "properties": {"path": path, "index": index}}},
        {"name": "apply_glossary", "description": "단어집 교정(오인식 표기, 욕설 초성, 이름 근접 오타)을 자막에 적용합니다. indices가 없으면 전체.",
         "inputSchema": {"type": "object", "required": ["path"], "properties": {"path": path, "indices": {"type": "array", "items": {"type": "integer"}}}}},
        {"name": "relayout_subtitles", "description": "각 자막의 줄바꿈을 다시 합니다 (최대 두 줄, 한 줄 line_chars자).",
         "inputSchema": {"type": "object", "required": ["path"], "properties": {"path": path, "line_chars": {"type": "integer", "minimum": 4, "maximum": 60}}}},
        {"name": "add_glossary_term", "description": "자주 틀리는 표기를 사용자 단어집에 더합니다 (다음 추출과 apply_glossary부터 적용). 예: ko=정커퀸, misheard=[전국퀸].",
         "inputSchema": {"type": "object", "required": ["ko"], "properties": {"ko": {"type": "string"}, "misheard": {"type": "array", "items": {"type": "string"}}, "note": {"type": "string"}}}}
    ])
}

fn s<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str)
}

fn path_arg(a: &Value) -> Result<PathBuf, String> {
    s(a, "path").map(PathBuf::from).ok_or_else(|| "path가 필요합니다".to_string())
}

fn index_arg(a: &Value, caps: &[Caption]) -> Result<usize, String> {
    let i = a.get("index").and_then(Value::as_u64).ok_or("index가 필요합니다")?;
    let i = usize::try_from(i).map_err(|e| e.to_string())?;
    if i == 0 || i > caps.len() {
        return Err(format!("자막 번호 {i}이(가) 없습니다 (1–{})", caps.len()));
    }
    Ok(i - 1)
}

fn cap_json(i: usize, c: &Caption) -> Value {
    json!({"index": i + 1, "start": review::clock(c.start_ms), "end": review::clock(c.end_ms), "text": c.text(), "cps": review::cps(c)})
}

/// Save with a one-time `.srt.bak` of the original.
fn save(path: &Path, caps: &[Caption]) -> Result<(), String> {
    let bak = PathBuf::from(format!("{}.bak", path.display()));
    if !bak.exists() && path.exists() {
        std::fs::copy(path, &bak).map_err(|e| format!("백업 실패 {}: {e}", bak.display()))?;
    }
    job::write_srt(path, caps)
}

/// The user glossary folder (`glossary/` beside the models), created on first use.
pub fn user_glossary_dir() -> PathBuf {
    job::models_dir().parent().map(|p| p.join("glossary")).unwrap_or_else(|| PathBuf::from("glossary"))
}

fn glossary() -> Result<overfilm_glossary::Glossary, String> {
    let d = user_glossary_dir();
    pack::glossary(d.is_dir().then_some(d.as_path()), true)
}

fn split_at_text(c: &Caption, before: &str) -> Result<(Caption, Caption), String> {
    let full = c.text().replace('\n', " ");
    let before = before.trim();
    let Some(pos) = full.find(before).map(|p| p + before.len()).filter(|_| !before.is_empty()) else {
        return Err("before_text가 자막 안에 없습니다".into());
    };
    let (a, b) = full.split_at(pos);
    let (a, b) = (a.trim().to_string(), b.trim().to_string());
    if a.is_empty() || b.is_empty() {
        return Err("나눈 한쪽이 비어 있습니다".into());
    }
    let la = i64::from(review::letters(&a));
    let total = (la + i64::from(review::letters(&b))).max(1);
    let mid = c.start_ms + (c.end_ms - c.start_ms) * la / total;
    Ok((Caption { start_ms: c.start_ms, end_ms: mid, lines: vec![a] }, Caption { start_ms: mid, end_ms: c.end_ms, lines: vec![b] }))
}

fn relayout(c: &Caption, line_chars: u32) -> Caption {
    let o = overfilm_captions::Layout { max_line: line_chars * 2, max_cue_ms: i64::MAX / 4, pause_ms: i64::MAX / 4, ..Default::default() };
    // one pseudo-word per word, evenly spread, so the layout only decides the line break
    let words: Vec<overfilm_captions::TimedWord> =
        c.text().split_whitespace().map(|w| overfilm_captions::TimedWord { text: w.to_string(), start_ms: c.start_ms, end_ms: c.start_ms }).collect();
    let mut lines: Vec<String> = overfilm_captions::layout(&words, &o).into_iter().flat_map(|x| x.lines).collect();
    if lines.is_empty() {
        lines = c.lines.clone();
    }
    // the layout may make more than one caption out of a long text: keep it in this one
    if lines.len() > 2 {
        let all = lines.join(" ");
        lines = vec![all];
    }
    Caption { start_ms: c.start_ms, end_ms: c.end_ms, lines }
}

impl Server {
    fn extract(&self, a: &Value) -> Result<Value, String> {
        let input = s(a, "input").ok_or("input이 필요합니다")?;
        let mut o = Options::new(PathBuf::from(input));
        o.output = s(a, "output").map(PathBuf::from);
        if let Some(m) = s(a, "mode") {
            o.mode = Mode::parse(m).ok_or_else(|| format!("모르는 mode: {m}"))?;
        }
        o.extra_terms = s(a, "extra_terms").unwrap_or_default().to_string();
        o.mask_profanity = a.get("mask_profanity").and_then(Value::as_bool).unwrap_or(true);
        o.gpu = a.get("gpu").and_then(Value::as_bool).unwrap_or(true);
        let d = user_glossary_dir();
        o.glossary_dir = d.is_dir().then_some(d);
        if !o.input.is_file() {
            return Err(format!("파일이 없습니다: {input}"));
        }
        let mut st = self.job.lock().unwrap_or_else(PoisonError::into_inner);
        if st.running {
            return Err(format!("이미 작업 중입니다: {} ({:.0}%)", st.input, st.progress * 100.0));
        }
        let id = st.id + 1;
        *st = JobState { id, input: input.to_string(), running: true, message: "시작".into(), ..JobState::default() };
        drop(st);
        self.cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        let (job_state, cancel) = (self.job.clone(), self.cancel.clone());
        std::thread::spawn(move || {
            let r = job::guarded(|| {
                job::run(&o, &mut |f, m| {
                    let mut st = job_state.lock().unwrap_or_else(PoisonError::into_inner);
                    st.progress = f;
                    st.message = m.to_string();
                    !cancel.load(std::sync::atomic::Ordering::Relaxed)
                })
            });
            let mut st = job_state.lock().unwrap_or_else(PoisonError::into_inner);
            st.running = false;
            match r {
                Ok(done) => st.result = Some(done),
                Err(e) => st.error = Some(e),
            }
        });
        Ok(json!({"job": id, "message": "자막 추출을 시작했습니다. job_status로 확인하세요."}))
    }

    fn edit(&self, a: &Value) -> Result<Value, String> {
        let p = path_arg(a)?;
        let mut caps = job::read_srt(&p)?;
        let i = index_arg(a, &caps)?;
        let c = caps.get_mut(i).ok_or("자막 번호가 없습니다")?;
        if let Some(t) = s(a, "text") {
            c.lines = t.replace("\\n", "\n").lines().map(str::to_string).collect();
        }
        if let Some(t) = s(a, "start") {
            c.start_ms = review::parse_clock(t).ok_or_else(|| format!("시간 형식이 아닙니다: {t}"))?;
        }
        if let Some(t) = s(a, "end") {
            c.end_ms = review::parse_clock(t).ok_or_else(|| format!("시간 형식이 아닙니다: {t}"))?;
        }
        if c.end_ms <= c.start_ms {
            return Err("끝 시간이 시작보다 빠릅니다".into());
        }
        let out = cap_json(i, c);
        save(&p, &caps)?;
        Ok(out)
    }

    fn structure(&self, name: &str, a: &Value) -> Result<Value, String> {
        let p = path_arg(a)?;
        let mut caps = job::read_srt(&p)?;
        let i = index_arg(a, &caps)?;
        match name {
            "split_subtitle" => {
                let c = caps.get(i).ok_or("자막 번호가 없습니다")?;
                let (x, y) = split_at_text(c, s(a, "before_text").unwrap_or_default())?;
                caps.splice(i..=i, [x, y]);
            }
            "merge_subtitles" => {
                let next = caps.get(i + 1).cloned().ok_or("다음 자막이 없습니다")?;
                let c = caps.get_mut(i).ok_or("자막 번호가 없습니다")?;
                let text = format!("{} {}", c.text().replace('\n', " "), next.text().replace('\n', " "));
                c.end_ms = next.end_ms.max(c.end_ms);
                c.lines = vec![text];
                *c = relayout(c, 16);
                caps.remove(i + 1);
            }
            _ => {
                caps.remove(i);
            }
        }
        save(&p, &caps)?;
        let around: Vec<Value> = caps.iter().enumerate().skip(i.saturating_sub(1)).take(3).map(|(k, c)| cap_json(k, c)).collect();
        Ok(json!({"captions": caps.len(), "around": around}))
    }

    fn call(&self, name: &str, a: &Value) -> Result<Value, String> {
        match name {
            "extract_subtitles" => self.extract(a),
            "job_status" => Ok(serde_json::to_value(&*self.job.lock().unwrap_or_else(PoisonError::into_inner)).unwrap_or_default()),
            "cancel_job" => {
                self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                Ok(json!({"message": "멈추는 중입니다"}))
            }
            "read_subtitles" => {
                let caps = job::read_srt(&path_arg(a)?)?;
                let from = a.get("from").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
                let count = (a.get("count").and_then(Value::as_u64).unwrap_or(50) as usize).min(MAX_ITEMS);
                let items: Vec<Value> = caps.iter().enumerate().skip(from - 1).take(count).map(|(i, c)| cap_json(i, c)).collect();
                Ok(json!({"total": caps.len(), "captions": items}))
            }
            "review_subtitles" => {
                let caps = job::read_srt(&path_arg(a)?)?;
                let kinds: Vec<&str> = a.get("kinds").and_then(Value::as_array).map(|v| v.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                let g = glossary()?;
                let issues: Vec<review::Issue> = review::review(&caps, &overfilm_captions::Layout::default(), &pack::corrector(&g))
                    .into_iter()
                    .filter(|i| kinds.is_empty() || kinds.contains(&i.kind))
                    .collect();
                let mut counts = std::collections::BTreeMap::new();
                for i in &issues {
                    *counts.entry(i.kind).or_insert(0usize) += 1;
                }
                let shown: Vec<Value> = issues
                    .iter()
                    .take(MAX_ITEMS)
                    .map(|i| {
                        let mut v = serde_json::to_value(i).unwrap_or_default();
                        if let (Some(c), Some(o)) = (caps.get(i.index.saturating_sub(1)), v.as_object_mut()) {
                            o.insert("text".into(), json!(c.text()));
                            o.insert("start".into(), json!(review::clock(c.start_ms)));
                        }
                        v
                    })
                    .collect();
                Ok(json!({"captions": caps.len(), "counts": counts, "issues": shown, "truncated": issues.len() > MAX_ITEMS}))
            }
            "find_silences" => {
                let caps = job::read_srt(&path_arg(a)?)?;
                let min = a.get("min_ms").and_then(Value::as_i64).unwrap_or(2000).max(200);
                let list: Vec<Value> = review::silences(&caps, min)
                    .iter()
                    .take(MAX_ITEMS)
                    .map(|g| json!({"after_caption": g.after, "start": review::clock(g.start_ms), "end": review::clock(g.end_ms), "seconds": (g.end_ms - g.start_ms) as f64 / 1000.0}))
                    .collect();
                Ok(json!({"silences": list}))
            }
            "edit_subtitle" => self.edit(a),
            "split_subtitle" | "merge_subtitles" | "delete_subtitle" => self.structure(name, a),
            "apply_glossary" => {
                let p = path_arg(a)?;
                let mut caps = job::read_srt(&p)?;
                let only: Option<Vec<u64>> = a.get("indices").and_then(Value::as_array).map(|v| v.iter().filter_map(Value::as_u64).collect());
                let corrector = pack::corrector(&glossary()?);
                let mut changes = Vec::new();
                for (k, c) in caps.iter_mut().enumerate() {
                    if only.as_ref().is_some_and(|o| !o.contains(&(k as u64 + 1))) {
                        continue;
                    }
                    let lines: Vec<_> = c.lines.iter().map(|l| corrector.apply(l)).collect();
                    for l in &lines {
                        changes.extend(l.changes.iter().map(|(x, y)| json!({"index": k + 1, "from": x, "to": y})));
                    }
                    c.lines = lines.into_iter().map(|l| l.text).collect();
                }
                if !changes.is_empty() {
                    save(&p, &caps)?;
                }
                Ok(json!({"changed": changes.len(), "changes": changes}))
            }
            "relayout_subtitles" => {
                let p = path_arg(a)?;
                let n = a.get("line_chars").and_then(Value::as_u64).unwrap_or(16).clamp(4, 60) as u32;
                let caps: Vec<Caption> = job::read_srt(&p)?.iter().map(|c| relayout(c, n)).collect();
                save(&p, &caps)?;
                Ok(json!({"captions": caps.len()}))
            }
            "add_glossary_term" => add_term(a),
            _ => Err(format!("모르는 도구: {name}")),
        }
    }

    /// Answer one JSON-RPC message; None for notifications.
    pub fn handle(&self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned()?;
        let method = s(msg, "method").unwrap_or_default();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => json!({
                "protocolVersion": s(&params, "protocolVersion").unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "overfilm-subs", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "오버워치 영상 한국어 자막 추출·검토 도구입니다. extract_subtitles로 SRT를 만들고 job_status로 기다린 뒤, review_subtitles로 문제를 찾아 edit_subtitle/apply_glossary로 고치세요. 자주 틀리는 표기는 add_glossary_term으로 단어집에 더하세요."
            }),
            "ping" => json!({}),
            "tools/list" => json!({"tools": tools()}),
            "tools/call" => {
                let name = s(&params, "name").unwrap_or_default();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match job::guarded(|| self.call(name, &args)) {
                    Ok(v) => json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default()}], "isError": false}),
                    Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
                }
            }
            _ => return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("method not found: {method}")}})),
        };
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }
}

fn add_term(a: &Value) -> Result<Value, String> {
    let ko = s(a, "ko").map(str::trim).filter(|k| !k.is_empty()).ok_or("ko가 필요합니다")?;
    let clean = |x: &str| x.replace(['\t', '\n', '\r', ';'], " ").trim().to_string();
    let misheard: Vec<String> = a
        .get("misheard")
        .and_then(Value::as_array)
        .map(|v| v.iter().filter_map(Value::as_str).map(clean).filter(|m| !m.is_empty()).collect())
        .unwrap_or_default();
    let dir = user_glossary_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join("user.tsv");
    let mut text = std::fs::read_to_string(&file).unwrap_or_else(|_| "id\tko\ten\taliases\told\tmisheard\tflags\tnote\n".to_string());
    let n = text.lines().count();
    let row = format!("user{n}\t{}\t\t\t\t{}\t\t{}\n", clean(ko), misheard.join("; "), clean(s(a, "note").unwrap_or_default()));
    text.push_str(&row);
    // check the table still parses before saving it
    overfilm_glossary::Glossary::default().add_table("user", &text).map_err(|e| e.to_string())?;
    std::fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(json!({"saved": file, "ko": ko, "misheard": misheard}))
}

/// Serve on stdin/stdout until stdin closes.
pub fn serve() -> Result<(), String> {
    let server = Server::default();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut line = String::new();
    let mut lock = stdin.lock();
    loop {
        line.clear();
        let n = std::io::Read::take(&mut lock, MAX_LINE as u64).read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(());
        }
        let reply = match serde_json::from_str::<Value>(line.trim()) {
            Ok(msg) => server.handle(&msg),
            Err(_) if line.trim().is_empty() => None,
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}})),
        };
        if let Some(r) = reply {
            writeln!(out, "{r}").and_then(|_| out.flush()).map_err(|e| e.to_string())?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(s: &Server, name: &str, args: Value) -> Value {
        let r = s.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}})).unwrap();
        r["result"].clone()
    }

    fn text(v: &Value) -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap_or(Value::Null)
    }

    #[test]
    fn protocol_basics() {
        let s = Server::default();
        let init = s.handle(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}})).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(s.handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
        let list = s.handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
        assert!(list["result"]["tools"].as_array().unwrap().len() >= 10);
        let bad = s.handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "nope"})).unwrap();
        assert_eq!(bad["error"]["code"], -32601);
        let r = call(&s, "read_subtitles", json!({"path": "/nonexistent.srt"}));
        assert_eq!(r["isError"], true);
    }

    #[test]
    fn review_and_edit_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.srt");
        std::fs::write(&p, "1\n00:00:01,000 --> 00:00:03,000\n씨발 겐지 나이스\n\n2\n00:00:03,000 --> 00:00:05,000\n두 번째 자막입니다\n\n").unwrap();
        let s = Server::default();
        let ps = p.to_string_lossy().to_string();
        let rv = text(&call(&s, "review_subtitles", json!({"path": ps, "kinds": ["glossary"]})));
        assert_eq!(rv["issues"][0]["index"], 1, "{rv}");
        let ap = text(&call(&s, "apply_glossary", json!({"path": ps})));
        assert!(ap["changed"].as_u64().unwrap() >= 1);
        assert!(std::fs::read_to_string(&p).unwrap().contains("ㅅㅂ"));
        assert!(d.path().join("a.srt.bak").exists());
        let ed = text(&call(&s, "edit_subtitle", json!({"path": ps, "index": 2, "text": "고친 자막", "end": "00:00:06,000"})));
        assert_eq!(ed["text"], "고친 자막");
        let sp = text(&call(&s, "split_subtitle", json!({"path": ps, "index": 2, "before_text": "고친"})));
        assert_eq!(sp["captions"], 3);
        let mg = text(&call(&s, "merge_subtitles", json!({"path": ps, "index": 2})));
        assert_eq!(mg["captions"], 2);
        let bad = call(&s, "edit_subtitle", json!({"path": ps, "index": 99}));
        assert_eq!(bad["isError"], true);
        let gaps = text(&call(&s, "find_silences", json!({"path": ps, "min_ms": 500})));
        assert_eq!(gaps["silences"][0]["end"], "00:00:01,000");
        let del = text(&call(&s, "delete_subtitle", json!({"path": ps, "index": 1})));
        assert_eq!(del["captions"], 1);
    }

    #[test]
    fn extraction_errors_come_back_as_tool_errors() {
        let s = Server::default();
        let r = call(&s, "extract_subtitles", json!({"input": "/nonexistent.mp4"}));
        assert_eq!(r["isError"], true);
        let st = text(&call(&s, "job_status", json!({})));
        assert_eq!(st["running"], false);
    }
}
