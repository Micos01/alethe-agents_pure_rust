//! Which Codex conversation a Codex tab is running (`codex_sessions.rs`, `session_watcher.rs` and
//! `codexSessionTracking.ts` upstream).
//!
//! Codex picks its own session id and writes it to `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`,
//! whose first line (`session_meta`) names the id and cwd. New rollouts are matched to the tab
//! that most plausibly started them: same cwd, opened before the rollout began, and the one a
//! prompt was just submitted in.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

/// How recently a prompt must have been submitted to claim a new rollout.
const SUBMIT_WINDOW_MS: u64 = 15_000;
/// Clock slack between the tab's spawn and the rollout's own timestamp.
const SPAWN_SLACK_MS: u64 = 2_000;
/// `session_meta` carries the whole base instructions; read no further than this.
const META_LINE_CAP: u64 = 512 * 1024;
/// Day folders looked at for new rollouts: today's and yesterday's (a session past midnight).
const RECENT_DAYS: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Rollout {
    pub id: String,
    pub cwd: String,
    /// `cli` for the interactive TUI; `exec`, `vscode`, `mcp`… for the others.
    pub source: Option<String>,
    pub started_ms: Option<u64>,
    pub path: PathBuf,
}

pub fn sessions_dir() -> Option<PathBuf> {
    crate::backend::mcp_support::provider_home_dir(&[".codex", "sessions"])
}

/// The `session_meta` of a rollout, or None while the file is still empty.
pub fn read_meta(path: &Path) -> Option<Rollout> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file.take(META_LINE_CAP));
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if value.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
        return None;
    }
    let payload = value.get("payload")?;
    let text = |key: &str| {
        payload
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    Some(Rollout {
        id: text("id")?,
        cwd: text("cwd")?,
        // Subagent threads carry an object here; only a plain string names a launch surface.
        source: text("source"),
        started_ms: text("timestamp")
            .as_deref()
            .and_then(crate::usage::parse_rfc3339_ms),
        path: path.to_path_buf(),
    })
}

fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs.reverse();
    dirs
}

/// `YYYY/MM/DD` folders, newest first.
fn day_dirs(root: &Path) -> impl Iterator<Item = PathBuf> {
    sorted_dirs(root).into_iter().flat_map(|year| {
        sorted_dirs(&year)
            .into_iter()
            .flat_map(|month| sorted_dirs(&month))
    })
}

fn jsonl_in(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .collect()
}

/// A known session's rollout: its file name ends with the id.
pub fn find_rollout(root: &Path, id: &str) -> Option<PathBuf> {
    let suffix = format!("{id}.jsonl");
    day_dirs(root).find_map(|day| {
        jsonl_in(&day).into_iter().find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(&suffix))
        })
    })
}

/// Reports rollouts created after it started.
pub struct RolloutScanner {
    root: Option<PathBuf>,
    seen: HashSet<PathBuf>,
}

impl RolloutScanner {
    pub fn new(root: Option<PathBuf>) -> Self {
        let mut scanner = Self {
            root,
            seen: HashSet::new(),
        };
        let existing = scanner.recent_files();
        scanner.seen.extend(existing);
        scanner
    }

    fn recent_files(&self) -> Vec<PathBuf> {
        let Some(root) = &self.root else {
            return Vec::new();
        };
        day_dirs(root)
            .take(RECENT_DAYS)
            .flat_map(|day| jsonl_in(&day))
            .collect()
    }

    pub fn poll(&mut self) -> Vec<Rollout> {
        let mut found = Vec::new();
        for path in self.recent_files() {
            if self.seen.contains(&path) {
                continue;
            }
            // An empty file gets its meta line shortly: look again next time.
            if let Some(rollout) = read_meta(&path) {
                self.seen.insert(path);
                found.push(rollout);
            }
        }
        found
    }
}

/// A running Codex tab, as `pickCodexPaneForRollout` sees it.
#[derive(Clone, Debug)]
pub struct Pane {
    pub tab_id: String,
    pub cwd: String,
    pub session_id: Option<String>,
    pub spawned_ms: u64,
    pub last_submit_ms: Option<u64>,
    /// `/new` or `/clear` was submitted since the tab last took a session.
    pub expects_new: bool,
}

pub fn normalize_cwd(cwd: &str) -> String {
    let mut s = cwd.trim().replace('\\', "/");
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    if cfg!(windows) {
        s = s.to_lowercase();
    }
    s
}

pub fn is_new_conversation(line: &str) -> bool {
    matches!(line.trim(), "/new" | "/clear")
}

