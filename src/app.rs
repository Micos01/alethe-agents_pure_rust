//! Application state and the actions that mutate it. Views (in `ui/`) only read state and queue
//! `Action`s; `apply` runs them at the end of the frame — the same split as the web build's
//! Zustand stores and React components.

use crate::i18n::{tr, Lang};
use crate::model::{
    now_ms, Group, Project, ProjectsFile, SubTab, Terminal, TodoItem, PROJECT_COLORS,
};
use crate::storage::Storage;
use crate::terminal::{SpawnSpec, Status, TermSize, TerminalSession};
use crate::theme::Theme;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Home,
    Project(String),
}

/// Which panel the right sidebar shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RightTab {
    Todos,
    Agents,
    Mcp,
}

/// Which panel the left sidebar shows below its switcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarTab {
    Projects,
    Git,
}

#[derive(Clone, Debug)]
pub enum RenameTarget {
    Project(String),
    Group(String),
    Terminal(String),
}

#[derive(Clone, Debug)]
pub enum Modal {
    NewProject {
        name: String,
        folder: String,
        group_id: Option<String>,
        color: usize,
    },
    NewGroup {
        name: String,
        color: usize,
    },
    Rename {
        target: RenameTarget,
        value: String,
    },
    DeleteProject {
        id: String,
    },
    /// Confirms stopping a group's or a project's terminals.
    Suspend {
        target: SuspendTarget,
    },
    Settings,
    About,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SuspendTarget {
    Group(String),
    Project(String),
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Action {
    OpenHome,
    Suspend(SuspendTarget),
    Reactivate(SuspendTarget),
    OpenProject(String),
    CloseTab(String),
    NewTerminal {
        project_id: String,
        agent: String,
        cwd: Option<String>,
        extra_args: Vec<String>,
        initial_input: Option<String>,
    },
    AddSubTab {
        terminal_id: String,
        agent: String,
    },
    SelectSubTab {
        terminal_id: String,
        tab_id: String,
    },
    CloseSubTab {
        terminal_id: String,
        tab_id: String,
    },
    CloseTerminal(String),
    RestartTab(String),
    FocusTerminal(String),
    ToggleMaximize(String),
    ToggleProjectCollapsed(String),
    ToggleGroupCollapsed(String),
    SetLayout {
        project_id: String,
        mode: String,
    },
    MoveProjectToGroup {
        project_id: String,
        group_id: Option<String>,
    },
    DeleteGroup(String),
    DeleteProject(String),
    ShowModal(Modal),
    CloseModal,
    CreateProject {
        name: String,
        folder: String,
        group_id: Option<String>,
        color: String,
    },
    CreateGroup {
        name: String,
        color: String,
    },
    Rename {
        target: RenameTarget,
        value: String,
    },
    QuickStart {
        agent: String,
        folder: String,
        prompt: String,
        yolo: bool,
    },
    AddTodo {
        title: String,
        tags: Vec<String>,
        project_id: Option<String>,
    },
    UpdateTodo {
        id: String,
        title: String,
        tags: Vec<String>,
    },
    /// Drag and drop: `dragged` takes `target`'s place.
    ReorderTodo {
        dragged: String,
        target: String,
    },
    ToggleTodo(String),
    DeleteTodo(String),
    SetTheme(String),
    SetTerminalTheme(Option<String>),
    SetLanguage(String),
    Reimport,
    OpenFolder(String),
    /// Adds an Orchestration board pane to the project.
    NewOrchestration(String),
    /// Asks for a `.md` file and opens it as a pane in the project.
    OpenMarkdown(String),
    OpenImage(String),
    /// The sidebar's Files button: a shell pane in the project folder that starts `nvim .`.
    OpenFiles(String),
    Toast(String),
}

pub struct HomeState {
    pub prompt: String,
    pub agent: String,
    pub folder: String,
    pub yolo: bool,
}

pub struct TodoState {
    pub input: String,
    pub tags: String,
    pub project: Option<String>,
    pub filter: crate::ui::todos::TodoFilter,
    pub editing: Option<crate::ui::todos::TodoEdit>,
    pub dragging: Option<String>,
    pub settings_open: bool,
}

pub struct AletheApp {
    pub storage: Storage,
    pub data: ProjectsFile,
    pub dirty_since: Option<Instant>,
    pub sessions: HashMap<String, Arc<TerminalSession>>,
    pub spawn_errors: HashMap<String, String>,
    pub view: View,
    pub open_tabs: Vec<String>,
    pub focused_terminal: Option<String>,
    pub focus_request: Option<String>,
    pub maximized: Option<String>,
    pub show_sidebar: bool,
    pub show_right: bool,
    pub right_tab: RightTab,
    /// Sub-tab ids whose agent was launched with the orchestrator tools (a job's `plannerId`).
    pub planner_tabs: std::collections::HashSet<String>,
    pub agents_panel: crate::ui::orchestrator::AgentsPanelState,
    pub mcp_panel: crate::ui::mcp::McpPanelState,
    /// When each Codex tab last took a rollout (a later `/new` expects another).
    codex_adopted: HashMap<String, Instant>,
    pub sidebar_search: String,
    pub sidebar_tab: SidebarTab,
    pub git_panel: crate::ui::git_panel::GitPanel,
    pub modal: Option<Modal>,
    pub home: HomeState,
    pub todo: TodoState,
    pub pomodoro: crate::pomodoro::Pomodoro,
    pub theme: Theme,
    applied_theme: String,
    applied_zoom: f32,
    pub toasts: Vec<(String, Instant)>,
    notified_revision: u64,
    notified_blocked: std::collections::HashSet<String>,
    pub memory_mb: Arc<AtomicU64>,
    pub resources: crate::resources::ResourceMonitor,
    memory_warned_at: Option<Instant>,
    pub usage: crate::usage::UsageMonitor,
    pub transcripts: crate::transcripts::TranscriptWatcher,
    transcripts_fed: Option<Instant>,
    pub orchestrator: Arc<crate::orchestrator::Orchestrator>,
    pub boards: HashMap<String, crate::ui::orchestrator::BoardState>,
    pub markdown: HashMap<String, crate::ui::markdown::MarkdownState>,
    pub images: HashMap<String, crate::ui::image_pane::ImageState>,
    actions: RefCell<Vec<Action>>,
    ctx: egui::Context,
}

impl AletheApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let storage = Storage::open();
        let mut data = storage.load();
        if data.version == 0 {
            data.version = crate::model::FILE_VERSION;
        }
        let theme = Theme::resolve(&data.preferences.ui_theme);
        theme.apply(&cc.egui_ctx);
        cc.egui_ctx
            .set_zoom_factor(data.preferences.ui_zoom.clamp(0.6, 2.0));

        let view = data
            .active_project_id
            .clone()
            .filter(|id| data.projects.iter().any(|p| &p.id == id))
            .map(View::Project)
            .unwrap_or(View::Home);
        let mut open_tabs: Vec<String> = data
            .workspace
            .recent_project_ids
            .iter()
            .filter(|id| data.projects.iter().any(|p| &p.id == *id && p.is_visible()))
            .take(6)
            .cloned()
            .collect();
        if let View::Project(id) = &view {
            if !open_tabs.contains(id) {
                open_tabs.insert(0, id.clone());
            }
        }
        let default_folder = data
            .projects
            .iter()
            .max_by_key(|p| p.last_used())
            .map(|p| p.cwd())
            .unwrap_or_else(crate::storage::home_dir);

        crate::backend::mcp_store::set_backup_dir(&storage.profile_dir());
        let memory_mb = Arc::new(AtomicU64::new(0));
        let resources =
            crate::resources::ResourceMonitor::start(Arc::clone(&memory_mb), cc.egui_ctx.clone());

        let mut toasts = Vec::new();
        let lang = Lang::from_code(&data.preferences.language);
        if storage.imported_from.is_some() {
            toasts.push((tr(lang, "toast.imported").to_string(), Instant::now()));
        }

        // Smoke test hook: `ALETHE_NATIVE_SMOKE="<agent>|<folder>|<prompt>"` starts that session at
        // launch, the same way the Home composer does. Used to exercise the PTY path end to end.
        let mut initial_actions = Vec::new();
        if let Ok(spec) = std::env::var("ALETHE_NATIVE_SMOKE") {
            if spec == "@settings" {
                initial_actions.push(Action::ShowModal(Modal::Settings));
            }
            let mut parts = spec.splitn(3, '|');
            let agent = parts.next().unwrap_or("shell").to_string();
            let folder = parts.next().unwrap_or_default().to_string();
            let prompt = parts.next().unwrap_or_default().to_string();
            if !agent.starts_with('@') {
                initial_actions.push(Action::QuickStart {
                    agent,
                    folder,
                    prompt,
                    yolo: false,
                });
            }
        }

        let orchestrator = crate::orchestrator::Orchestrator::new(
            cc.egui_ctx.clone(),
            storage.profile_dir().join("orchestrator-jobs.json"),
        );

        let pomodoro = crate::pomodoro::Pomodoro::hydrate(&data.preferences.extra, now_ms());
        Self {
            focused_terminal: data.workspace.focused_terminal_id.clone(),
            applied_theme: theme.id.clone(),
            applied_zoom: data.preferences.ui_zoom,
            storage,
            data,
            dirty_since: None,
            sessions: HashMap::new(),
            spawn_errors: HashMap::new(),
            view,
            open_tabs,
            focus_request: None,
            // `ALETHE_NATIVE_SMOKE_MAXIMIZE=<pane id>` opens with that pane maximized.
            maximized: std::env::var("ALETHE_NATIVE_SMOKE_MAXIMIZE").ok(),
            show_sidebar: true,
            // `ALETHE_NATIVE_SMOKE=@agents` / `@todo` / `@mcp` open with that right panel showing.
            show_right: matches!(
                std::env::var("ALETHE_NATIVE_SMOKE").as_deref(),
                Ok("@agents" | "@todo" | "@mcp" | "@skills")
            ),
            right_tab: match std::env::var("ALETHE_NATIVE_SMOKE").as_deref() {
                Ok("@agents") => RightTab::Agents,
                Ok("@mcp" | "@skills") => RightTab::Mcp,
                _ => RightTab::Todos,
            },
            planner_tabs: Default::default(),
            agents_panel: Default::default(),
            mcp_panel: Default::default(),
            codex_adopted: HashMap::new(),
            sidebar_search: String::new(),
            // `ALETHE_NATIVE_SMOKE=@git` opens on the Source Control tab.
            sidebar_tab: if std::env::var("ALETHE_NATIVE_SMOKE").as_deref() == Ok("@git") {
                SidebarTab::Git
            } else {
                SidebarTab::Projects
            },
            git_panel: Default::default(),
            modal: None,
            home: HomeState {
                prompt: String::new(),
                agent: "claude".into(),
                folder: default_folder,
                yolo: false,
            },
            todo: TodoState {
                input: String::new(),
                tags: String::new(),
                project: None,
                filter: Default::default(),
                editing: None,
                dragging: None,
                settings_open: false,
            },
            theme,
            toasts,
            pomodoro,
            notified_revision: 0,
            notified_blocked: Default::default(),
            memory_mb,
            resources,
            memory_warned_at: None,
            usage: crate::usage::UsageMonitor::start(cc.egui_ctx.clone()),
            transcripts: crate::transcripts::TranscriptWatcher::start(cc.egui_ctx.clone()),
            transcripts_fed: None,
            orchestrator,
            boards: HashMap::new(),
            markdown: HashMap::new(),
            images: HashMap::new(),
            actions: RefCell::new(initial_actions),
            ctx: cc.egui_ctx.clone(),
        }
    }

