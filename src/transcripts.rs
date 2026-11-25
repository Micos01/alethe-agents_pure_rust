//! Chat titles and per-terminal cost from Claude Code transcripts
//! (`claude_sessions.rs` `get_claude_session_title`, `agent_cost.rs` `get_session_cost`).
//!
//! Each Claude tab is started with `--session-id`, so its transcript is
//! `~/.claude/projects/<encoded cwd>/<session id>.jsonl`. A background thread follows the running
//! tabs every 4 s and reads only what was appended since the last pass: a long session is read
//! once, not once per poll.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

const POLL: Duration = Duration::from_secs(4);
/// Claude truncates longer project dir names and appends a hash.
const CLAUDE_DIR_NAME_LIMIT: usize = 200;

/// A running Claude tab to follow.
#[derive(Clone, PartialEq, Debug)]
pub struct Live {
    pub tab_id: String,
    pub cwd: String,
    pub session_id: String,
}

#[derive(Clone, Debug, Default)]
pub struct SessionStats {
    /// `ai-title`, else the first prompt.
    pub title: Option<String>,
    /// None when no model in the session has a known price.
    pub cost_usd: Option<f64>,
    pub total_tokens: u64,
    /// The model with the most output.
    pub model: Option<String>,
    pub cwd: String,
}

pub struct TranscriptWatcher {
    live: Arc<Mutex<Vec<Live>>>,
    stats: Arc<Mutex<HashMap<String, SessionStats>>>,
}

impl TranscriptWatcher {
    pub fn start(ctx: egui::Context) -> Self {
        let watcher = Self {
            live: Arc::default(),
            stats: Arc::default(),
        };
        let live = Arc::clone(&watcher.live);
        let stats = Arc::clone(&watcher.stats);
        let _ = std::thread::Builder::new()
            .name("transcripts".into())
            .spawn(move || {
                let mut trackers: HashMap<String, Tracker> = HashMap::new();
                loop {
                    let tabs = live.lock().unwrap().clone();
                    trackers.retain(|id, t| {
                        tabs.iter()
                            .any(|l| &l.tab_id == id && l.session_id == t.session_id)
                    });
                    let mut changed = false;
                    for tab in &tabs {
                        let tracker = trackers
                            .entry(tab.tab_id.clone())
                            .or_insert_with(|| Tracker::new(&tab.session_id));
                        changed |= tracker.poll(&tab.cwd);
                    }
                    let next: HashMap<String, SessionStats> = tabs
                        .iter()
                        .filter_map(|tab| {
                            let t = trackers.get(&tab.tab_id)?;
                            t.path.as_ref()?;
                            Some((tab.tab_id.clone(), t.stats(&tab.cwd)))
                        })
                        .collect();
                    let mut shared = stats.lock().unwrap();
                    if changed || shared.len() != next.len() {
                        *shared = next;
                        ctx.request_repaint();
                    }
                    drop(shared);
                    std::thread::sleep(POLL);
                }
            });
        watcher
    }

    /// The Claude tabs running now; tabs no longer listed are forgotten.
    pub fn set_live(&self, tabs: Vec<Live>) {
        let mut live = self.live.lock().unwrap();
        if *live != tabs {
            *live = tabs;
        }
    }

    pub fn stats(&self, tab_id: &str) -> Option<SessionStats> {
        self.stats.lock().unwrap().get(tab_id).cloned()
    }

    pub fn all(&self) -> Vec<(String, SessionStats)> {
        let mut rows: Vec<_> = self
            .stats
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq)]
struct Usage {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
}

impl Usage {
    fn from_json(usage: &Value) -> Self {
        let u = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
        let cc = usage.get("cache_creation");
        let split = |k: &str| cc.and_then(|c| c.get(k)).and_then(Value::as_u64);
        let (cache_write_5m, cache_write_1h) = match (
            split("ephemeral_5m_input_tokens"),
            split("ephemeral_1h_input_tokens"),
        ) {
            (Some(a), Some(b)) => (a, b),
            _ => (u("cache_creation_input_tokens"), 0),
        };
        Self {
            input: u("input_tokens"),
            output: u("output_tokens"),
            cache_read: u("cache_read_input_tokens"),
            cache_write_5m,
            cache_write_1h,
        }
    }

    fn add(&mut self, other: Usage, sign: i64) {
        let f = |a: &mut u64, b: u64| {
            *a = if sign >= 0 {
                a.saturating_add(b)
            } else {
                a.saturating_sub(b)
            }
        };
        f(&mut self.input, other.input);
        f(&mut self.output, other.output);
        f(&mut self.cache_read, other.cache_read);
        f(&mut self.cache_write_5m, other.cache_write_5m);
        f(&mut self.cache_write_1h, other.cache_write_1h);
    }

    fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write_5m + self.cache_write_1h
    }

    /// `pricing_for` in `agent_cost.rs`: $/MTok, cache writes 1.25x (5 m) and 2x (1 h), reads 0.1x.
    fn cost(&self, model: &str) -> Option<f64> {
        let m = model.to_ascii_lowercase();
        let (input, output) = if m.contains("opus") {
            (5.0, 25.0)
        } else if m.contains("sonnet") {
            (3.0, 15.0)
        } else if m.contains("haiku") {
            (1.0, 5.0)
        } else {
            return None;
        };
        let per = |tokens: u64, rate: f64| tokens as f64 / 1_000_000.0 * rate;
        Some(
            per(self.input, input)
                + per(self.output, output)
                + per(self.cache_read, input * 0.1)
                + per(self.cache_write_5m, input * 1.25)
                + per(self.cache_write_1h, input * 2.0),
        )
    }
}

/// Follows one transcript.
struct Tracker {
    session_id: String,
    path: Option<PathBuf>,
    offset: u64,
    by_model: HashMap<String, Usage>,
    ai_title: Option<String>,
    first_prompt: Option<String>,
    /// A streamed reply is written once per content block, each line repeating the message's
    /// usage: only the last line of a message counts.
    last_message: Option<(String, String, Usage)>,
}

impl Tracker {
    fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            path: None,
            offset: 0,
            by_model: HashMap::new(),
            ai_title: None,
            first_prompt: None,
            last_message: None,
        }
    }

    /// Reads what was appended; true when anything new was read.
    fn poll(&mut self, cwd: &str) -> bool {
        if self.path.is_none() {
            self.path = transcript_path(cwd, &self.session_id);
        }
        let Some(path) = self.path.clone() else {
            return false;
        };
        let Ok(len) = std::fs::metadata(&path).map(|m| m.len()) else {
            return false;
        };
        if len < self.offset {
            // Rewritten from scratch: start over.
            *self = Self {
                path: Some(path.clone()),
                ..Self::new(&self.session_id)
            };
        }
        if len == self.offset {
            return false;
        }
        let Ok(mut file) = File::open(&path) else {
            return false;
        };
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return false;
        }
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let mut line = Vec::new();
        let mut read_any = false;
        loop {
            line.clear();
            let Ok(n) = reader.read_until(b'\n', &mut line) else {
                break;
            };
            // A line still being written has no newline yet: leave it for the next pass.
            if n == 0 || line.last() != Some(&b'\n') {
                break;
            }
            self.offset += n as u64;
            read_any = true;
            self.ingest(&line);
        }
        read_any
    }

    fn ingest(&mut self, line: &[u8]) {
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("ai-title") => {
                if let Some(title) = value
                    .get("aiTitle")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                {
                    self.ai_title = Some(title.to_string());
                }
            }
            Some("user") if self.first_prompt.is_none() => {
                let meta = value.get("isMeta").and_then(Value::as_bool) == Some(true);
                if !meta {
                    self.first_prompt = first_text(&value)
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty() && !t.starts_with('<'));
                }
            }
            _ => {}
        }
        let Some(message) = value.get("message") else {
            return;
        };
        let Some(usage) = message.get("usage") else {
            return;
        };
        let model = message
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let usage = Usage::from_json(usage);
        let id = message
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if let Some((last_id, last_model, last_usage)) = &self.last_message {
            if !id.is_empty() && *last_id == id {
                if let Some(entry) = self.by_model.get_mut(last_model) {
                    entry.add(*last_usage, -1);
                }
            }
        }
        self.by_model
            .entry(model.clone())
            .or_default()
            .add(usage, 1);
        self.last_message = Some((id, model, usage));
    }

    fn stats(&self, cwd: &str) -> SessionStats {
        let mut cost: Option<f64> = None;
        let mut total = 0;
        let mut dominant: Option<(u64, &str)> = None;
        for (model, usage) in &self.by_model {
            total += usage.total();
            if let Some(c) = usage.cost(model) {
                cost = Some(cost.unwrap_or(0.0) + c);
            }
            if model != "<synthetic>" && dominant.is_none_or(|(out, _)| usage.output > out) {
                dominant = Some((usage.output, model));
            }
        }
        SessionStats {
            title: self
                .ai_title
                .clone()
                .or_else(|| self.first_prompt.clone())
                .map(|t| truncate(&t, 80)),
            cost_usd: cost,
            total_tokens: total,
            model: dominant.map(|(_, m)| m.to_string()),
            cwd: cwd.to_string(),
        }
    }
}

fn first_text(value: &Value) -> Option<String> {
    match value.pointer("/message/content")? {
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => blocks
            .iter()
            .find_map(|b| b.get("text").and_then(Value::as_str).map(String::from)),
        _ => None,
    }
}

fn truncate(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() <= max {
        return line.to_string();
    }
    let mut out: String = line.chars().take(max).collect();
    out.push('…');
    out
}

