//! `projects.json` schema (version 9), mirrored from `src/lib/types.ts` of the web build.
//!
//! Every struct keeps the fields it does not know about in `extra`, so a file written by the
//! Tauri build round-trips through this one without losing the orchestration, MCP, voice,
//! plugin or sync settings the native UI does not implement yet.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const FILE_VERSION: u32 = 9;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn new_id() -> String {
    nanoid::nanoid!()
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectsFile {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub ungrouped_order: Vec<String>,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub todos: Vec<TodoItem>,
    #[serde(default)]
    pub active_project_id: Option<String>,
    #[serde(default)]
    pub workspace: Workspace,
    #[serde(default)]
    pub preferences: Preferences,
    #[serde(default)]
    pub cli_paths: Map<String, Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_version() -> u32 {
    FILE_VERSION
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    #[serde(default)]
    pub recent_project_ids: Vec<String>,
    #[serde(default)]
    pub active_tab_id: Option<String>,
    #[serde(default)]
    pub focused_terminal_id: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub project_ids: Vec<String>,
    #[serde(default)]
    pub parent_group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_color() -> String {
    "#7d72ff".into()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_cwd: Option<String>,
    #[serde(default)]
    pub terminals: Vec<Terminal>,
    #[serde(default = "default_layout")]
    pub layout_mode: String,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_layout() -> String {
    "auto".into()
}

impl Project {
    pub fn new(name: String, cwd: Option<String>, color: String) -> Self {
        Self {
            id: new_id(),
            name,
            color: Some(color),
            group_id: None,
            default_cwd: cwd,
            terminals: Vec::new(),
            layout_mode: default_layout(),
            collapsed: false,
            archived: None,
            hidden: None,
            created_at: now_ms(),
            extra: Map::new(),
        }
    }

    pub fn is_visible(&self) -> bool {
        !self.archived.unwrap_or(false) && !self.hidden.unwrap_or(false)
    }

    pub fn last_used(&self) -> u64 {
        self.terminals
            .iter()
            .filter_map(|t| t.last_used_at)
            .max()
            .unwrap_or(self.created_at)
    }

    pub fn cwd(&self) -> String {
        self.default_cwd
            .clone()
            .filter(|c| !c.trim().is_empty())
            .or_else(|| self.terminals.first().map(|t| t.cwd.clone()))
            .unwrap_or_else(crate::storage::home_dir)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Terminal {
    pub id: String,
    pub name: String,
    pub cwd: String,
    #[serde(default)]
    pub tabs: Vec<SubTab>,
    #[serde(default)]
    pub active_tab_id: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub lane_visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_name: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<u64>,
    /// `terminal` (default) | markdown | web | … — only terminals are rendered natively today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Terminal {
    pub fn new(agent: &str, cwd: String) -> Self {
        let tab = SubTab::new(agent, cwd.clone());
        Self {
            id: new_id(),
            name: crate::agents::info(agent).label.to_string(),
            cwd,
            active_tab_id: tab.id.clone(),
            tabs: vec![tab],
            disabled: false,
            lane_visible: None,
            custom_name: None,
            last_used_at: Some(now_ms()),
            kind: None,
            extra: Map::new(),
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.kind.as_deref().is_none_or(|k| k == "terminal")
    }

    pub fn active_tab(&self) -> Option<&SubTab> {
        self.tabs
            .iter()
            .find(|t| t.id == self.active_tab_id)
            .or_else(|| self.tabs.first())
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut SubTab> {
        let id = self.active_tab_id.clone();
        if self.tabs.iter().any(|t| t.id == id) {
            self.tabs.iter_mut().find(|t| t.id == id)
        } else {
            self.tabs.first_mut()
        }
    }

    pub fn agent(&self) -> String {
        self.active_tab()
            .map(|t| t.agent.clone())
            .unwrap_or_else(|| "shell".into())
    }

    /// Sidebar title: an explicit rename wins over the agent label.
    pub fn title(&self) -> String {
        if self.custom_name.unwrap_or(false) || self.tabs.is_empty() {
            return self.name.clone();
        }
        self.active_tab()
            .map(|t| t.name.clone())
            .unwrap_or_else(|| self.name.clone())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SubTab {
    pub id: String,
    #[serde(rename = "type")]
    pub agent: String,
    pub name: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<u64>,
    #[serde(default)]
    pub pty_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_as_admin: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_input: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl SubTab {
    pub fn new(agent: &str, cwd: String) -> Self {
        Self {
            id: new_id(),
            agent: agent.to_string(),
            name: crate::agents::info(agent).short.to_string(),
            cwd,
            last_used_at: Some(now_ms()),
            pty_id: None,
            session_id: None,
            extra_args: None,
            run_as_admin: None,
            initial_input: None,
            extra: Map::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_ui_theme")]
    pub ui_theme: String,
    #[serde(default = "default_terminal_theme")]
    pub terminal_theme: Option<String>,
    #[serde(default = "default_zoom")]
    pub ui_zoom: f32,
    #[serde(default)]
    pub display_name: String,
    /// Native-only key: the web build sizes its xterm from `uiZoom`.
    #[serde(default = "default_font_size", rename = "nativeTerminalFontSize")]
    pub terminal_font_size: f32,
    #[serde(default)]
    pub shell_path: Option<String>,
    #[serde(default)]
    pub onboarding_done: bool,
    #[serde(default)]
    pub enabled_agents: Map<String, Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_language() -> String {
    "pt-BR".into()
}
fn default_ui_theme() -> String {
    "elite-pure-black".into()
}
fn default_terminal_theme() -> Option<String> {
    Some("dark-lemon".into())
}
fn default_zoom() -> f32 {
    0.9
}
fn default_font_size() -> f32 {
    13.0
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            language: default_language(),
            ui_theme: default_ui_theme(),
            terminal_theme: default_terminal_theme(),
            ui_zoom: default_zoom(),
            display_name: String::new(),
            terminal_font_size: default_font_size(),
            shell_path: None,
            onboarding_done: false,
            enabled_agents: Map::new(),
            extra: Map::new(),
        }
    }
}

impl Preferences {
    pub fn agent_enabled(&self, agent: &str) -> bool {
        self.enabled_agents
            .get(agent)
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }
}

/// Project colors offered when creating a project (sidebar palette of the web build).
pub const PROJECT_COLORS: &[&str] = &[
    "#a78bfa", "#60a5fa", "#34d399", "#fbbf24", "#f87171", "#f472b6", "#22d3ee", "#a3e635",
];

pub fn parse_hex_color(value: &str) -> Option<egui::Color32> {
    let v = value.trim().trim_start_matches('#');
    let n = u32::from_str_radix(v.get(0..6)?, 16).ok()?;
    Some(crate::theme::hex(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_fields_survive_a_round_trip() {
        let raw = r#"{"version":9,"groups":[],"ungroupedOrder":["p1"],"projects":[{"id":"p1","name":"demo","groupId":null,"terminals":[{"id":"t1","name":"Claude","cwd":"C:\\","tabs":[{"id":"s1","type":"claude","name":"Claude","cwd":"C:\\","ptyId":null,"useRouter9":true}],"activeTabId":"s1","disabled":false,"laneVisible":null,"remoteShared":true}],"layoutMode":"auto","collapsed":false,"createdAt":1,"worktreeMode":"gitWorktree"}],"todos":[],"activeProjectId":"p1","workspace":{"containers":[],"recentProjectIds":["p1"],"tabs":[],"activeTabId":null,"activeGroupId":null,"focusedTerminalId":null,"history":[],"historyIndex":0},"preferences":{"language":"pt-BR","uiTheme":"dark","voiceProviders":{}},"cliPaths":{},"futureKey":42}"#;
        let file: ProjectsFile = serde_json::from_str(raw).unwrap();
        let out = serde_json::to_value(&file).unwrap();
        assert_eq!(out["futureKey"], 42);
        assert_eq!(out["projects"][0]["worktreeMode"], "gitWorktree");
        assert_eq!(out["projects"][0]["terminals"][0]["remoteShared"], true);
        assert_eq!(
            out["projects"][0]["terminals"][0]["tabs"][0]["useRouter9"],
            true
        );
        assert_eq!(
            out["projects"][0]["terminals"][0]["tabs"][0]["type"],
            "claude"
        );
        assert_eq!(out["workspace"]["historyIndex"], 0);
        assert!(out["preferences"]["voiceProviders"].is_object());
    }
}