    /// `preferences.enabledFeatures[id]`, the toggles the Tauri build keeps for optional features.
    pub fn feature_enabled(&self, id: &str) -> bool {
        self.data
            .preferences
            .extra
            .get("enabledFeatures")
            .and_then(|f| f.get(id))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    pub fn set_feature(&mut self, id: &str, on: bool) {
        let features = self
            .data
            .preferences
            .extra
            .entry("enabledFeatures")
            .or_insert_with(|| serde_json::json!({}));
        if !features.is_object() {
            *features = serde_json::json!({});
        }
        features[id] = serde_json::Value::Bool(on);
        self.mark_dirty();
    }

    pub fn lang(&self) -> Lang {
        Lang::from_code(&self.data.preferences.language)
    }

    pub fn t(&self, key: &str) -> &'static str {
        tr(self.lang(), key)
    }

    pub fn act(&self, action: Action) {
        self.actions.borrow_mut().push(action);
    }

    pub fn mark_dirty(&mut self) {
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    /// Tells the transcript watcher which Claude tabs are running (at most once a second).
    fn feed_transcripts(&mut self) {
        if self
            .transcripts_fed
            .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.transcripts_fed = Some(Instant::now());
        self.adopt_codex_rollouts();
        self.resources.set_roots(
            self.sessions
                .iter()
                .filter_map(|(tab, s)| Some((tab.clone(), s.pid?)))
                .collect(),
        );
        self.warn_memory();
        let mut live = Vec::new();
        for project in &self.data.projects {
            for terminal in &project.terminals {
                for tab in &terminal.tabs {
                    let Some(session_id) = tab.session_id.as_deref() else {
                        continue;
                    };
                    let codex = tab.agent == "codex";
                    if !(tab.agent == "claude" || codex) || !self.sessions.contains_key(&tab.id) {
                        continue;
                    }
                    let cwd = [&tab.cwd, &terminal.cwd]
                        .into_iter()
                        .find(|c| !c.trim().is_empty())
                        .cloned()
                        .unwrap_or_else(|| project.cwd());
                    live.push(crate::transcripts::Live {
                        tab_id: tab.id.clone(),
                        cwd,
                        session_id: session_id.to_string(),
                        codex,
                    });
                }
            }
        }
        self.transcripts.set_live(live);
    }

    /// Gives each new Codex rollout to the tab that started it (`trackCodexRollout`); the id is
    /// saved on the tab, so a restart resumes that conversation.
    fn adopt_codex_rollouts(&mut self) {
        let rollouts = self.transcripts.take_rollouts();
        if rollouts.is_empty() {
            return;
        }
        let now = crate::model::now_ms();
        let wall = |at: Instant| now.saturating_sub(at.elapsed().as_millis() as u64);
        let mut panes = Vec::new();
        for project in &self.data.projects {
            for terminal in &project.terminals {
                for tab in terminal.tabs.iter().filter(|t| t.agent == "codex") {
                    let Some(session) = self.sessions.get(&tab.id) else {
                        continue;
                    };
                    if session.is_exited() {
                        continue;
                    }
                    let submit = session.last_submit();
                    let adopted = self.codex_adopted.get(&tab.id).copied();
                    let expects_new = submit.as_ref().is_some_and(|(at, line)| {
                        crate::codex_sessions::is_new_conversation(line)
                            && adopted.is_none_or(|a| *at > a)
                    });
                    let cwd = [&tab.cwd, &terminal.cwd]
                        .into_iter()
                        .find(|c| !c.trim().is_empty())
                        .cloned()
                        .unwrap_or_else(|| project.cwd());
                    panes.push(crate::codex_sessions::Pane {
                        tab_id: tab.id.clone(),
                        cwd,
                        session_id: tab.session_id.clone().filter(|s| !s.is_empty()),
                        spawned_ms: wall(session.started_at),
                        last_submit_ms: submit.map(|(at, _)| wall(at)),
                        expects_new,
                    });
                }
            }
        }
        for rollout in rollouts {
            let Some(tab_id) =
                crate::codex_sessions::pick_pane(&panes, &rollout, now).map(|p| p.tab_id.clone())
            else {
                continue;
            };
            if let Some(pane) = panes.iter_mut().find(|p| p.tab_id == tab_id) {
                pane.session_id = Some(rollout.id.clone());
                pane.expects_new = false;
            }
            self.codex_adopted.insert(tab_id.clone(), Instant::now());
            for project in &mut self.data.projects {
                for terminal in &mut project.terminals {
                    for tab in &mut terminal.tabs {
                        if tab.id == tab_id {
                            tab.session_id = Some(rollout.id.clone());
                        }
                    }
                }
            }
            self.mark_dirty();
        }
    }

    pub fn find_tab(&self, tab_id: &str) -> Option<&crate::model::SubTab> {
        self.data
            .projects
            .iter()
            .flat_map(|p| &p.terminals)
            .flat_map(|t| &t.tabs)
            .find(|t| t.id == tab_id)
    }

    /// `resources.criticalTitle`: the system is about to run out of memory. At most every 2 min.
    fn warn_memory(&mut self) {
        let memory = self.resources.snapshot();
        if memory.pressure != crate::resources::Pressure::Critical
            || self
                .memory_warned_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(120))
        {
            return;
        }
        self.memory_warned_at = Some(Instant::now());
        let body = self
            .t("resources.criticalBody")
            .replace("{free}", &memory.system_available_mb.to_string())
            .replace("{ptys}", &memory.terminals_mb().to_string());
        let title = self.t("resources.criticalTitle").to_string();
        self.notify(title, body, None);
    }