/// Claude's project dir name: every non-alphanumeric UTF-16 unit becomes a hyphen.
fn encode_cwd(cwd: &str) -> String {
    let trimmed = cwd.trim_end_matches(['\\', '/']);
    let path = if trimmed.is_empty() || trimmed.ends_with(':') {
        cwd
    } else {
        trimmed
    };
    let mut encoded = String::with_capacity(path.len());
    for c in path.chars() {
        if c.is_ascii_alphanumeric() {
            encoded.push(c);
        } else {
            for _ in 0..c.len_utf16() {
                encoded.push('-');
            }
        }
    }
    encoded
}

fn projects_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join("projects"));
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".claude").join("projects"))
}

fn transcript_path(cwd: &str, session_id: &str) -> Option<PathBuf> {
    if session_id.is_empty() || session_id.contains(['/', '\\', '.']) {
        return None;
    }
    let root = projects_dir()?;
    project_dirs(&root, &encode_cwd(cwd))
        .into_iter()
        .map(|dir| dir.join(format!("{session_id}.jsonl")))
        .find(|p| p.is_file())
}

/// The project dir for `encoded`, matching case-insensitively (drive letters) and Claude's
/// truncated names.
fn project_dirs(root: &Path, encoded: &str) -> Vec<PathBuf> {
    let direct = root.join(encoded);
    if direct.is_dir() {
        return vec![direct];
    }
    let target = encoded.to_ascii_lowercase();
    let prefix = (target.len() > CLAUDE_DIR_NAME_LIMIT)
        .then(|| format!("{}-", &target[..CLAUDE_DIR_NAME_LIMIT]));
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                return false;
            };
            let name = name.to_ascii_lowercase();
            match &prefix {
                Some(prefix) => name.starts_with(prefix),
                None => name == target,
            }
        })
        .collect()
}

/// "$0.42", "$12.30", "<$0.01".
pub fn fmt_usd(usd: f64) -> String {
    if usd > 0.0 && usd < 0.01 {
        "<$0.01".into()
    } else {
        format!("${usd:.2}")
    }
}

/// "1.2M", "340k", "812".
pub fn fmt_tokens(tokens: u64) -> String {
    match tokens {
        t if t >= 1_000_000 => format!("{:.1}M", t as f64 / 1_000_000.0),
        t if t >= 1_000 => format!("{:.0}k", t as f64 / 1_000.0),
        t => t.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn encodes_cwd_like_claude() {
        assert_eq!(
            encode_cwd(r"C:\Users\me\Projetos_Merca\app\"),
            "C--Users-me-Projetos-Merca-app"
        );
        assert_eq!(encode_cwd("C:\\"), "C--");
    }

    #[test]
    fn follows_appends_dedupes_streamed_usage_and_titles() {
        let dir = std::env::temp_dir().join(format!("alethe-transcript-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.jsonl");
        let mut file = File::create(&path).unwrap();
        let usage = r#"{"input_tokens":1000000,"output_tokens":100000,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}"#;
        writeln!(
            file,
            r#"{{"type":"user","isMeta":true,"message":{{"content":"<caveat>"}}}}"#
        )
        .unwrap();
        writeln!(file, r#"{{"type":"user","message":{{"content":[{{"type":"text","text":"Corrigir o login\nmais detalhes"}}]}}}}"#).unwrap();
        // The same message on two lines (two content blocks): counted once.
        for _ in 0..2 {
            writeln!(file, r#"{{"type":"assistant","message":{{"id":"m1","model":"claude-opus-5-5","usage":{usage}}}}}"#).unwrap();
        }
        // A partial line is not read yet.
        write!(file, r#"{{"type":"ai-title","aiTitle":"Login"#).unwrap();
        file.flush().unwrap();

        let mut tracker = Tracker::new("s");
        tracker.path = Some(path.clone());
        assert!(tracker.poll(""));
        let stats = tracker.stats("");
        assert_eq!(stats.title.as_deref(), Some("Corrigir o login"));
        assert_eq!(stats.total_tokens, 1_100_000);
        // 1 MTok in at $5 + 0.1 MTok out at $25.
        assert!((stats.cost_usd.unwrap() - 7.5).abs() < 1e-9);
        assert_eq!(stats.model.as_deref(), Some("claude-opus-5-5"));

        writeln!(file, r#" quebrado"}}"#).unwrap();
        writeln!(file, r#"{{"type":"ai-title","aiTitle":"Login quebrado"}}"#).unwrap();
        file.flush().unwrap();
        assert!(tracker.poll(""));
        assert_eq!(tracker.stats("").title.as_deref(), Some("Login quebrado"));
        assert!(!tracker.poll(""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_usd(0.004), "<$0.01");
        assert_eq!(fmt_usd(12.3), "$12.30");
        assert_eq!(fmt_tokens(1_234_567), "1.2M");
        assert_eq!(fmt_tokens(340_100), "340k");
    }
}