/// The tab that started `rollout`, if it can be told apart.
pub fn pick_pane<'a>(panes: &'a [Pane], rollout: &Rollout, now_ms: u64) -> Option<&'a Pane> {
    if rollout.source.as_deref() != Some("cli") {
        return None;
    }
    if panes
        .iter()
        .any(|p| p.session_id.as_deref() == Some(rollout.id.as_str()))
    {
        return None;
    }
    let cwd = normalize_cwd(&rollout.cwd);
    let eligible: Vec<&Pane> = panes
        .iter()
        .filter(|p| {
            normalize_cwd(&p.cwd) == cwd
                && rollout
                    .started_ms
                    .is_none_or(|at| at + SPAWN_SLACK_MS >= p.spawned_ms)
        })
        .collect();
    let expecting: Vec<&Pane> = eligible
        .iter()
        .copied()
        .filter(|p| p.session_id.is_none() || p.expects_new)
        .collect();
    let recent = |p: &&Pane| {
        p.last_submit_ms
            .is_some_and(|at| now_ms.saturating_sub(at) <= SUBMIT_WINDOW_MS)
    };
    let latest = |list: Vec<&'a Pane>| list.into_iter().max_by_key(|p| p.last_submit_ms);
    latest(expecting.iter().copied().filter(recent).collect())
        .or_else(|| latest(eligible.iter().copied().filter(recent).collect()))
        .or_else(|| (expecting.len() == 1).then(|| expecting[0]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str, cwd: &str, session: Option<&str>, submit: Option<u64>) -> Pane {
        Pane {
            tab_id: id.into(),
            cwd: cwd.into(),
            session_id: session.map(str::to_string),
            spawned_ms: 10_000,
            last_submit_ms: submit,
            expects_new: false,
        }
    }

    fn rollout(id: &str, cwd: &str) -> Rollout {
        Rollout {
            id: id.into(),
            cwd: cwd.into(),
            source: Some("cli".into()),
            started_ms: Some(10_000),
            path: PathBuf::new(),
        }
    }

    #[test]
    fn the_pane_that_just_submitted_takes_the_rollout() {
        let panes = [
            pane("a", "C:\\repo", None, Some(9_000)),
            pane("b", "c:/repo/", None, Some(3_000)),
        ];
        let picked = pick_pane(&panes, &rollout("s1", "C:\\repo"), 10_000).unwrap();
        assert_eq!(picked.tab_id, "a");
    }

    #[test]
    fn a_lone_waiting_pane_takes_it_without_a_submit() {
        let panes = [
            pane("a", "/repo", None, None),
            pane("b", "/repo", Some("old"), None),
            pane("c", "/other", None, None),
        ];
        let picked = pick_pane(&panes, &rollout("s1", "/repo"), 60_000).unwrap();
        assert_eq!(picked.tab_id, "a");
    }

    #[test]
    fn ambiguous_known_or_foreign_rollouts_are_left_alone() {
        let two = [
            pane("a", "/repo", None, None),
            pane("b", "/repo", None, None),
        ];
        assert!(pick_pane(&two, &rollout("s1", "/repo"), 60_000).is_none());

        let owner = [pane("a", "/repo", Some("s1"), Some(59_000))];
        assert!(pick_pane(&owner, &rollout("s1", "/repo"), 60_000).is_none());

        let mut exec = rollout("s2", "/repo");
        exec.source = Some("exec".into());
        assert!(pick_pane(&[pane("a", "/repo", None, None)], &exec, 60_000).is_none());

        let mut older = rollout("s3", "/repo");
        older.started_ms = Some(10_000 - SPAWN_SLACK_MS - 1);
        assert!(pick_pane(&[pane("a", "/repo", None, None)], &older, 60_000).is_none());
    }

    #[test]
    fn new_conversation_moves_a_pane_to_the_next_rollout() {
        let mut p = pane("a", "/repo", Some("s1"), Some(59_000));
        p.expects_new = true;
        let panes = [p, pane("b", "/repo", Some("s0"), None)];
        assert_eq!(
            pick_pane(&panes, &rollout("s2", "/repo"), 60_000)
                .unwrap()
                .tab_id,
            "a"
        );
    }

    #[test]
    fn scanner_reports_only_new_rollouts_and_finds_by_id() {
        let root = std::env::temp_dir().join(format!("alethe-codex-{}", std::process::id()));
        let day = root.join("2026").join("11").join("25");
        std::fs::create_dir_all(&day).unwrap();
        let meta = |id: &str| {
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"/repo\",\"source\":\"cli\",\"timestamp\":\"2026-11-25T10:00:00.000Z\"}}}}\n"
            )
        };
        std::fs::write(
            day.join("rollout-2026-11-25T10-00-00-old.jsonl"),
            meta("old"),
        )
        .unwrap();
        let mut scanner = RolloutScanner::new(Some(root.clone()));
        assert!(scanner.poll().is_empty());

        let fresh = day.join("rollout-2026-11-25T10-05-00-fresh.jsonl");
        std::fs::write(&fresh, "").unwrap();
        assert!(scanner.poll().is_empty(), "no meta line yet");
        std::fs::write(&fresh, meta("fresh")).unwrap();
        let found = scanner.poll();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "fresh");
        assert_eq!(found[0].source.as_deref(), Some("cli"));
        assert!(found[0].started_ms.is_some());
        assert!(scanner.poll().is_empty());

        assert_eq!(
            find_rollout(&root, "old"),
            Some(day.join("rollout-2026-11-25T10-00-00-old.jsonl"))
        );
        assert_eq!(find_rollout(&root, "missing"), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