    /// The projects a suspend target covers: a group's, its subgroups' included.
    pub fn suspend_scope(&self, target: &SuspendTarget) -> Vec<String> {
        match target {
            SuspendTarget::Project(id) => vec![id.clone()],
            SuspendTarget::Group(id) => {
                let mut groups = vec![id.clone()];
                let mut i = 0;
                while i < groups.len() {
                    for g in &self.data.groups {
                        if g.parent_group_id.as_deref() == Some(groups[i].as_str())
                            && !groups.contains(&g.id)
                        {
                            groups.push(g.id.clone());
                        }
                    }
                    i += 1;
                }
                self.data
                    .projects
                    .iter()
                    .filter(|p| p.group_id.as_ref().is_some_and(|g| groups.contains(g)))
                    .map(|p| p.id.clone())
                    .collect()
            }
        }
    }

    /// Running terminal tabs (processes) a suspend would stop.
    pub fn running_in(&self, target: &SuspendTarget) -> usize {
        let scope = self.suspend_scope(target);
        self.data
            .projects
            .iter()
            .filter(|p| scope.contains(&p.id))
            .flat_map(|p| &p.terminals)
            .flat_map(|t| &t.tabs)
            .filter(|tab| self.sessions.contains_key(&tab.id))
            .count()
    }

    fn set_suspended(&mut self, target: &SuspendTarget, suspend: bool) {
        let scope = self.suspend_scope(target);
        let mut tabs = Vec::new();
        for project in self
            .data
            .projects
            .iter_mut()
            .filter(|p| scope.contains(&p.id))
        {
            if suspend {
                project
                    .extra
                    .insert("suspended".into(), serde_json::json!(true));
            } else {
                project.extra.remove("suspended");
            }
            for terminal in &mut project.terminals {
                terminal.disabled = suspend;
                tabs.extend(terminal.tabs.iter().map(|t| t.id.clone()));
            }
        }
        if suspend {
            // Ends the process trees; reactivating starts them again, Claude resuming its session.
            for tab in &tabs {
                self.kill_tab(tab);
            }
        }
        if let SuspendTarget::Group(id) = target {
            if let Some(group) = self.data.groups.iter_mut().find(|g| &g.id == id) {
                group.suspended = suspend.then_some(true);
            }
        }
        self.mark_dirty();
    }

    /// A sub-tab's label: its own name once renamed, else the chat title, else the agent name.
    pub fn tab_title(&self, tab: &crate::model::SubTab) -> String {
        if tab.name == crate::agents::info(&tab.agent).short {
            if let Some(title) = self.transcripts.stats(&tab.id).and_then(|s| s.title) {
                return title;
            }
        }
        tab.name.clone()
    }

    /// `Terminal::title`, with the chat title for an agent tab nobody renamed.
    pub fn terminal_title(&self, terminal: &crate::model::Terminal) -> String {
        if terminal.custom_name.unwrap_or(false) {
            return terminal.title();
        }
        terminal
            .active_tab()
            .map(|t| self.tab_title(t))
            .unwrap_or_else(|| terminal.title())
    }

