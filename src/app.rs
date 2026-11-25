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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Home,
    Project(String),
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
    Settings,
    About,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Action {
    OpenHome,
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
    ToggleTodo(String),
    DeleteTodo(String),
    SetTheme(String),
    SetTerminalTheme(Option<String>),
    SetLanguage(String),
    Reimport,
    OpenFolder(String),
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
    pub sidebar_search: String,
    pub modal: Option<Modal>,
    pub home: HomeState,
    pub todo: TodoState,
    pub theme: Theme,
    applied_theme: String,
    applied_zoom: f32,
    pub toasts: Vec<(String, Instant)>,
    pub memory_mb: Arc<AtomicU64>,
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

        let memory_mb = Arc::new(AtomicU64::new(0));
        spawn_memory_probe(Arc::clone(&memory_mb), cc.egui_ctx.clone());

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
                initial_actions.push(Action::QuickStart { agent, folder, prompt, yolo: false });
            }
        }

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
            maximized: None,
            show_sidebar: true,
            show_right: false,
            sidebar_search: String::new(),
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
            },
            theme,
            toasts,
            memory_mb,
            actions: RefCell::new(initial_actions),
            ctx: cc.egui_ctx.clone(),
        }
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
        let terminal = &mut project.terminals[index];
        terminal.last_used_at = Some(now_ms());
        let terminal_cwd = terminal.cwd.clone();
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
            Ok(b) => b,
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
                    self.data.todos.insert(
                        0,
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
            Action::ToggleTodo(id) => {
                if let Some(todo) = self.data.todos.iter_mut().find(|t| t.id == id) {
                    todo.completed = !todo.completed;
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
fn spawn_memory_probe(target: Arc<AtomicU64>, ctx: egui::Context) {
    std::thread::Builder::new()
        .name("memory-probe".into())
        .spawn(move || {
            use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
            let me = Pid::from_u32(std::process::id());
            let mut system = System::new();
            loop {
                system.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true,
                    ProcessRefreshKind::nothing().with_memory(),
                );
                let processes = system.processes();
                let mut total = 0u64;
                for (pid, process) in processes {
                    let mut cursor = Some(*pid);
                    let mut depth = 0;
                    while let Some(current) = cursor {
                        if current == me {
                            total += process.memory();
                            break;
                        }
                        cursor = processes.get(&current).and_then(|p| p.parent());
                        depth += 1;
                        if depth > 32 {
                            break;
                        }
                    }
                }
                target.store(total / (1024 * 1024), Ordering::Relaxed);
                ctx.request_repaint();
                std::thread::sleep(Duration::from_secs(3));
            }
        })
        .ok();
}

impl eframe::App for AletheApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame_start(&ctx);
        crate::ui::root(self, ui);
        self.frame_end();
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
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.theme.bg.to_normalized_gamma_f32()
    }
}