    /// `preferences.usageAccess[provider]`: off unless the user turned it on.
    pub fn usage_access(&self, provider: &str) -> bool {
        self.data
            .preferences
            .extra
            .get("usageAccess")
            .and_then(|a| a.get(provider))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    pub fn set_usage_access(&mut self, provider: &str, on: bool) {
        let access = self
            .data
            .preferences
            .extra
            .entry("usageAccess")
            .or_insert(serde_json::Value::Null);
        if !access.is_object() {
            *access = serde_json::json!({
                "claude": false, "codex": false, "antigravity": false, "kimi": false, "openrouter": false
            });
        }
        access[provider] = serde_json::json!(on);
        self.mark_dirty();
    }

    /// Applies a Pomodoro transition and persists the session.
    pub fn pomodoro_update(
        &mut self,
        change: impl FnOnce(&mut crate::pomodoro::Pomodoro, crate::pomodoro::Durations, u64),
    ) {
        let durations = crate::pomodoro::Durations::from_prefs(&self.data.preferences.extra);
        change(&mut self.pomodoro, durations, now_ms());
        self.pomodoro.persist(&mut self.data.preferences.extra);
        self.mark_dirty();
    }

    /// Ends a Pomodoro phase whose time is up and says so.
    fn tick_pomodoro(&mut self) {
        use crate::pomodoro::{Phase, Status};
        if self.pomodoro.status != Status::Running {
            return;
        }
        let now = now_ms();
        match self.pomodoro.tick(now) {
            Some(phase) => {
                self.pomodoro.persist(&mut self.data.preferences.extra);
                self.mark_dirty();
                let (title, body) = if phase == Phase::Work {
                    (
                        "pomodoro.notifyWorkDoneTitle",
                        "pomodoro.notifyWorkDoneBody",
                    )
                } else {
                    (
                        "pomodoro.notifyBreakDoneTitle",
                        "pomodoro.notifyBreakDoneBody",
                    )
                };
                let (title, body) = (self.t(title).to_string(), self.t(body).to_string());
                self.notify(title, body, None);
            }
            None => {
                let left = self.pomodoro.remaining_ms(now).clamp(50, 1000);
                self.ctx.request_repaint_after(Duration::from_millis(left));
            }
        }
    }

    /// The focused terminal's active sub-tab id.
    pub fn focused_tab(&self) -> Option<String> {
        let focused = self.focused_terminal.as_ref()?;
        let terminal = self
            .data
            .projects
            .iter()
            .flat_map(|p| p.terminals.iter())
            .find(|t| &t.id == focused)?;
        let tab = terminal
            .tabs
            .iter()
            .find(|tab| tab.id == terminal.active_tab_id)
            .or_else(|| terminal.tabs.first())?;
        Some(tab.id.clone())
    }

    /// The focused terminal's active sub-tab id, when that tab is an orchestrator planner.
    pub fn focused_planner(&self) -> Option<String> {
        self.focused_tab()
            .filter(|tab| self.planner_tabs.contains(tab))
    }

    /// `lib/notifications.ts`: a toast while the window is in front (skipped for the terminal
    /// already on screen), otherwise also an OS notification and a taskbar flash.
    pub fn notify(&mut self, title: String, body: String, source_tab: Option<&str>) {
        let in_front = self.ctx.input(|i| {
            i.viewport().focused.unwrap_or(true) && !i.viewport().minimized.unwrap_or(false)
        });
        if in_front {
            if source_tab.is_none() || self.focused_tab().as_deref() != source_tab {
                self.toasts
                    .push((format!("{title}\n{body}"), Instant::now()));
            }
            return;
        }
        self.toasts
            .push((format!("{title}\n{body}"), Instant::now()));
        crate::notify::os_notify(title, body);
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Informational,
            ));
    }

    /// Agents that finished answering, and workers that now wait on an approval.
    fn poll_notifications(&mut self) {
        let mut done = Vec::new();
        let mut pending = false;
        for (tab, session) in &self.sessions {
            if session.take_completion() {
                done.push(tab.clone());
            }
            pending |= session.completion_pending();
        }
        if pending {
            self.ctx.request_repaint_after(Duration::from_secs(1));
        }
        for tab_id in done {
            let found = self.data.projects.iter().find_map(|p| {
                p.terminals.iter().find_map(|t| {
                    t.tabs.iter().find(|tab| tab.id == tab_id).map(|tab| {
                        (
                            tab.agent.clone(),
                            format!("{} · {}", p.name, t.name),
                            tab.cwd.clone(),
                        )
                    })
                })
            });
            let Some((agent, label, cwd)) = found else {
                continue;
            };
            let title = self
                .t("notif.agentDoneTitle")
                .replace("{agent}", crate::agents::info(&agent).label);
            let body = self
                .t("notif.respondedInPath")
                .replace("{label}", &label)
                .replace("{path}", &crate::notify::short_path(&cwd));
            self.notify(title, body, Some(&tab_id));
        }

        let revision = self.orchestrator.revision();
        if revision == self.notified_revision {
            return;
        }
        self.notified_revision = revision;
        let board = self.orchestrator.board();
        let mut blocked = std::collections::HashSet::new();
        let mut fresh = Vec::new();
        for job in board
            .get("jobs")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            if job.get("status").and_then(serde_json::Value::as_str) != Some("blocked") {
                continue;
            }
            let Some(id) = job.get("id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if !self.notified_blocked.contains(id) {
                let ask = job.get("pendingApproval");
                let text = |key: &str| {
                    ask.and_then(|a| a.get(key))
                        .and_then(serde_json::Value::as_str)
                };
                let detail = text("command")
                    .or_else(|| text("reason"))
                    .map(str::to_string)
                    .or_else(|| {
                        job.get("spec")
                            .and_then(serde_json::Value::as_str)
                            .and_then(|s| s.lines().map(str::trim).find(|l| !l.is_empty()))
                            .map(str::to_string)
                    })
                    .unwrap_or_default();
                fresh.push((id.to_string(), detail));
            }
            blocked.insert(id.to_string());
        }
        self.notified_blocked = blocked;
        for (id, detail) in fresh {
            let title = self.t("notif.approvalTitle").replace("{id}", &id);
            self.notify(title, detail, None);
        }
    }

    pub fn project(&self, id: &str) -> Option<&Project> {
        self.data.projects.iter().find(|p| p.id == id)
    }

    fn project_mut(&mut self, id: &str) -> Option<&mut Project> {
        self.data.projects.iter_mut().find(|p| p.id == id)
    }

    pub fn active_project_id(&self) -> Option<&str> {
        match &self.view {
            View::Project(id) => Some(id),
            View::Home => None,
        }
    }

    fn find_terminal_mut(&mut self, terminal_id: &str) -> Option<(&mut Project, usize)> {
        for project in self.data.projects.iter_mut() {
            if let Some(i) = project.terminals.iter().position(|t| t.id == terminal_id) {
                return Some((project, i));
            }
        }
        None
    }

    pub fn terminal_project(&self, terminal_id: &str) -> Option<&Project> {
        self.data
            .projects
            .iter()
            .find(|p| p.terminals.iter().any(|t| t.id == terminal_id))
    }

    pub fn terminal_status(&self, terminal: &Terminal) -> Option<Status> {
        let tab = terminal.active_tab()?;
        self.sessions.get(&tab.id).map(|s| s.status())
    }

    pub fn terminal_theme_id(&self) -> String {
        self.data
            .preferences
            .terminal_theme
            .clone()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| self.data.preferences.ui_theme.clone())
    }

    // ---------------------------------------------------------------- sessions

    /// Boots the PTY for a sub-tab the first time it is shown.
    pub fn ensure_session(
        &mut self,
        terminal_id: &str,
        size: TermSize,
    ) -> Option<Arc<TerminalSession>> {
        let current_tab = self
            .data
            .projects
            .iter()
            .flat_map(|p| p.terminals.iter())
            .find(|t| t.id == terminal_id)?
            .active_tab()?
            .id
            .clone();
        if let Some(session) = self.sessions.get(&current_tab) {
            return Some(Arc::clone(session));
        }
        if self.spawn_errors.contains_key(&current_tab) {
            return None;
        }

        let (project, index) = self.find_terminal_mut(terminal_id)?;
        let project_name = project.name.clone();
        let terminal = &mut project.terminals[index];
        terminal.last_used_at = Some(now_ms());
        let terminal_cwd = terminal.cwd.clone();
        // Planner tabs on the board read "project · terminal": every Claude terminal is named
        // "Claude Code" otherwise.
        let terminal_name = format!("{project_name} · {}", terminal.name);
        let tab = terminal.active_tab_mut()?;

        let info = crate::agents::info(&tab.agent);
        let base_args = tab.extra_args.clone().unwrap_or_default();
        let launch =
            crate::launch::build_agent_launch(&tab.agent, &base_args, tab.session_id.as_deref());
        let mut args = launch.args;
        // Claude and Codex take the first prompt as a positional argument; the others get it typed.
        let mut typed_input = None;
        if let Some(input) = tab.initial_input.take().filter(|s| !s.trim().is_empty()) {
            if matches!(tab.agent.as_str(), "claude" | "codex") {
                args.push(input);
            } else {
                typed_input = Some(input);
            }
        }
        tab.session_id = launch.session_id.or(tab.session_id.take());
        tab.last_used_at = Some(now_ms());
        let tab_id = tab.id.clone();
        let agent = tab.agent.clone();
        let cwd = if tab.cwd.trim().is_empty() {
            terminal_cwd
        } else {
            tab.cwd.clone()
        };
        let run_as_admin = tab.run_as_admin.unwrap_or(false);

        // With the orchestrator on, Claude and Codex terminals are planners: they get the `alethe`
        // MCP server and can delegate to workers shown on the Orchestration board.
        let mut planner_env: Option<String> = None;
        if self.feature_enabled("orchestrator") && matches!(agent.as_str(), "claude" | "codex") {
            let planner = crate::backend::orchestrator_core::Planner {
                id: tab_id.clone(),
                label: terminal_name,
                agent: agent.clone(),
            };
            self.orchestrator
                .apply_settings(self.data.preferences.extra.get("orchestration"));
            if agent == "claude" {
                match self.orchestrator.claude_config(&planner) {
                    Ok(path) => {
                        self.planner_tabs.insert(tab_id.clone());
                        let mut with_mcp = vec![
                            "--mcp-config".to_string(),
                            path.to_string_lossy().to_string(),
                        ];
                        with_mcp.append(&mut args);
                        args = with_mcp;
                    }
                    Err(error) => eprintln!("[orchestrator] {error}"),
                }
            } else {
                match self.orchestrator.codex_config(&planner, &cwd) {
                    Ok(()) => {
                        self.planner_tabs.insert(tab_id.clone());
                        planner_env = Some(tab_id.clone());
                    }
                    Err(error) => eprintln!("[orchestrator] {error}"),
                }
            }
        }

        let command: Option<String> = match info.command {
            Some(c) => Some(c.to_string()),
            None if info.id == "unknown" => Some(agent.clone()),
            None => None,
        };
        let resolved = match &command {
            Some(cmd) => crate::backend::cli_resolver::find_windows_cli_launcher(cmd)
                .map(|p| p.to_string_lossy().to_string()),
            None => self
                .data
                .preferences
                .shell_path
                .clone()
                .filter(|p| std::path::Path::new(p).is_file()),
        };
        let builder = crate::backend::cli_resolver::command_builder_for_terminal(
            command.as_deref(),
            resolved.as_deref(),
            &args,
            None,
            run_as_admin,
        );
        self.mark_dirty();
        let builder = match builder {
            Ok(mut b) => {
                if let Some(planner) = &planner_env {
                    b.env("ALETHE_PLANNER", planner);
                }
                b
            }
            Err(error) => {
                self.spawn_errors.insert(tab_id, error);
                return None;
            }
        };
        let palette = crate::theme::term_palette(&self.terminal_theme_id());
        match TerminalSession::spawn(
            SpawnSpec {
                program_label: agent,
                builder,
                cwd,
                size,
                initial_input: typed_input,
            },
            palette,
            self.ctx.clone(),
        ) {
            Ok(session) => {
                self.sessions.insert(tab_id, Arc::clone(&session));
                Some(session)
            }
            Err(error) => {
                self.spawn_errors.insert(tab_id, error);
                None
            }
        }
    }

    fn kill_tab(&mut self, tab_id: &str) {
        if let Some(session) = self.sessions.remove(tab_id) {
            session.kill();
        }
        self.spawn_errors.remove(tab_id);
    }

    fn kill_terminal(&mut self, terminal: &Terminal) {
        for tab in &terminal.tabs {
            self.kill_tab(&tab.id);
        }
    }

    // ---------------------------------------------------------------- frame

    pub fn frame_start(&mut self, ctx: &egui::Context) {
        if self.applied_theme != self.data.preferences.ui_theme {
            self.theme = Theme::resolve(&self.data.preferences.ui_theme);
            self.theme.apply(ctx);
            self.applied_theme = self.data.preferences.ui_theme.clone();
            let palette = crate::theme::term_palette(&self.terminal_theme_id());
            for session in self.sessions.values() {
                session.set_palette(palette);
            }
        }
        if (self.applied_zoom - self.data.preferences.ui_zoom).abs() > f32::EPSILON {
            ctx.set_zoom_factor(self.data.preferences.ui_zoom.clamp(0.6, 2.0));
            self.applied_zoom = self.data.preferences.ui_zoom;
        }
        ctx.data_mut(|d| {
            d.insert_temp(
                egui::Id::unique("lang"),
                self.data.preferences.language.clone(),
            )
        });
        self.toasts
            .retain(|(_, at)| at.elapsed() < Duration::from_secs(4));
        // Agents that keep working need the status dots to refresh.
        if self
            .sessions
            .values()
            .any(|s| s.status() == Status::Working)
        {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        self.shortcuts(ctx);
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
        let new_terminal =
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::T)));
        let new_project =
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(ctrl_shift, Key::P)));
        let new_group =
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(ctrl_shift, Key::G)));
        let toggle_sidebar =
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::B)));
        let settings = ctx
            .input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::Comma)));
        let close =
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(ctrl_shift, Key::W)));
        let maximize =
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(ctrl_shift, Key::Enter)));
        let home =
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(ctrl_shift, Key::H)));

        if new_terminal {
            match self.active_project_id().map(str::to_string) {
                Some(project_id) => self.act(Action::NewTerminal {
                    project_id,
                    agent: "shell".into(),
                    cwd: None,
                    extra_args: vec![],
                    initial_input: None,
                }),
                None => self.act(Action::QuickStart {
                    agent: "shell".into(),
                    folder: self.home.folder.clone(),
                    prompt: String::new(),
                    yolo: false,
                }),
            }
        }
        if new_project {
            self.act(Action::ShowModal(self.new_project_modal(None)));
        }
        if new_group {
            self.act(Action::ShowModal(Modal::NewGroup {
                name: String::new(),
                color: 1,
            }));
        }
        if toggle_sidebar {
            self.show_sidebar = !self.show_sidebar;
        }
        if settings {
            self.act(Action::ShowModal(Modal::Settings));
        }
        if home {
            self.act(Action::OpenHome);
        }
        if let Some(focused) = self.focused_terminal.clone() {
            if close {
                self.act(Action::CloseTerminal(focused.clone()));
            }
            if maximize {
                self.act(Action::ToggleMaximize(focused));
            }
        }
    }

    pub fn new_project_modal(&self, group_id: Option<String>) -> Modal {
        Modal::NewProject {
            name: String::new(),
            folder: String::new(),
            group_id,
            color: self.data.projects.len() % PROJECT_COLORS.len(),
        }
    }

    pub fn frame_end(&mut self) {
        self.usage
            .set_access(self.usage_access("claude"), self.usage_access("codex"));
        self.poll_notifications();
        let now = self.ctx.input(|i| i.time);
        crate::ui::image_pane::evict(self, now);
        self.tick_pomodoro();
        self.feed_transcripts();
        let actions: Vec<Action> = self.actions.borrow_mut().drain(..).collect();
        for action in actions {
            self.apply(action);
        }
        if let Some(since) = self.dirty_since {
            if since.elapsed() > Duration::from_millis(800) {
                self.save_now();
            } else {
                self.ctx.request_repaint_after(Duration::from_millis(850));
            }
        }
    }

    pub fn save_now(&mut self) {
        self.data.active_project_id = self.active_project_id().map(str::to_string);
        self.data.workspace.focused_terminal_id = self.focused_terminal.clone();
        if let Err(error) = self.storage.save(&self.data) {
            let msg = format!("{}: {error}", self.t("toast.saveFailed"));
            self.toasts.push((msg, Instant::now()));
        }
        self.dirty_since = None;
    }

    fn touch_recent(&mut self, project_id: &str) {
        let recent = &mut self.data.workspace.recent_project_ids;
        recent.retain(|id| id != project_id);
        recent.insert(0, project_id.to_string());
        recent.truncate(20);
    }

    fn open_project(&mut self, project_id: &str) {
        if self.project(project_id).is_none() {
            return;
        }
        if !self.open_tabs.iter().any(|id| id == project_id) {
            self.open_tabs.push(project_id.to_string());
        }
        self.view = View::Project(project_id.to_string());
        self.maximized = None;
        self.touch_recent(project_id);
        let first = self
            .project(project_id)
            .and_then(|p| p.terminals.first().map(|t| t.id.clone()));
        let keep = self.focused_terminal.as_ref().is_some_and(|f| {
            self.project(project_id)
                .is_some_and(|p| p.terminals.iter().any(|t| &t.id == f))
        });
        if !keep {
            self.focused_terminal = first;
        }
        self.focus_request = self.focused_terminal.clone();
        self.mark_dirty();
    }

    fn add_terminal(
        &mut self,
        project_id: &str,
        agent: &str,
        cwd: Option<String>,
        extra_args: Vec<String>,
        initial_input: Option<String>,
    ) {
        let Some(project) = self.project_mut(project_id) else {
            return;
        };
        let cwd = cwd
            .filter(|c| !c.trim().is_empty())
            .unwrap_or_else(|| project.cwd());
        let mut terminal = Terminal::new(agent, cwd);
        if let Some(tab) = terminal.tabs.first_mut() {
            if !extra_args.is_empty() {
                tab.extra_args = Some(extra_args);
            }
            tab.initial_input = initial_input;
        }
        let id = terminal.id.clone();
        project.terminals.push(terminal);
        project.collapsed = false;
        self.open_project(project_id);
        self.focused_terminal = Some(id.clone());
        self.focus_request = Some(id);
        self.mark_dirty();
    }

    /// A pane that is not a terminal: no sub-tabs, no PTY, rendered by its `kind`.
    fn add_pane(&mut self, project_id: &str, kind: &str, name: String, file_path: Option<String>) {
        let Some(project) = self.project_mut(project_id) else {
            return;
        };
        let mut pane = Terminal::new("shell", project.cwd());
        pane.tabs.clear();
        pane.active_tab_id = String::new();
        pane.kind = Some(kind.to_string());
        pane.name = name;
        if kind == "orchestrator" {
            pane.id = format!("orchestrator-{}", pane.id);
        }
        if let Some(path) = file_path {
            pane.extra
                .insert("filePath".into(), serde_json::Value::String(path));
        }
        let id = pane.id.clone();
        project.terminals.push(pane);
        project.collapsed = false;
        self.open_project(project_id);
        self.focused_terminal = Some(id);
        self.mark_dirty();
    }

    fn create_project(
        &mut self,
        name: String,
        folder: String,
        group_id: Option<String>,
        color: String,
    ) -> String {
        let folder = folder.trim().to_string();
        let name = if name.trim().is_empty() {
            std::path::Path::new(&folder)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "projeto".into())
        } else {
            name.trim().to_string()
        };
        let mut project = Project::new(name, (!folder.is_empty()).then_some(folder), color);
        project.group_id = group_id.clone();
        let id = project.id.clone();
        self.data.projects.push(project);
        match group_id.and_then(|g| self.data.groups.iter_mut().find(|x| x.id == g)) {
            Some(group) => group.project_ids.push(id.clone()),
            None => self.data.ungrouped_order.push(id.clone()),
        }
        self.mark_dirty();
        id
    }

    fn detach_from_groups(&mut self, project_id: &str) {
        for group in &mut self.data.groups {
            group.project_ids.retain(|id| id != project_id);
        }
        self.data.ungrouped_order.retain(|id| id != project_id);
    }

    pub fn apply(&mut self, action: Action) {
        match action {
            Action::OpenHome => {
                self.view = View::Home;
                self.mark_dirty();
            }
            Action::OpenProject(id) => self.open_project(&id),
            Action::CloseTab(id) => {
                let position = self.open_tabs.iter().position(|t| *t == id);
                self.open_tabs.retain(|t| *t != id);
                if self.view == View::Project(id) {
                    let next =
                        position.and_then(|p| self.open_tabs.get(p.saturating_sub(1)).cloned());
                    match next {
                        Some(next) => self.open_project(&next),
                        None => self.view = View::Home,
                    }
                }
            }
            Action::NewTerminal {
                project_id,
                agent,
                cwd,
                extra_args,
                initial_input,
            } => self.add_terminal(&project_id, &agent, cwd, extra_args, initial_input),
            Action::AddSubTab { terminal_id, agent } => {
                if let Some((project, i)) = self.find_terminal_mut(&terminal_id) {
                    let terminal = &mut project.terminals[i];
                    let tab = SubTab::new(&agent, terminal.cwd.clone());
                    terminal.active_tab_id = tab.id.clone();
                    terminal.tabs.push(tab);
                    self.focus_request = Some(terminal_id);
                    self.mark_dirty();
                }
            }
            Action::SelectSubTab {
                terminal_id,
                tab_id,
            } => {
                if let Some((project, i)) = self.find_terminal_mut(&terminal_id) {
                    project.terminals[i].active_tab_id = tab_id;
                    self.focus_request = Some(terminal_id);
                    self.mark_dirty();
                }
            }
            Action::CloseSubTab {
                terminal_id,
                tab_id,
            } => {
                let mut close_terminal = false;
                if let Some((project, i)) = self.find_terminal_mut(&terminal_id) {
                    let terminal = &mut project.terminals[i];
                    terminal.tabs.retain(|t| t.id != tab_id);
                    if terminal.tabs.is_empty() {
                        close_terminal = true;
                    } else if terminal.active_tab_id == tab_id {
                        terminal.active_tab_id = terminal
                            .tabs
                            .last()
                            .map(|t| t.id.clone())
                            .unwrap_or_default();
                    }
                }
                self.kill_tab(&tab_id);
                if close_terminal {
                    self.apply(Action::CloseTerminal(terminal_id));
                }
                self.mark_dirty();
            }
            Action::CloseTerminal(id) => {
                if let Some((project, i)) = self.find_terminal_mut(&id) {
                    let terminal = project.terminals.remove(i);
                    let next = project
                        .terminals
                        .get(i.saturating_sub(1))
                        .or(project.terminals.first())
                        .map(|t| t.id.clone());
                    self.kill_terminal(&terminal);
                    if self.focused_terminal.as_deref() == Some(&id) {
                        self.focused_terminal = next.clone();
                        self.focus_request = next;
                    }
                    if self.maximized.as_deref() == Some(&id) {
                        self.maximized = None;
                    }
                    // A closed image pane gives its texture back now, not on the next eviction.
                    self.images.remove(&id);
                    self.markdown.remove(&id);
                    self.mark_dirty();
                }
            }
            Action::RestartTab(tab_id) => {
                self.kill_tab(&tab_id);
                self.ctx.request_repaint();
            }
            Action::FocusTerminal(id) => {
                if let Some(project_id) = self.terminal_project(&id).map(|p| p.id.clone()) {
                    if self.view != View::Project(project_id.clone()) {
                        self.open_project(&project_id);
                    }
                }
                if self.maximized.is_some() && self.maximized.as_deref() != Some(&id) {
                    self.maximized = Some(id.clone());
                }
                self.focused_terminal = Some(id.clone());
                self.focus_request = Some(id);
            }
            Action::ToggleMaximize(id) => {
                self.maximized = if self.maximized.as_deref() == Some(&id) {
                    None
                } else {
                    Some(id.clone())
                };
                self.focus_request = Some(id);
            }
            Action::ToggleProjectCollapsed(id) => {
                if let Some(p) = self.project_mut(&id) {
                    p.collapsed = !p.collapsed;
                    self.mark_dirty();
                }
            }
            Action::ToggleGroupCollapsed(id) => {
                if let Some(g) = self.data.groups.iter_mut().find(|g| g.id == id) {
                    g.collapsed = !g.collapsed;
                    self.mark_dirty();
                }
            }
            Action::SetLayout { project_id, mode } => {
                if let Some(p) = self.project_mut(&project_id) {
                    p.layout_mode = mode;
                    self.mark_dirty();
                }
            }
            Action::MoveProjectToGroup {
                project_id,
                group_id,
            } => {
                self.detach_from_groups(&project_id);
                match group_id
                    .clone()
                    .and_then(|g| self.data.groups.iter_mut().find(|x| x.id == g))
                {
                    Some(group) => group.project_ids.push(project_id.clone()),
                    None => self.data.ungrouped_order.push(project_id.clone()),
                }
                if let Some(p) = self.project_mut(&project_id) {
                    p.group_id = group_id;
                }
                self.mark_dirty();
            }
            Action::Suspend(target) => self.set_suspended(&target, true),
            Action::Reactivate(target) => self.set_suspended(&target, false),
            Action::DeleteGroup(id) => {
                if let Some(pos) = self.data.groups.iter().position(|g| g.id == id) {
                    let group = self.data.groups.remove(pos);
                    for project_id in group.project_ids {
                        if let Some(p) = self.project_mut(&project_id) {
                            p.group_id = None;
                        }
                        self.data.ungrouped_order.push(project_id);
                    }
                    for child in self
                        .data
                        .groups
                        .iter_mut()
                        .filter(|g| g.parent_group_id.as_deref() == Some(&id))
                    {
                        child.parent_group_id = None;
                    }
                    self.mark_dirty();
                }
            }
            Action::DeleteProject(id) => {
                if let Some(pos) = self.data.projects.iter().position(|p| p.id == id) {
                    let project = self.data.projects.remove(pos);
                    for terminal in &project.terminals {
                        self.kill_terminal(terminal);
                    }
                    self.detach_from_groups(&id);
                    self.data.workspace.recent_project_ids.retain(|p| *p != id);
                    self.apply(Action::CloseTab(id));
                    self.modal = None;
                    self.mark_dirty();
                }
            }
            Action::ShowModal(modal) => self.modal = Some(modal),
            Action::CloseModal => self.modal = None,
            Action::CreateProject {
                name,
                folder,
                group_id,
                color,
            } => {
                let id = self.create_project(name, folder, group_id, color);
                self.modal = None;
                self.open_project(&id);
            }
            Action::CreateGroup { name, color } => {
                let name = if name.trim().is_empty() {
                    "grupo".to_string()
                } else {
                    name.trim().to_string()
                };
                self.data.groups.push(Group {
                    id: crate::model::new_id(),
                    name,
                    color,
                    collapsed: false,
                    project_ids: vec![],
                    parent_group_id: None,
                    archived: None,
                    suspended: None,
                    created_at: now_ms(),
                    extra: Default::default(),
                });
                self.modal = None;
                self.mark_dirty();
            }
            Action::Rename { target, value } => {
                let value = value.trim().to_string();
                if !value.is_empty() {
                    match target {
                        RenameTarget::Project(id) => {
                            if let Some(p) = self.project_mut(&id) {
                                p.name = value;
                            }
                        }
                        RenameTarget::Group(id) => {
                            if let Some(g) = self.data.groups.iter_mut().find(|g| g.id == id) {
                                g.name = value;
                            }
                        }
                        RenameTarget::Terminal(id) => {
                            if let Some((project, i)) = self.find_terminal_mut(&id) {
                                project.terminals[i].name = value;
                                project.terminals[i].custom_name = Some(true);
                            }
                        }
                    }
                    self.mark_dirty();
                }
                self.modal = None;
            }
            Action::QuickStart {
                agent,
                folder,
                prompt,
                yolo,
            } => {
                let folder = if folder.trim().is_empty() {
                    crate::storage::home_dir()
                } else {
                    folder
                };
                let normalized = |p: &str| p.trim_end_matches(['/', '\\']).to_lowercase();
                let existing = self
                    .data
                    .projects
                    .iter()
                    .find(|p| {
                        p.default_cwd
                            .as_deref()
                            .is_some_and(|c| normalized(c) == normalized(&folder))
                    })
                    .map(|p| p.id.clone());
                let project_id = existing.unwrap_or_else(|| {
                    let color =
                        PROJECT_COLORS[self.data.projects.len() % PROJECT_COLORS.len()].to_string();
                    self.create_project(String::new(), folder.clone(), None, color)
                });
                let extra_args = if yolo {
                    crate::agents::unrestricted_flag(&agent)
                        .map(|f| vec![f.to_string()])
                        .unwrap_or_default()
                } else {
                    vec![]
                };
                let input = (!prompt.trim().is_empty()).then_some(prompt);
                self.add_terminal(&project_id, &agent, Some(folder.clone()), extra_args, input);
                self.home.prompt.clear();
                self.home.folder = folder;
            }
            Action::AddTodo {
                title,
                tags,
                project_id,
            } => {
                if !title.trim().is_empty() {
                    // New tasks go last among the open ones, above the completed.
                    let at = self
                        .data
                        .todos
                        .iter()
                        .position(|t| t.completed)
                        .unwrap_or(self.data.todos.len());
                    self.data.todos.insert(
                        at,
                        TodoItem {
                            id: crate::model::new_id(),
                            title: title.trim().to_string(),
                            completed: false,
                            tags,
                            project_id,
                            extra: Default::default(),
                        },
                    );
                    self.mark_dirty();
                }
            }
            Action::UpdateTodo { id, title, tags } => {
                if let Some(todo) = self.data.todos.iter_mut().find(|t| t.id == id) {
                    if !title.trim().is_empty() {
                        todo.title = title.trim().to_string();
                    }
                    todo.tags = tags;
                    self.mark_dirty();
                }
            }
            Action::ReorderTodo { dragged, target } => {
                let todos = &mut self.data.todos;
                if let (Some(from), Some(to)) = (
                    todos.iter().position(|t| t.id == dragged),
                    todos.iter().position(|t| t.id == target),
                ) {
                    let item = todos.remove(from);
                    todos.insert(to, item);
                    self.mark_dirty();
                }
            }
            Action::ToggleTodo(id) => {
                // Completing moves a task to the end; reopening puts it back above the completed.
                if let Some(from) = self.data.todos.iter().position(|t| t.id == id) {
                    let mut todo = self.data.todos.remove(from);
                    todo.completed = !todo.completed;
                    let at = if todo.completed {
                        self.data.todos.len()
                    } else {
                        self.data
                            .todos
                            .iter()
                            .position(|t| t.completed)
                            .unwrap_or(self.data.todos.len())
                    };
                    if todo.completed && self.pomodoro.focus_todo_id.as_deref() == Some(id.as_str())
                    {
                        self.pomodoro_update(|p, _, _| p.focus_todo_id = None);
                    }
                    self.data.todos.insert(at, todo);
                    self.mark_dirty();
                }
            }
            Action::DeleteTodo(id) => {
                self.data.todos.retain(|t| t.id != id);
                self.mark_dirty();
            }
            Action::SetTheme(id) => {
                self.data.preferences.ui_theme = id;
                self.mark_dirty();
            }
            Action::SetTerminalTheme(id) => {
                self.data.preferences.terminal_theme = id;
                let palette = crate::theme::term_palette(&self.terminal_theme_id());
                for session in self.sessions.values() {
                    session.set_palette(palette);
                }
                self.mark_dirty();
            }
            Action::SetLanguage(code) => {
                self.data.preferences.language = code;
                self.mark_dirty();
            }
            Action::Reimport => match self.storage.reimport() {
                Ok(file) => {
                    for session in self.sessions.values() {
                        session.kill();
                    }
                    self.sessions.clear();
                    self.spawn_errors.clear();
                    self.data = file;
                    self.view = View::Home;
                    self.open_tabs.clear();
                    self.focused_terminal = None;
                    self.toasts
                        .push((self.t("toast.imported").to_string(), Instant::now()));
                }
                Err(error) => self.toasts.push((error, Instant::now())),
            },
            Action::OpenFolder(path) => open_in_file_manager(&path),
            Action::NewOrchestration(project_id) => {
                let name = self.t("orchestrator.title").to_string();
                self.add_pane(&project_id, "orchestrator", name, None);
            }
            Action::OpenMarkdown(project_id) => {
                let start = self
                    .project(&project_id)
                    .map(|p| p.cwd())
                    .unwrap_or_default();
                let picked = rfd::FileDialog::new()
                    .add_filter("Markdown", &["md", "markdown", "mdx", "txt"])
                    .set_directory(&start)
                    .pick_file();
                if let Some(path) = picked {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "README.md".into());
                    self.add_pane(
                        &project_id,
                        "markdown",
                        name,
                        Some(path.to_string_lossy().to_string()),
                    );
                }
            }
            Action::OpenFiles(project_id) => {
                // Typed into a shell rather than launched directly: quitting nvim leaves a shell in
                // the folder instead of a dead pane, and `nvim` resolves through the user's PATH.
                self.add_terminal(&project_id, "shell", None, Vec::new(), Some("nvim .".into()));
                if let Some(id) = self.focused_terminal.clone() {
                    if let Some((project, index)) = self.find_terminal_mut(&id) {
                        let terminal = &mut project.terminals[index];
                        terminal.name = "nvim".into();
                        terminal.custom_name = Some(true);
                    }
                }
            }
            Action::OpenImage(project_id) => {
                let start = self
                    .project(&project_id)
                    .map(|p| p.cwd())
                    .unwrap_or_default();
                let picked = rfd::FileDialog::new()
                    .add_filter(self.t("image.filter"), crate::ui::image_pane::EXTENSIONS)
                    .set_directory(&start)
                    .pick_file();
                if let Some(path) = picked {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    self.add_pane(
                        &project_id,
                        "image",
                        name,
                        Some(path.to_string_lossy().to_string()),
                    );
                }
            }
            Action::Toast(text) => self.toasts.push((text, Instant::now())),
        }
    }
}

fn open_in_file_manager(path: &str) {
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    let mut command = std::process::Command::new(program);
    command.arg(path);
    crate::backend::child_env::scrub_command(&mut command);
    let _ = command.spawn();
}

/// RAM of this process plus every descendant (agents, shells) — the title bar's `553 MB` pill.

/// eframe is built without its `webbrowser` support, so links (`ui.hyperlink`, markdown) are
/// opened here: only http(s), in the default browser.
fn open_requested_urls(ctx: &egui::Context) {
    let urls = ctx.output_mut(|o| {
        let mut urls = Vec::new();
        o.commands.retain(|command| match command {
            egui::OutputCommand::OpenUrl(open) => {
                urls.push(open.url.clone());
                false
            }
            _ => true,
        });
        urls
    });
    for url in urls {
        open_in_browser(&url);
    }
}

pub fn is_web_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

pub fn open_in_browser(url: &str) {
    if !is_web_url(url) {
        return;
    }
    // `FileProtocolHandler` hands the URL to the default browser without a shell parsing it.
    #[cfg(windows)]
    let _ = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url.trim()])
        .spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open")
        .arg(url.trim())
        .spawn();
}

impl eframe::App for AletheApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame_start(&ctx);
        crate::ui::root(self, ui);
        self.frame_end();
        open_requested_urls(&ctx);
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        if self.dirty_since.is_some() {
            self.save_now();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_now();
        for session in self.sessions.values() {
            session.kill_blocking();
        }
        self.orchestrator.kill_shells();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.theme.bg.to_normalized_gamma_f32()
    }
}
