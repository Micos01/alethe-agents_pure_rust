//! Source Control panel — port of the web build's `src/plugins/git-control` plugin
//! (`GitControl`, `GitGraph`, `GitGraphList`, `GitGraphCommitDetail`, `IncomingOutgoing`).
//! AGPL-3.0-or-later.
//!
//! Every git call runs on a worker thread and reports back over a channel; the UI only drains
//! that channel each frame, so it never blocks on git.

use super::icons;
use super::widgets::{bold, mono, sans};
use crate::app::{Action, AletheApp};
use crate::backend::git_control::{
    self as git, GitCommitEntry, GitFileChange, GitRepositoryStatus, IncomingOutgoing,
};
use crate::i18n::Lang;
use crate::theme::Theme;
use egui::epaint::CubicBezierShape;
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Galley, Id, Key, Layout, Margin, Modifiers, Pos2,
    Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

// Graph geometry, from GitGraphList.tsx / GitGraph.module.css.
const ROW_HEIGHT: f32 = 34.0;
const LANE_WIDTH: f32 = 18.0;
const DOT_RADIUS: f32 = 5.0;
const GRAPH_MAX_HEIGHT: f32 = 380.0;
/// The web build loads the whole history (`MAX_COMMITS = 0`). Natively the list is capped: the
/// graph is for recent work, and an unbounded `git log` on a large repository would cost memory
/// for rows nobody scrolls to.
const MAX_COMMITS: u32 = 500;
/// GitControl polls `git status` every 3 s while visible, throttled to 1.5 s for focus events.
const AUTO_REFRESH: Duration = Duration::from_secs(3);
const FOCUS_THROTTLE: Duration = Duration::from_millis(1500);

/// Lane palette: the theme's `--agent-*` colors, minus `shell` (same hex as the main lane).
const LANE_AGENTS: [&str; 11] = [
    "claude",
    "codex",
    "opencode",
    "freebuff",
    "mimo",
    "antigravity",
    "kiro",
    "cursor",
    "kimi",
    "grok",
    "codewhale",
];

macro_rules! messages {
    ($($key:literal => $pt:literal, $en:literal;)*) => {
        fn tr(lang: Lang, key: &str) -> &'static str {
            match (key, lang) {
                $(($key, Lang::PtBr) => $pt, ($key, Lang::En) => $en,)*
                _ => "?",
            }
        }
    };
}

// From `src/lib/i18n/messages/{pt-BR,en}.ts`.
messages! {
    "ui.sidebar.git" => "Controle Git", "Git Control";
    "ui.sidebar.sourceControl" => "Controle de versão", "Source Control";
    "ui.sidebar.emptyAction" => "Criar projeto", "Create project";
    "files.reveal" => "Mostrar no Explorador de Arquivos", "Reveal in File Explorer";
    "files.revealFolder" => "Abrir pasta no Explorador de Arquivos", "Open folder in File Explorer";
    "common.back" => "Voltar", "Back";
    "common.cancel" => "Cancelar", "Cancel";
    "common.confirm" => "Confirmar", "Confirm";
    "git.loading" => "Lendo repositório...", "Reading repository...";
    "git.refresh" => "Atualizar repositório", "Refresh repository";
    "git.detached" => "desanexado", "detached";
    "git.commit.placeholder" => "Mensagem (Ctrl+Enter para commit)", "Message (Ctrl+Enter to commit)";
    "git.commit.action" => "Commit", "Commit";
    "git.commit.busy" => "Processando...", "Working...";
    "git.commit.done" => "Commit criado", "Commit created";
    "git.sync.action" => "Sincronizar", "Sync";
    "git.sync.title" => "Sincronizar — pull {behind} · push {ahead}", "Sync changes — pull {behind} · push {ahead}";
    "git.sync.done" => "Sincronizado com o remoto", "Synced with remote";
    "git.group.staged" => "Alterações preparadas", "Staged Changes";
    "git.group.changes" => "Alterações", "Changes";
    "git.group.untracked" => "Não rastreados", "Untracked";
    "git.group.conflicts" => "Conflitos de merge", "Merge Changes";
    "git.stage" => "Preparar alteração", "Stage change";
    "git.stageAll" => "Preparar todas as alterações", "Stage all changes";
    "git.unstage" => "Remover da preparação", "Unstage change";
    "git.unstageAll" => "Remover todas da preparação", "Unstage all changes";
    "git.discard" => "Descartar alteração", "Discard change";
    "git.discardAll" => "Descartar todas as alterações", "Discard all changes";
    "git.confirm.discard" => "Descartar {count} alteração(ões)? Isso não pode ser desfeito.", "Discard {count} change(s)? This cannot be undone.";
    "git.confirm.stageAllCommit" => "Nenhuma alteração está preparada. Preparar todas e criar o commit?", "No changes are staged. Stage all changes and commit?";
    "git.clean" => "Diretório de trabalho limpo", "Working tree clean";
    "git.cleanDesc" => "Não há alterações para commit.", "There are no changes to commit.";
    "git.empty.noFolder" => "Nenhuma pasta ativa", "No active folder";
    "git.empty.noFolderDesc" => "O terminal selecionado ainda não possui um diretório de trabalho.", "The selected terminal does not have a working directory yet.";
    "git.empty.noTerminal" => "Nenhum terminal selecionado", "No terminal selected";
    "git.empty.noTerminalDesc" => "Abra um terminal de projeto para inspecionar seu repositório Git.", "Open a project terminal to inspect its Git repository.";
    "git.graph.title" => "Gráfico de commits", "Commit graph";
    "git.graph.loading" => "Carregando histórico…", "Loading history…";
    "git.graph.empty" => "Nenhum commit ainda.", "No commits yet.";
    "git.graph.timeNow" => "agora mesmo", "just now";
    "git.graph.timeMinutes" => "há {count}min", "{count}m ago";
    "git.graph.timeHours" => "há {count}h", "{count}h ago";
    "git.graph.timeDays" => "há {count}d", "{count}d ago";
    "git.graph.filesLoading" => "Carregando arquivos alterados…", "Loading changed files…";
    "git.graph.filesEmpty" => "Nenhum arquivo alterado (commit vazio ou de merge).", "No file changes (empty or merge commit).";
    "git.graph.detail.loadingMessage" => "Carregando mensagem do commit…", "Loading commit message…";
    "git.graph.detail.filesTitle" => "Arquivos alterados", "Changed files";
    "git.graph.menu.copyHash" => "Copiar hash do commit", "Copy commit hash";
    "git.graph.menu.createBranch" => "Criar branch aqui", "Create branch here";
    "git.graph.menu.createBranchPrompt" => "Nome da nova branch", "New branch name";
    "git.graph.menu.branchCreated" => "Branch criada", "Branch created";
    "git.graph.menu.cherryPick" => "Cherry-pick deste commit", "Cherry-pick this commit";
    "git.graph.menu.cherryPicked" => "Cherry-pick aplicado", "Commit cherry-picked";
    "git.graph.menu.revert" => "Reverter este commit", "Revert this commit";
    "git.graph.menu.reverted" => "Commit revertido", "Commit reverted";
    "git.graph.menu.resetSoft" => "Resetar branch aqui (soft)", "Reset branch here (soft)";
    "git.graph.menu.resetMixed" => "Resetar branch aqui (mixed)", "Reset branch here (mixed)";
    "git.graph.menu.resetHard" => "Resetar branch aqui (hard)", "Reset branch here (hard)";
    "git.graph.menu.resetHardConfirm" => "O reset hard descartará TODAS as alterações não salvas da workspace. Continuar?", "Hard reset will discard ALL unstaged and staged changes in your workspace. Continue?";
    "git.graph.menu.resetDone" => "Branch resetada", "Branch reset";
    "git.graph.searchPlaceholder" => "Filtrar commits por mensagem, hash, autor ou ref…", "Filter commits by subject, hash, author, or ref…";
    "git.graph.noSearchResults" => "Nenhum commit atende ao filtro de busca.", "No commits match your filter.";
    "git.graph.refresh" => "Atualizar gráfico", "Refresh graph";
    "git.incomingOutgoing.title" => "Alterações a receber / enviar", "Incoming / Outgoing changes";
    "git.incomingOutgoing.loading" => "Carregando…", "Loading…";
    "git.incomingOutgoing.empty" => "Nada pra sincronizar.", "Nothing to sync.";
    "git.incomingOutgoing.incoming" => "A receber ({count})", "Incoming ({count})";
    "git.incomingOutgoing.outgoing" => "A enviar ({count})", "Outgoing ({count})";
    "git.error.notFound" => "Git não foi encontrado", "Git was not found";
    "git.error.notRepository" => "Esta pasta não é um repositório Git", "This folder is not a Git repository";
    "git.error.directory" => "A pasta ativa não está disponível", "The active folder is unavailable";
    "git.error.generic" => "Não foi possível ler o repositório", "Could not read the repository";
    "git.error.action" => "A ação Git falhou", "Git action failed";
    "git.error.conflicts" => "Resolva ou prepare os conflitos de merge antes do commit", "Resolve or stage merge conflicts before committing";
    "git.initOffer.body" => "Isolamento de agentes, worktrees e merges precisam que esta pasta seja um repositório Git. Inicializar agora?", "Agent isolation, worktrees and merges all need this folder to be a Git repository. Initialize one now?";
    "git.initOffer.button" => "Inicializar repositório Git", "Initialize Git repository";
    "git.initOffer.busy" => "Inicializando…", "Initializing…";
    "git.initOffer.successTitle" => "Repositório Git inicializado", "Git repository initialized";
}

/// Error codes the backend returns, mapped to their message keys (`ERROR_KEYS`).
const ERROR_KEYS: [(&str, &str); 3] = [
    ("git_not_found", "git.error.notFound"),
    ("not_a_git_repository", "git.error.notRepository"),
    ("directory_not_found", "git.error.directory"),
];

fn error_code(error: &str) -> String {
    ERROR_KEYS
        .iter()
        .find(|(code, _)| error.contains(code))
        .map(|(code, _)| code.to_string())
        .unwrap_or_else(|| error.to_string())
}

/// `readableError`: drops the backend's machine prefix.
fn readable(error: &str) -> String {
    error
        .strip_prefix("git_command_failed:")
        .or_else(|| error.strip_prefix("git_exec_failed:"))
        .unwrap_or(error)
        .trim()
        .to_string()
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Group {
    Staged,
    Conflicts,
    Changes,
    Untracked,
}

impl Group {
    const ORDER: [Group; 4] = [
        Group::Staged,
        Group::Conflicts,
        Group::Changes,
        Group::Untracked,
    ];

    fn key(self) -> &'static str {
        match self {
            Group::Staged => "staged",
            Group::Conflicts => "conflicts",
            Group::Changes => "changes",
            Group::Untracked => "untracked",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Group::Staged => "git.group.staged",
            Group::Conflicts => "git.group.conflicts",
            Group::Changes => "git.group.changes",
            Group::Untracked => "git.group.untracked",
        }
    }

    fn items(self, status: &GitRepositoryStatus) -> &[GitFileChange] {
        match self {
            Group::Staged => &status.staged,
            Group::Conflicts => &status.conflicts,
            Group::Changes => &status.changes,
            Group::Untracked => &status.untracked,
        }
    }

    fn discardable(self) -> bool {
        matches!(self, Group::Changes | Group::Untracked)
    }
}

/// A git operation, run on a worker thread.
#[derive(Clone, Debug)]
enum Op {
    Stage(Vec<String>),
    Unstage(Vec<String>),
    Discard {
        paths: Vec<String>,
        untracked: bool,
    },
    Commit {
        message: String,
        stage_first: Vec<String>,
    },
    Sync {
        pull: bool,
    },
    Init,
    CreateBranch {
        hash: String,
        name: String,
    },
    CherryPick(String),
    Revert(String),
    Reset {
        hash: String,
        mode: &'static str,
    },
}

impl Op {
    fn success_key(&self) -> Option<&'static str> {
        match self {
            Op::Commit { .. } => Some("git.commit.done"),
            Op::Sync { .. } => Some("git.sync.done"),
            Op::Init => Some("git.initOffer.successTitle"),
            Op::CreateBranch { .. } => Some("git.graph.menu.branchCreated"),
            Op::CherryPick(_) => Some("git.graph.menu.cherryPicked"),
            Op::Revert(_) => Some("git.graph.menu.reverted"),
            Op::Reset { .. } => Some("git.graph.menu.resetDone"),
            _ => None,
        }
    }

    fn run(self, repo: String, cwd: String) -> Result<(), String> {
        match self {
            Op::Stage(paths) => git::git_stage_inner(repo, paths),
            Op::Unstage(paths) => git::git_unstage_inner(repo, paths),
            Op::Discard { paths, untracked } => git::git_discard_inner(repo, paths, untracked),
            Op::Commit {
                message,
                stage_first,
            } => {
                if !stage_first.is_empty() {
                    git::git_stage_inner(repo.clone(), stage_first)?;
                }
                git::git_commit_inner(repo, message).map(drop)
            }
            Op::Sync { pull } => {
                if pull {
                    git::git_pull_inner(repo.clone())?;
                }
                git::git_push_inner(repo).map(drop)
            }
            Op::Init => git::git_init_inner(cwd).map(drop),
            Op::CreateBranch { hash, name } => {
                git::git_create_branch_from_commit_inner(repo, hash, name)
            }
            Op::CherryPick(hash) => git::git_cherry_pick_commit_inner(repo, hash).map(drop),
            Op::Revert(hash) => git::git_revert_commit_inner(repo, hash).map(drop),
            Op::Reset { hash, mode } => {
                git::git_reset_to_commit_inner(repo, hash, mode.to_string())
            }
        }
    }
}

enum Msg {
    Status {
        request: u64,
        result: Result<GitRepositoryStatus, String>,
    },
    OpDone {
        generation: u64,
        success: Option<&'static str>,
        result: Result<(), String>,
    },
    Graph {
        generation: u64,
        result: Result<Vec<GitCommitEntry>, String>,
    },
    Detail {
        hash: String,
        result: Result<(String, Vec<GitFileChange>), String>,
    },
    Io {
        generation: u64,
        result: Result<IncomingOutgoing, String>,
    },
}

enum Dialog {
    Confirm { text: String, op: Op },
    BranchName { hash: String, name: String },
}

/// Changed-file tree of one group (`buildTree` + `compress`), indices into the group's items.
enum Node {
    Dir {
        name: String,
        path: String,
        children: Vec<Node>,
    },
    File {
        name: String,
        index: usize,
    },
}

struct GraphRow {
    lane: usize,
    color: Option<u8>,
    before: Vec<Option<u32>>,
    after: Vec<Option<u32>>,
    colors_before: Vec<Option<u8>>,
    colors_after: Vec<Option<u8>>,
    last: bool,
}

struct Graph {
    commits: Vec<GitCommitEntry>,
    rows: Vec<GraphRow>,
    lane_count: usize,
}

struct Detail {
    hash: String,
    message: Option<String>,
    files: Option<Vec<GitFileChange>>,
    error: Option<String>,
}

/// What the panel looks at: the sidebar's project and its working directory.
pub struct Target {
    pub cwd: String,
    pub name: String,
}

pub struct GitPanel {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// Bumped whenever the folder changes, so late answers for the old folder are dropped.
    generation: u64,
    cwd: String,
    status: Option<GitRepositoryStatus>,
    trees: Vec<(Group, Vec<Node>)>,
    error: Option<String>,
    loading: bool,
    status_request: u64,
    status_in_flight: bool,
    last_refresh: Option<Instant>,
    busy: bool,
    message: String,
    collapsed: HashSet<String>,
    io_open: bool,
    io: Option<IncomingOutgoing>,
    io_error: Option<String>,
    io_for: (u32, u32),
    graph_open: bool,
    graph: Option<Graph>,
    graph_error: Option<String>,
    graph_loading: bool,
    search: String,
    detail: Option<Detail>,
    list_offset: f32,
    restore_offset: bool,
    dialog: Option<Dialog>,
}

impl Default for GitPanel {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            generation: 0,
            cwd: String::new(),
            status: None,
            trees: Vec::new(),
            error: None,
            loading: false,
            status_request: 0,
            status_in_flight: false,
            last_refresh: None,
            busy: false,
            message: String::new(),
            collapsed: HashSet::new(),
            io_open: false,
            io: None,
            io_error: None,
            io_for: (0, 0),
            graph_open: true,
            graph: None,
            graph_error: None,
            graph_loading: false,
            search: String::new(),
            detail: None,
            list_offset: 0.0,
            restore_offset: false,
            dialog: None,
        }
    }
}

fn spawn<T: Send + 'static>(
    ctx: &egui::Context,
    tx: &Sender<Msg>,
    job: impl FnOnce() -> T + Send + 'static,
    wrap: impl FnOnce(T) -> Msg + Send + 'static,
) {
    let ctx = ctx.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(wrap(job()));
        ctx.request_repaint();
    });
}

impl GitPanel {
    fn reset_for(&mut self, cwd: &str) {
        self.generation += 1;
        self.cwd = cwd.to_string();
        self.status = None;
        self.trees.clear();
        self.error = None;
        self.loading = false;
        self.status_in_flight = false;
        self.last_refresh = None;
        self.busy = false;
        self.io = None;
        self.io_error = None;
        self.graph = None;
        self.graph_error = None;
        self.graph_loading = false;
        self.detail = None;
        self.list_offset = 0.0;
        self.dialog = None;
    }

    fn repo(&self) -> Option<String> {
        self.status.as_ref().map(|s| s.repo_root.clone())
    }

    /// `refresh(quiet)`: a loud refresh shows the loading state, a quiet one (polling) doesn't.
    fn refresh(&mut self, ctx: &egui::Context, quiet: bool) {
        if self.cwd.is_empty() {
            return;
        }
        self.status_request += 1;
        self.status_in_flight = true;
        self.last_refresh = Some(Instant::now());
        if !quiet {
            self.loading = true;
        }
        let request = self.status_request;
        let cwd = self.cwd.clone();
        spawn(
            ctx,
            &self.tx,
            move || git::git_status_inner(cwd),
            move |result| Msg::Status { request, result },
        );
    }

    fn load_graph(&mut self, ctx: &egui::Context) {
        let Some(repo) = self.repo() else { return };
        self.graph_loading = true;
        let generation = self.generation;
        spawn(
            ctx,
            &self.tx,
            move || git::git_log_graph_inner(repo, MAX_COMMITS),
            move |result| Msg::Graph { generation, result },
        );
    }

    fn load_io(&mut self, ctx: &egui::Context) {
        let Some(status) = &self.status else { return };
        self.io_for = (status.ahead, status.behind);
        let repo = status.repo_root.clone();
        let generation = self.generation;
        spawn(
            ctx,
            &self.tx,
            move || git::git_incoming_outgoing_inner(repo),
            move |result| Msg::Io { generation, result },
        );
    }

    fn open_detail(&mut self, ctx: &egui::Context, hash: String) {
        let Some(repo) = self.repo() else { return };
        self.detail = Some(Detail {
            hash: hash.clone(),
            message: None,
            files: None,
            error: None,
        });
        let job_hash = hash.clone();
        spawn(
            ctx,
            &self.tx,
            move || -> Result<(String, Vec<GitFileChange>), String> {
                let message = git::git_show_commit_message_inner(repo.clone(), job_hash.clone())?;
                let files = git::git_show_commit_files_inner(repo, job_hash)?;
                Ok((message, files))
            },
            move |result| Msg::Detail { hash, result },
        );
    }

    fn run(&mut self, ctx: &egui::Context, op: Op) {
        if self.busy {
            return;
        }
        let repo = self.repo().unwrap_or_default();
        let cwd = self.cwd.clone();
        self.busy = true;
        let success = op.success_key();
        let generation = self.generation;
        spawn(
            ctx,
            &self.tx,
            move || op.run(repo, cwd),
            move |result| Msg::OpDone {
                generation,
                success,
                result,
            },
        );
    }

    fn drain(&mut self, ctx: &egui::Context, lang: Lang, actions: &mut Vec<Action>) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Status { request, result } => {
                    if request != self.status_request {
                        continue;
                    }
                    self.status_in_flight = false;
                    self.loading = false;
                    match result {
                        Ok(status) => {
                            let repo_changed = self.status.as_ref().map(|s| &s.repo_root)
                                != Some(&status.repo_root);
                            self.trees = Group::ORDER
                                .iter()
                                .map(|g| (*g, build_tree(g.items(&status))))
                                .collect();
                            self.status = Some(status);
                            self.error = None;
                            if repo_changed
                                || (self.graph_open && self.graph.is_none() && !self.graph_loading)
                            {
                                self.load_graph(ctx);
                            }
                            // IncomingOutgoing refetches when ahead/behind move (its effect deps).
                            if self.io_open {
                                let now = self
                                    .status
                                    .as_ref()
                                    .map(|s| (s.ahead, s.behind))
                                    .unwrap_or_default();
                                if now != self.io_for {
                                    self.load_io(ctx);
                                }
                            }
                        }
                        Err(error) => {
                            self.status = None;
                            self.trees.clear();
                            self.error = Some(error_code(&error));
                        }
                    }
                }
                Msg::OpDone {
                    generation,
                    success,
                    result,
                } => {
                    if generation != self.generation {
                        continue;
                    }
                    self.busy = false;
                    match result {
                        Ok(()) => {
                            if let Some(key) = success {
                                actions.push(Action::Toast(tr(lang, key).to_string()));
                            }
                        }
                        Err(error) => actions.push(Action::Toast(format!(
                            "{}: {}",
                            tr(lang, "git.error.action"),
                            readable(&error)
                        ))),
                    }
                    self.refresh(ctx, true);
                    if self.graph.is_some() {
                        self.load_graph(ctx);
                    }
                }
                Msg::Graph { generation, result } => {
                    if generation != self.generation {
                        continue;
                    }
                    self.graph_loading = false;
                    match result {
                        Ok(commits) => {
                            let (rows, lane_count) = build_graph_rows(&commits);
                            if let Some(detail) = &self.detail {
                                if !commits.iter().any(|c| c.hash == detail.hash) {
                                    self.detail = None;
                                }
                            }
                            self.graph = Some(Graph {
                                commits,
                                rows,
                                lane_count,
                            });
                            self.graph_error = None;
                        }
                        Err(error) => self.graph_error = Some(error),
                    }
                }
                Msg::Detail { hash, result } => {
                    if let Some(detail) = self.detail.as_mut().filter(|d| d.hash == hash) {
                        match result {
                            Ok((message, files)) => {
                                detail.message = Some(message);
                                detail.files = Some(files);
                            }
                            Err(error) => detail.error = Some(error),
                        }
                    }
                }
                Msg::Io { generation, result } => {
                    if generation != self.generation || !self.io_open {
                        continue;
                    }
                    match result {
                        Ok(io) => {
                            self.io = Some(io);
                            self.io_error = None;
                        }
                        Err(error) => self.io_error = Some(error),
                    }
                }
            }
        }
    }
}

/// Tooltip of the sidebar switcher button.
pub fn tab_label(lang: Lang) -> &'static str {
    tr(lang, "ui.sidebar.git")
}

/// The sidebar's Source Control section: panel header plus the panel, down to `bottom`.
pub fn sidebar_section(app: &mut AletheApp, ui: &mut Ui, bottom: f32) {
    let theme = app.theme.clone();
    let lang = app.lang();

    // `.explorerHeader`: 34px, uppercase label, bottom border.
    let (header, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
    let label: String = tr(lang, "ui.sidebar.sourceControl")
        .to_uppercase()
        .chars()
        .flat_map(|c| [c, '\u{200a}'])
        .collect();
    ui.painter().text(
        Pos2::new(header.left() + 12.0, header.center().y),
        Align2::LEFT_CENTER,
        label,
        bold(11.0),
        theme.fg_muted,
    );
    hline(
        ui,
        &theme,
        header.bottom() - 0.5,
        header.left(),
        header.right(),
    );

    // Source Control follows the selected project (GitTab), falling back to the most recent one
    // while Home is showing. Its cwd is the focused terminal's, else the project's default.
    let project_id = app
        .active_project_id()
        .map(str::to_string)
        .or_else(|| app.data.workspace.recent_project_ids.first().cloned());
    let target = project_id
        .as_deref()
        .and_then(|id| app.project(id))
        .map(|project| {
            let terminal = app
                .focused_terminal
                .as_deref()
                .and_then(|tid| project.terminals.iter().find(|t| t.id == tid));
            let cwd = terminal
                .map(|t| t.cwd.clone())
                .filter(|c| !c.trim().is_empty())
                .unwrap_or_else(|| project.cwd());
            Target {
                cwd,
                name: terminal
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| project.name.clone()),
            }
        });

    let rect = Rect::from_min_max(
        ui.cursor().min,
        Pos2::new(ui.max_rect().right(), bottom.max(ui.cursor().top() + 40.0)),
    );
    let mut body = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    body.set_clip_rect(rect.intersect(ui.clip_rect()));
    let mut actions = Vec::new();
    match &target {
        Some(target) => show(
            &mut body,
            &mut app.git_panel,
            &theme,
            lang,
            target,
            &mut actions,
        ),
        None => {
            if empty_state(&mut body, &theme, lang) {
                actions.push(Action::ShowModal(app.new_project_modal(None)));
            }
        }
    }
    ui.allocate_rect(rect, Sense::hover());
    for action in actions {
        app.act(action);
    }
}

fn empty_state(ui: &mut Ui, theme: &Theme, lang: Lang) -> bool {
    let mut clicked = false;
    ui.add_space(28.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(icons::SOURCE_CONTROL)
                .font(mono(18.0))
                .color(theme.fg_faint),
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new(tr(lang, "git.empty.noTerminal"))
                .font(bold(12.0))
                .color(theme.fg),
        );
        ui.add(
            egui::Label::new(
                RichText::new(tr(lang, "git.empty.noTerminalDesc"))
                    .font(sans(10.5))
                    .color(theme.fg_muted),
            )
            .wrap(),
        );
        ui.add_space(8.0);
        clicked =
            super::widgets::primary_button(ui, theme, tr(lang, "ui.sidebar.emptyAction")).clicked();
    });
    clicked
}

pub fn show(
    ui: &mut Ui,
    panel: &mut GitPanel,
    theme: &Theme,
    lang: Lang,
    target: &Target,
    actions: &mut Vec<Action>,
) {
    let ctx = ui.ctx().clone();
    if panel.cwd != target.cwd {
        panel.reset_for(&target.cwd);
        panel.refresh(&ctx, false);
    }
    panel.drain(&ctx, lang, actions);

    // Polling while visible + refresh on window focus, as GitControl does.
    let focused = ui.input(|i| {
        i.events
            .iter()
            .any(|e| matches!(e, egui::Event::WindowFocused(true)))
    });
    if !panel.status_in_flight {
        let since = panel
            .last_refresh
            .map(|t| t.elapsed())
            .unwrap_or(Duration::MAX);
        if since >= AUTO_REFRESH || (focused && since >= FOCUS_THROTTLE) {
            panel.refresh(&ctx, true);
        } else {
            ctx.request_repaint_after(AUTO_REFRESH - since);
        }
    }

    if target.cwd.trim().is_empty() {
        message(
            ui,
            theme,
            tr(lang, "git.empty.noFolder"),
            Some(tr(lang, "git.empty.noFolderDesc")),
        );
        return;
    }
    if panel.status.is_none() {
        if panel.error.is_none() || panel.loading {
            message(ui, theme, tr(lang, "git.loading"), None);
        } else {
            error_state(ui, panel, theme, lang, &ctx);
        }
        dialogs(&ctx, panel, theme, lang);
        return;
    }

    repo_header(ui, panel, theme, lang, target, actions, &ctx);
    branch_row(ui, panel, theme, lang, &ctx);
    commit_box(ui, panel, theme, lang, &ctx, actions);

    // `.groups` takes what the graph leaves; the graph keeps up to 380px at the bottom.
    let remaining = ui.available_height();
    let graph_height = if panel.graph_open {
        (GRAPH_MAX_HEIGHT + 70.0).min(remaining * 0.55)
    } else {
        36.0
    };
    egui::ScrollArea::vertical()
        .id_salt("git-groups")
        .max_height((remaining - graph_height).max(60.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            groups(ui, panel, theme, lang, &ctx, actions);
            incoming_outgoing(ui, panel, theme, lang, &ctx);
        });
    graph_section(ui, panel, theme, lang, &ctx);
    dialogs(&ctx, panel, theme, lang);
}

// ---------------------------------------------------------------- painting helpers

fn hline(ui: &Ui, theme: &Theme, y: f32, left: f32, right: f32) {
    ui.painter()
        .hline(left..=right, y, Stroke::new(1.0, theme.border));
}

/// Single-line galley truncated with an ellipsis (`text-overflow: ellipsis`).
fn trunc(ui: &Ui, text: &str, font: FontId, color: Color32, max_width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = TextWrapping {
        max_width: max_width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.painter().layout_job(job)
}

/// Small square icon button drawn at `rect` (`.iconButton` / `.fileActions button`).
fn mini_button(
    ui: &mut Ui,
    theme: &Theme,
    rect: Rect,
    id: Id,
    glyph: &str,
    tooltip: &str,
    enabled: bool,
    size: f32,
) -> bool {
    let response = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hovered = enabled && response.hovered();
    if hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), theme.panel_hover);
    }
    let color = if !enabled {
        theme.fg_muted.gamma_multiply(0.4)
    } else if hovered {
        theme.fg
    } else {
        theme.fg_muted
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        mono(size),
        color,
    );
    let clicked = enabled && response.clicked();
    if !tooltip.is_empty() {
        response.on_hover_text(tooltip);
    }
    clicked
}

fn message(ui: &mut Ui, theme: &Theme, title: &str, description: Option<&str>) {
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width(240.0);
        ui.label(
            RichText::new(icons::SOURCE_CONTROL)
                .font(mono(22.0))
                .color(theme.fg_faint),
        );
        ui.add_space(6.0);
        ui.label(RichText::new(title).font(bold(12.0)).color(theme.fg));
        if let Some(description) = description {
            ui.add(
                egui::Label::new(
                    RichText::new(description)
                        .font(sans(10.5))
                        .color(theme.fg_muted),
                )
                .wrap(),
            );
        }
    });
}

/// `.retry` / `.retryPrimary` full-width buttons of the error state.
fn retry_button(
    ui: &mut Ui,
    theme: &Theme,
    glyph: &str,
    label: &str,
    primary: bool,
    enabled: bool,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 30.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hovered = enabled && response.hovered();
    let (fill, fg) = if primary {
        (
            if hovered {
                theme.accent_strong
            } else {
                theme.accent
            },
            theme.accent_on,
        )
    } else {
        (
            if hovered {
                theme.panel_hover
            } else {
                theme.panel
            },
            theme.fg,
        )
    };
    ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
    if !primary {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.0, theme.border),
            StrokeKind::Inside,
        );
    }
    let galley = ui
        .painter()
        .layout_no_wrap(format!("{glyph}  {label}"), sans(11.0), fg);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, fg);
    enabled && response.clicked()
}

fn error_state(ui: &mut Ui, panel: &mut GitPanel, theme: &Theme, lang: Lang, ctx: &egui::Context) {
    let error = panel.error.clone().unwrap_or_default();
    let is_not_repo = error == "not_a_git_repository";
    let title = ERROR_KEYS
        .iter()
        .find(|(code, _)| *code == error)
        .map(|(_, key)| tr(lang, key))
        .unwrap_or(tr(lang, "git.error.generic"));
    let description = if is_not_repo {
        Some(tr(lang, "git.initOffer.body").to_string())
    } else {
        error
            .strip_prefix("git_command_failed:")
            .map(str::to_string)
    };
    message(ui, theme, title, description.as_deref());
    ui.add_space(10.0);
    egui::Frame::new()
        .inner_margin(Margin::symmetric(18, 0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            if is_not_repo {
                let label = if panel.busy {
                    tr(lang, "git.initOffer.busy")
                } else {
                    tr(lang, "git.initOffer.button")
                };
                if retry_button(ui, theme, icons::SOURCE_CONTROL, label, true, !panel.busy) {
                    panel.run(ctx, Op::Init);
                }
            }
            if retry_button(ui, theme, REFRESH, tr(lang, "git.refresh"), false, true) {
                panel.refresh(ctx, false);
            }
        });
}

// Codicons used only here (CaskaydiaCove Nerd Font carries the codicon set at its codepoints).
const REFRESH: &str = "\u{eb37}";
const SYNC: &str = "\u{ea77}";
const ARROW_DOWN: &str = "\u{ea9a}";
const ARROW_UP: &str = "\u{eaa1}";
const ARROW_LEFT: &str = "\u{ea9b}";
const REMOVE: &str = "\u{eb3b}";
const DISCARD: &str = "\u{eae2}";
const GIT_COMMIT: &str = "\u{eafc}";
const GIT_COMPARE: &str = "\u{eafd}";
const COPY: &str = "\u{ebcc}";
const GIT_BRANCH_CREATE: &str = "\u{ea68}";
const UNDO: &str = "\u{eb38}";

// ---------------------------------------------------------------- header, branch, commit box

fn repo_header(
    ui: &mut Ui,
    panel: &mut GitPanel,
    theme: &Theme,
    lang: Lang,
    target: &Target,
    actions: &mut Vec<Action>,
    ctx: &egui::Context,
) {
    let Some(status) = &panel.status else { return };
    let repo_root = status.repo_root.clone();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.0), Sense::hover());
    let text_right = rect.right() - 10.0 - 24.0 * 2.0 - 8.0 * 2.0;
    let text_left = rect.left() + 12.0;
    let name = trunc(
        ui,
        &target.name,
        bold(11.0),
        theme.fg,
        text_right - text_left,
    );
    let path = trunc(
        ui,
        &repo_root,
        sans(9.5),
        theme.fg_faint,
        text_right - text_left,
    );
    let total = name.size().y + 2.0 + path.size().y;
    let top = rect.center().y - total / 2.0;
    ui.painter()
        .galley(Pos2::new(text_left, top), name.clone(), theme.fg);
    ui.painter().galley(
        Pos2::new(text_left, top + name.size().y + 2.0),
        path,
        theme.fg_faint,
    );
    response.on_hover_text(&repo_root);

    let refresh = Rect::from_center_size(
        Pos2::new(rect.right() - 10.0 - 12.0, rect.center().y),
        Vec2::splat(24.0),
    );
    let reveal = refresh.translate(Vec2::new(-32.0, 0.0));
    if mini_button(
        ui,
        theme,
        reveal,
        Id::unique("git-reveal"),
        icons::FOLDER_OPENED,
        tr(lang, "files.revealFolder"),
        true,
        13.0,
    ) {
        actions.push(Action::OpenFolder(repo_root));
    }
    let idle = !panel.loading && !panel.busy;
    if mini_button(
        ui,
        theme,
        refresh,
        Id::unique("git-refresh"),
        REFRESH,
        tr(lang, "git.refresh"),
        idle,
        13.0,
    ) {
        panel.refresh(ctx, false);
    }
    hline(ui, theme, rect.bottom() - 0.5, rect.left(), rect.right());
}

fn sync_title(lang: Lang, status: &GitRepositoryStatus) -> String {
    tr(lang, "git.sync.title")
        .replace("{ahead}", &status.ahead.to_string())
        .replace("{behind}", &status.behind.to_string())
}

fn branch_row(ui: &mut Ui, panel: &mut GitPanel, theme: &Theme, lang: Lang, ctx: &egui::Context) {
    let Some(status) = &panel.status else { return };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
    let font = mono(10.0);

    // Sync button (`.syncButton`), right-aligned: refresh glyph + ↓behind ↑ahead.
    let counts = format!(
        "{SYNC}  {ARROW_DOWN}{}  {ARROW_UP}{}",
        status.behind, status.ahead
    );
    let counts_galley = ui
        .painter()
        .layout_no_wrap(counts, font.clone(), theme.fg_muted);
    let sync_rect = Rect::from_center_size(
        Pos2::new(
            rect.right() - 8.0 - (counts_galley.size().x + 14.0) / 2.0,
            rect.center().y,
        ),
        Vec2::new(counts_galley.size().x + 14.0, 22.0),
    );
    let enabled = !panel.busy && !status.detached;
    let response = ui.interact(
        sync_rect,
        Id::unique("git-sync-mini"),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if enabled && response.hovered() {
        ui.painter()
            .rect_filled(sync_rect, CornerRadius::same(4), theme.panel_hover);
    }
    let color = if !enabled {
        theme.fg_muted.gamma_multiply(0.4)
    } else if response.hovered() {
        theme.fg
    } else {
        theme.fg_muted
    };
    ui.painter().galley(
        sync_rect.center() - counts_galley.size() / 2.0,
        counts_galley,
        color,
    );
    let sync_clicked = enabled && response.clicked();
    response.on_hover_text(sync_title(lang, status));

    let mut x = rect.left() + 12.0;
    let glyph = ui.painter().layout_no_wrap(
        icons::SOURCE_CONTROL.to_string(),
        mono(12.0),
        theme.fg_muted,
    );
    ui.painter().galley(
        Pos2::new(x, rect.center().y - glyph.size().y / 2.0),
        glyph.clone(),
        theme.fg_muted,
    );
    x += glyph.size().x + 6.0;
    let detached = status.detached.then(|| {
        ui.painter().layout_no_wrap(
            tr(lang, "git.detached").to_string(),
            font.clone(),
            theme.fg_faint,
        )
    });
    let reserved = detached.as_ref().map(|g| g.size().x + 6.0).unwrap_or(0.0);
    let branch = trunc(
        ui,
        &status.branch,
        font,
        theme.fg,
        sync_rect.left() - 6.0 - reserved - x,
    );
    ui.painter().galley(
        Pos2::new(x, rect.center().y - branch.size().y / 2.0),
        branch.clone(),
        theme.fg,
    );
    if let Some(detached) = detached {
        let at = Pos2::new(
            x + branch.size().x + 6.0,
            rect.center().y - detached.size().y / 2.0,
        );
        ui.painter().galley(at, detached, theme.fg_faint);
    }
    hline(ui, theme, rect.bottom() - 0.5, rect.left(), rect.right());

    if sync_clicked {
        let pull = status.behind > 0;
        panel.run(ctx, Op::Sync { pull });
    }
}

fn stageable(status: &GitRepositoryStatus) -> Vec<String> {
    unique_paths(
        status
            .changes
            .iter()
            .chain(&status.untracked)
            .chain(&status.conflicts),
    )
}

fn unique_paths<'a>(items: impl IntoIterator<Item = &'a GitFileChange>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|c| seen.insert(c.path.as_str()))
        .map(|c| c.path.clone())
        .collect()
}

fn commit(panel: &mut GitPanel, lang: Lang, ctx: &egui::Context, actions: &mut Vec<Action>) {
    let Some(status) = &panel.status else { return };
    let message = panel.message.trim().to_string();
    if message.is_empty() || panel.busy {
        return;
    }
    if !status.conflicts.is_empty() {
        actions.push(Action::Toast(tr(lang, "git.error.conflicts").to_string()));
        return;
    }
    if status.staged.is_empty() {
        let all = stageable(status);
        if all.is_empty() {
            return;
        }
        panel.dialog = Some(Dialog::Confirm {
            text: tr(lang, "git.confirm.stageAllCommit").to_string(),
            op: Op::Commit {
                message,
                stage_first: all,
            },
        });
        return;
    }
    panel.message.clear();
    panel.run(
        ctx,
        Op::Commit {
            message,
            stage_first: Vec::new(),
        },
    );
}

fn commit_box(
    ui: &mut Ui,
    panel: &mut GitPanel,
    theme: &Theme,
    lang: Lang,
    ctx: &egui::Context,
    actions: &mut Vec<Action>,
) {
    let Some(status) = &panel.status else { return };
    let total = status.staged.len()
        + status.changes.len()
        + status.untracked.len()
        + status.conflicts.len();
    let detached = status.detached;
    let title = sync_title(lang, status);
    let behind = status.behind > 0;
    let top = ui.cursor().top();
    egui::Frame::new()
        .inner_margin(Margin::symmetric(10, 9))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let editor_id = Id::unique("git-commit-message");
            let focused = ui.memory(|m| m.has_focus(editor_id));
            // Ctrl+Enter commits; consumed before the multiline editor turns it into a newline.
            if focused && ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Enter)) {
                commit(panel, lang, ctx, actions);
            }
            egui::Frame::new()
                .fill(theme.bg_elevated)
                .stroke(Stroke::new(
                    1.0,
                    if focused { theme.accent } else { theme.border },
                ))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::symmetric(8, 7))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut panel.message)
                            .id(editor_id)
                            .hint_text(
                                RichText::new(tr(lang, "git.commit.placeholder"))
                                    .color(theme.fg_faint),
                            )
                            .frame(egui::Frame::NONE)
                            .font(sans(11.0))
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    );
                });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let sync_label = format!("{SYNC}  {}", tr(lang, "git.sync.action"));
                let sync_galley = ui
                    .painter()
                    .layout_no_wrap(sync_label, sans(11.0), theme.fg);
                let sync_width = sync_galley.size().x + 24.0;
                let commit_width = (ui.available_width() - sync_width - 6.0).max(60.0);

                let can_commit = !panel.message.trim().is_empty() && !panel.busy && total > 0;
                let label = if panel.busy {
                    tr(lang, "git.commit.busy")
                } else {
                    tr(lang, "git.commit.action")
                };
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(commit_width, 30.0),
                    if can_commit {
                        Sense::click()
                    } else {
                        Sense::hover()
                    },
                );
                let fill = if can_commit && response.hovered() {
                    theme.accent_strong
                } else {
                    theme.accent
                };
                let alpha = if can_commit { 1.0 } else { 0.42 };
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(4), fill.gamma_multiply(alpha));
                let galley = ui.painter().layout_no_wrap(
                    format!("{}  {label}", icons::CHECK),
                    bold(11.0),
                    theme.accent_on,
                );
                ui.painter().galley(
                    rect.center() - galley.size() / 2.0,
                    galley,
                    theme.accent_on.gamma_multiply(alpha),
                );
                if can_commit && response.clicked() {
                    commit(panel, lang, ctx, actions);
                }

                let can_sync = !panel.busy && !detached;
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(sync_width, 30.0),
                    if can_sync {
                        Sense::click()
                    } else {
                        Sense::hover()
                    },
                );
                let hovered = can_sync && response.hovered();
                let alpha = if can_sync { 1.0 } else { 0.42 };
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::same(4),
                    if hovered {
                        theme.panel_hover
                    } else {
                        theme.panel
                    },
                );
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(4),
                    Stroke::new(1.0, if hovered { theme.accent } else { theme.border }),
                    StrokeKind::Inside,
                );
                ui.painter().galley(
                    rect.center() - sync_galley.size() / 2.0,
                    sync_galley,
                    theme.fg.gamma_multiply(alpha),
                );
                if can_sync && response.clicked() {
                    panel.run(ctx, Op::Sync { pull: behind });
                }
                response.on_hover_text(title);
            });
        });
    let rect = ui.min_rect();
    hline(
        ui,
        theme,
        ui.cursor().top() - 0.5,
        rect.left(),
        rect.right().max(ui.max_rect().right()),
    );
    let _ = top;
}

// ---------------------------------------------------------------- change groups

fn build_tree(items: &[GitFileChange]) -> Vec<Node> {
    let mut root: Vec<Node> = Vec::new();
    for (index, change) in items.iter().enumerate() {
        let mut parts: Vec<&str> = change.path.split('/').collect();
        let file_name = parts.pop().unwrap_or(&change.path).to_string();
        let mut cursor = &mut root;
        let mut acc = String::new();
        for part in parts {
            if acc.is_empty() {
                acc = part.to_string();
            } else {
                acc = format!("{acc}/{part}");
            }
            let position = cursor
                .iter()
                .position(|n| matches!(n, Node::Dir { name, .. } if name == part));
            let position = match position {
                Some(p) => p,
                None => {
                    cursor.push(Node::Dir {
                        name: part.to_string(),
                        path: acc.clone(),
                        children: Vec::new(),
                    });
                    cursor.len() - 1
                }
            };
            cursor = match &mut cursor[position] {
                Node::Dir { children, .. } => children,
                Node::File { .. } => unreachable!(),
            };
        }
        cursor.push(Node::File {
            name: file_name,
            index,
        });
    }
    let mut nodes: Vec<Node> = root.into_iter().map(compress).collect();
    sort_nodes(&mut nodes);
    nodes
}

/// Folds single-child directory chains into `a/b/c`.
fn compress(node: Node) -> Node {
    match node {
        Node::File { .. } => node,
        Node::Dir {
            mut name,
            mut path,
            mut children,
        } => {
            while children.len() == 1 && matches!(children[0], Node::Dir { .. }) {
                if let Some(Node::Dir {
                    name: child_name,
                    path: child_path,
                    children: grand,
                }) = children.pop()
                {
                    name = format!("{name}/{child_name}");
                    path = child_path;
                    children = grand;
                }
            }
            let mut children: Vec<Node> = children.into_iter().map(compress).collect();
            sort_nodes(&mut children);
            Node::Dir {
                name,
                path,
                children,
            }
        }
    }
}

fn sort_nodes(nodes: &mut [Node]) {
    nodes.sort_by(|a, b| match (a, b) {
        (Node::Dir { .. }, Node::File { .. }) => std::cmp::Ordering::Less,
        (Node::File { .. }, Node::Dir { .. }) => std::cmp::Ordering::Greater,
        (Node::Dir { name: x, .. }, Node::Dir { name: y, .. })
        | (Node::File { name: x, .. }, Node::File { name: y, .. }) => {
            x.to_lowercase().cmp(&y.to_lowercase())
        }
    });
}

fn collect_paths(node: &Node, items: &[GitFileChange], out: &mut Vec<String>) {
    match node {
        Node::File { index, .. } => out.push(items[*index].path.clone()),
        Node::Dir { children, .. } => children.iter().for_each(|c| collect_paths(c, items, out)),
    }
}

fn status_char(group: Group, status: &str) -> String {
    match group {
        Group::Untracked => "U".into(),
        Group::Conflicts => "!".into(),
        _ => status
            .trim()
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase().to_string())
            .unwrap_or_else(|| "•".into()),
    }
}

fn status_color(theme: &Theme, group: Group, status: &str) -> Color32 {
    match group {
        Group::Untracked => theme.working,
        Group::Conflicts => theme.offline,
        _ => match status.trim().chars().next().map(|c| c.to_ascii_uppercase()) {
            Some('A') => theme.working,
            Some('D') => theme.offline,
            Some('R') | Some('C') => theme.agent_codex,
            Some('M') => theme.waiting,
            _ => theme.fg_muted,
        },
    }
}

/// What a click in the groups asked for; applied after drawing (the tree borrows the status).
enum GroupAction {
    Primary(Group, Vec<String>),
    Discard(Group, Vec<String>),
    Reveal(String),
}

fn groups(
    ui: &mut Ui,
    panel: &mut GitPanel,
    theme: &Theme,
    lang: Lang,
    ctx: &egui::Context,
    actions: &mut Vec<Action>,
) {
    let Some(status) = &panel.status else { return };
    let mut requested: Option<GroupAction> = None;
    let mut toggled: Option<String> = None;
    let busy = panel.busy;
    let total = status.staged.len()
        + status.changes.len()
        + status.untracked.len()
        + status.conflicts.len();

    for (group, tree) in &panel.trees {
        let items = group.items(status);
        if items.is_empty() {
            continue;
        }
        let group_key = format!("g:{}", group.key());
        let open = !panel.collapsed.contains(&group_key);
        // `.groupHeader`: 32px, chevron + uppercase label + count pill, actions at the right.
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click());
        let staged = *group == Group::Staged;
        let primary_tip = if staged {
            tr(lang, "git.unstageAll")
        } else {
            tr(lang, "git.stageAll")
        };
        let primary_rect = Rect::from_center_size(
            Pos2::new(rect.right() - 6.0 - 11.0, rect.center().y),
            Vec2::splat(22.0),
        );
        let discard_rect = primary_rect.translate(Vec2::new(-24.0, 0.0));
        let mut x = rect.left() + 8.0;
        let chevron = if open {
            icons::CHEVRON_DOWN
        } else {
            icons::CHEVRON_RIGHT
        };
        ui.painter().text(
            Pos2::new(x, rect.center().y),
            Align2::LEFT_CENTER,
            chevron,
            mono(12.0),
            theme.fg,
        );
        x += 18.0;
        let label_limit = discard_rect.left() - 32.0 - x;
        let label = trunc(
            ui,
            &tr(lang, group.label()).to_uppercase(),
            bold(10.0),
            theme.fg,
            label_limit,
        );
        ui.painter().galley(
            Pos2::new(x, rect.center().y - label.size().y / 2.0),
            label.clone(),
            theme.fg,
        );
        x += label.size().x + 6.0;
        let count = ui
            .painter()
            .layout_no_wrap(items.len().to_string(), sans(9.0), theme.fg_muted);
        let pill = Rect::from_min_size(
            Pos2::new(x, rect.center().y - 9.0),
            Vec2::new((count.size().x + 10.0).max(18.0), 18.0),
        );
        ui.painter()
            .rect_filled(pill, CornerRadius::same(9), theme.panel);
        ui.painter()
            .galley(pill.center() - count.size() / 2.0, count, theme.fg_muted);

        let paths = unique_paths(items);
        if group.discardable()
            && mini_button(
                ui,
                theme,
                discard_rect,
                Id::unique(("git-gdiscard", group.key())),
                DISCARD,
                tr(lang, "git.discardAll"),
                !busy,
                13.0,
            )
        {
            requested = Some(GroupAction::Discard(*group, paths.clone()));
        }
        let glyph = if staged { REMOVE } else { icons::ADD };
        if mini_button(
            ui,
            theme,
            primary_rect,
            Id::unique(("git-gprimary", group.key())),
            glyph,
            primary_tip,
            !busy,
            14.0,
        ) {
            requested = Some(GroupAction::Primary(*group, paths));
        } else if response.clicked() && requested.is_none() {
            toggled = Some(group_key);
        }

        if open {
            egui::Frame::new()
                .inner_margin(Margin {
                    left: 4,
                    right: 4,
                    top: 0,
                    bottom: 4,
                })
                .show(ui, |ui| {
                    for node in tree {
                        tree_node(
                            ui,
                            theme,
                            lang,
                            *group,
                            node,
                            items,
                            &status.repo_root,
                            0,
                            busy,
                            &panel.collapsed,
                            &mut requested,
                            &mut toggled,
                        );
                    }
                });
        }
        let r = ui.min_rect();
        hline(ui, theme, ui.cursor().top() - 0.5, r.left(), r.right());
    }

    if total == 0 {
        // `.clean`
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(icons::CHECK)
                    .font(mono(18.0))
                    .color(theme.working),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(tr(lang, "git.clean"))
                    .font(bold(12.0))
                    .color(theme.fg),
            );
            ui.label(
                RichText::new(tr(lang, "git.cleanDesc"))
                    .font(sans(10.0))
                    .color(theme.fg_muted),
            );
        });
        ui.add_space(24.0);
    }

    if let Some(key) = toggled {
        if !panel.collapsed.remove(&key) {
            panel.collapsed.insert(key);
        }
    }
    match requested {
        Some(GroupAction::Primary(group, paths)) => {
            let op = if group == Group::Staged {
                Op::Unstage(paths)
            } else {
                Op::Stage(paths)
            };
            panel.run(ctx, op);
        }
        Some(GroupAction::Discard(group, paths)) => {
            panel.dialog = Some(Dialog::Confirm {
                text: tr(lang, "git.confirm.discard").replace("{count}", &paths.len().to_string()),
                op: Op::Discard {
                    paths,
                    untracked: group == Group::Untracked,
                },
            });
        }
        Some(GroupAction::Reveal(path)) => actions.push(Action::OpenFolder(path)),
        None => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn tree_node(
    ui: &mut Ui,
    theme: &Theme,
    lang: Lang,
    group: Group,
    node: &Node,
    items: &[GitFileChange],
    repo_root: &str,
    depth: usize,
    busy: bool,
    collapsed: &HashSet<String>,
    requested: &mut Option<GroupAction>,
    toggled: &mut Option<String>,
) {
    let staged = group == Group::Staged;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), theme.panel_hover);
    }
    let left = rect.left() + 4.0 + 8.0 + depth as f32 * 12.0;
    let primary_tip = if staged {
        tr(lang, "git.unstage")
    } else {
        tr(lang, "git.stage")
    };
    let primary_glyph = if staged { REMOVE } else { icons::ADD };

    match node {
        Node::File { name, index } => {
            let change = &items[*index];
            let id = Id::unique(("git-file", group.key(), &change.path));
            // Actions replace the status letter while hovered (`.file:hover .status { display:none }`).
            let mut slots = vec![(icons::FOLDER_OPENED, tr(lang, "files.reveal"), 0u8)];
            if group.discardable() {
                slots.push((DISCARD, tr(lang, "git.discard"), 1));
            }
            slots.push((primary_glyph, primary_tip, 2));
            let actions_width = slots.len() as f32 * 21.0;
            let text_right = if hovered {
                rect.right() - 4.0 - actions_width
            } else {
                rect.right() - 4.0 - 18.0
            };
            let galley = trunc(ui, name, sans(11.0), theme.fg, text_right - left);
            ui.painter().galley(
                Pos2::new(left, rect.center().y - galley.size().y / 2.0),
                galley,
                theme.fg,
            );
            if hovered {
                let mut x = rect.right() - 4.0 - actions_width;
                for (glyph, tip, kind) in slots {
                    let button = Rect::from_min_size(
                        Pos2::new(x, rect.center().y - 10.0),
                        Vec2::splat(20.0),
                    );
                    let enabled = kind == 0 || !busy;
                    if mini_button(ui, theme, button, id.with(kind), glyph, tip, enabled, 12.0) {
                        let path = change.path.clone();
                        *requested = Some(match kind {
                            0 => GroupAction::Reveal(absolute_repo_path(repo_root, &path, true)),
                            1 => GroupAction::Discard(group, vec![path]),
                            _ => GroupAction::Primary(group, vec![path]),
                        });
                    }
                    x += 21.0;
                }
            } else {
                ui.painter().text(
                    Pos2::new(rect.right() - 4.0 - 9.0, rect.center().y),
                    Align2::CENTER_CENTER,
                    status_char(group, &change.status),
                    FontId::new(11.0, egui::FontFamily::Name("mono-bold".into())),
                    status_color(theme, group, &change.status),
                );
            }
            let tip = match &change.original_path {
                Some(original) => format!("{original} → {}", change.path),
                None => change.path.clone(),
            };
            response.on_hover_text(tip);
        }
        Node::Dir {
            name,
            path,
            children,
        } => {
            let key = format!("d:{}:{path}", group.key());
            let open = !collapsed.contains(&key);
            let mut slots = Vec::new();
            if group.discardable() {
                slots.push((DISCARD, tr(lang, "git.discardAll"), 1u8));
            }
            slots.push((
                primary_glyph,
                if staged {
                    tr(lang, "git.unstageAll")
                } else {
                    tr(lang, "git.stageAll")
                },
                2,
            ));
            let actions_width = slots.len() as f32 * 21.0;
            let chevron = if open {
                icons::CHEVRON_DOWN
            } else {
                icons::CHEVRON_RIGHT
            };
            ui.painter().text(
                Pos2::new(left, rect.center().y),
                Align2::LEFT_CENTER,
                chevron,
                mono(11.0),
                theme.fg_faint,
            );
            ui.painter().text(
                Pos2::new(left + 15.0, rect.center().y),
                Align2::LEFT_CENTER,
                icons::FOLDER,
                mono(12.0),
                theme.fg_muted,
            );
            let text_left = left + 32.0;
            let text_right = rect.right() - 4.0 - if hovered { actions_width } else { 0.0 };
            let galley = trunc(ui, name, sans(11.0), theme.fg, text_right - text_left);
            ui.painter().galley(
                Pos2::new(text_left, rect.center().y - galley.size().y / 2.0),
                galley,
                theme.fg,
            );
            let mut clicked_action = false;
            if hovered {
                let mut x = rect.right() - 4.0 - actions_width;
                for (glyph, tip, kind) in slots {
                    let button = Rect::from_min_size(
                        Pos2::new(x, rect.center().y - 10.0),
                        Vec2::splat(20.0),
                    );
                    if mini_button(
                        ui,
                        theme,
                        button,
                        Id::unique(("git-dir", &key, kind)),
                        glyph,
                        tip,
                        !busy,
                        12.0,
                    ) {
                        let mut paths = Vec::new();
                        collect_paths(node, items, &mut paths);
                        *requested = Some(if kind == 1 {
                            GroupAction::Discard(group, paths)
                        } else {
                            GroupAction::Primary(group, paths)
                        });
                        clicked_action = true;
                    }
                    x += 21.0;
                }
            }
            if response.clicked() && !clicked_action {
                *toggled = Some(key.clone());
            }
            if open {
                for child in children {
                    tree_node(
                        ui,
                        theme,
                        lang,
                        group,
                        child,
                        items,
                        repo_root,
                        depth + 1,
                        busy,
                        collapsed,
                        requested,
                        toggled,
                    );
                }
            }
        }
    }
}

/// `absoluteRepoPath`; with `parent`, the containing folder (what Explorer can open).
fn absolute_repo_path(repo_root: &str, relative: &str, parent: bool) -> String {
    let separator = if repo_root.contains('\\') { '\\' } else { '/' };
    let mut path = format!(
        "{}{separator}{}",
        repo_root.trim_end_matches(['\\', '/']),
        relative
            .trim_start_matches(['\\', '/'])
            .replace(['\\', '/'], &separator.to_string())
    );
    if parent {
        if let Some(cut) = path.rfind(separator) {
            path.truncate(cut);
        }
    }
    path
}

// ---------------------------------------------------------------- incoming / outgoing

fn section_header(
    ui: &mut Ui,
    theme: &Theme,
    open: bool,
    glyph: &str,
    title: &str,
    extra: Option<String>,
) -> (egui::Response, Rect) {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
    let color = if response.hovered() {
        theme.fg
    } else {
        theme.fg_muted
    };
    let mut x = rect.left() + 6.0;
    let chevron = if open {
        icons::CHEVRON_DOWN
    } else {
        icons::CHEVRON_RIGHT
    };
    ui.painter().text(
        Pos2::new(x, rect.center().y),
        Align2::LEFT_CENTER,
        chevron,
        mono(12.0),
        color,
    );
    x += 18.0;
    ui.painter().text(
        Pos2::new(x, rect.center().y),
        Align2::LEFT_CENTER,
        glyph,
        mono(12.0),
        color,
    );
    x += 18.0;
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_string(), bold(12.0), color);
    ui.painter().galley(
        Pos2::new(x, rect.center().y - galley.size().y / 2.0),
        galley.clone(),
        color,
    );
    if let Some(extra) = extra {
        x += galley.size().x + 6.0;
        ui.painter().text(
            Pos2::new(x, rect.center().y),
            Align2::LEFT_CENTER,
            extra,
            sans(11.0),
            color.gamma_multiply(0.6),
        );
    }
    (response, rect)
}

fn incoming_outgoing(
    ui: &mut Ui,
    panel: &mut GitPanel,
    theme: &Theme,
    lang: Lang,
    ctx: &egui::Context,
) {
    let Some(status) = &panel.status else { return };
    if status.ahead == 0 && status.behind == 0 {
        return;
    }
    ui.add_space(4.0);
    let (response, _) = section_header(
        ui,
        theme,
        panel.io_open,
        GIT_COMPARE,
        tr(lang, "git.incomingOutgoing.title"),
        None,
    );
    if response.clicked() {
        panel.io_open = !panel.io_open;
        if panel.io_open {
            panel.load_io(ctx);
        } else {
            // Only fetched while open; nothing is kept once it closes.
            panel.io = None;
            panel.io_error = None;
        }
    }
    if !panel.io_open {
        return;
    }
    egui::Frame::new()
        .inner_margin(Margin {
            left: 12,
            right: 8,
            top: 2,
            bottom: 8,
        })
        .show(ui, |ui| {
            if let Some(error) = &panel.io_error {
                ui.label(RichText::new(error).font(sans(11.0)).color(theme.offline));
            }
            match &panel.io {
                None if panel.io_error.is_none() => {
                    ui.label(
                        RichText::new(tr(lang, "git.incomingOutgoing.loading"))
                            .font(sans(11.0))
                            .color(theme.fg_faint),
                    );
                }
                None => {}
                Some(io) if io.incoming.is_empty() && io.outgoing.is_empty() => {
                    ui.label(
                        RichText::new(tr(lang, "git.incomingOutgoing.empty"))
                            .font(sans(11.0))
                            .color(theme.fg_faint),
                    );
                }
                Some(io) => {
                    for (key, items) in [
                        ("git.incomingOutgoing.incoming", &io.incoming),
                        ("git.incomingOutgoing.outgoing", &io.outgoing),
                    ] {
                        if items.is_empty() {
                            continue;
                        }
                        let label = tr(lang, key).replace("{count}", &items.len().to_string());
                        ui.label(RichText::new(label).font(bold(10.5)).color(theme.fg_muted));
                        for item in items.iter() {
                            file_line(ui, theme, &item.status, &item.path);
                        }
                        ui.add_space(4.0);
                    }
                }
            }
        });
}

/// `status letter + path` row used by incoming/outgoing and the commit detail.
fn file_line(ui: &mut Ui, theme: &Theme, status: &str, path: &str) {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::hover());
    let letter = status
        .trim()
        .chars()
        .next()
        .map(|c| c.to_ascii_uppercase().to_string())
        .unwrap_or_else(|| "•".into());
    ui.painter().text(
        Pos2::new(rect.left() + 7.0, rect.center().y),
        Align2::CENTER_CENTER,
        letter,
        FontId::new(10.0, egui::FontFamily::Name("mono-bold".into())),
        theme.fg_faint,
    );
    let galley = trunc(ui, path, sans(11.5), theme.fg_muted, rect.width() - 22.0);
    ui.painter().galley(
        Pos2::new(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme.fg_muted,
    );
    response.on_hover_text(path);
}

// ---------------------------------------------------------------- commit graph

/// `buildGraphRows`: topological lane assignment with a color per lane identity. Lanes hold the
/// index of the commit they are waiting for, so the rows stay small.
fn build_graph_rows(commits: &[GitCommitEntry]) -> (Vec<GraphRow>, usize) {
    let index: HashMap<&str, u32> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c.hash.as_str(), i as u32))
        .collect();
    let branch_by_hash = branch_by_hash(commits);
    let mut assigned: HashMap<String, u8> = HashMap::new();
    let mut next_color = 0usize;
    let mut color_for = |identity: &str| -> u8 {
        if let Some(color) = assigned.get(identity) {
            return *color;
        }
        let color = (next_color % LANE_AGENTS.len()) as u8;
        next_color += 1;
        assigned.insert(identity.to_string(), color);
        color
    };
    let identity = |i: u32| -> String {
        let hash = commits[i as usize].hash.as_str();
        branch_by_hash
            .get(hash)
            .cloned()
            .unwrap_or_else(|| hash.to_string())
    };

    let mut lanes: Vec<Option<u32>> = Vec::new();
    let mut lane_colors: Vec<Option<u8>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    let mut lane_count = 1usize;

    fn free_lane(lanes: &mut Vec<Option<u32>>, colors: &mut Vec<Option<u8>>) -> usize {
        if let Some(free) = lanes.iter().position(Option::is_none) {
            return free;
        }
        lanes.push(None);
        colors.push(None);
        lanes.len() - 1
    }

    for (i, commit) in commits.iter().enumerate() {
        let i = i as u32;
        let before = lanes.clone();
        let colors_before = lane_colors.clone();

        let lane = match lanes.iter().position(|l| *l == Some(i)) {
            Some(lane) => lane,
            None => {
                let lane = free_lane(&mut lanes, &mut lane_colors);
                lane_colors[lane] = Some(color_for(&identity(i)));
                lane
            }
        };
        let color = lane_colors[lane].or_else(|| Some(color_for(&identity(i))));

        for slot in lanes.iter_mut() {
            if *slot == Some(i) {
                *slot = None;
            }
        }

        // Parents still ahead in the list (`remainingHashes`): index greater than this one.
        let pending = |hash: &str| index.get(hash).copied().filter(|p| *p > i);
        match commit.parents.first() {
            Some(first) => {
                lanes[lane] = pending(first);
                for parent in commit.parents.iter().skip(1) {
                    if let Some(p) = pending(parent) {
                        if !lanes.contains(&Some(p)) {
                            let free = free_lane(&mut lanes, &mut lane_colors);
                            lanes[free] = Some(p);
                            lane_colors[free] = Some(color_for(&identity(p)));
                        }
                    }
                }
            }
            None => lanes[lane] = None,
        }

        let last = i as usize == commits.len() - 1;
        let after = if last {
            vec![None; lanes.len()]
        } else {
            lanes.clone()
        };
        let colors_after = if last {
            vec![None; lane_colors.len()]
        } else {
            lane_colors.clone()
        };
        lane_count = lane_count.max(before.len()).max(after.len()).max(lane + 1);
        rows.push(GraphRow {
            lane,
            color,
            before,
            after,
            colors_before,
            colors_after,
            last,
        });
    }
    (rows, lane_count)
}

/// `buildBranchByHash`: local branch preferred over `origin/*`, tags ignored.
fn branch_by_hash(commits: &[GitCommitEntry]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for commit in commits {
        let mut best: Option<String> = None;
        let mut best_local = false;
        for raw in &commit.refs {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with("tag: ") {
                continue;
            }
            let name = trimmed
                .split_once(" -> ")
                .map(|(_, n)| n.trim())
                .unwrap_or(trimmed);
            if name.is_empty() {
                continue;
            }
            let local = !name.starts_with("origin/");
            let name = name.strip_prefix("origin/").unwrap_or(name);
            if best.is_none() || (local && !best_local) {
                best = Some(name.to_string());
                best_local = local;
            }
        }
        if let Some(best) = best {
            map.insert(commit.hash.clone(), best);
        }
    }
    map
}

fn lane_color(theme: &Theme, lane: usize, color: Option<u8>) -> Color32 {
    if lane == 0 {
        return theme.working;
    }
    match color {
        Some(c) => theme.agent_color(LANE_AGENTS[c as usize % LANE_AGENTS.len()]),
        None => theme.fg_faint,
    }
}

fn lane_stroke(lane: usize) -> f32 {
    if lane == 0 {
        3.0
    } else {
        2.0
    }
}

fn relative_time(lang: Lang, timestamp: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let minutes = ((now - timestamp) as f64 / 60.0).round() as i64;
    if minutes < 1 {
        return tr(lang, "git.graph.timeNow").to_string();
    }
    if minutes < 60 {
        return tr(lang, "git.graph.timeMinutes").replace("{count}", &minutes.to_string());
    }
    let hours = (minutes as f64 / 60.0).round() as i64;
    if hours < 24 {
        return tr(lang, "git.graph.timeHours").replace("{count}", &hours.to_string());
    }
    let days = (hours as f64 / 24.0).round() as i64;
    if days < 30 {
        return tr(lang, "git.graph.timeDays").replace("{count}", &days.to_string());
    }
    use chrono::TimeZone;
    let date = chrono::Local.timestamp_opt(timestamp, 0).single();
    let format = if lang == Lang::En {
        "%m/%d/%Y"
    } else {
        "%d/%m/%Y"
    };
    date.map(|d| d.format(format).to_string())
        .unwrap_or_default()
}

/// `RefBadges`: at most two badges plus a `+N` counter. Returns the width used.
fn ref_badges(
    ui: &Ui,
    theme: &Theme,
    refs: &[String],
    left: f32,
    center_y: f32,
    max_width: f32,
) -> f32 {
    let mut x = left;
    for raw in refs.iter().take(2) {
        let (fill, fg) = if raw.contains("HEAD") {
            (theme.agent_shell, theme.bg)
        } else if raw.contains("tag:") {
            (theme.waiting, theme.fg_muted)
        } else {
            (theme.accent_soft, theme.fg_muted)
        };
        let room = (left + max_width - x).min(110.0) - 10.0;
        if room < 16.0 {
            break;
        }
        let galley = trunc(ui, raw, sans(9.5), fg, room);
        let rect = Rect::from_min_size(
            Pos2::new(x, center_y - 8.0),
            Vec2::new(galley.size().x + 10.0, 16.0),
        );
        ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.0, theme.border),
            StrokeKind::Inside,
        );
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, fg);
        x = rect.right() + 3.0;
    }
    if refs.len() > 2 && x - left < max_width - 20.0 {
        let galley =
            ui.painter()
                .layout_no_wrap(format!("+{}", refs.len() - 2), bold(9.0), theme.accent);
        let rect = Rect::from_min_size(
            Pos2::new(x, center_y - 8.0),
            Vec2::new(galley.size().x + 8.0, 16.0),
        );
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), theme.accent_soft);
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.0, theme.accent_border),
            StrokeKind::Inside,
        );
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, theme.accent);
        x = rect.right() + 3.0;
    }
    x - left
}

/// One row of the graph (`GraphRowView`): pass-through lanes, the curves into and out of the
/// commit, its dot, then subject, ref badges and author · time.
fn paint_graph_row(
    ui: &Ui,
    theme: &Theme,
    lang: Lang,
    row: &GraphRow,
    commit: &GitCommitEntry,
    index: u32,
    rect: Rect,
    graph_width: f32,
    dim: bool,
) {
    let alpha = if dim { 0.35 } else { 1.0 };
    let painter = ui.painter_at(rect);
    let origin = rect.left_top() + Vec2::new(6.0, 0.0);
    let x_of = |lane: usize| origin.x + lane as f32 * LANE_WIDTH + LANE_WIDTH / 2.0;
    let cx = x_of(row.lane);
    let top = origin.y;
    let cy = top + ROW_HEIGHT / 2.0;
    let bottom = top + ROW_HEIGHT;
    let stroke = |lane: usize, color: Option<u8>| {
        Stroke::new(
            lane_stroke(lane),
            lane_color(theme, lane, color).gamma_multiply(alpha),
        )
    };
    let curve = |from: Pos2, c1: Pos2, c2: Pos2, to: Pos2, stroke: Stroke| {
        painter.add(CubicBezierShape::from_points_stroke(
            [from, c1, c2, to],
            false,
            Color32::TRANSPARENT,
            stroke,
        ));
    };

    // Top half: lines entering this row.
    for (l, slot) in row.before.iter().enumerate() {
        let Some(waiting) = slot else { continue };
        let s = stroke(l, row.colors_before.get(l).copied().flatten());
        let x = x_of(l);
        if l == row.lane {
            painter.line_segment([Pos2::new(cx, top), Pos2::new(cx, cy)], s);
        } else if *waiting == index {
            // Another lane converging into this commit.
            let mid = top + (cy - top) / 2.0;
            curve(
                Pos2::new(x, top),
                Pos2::new(x, mid),
                Pos2::new(cx, mid),
                Pos2::new(cx, cy),
                s,
            );
        } else {
            painter.line_segment([Pos2::new(x, top), Pos2::new(x, cy)], s);
        }
    }
    // Bottom half: lines leaving toward the parents.
    if !row.last {
        for (l, slot) in row.after.iter().enumerate() {
            let Some(parent) = slot else { continue };
            let s = stroke(l, row.colors_after.get(l).copied().flatten());
            let x = x_of(l);
            if l == row.lane {
                painter.line_segment([Pos2::new(cx, cy), Pos2::new(cx, bottom)], s);
            } else if row.before.get(l).copied().flatten() == Some(*parent) {
                painter.line_segment([Pos2::new(x, cy), Pos2::new(x, bottom)], s);
            } else {
                let mid = (cy + bottom) / 2.0;
                curve(
                    Pos2::new(cx, cy),
                    Pos2::new(cx, mid),
                    Pos2::new(x, mid),
                    Pos2::new(x, bottom),
                    s,
                );
            }
        }
    }
    let radius = if row.lane == 0 {
        DOT_RADIUS + 1.5
    } else {
        DOT_RADIUS
    };
    painter.circle(
        Pos2::new(cx, cy),
        radius,
        lane_color(theme, row.lane, row.color).gamma_multiply(alpha),
        Stroke::new(2.0, theme.bg),
    );

    // Text: meta pinned right, badges, subject takes the rest.
    let info_left = origin.x + graph_width + 10.0;
    let right = rect.right() - 6.0;
    let meta = format!(
        "{} · {}",
        commit.author_name,
        relative_time(lang, commit.timestamp)
    );
    let meta = trunc(
        ui,
        &meta,
        sans(10.0),
        theme.fg_faint.gamma_multiply(alpha),
        ((right - info_left) * 0.45).max(30.0),
    );
    let meta_left = right - meta.size().x;
    painter.galley(
        Pos2::new(meta_left, cy - meta.size().y / 2.0),
        meta,
        theme.fg_faint,
    );
    let available = meta_left - 6.0 - info_left;
    let subject_max = if commit.refs.is_empty() {
        available
    } else {
        (available * 0.6).max(40.0)
    };
    let subject = trunc(
        ui,
        &commit.subject,
        sans(11.0),
        theme.fg.gamma_multiply(alpha),
        subject_max,
    );
    let subject_width = subject.size().x;
    painter.galley(
        Pos2::new(info_left, cy - subject.size().y / 2.0),
        subject,
        theme.fg,
    );
    if !commit.refs.is_empty() {
        let badges_left = info_left + subject_width + 6.0;
        ref_badges(
            ui,
            theme,
            &commit.refs,
            badges_left,
            cy,
            (meta_left - 6.0 - badges_left).min(160.0),
        );
    }
}

fn graph_section(
    ui: &mut Ui,
    panel: &mut GitPanel,
    theme: &Theme,
    lang: Lang,
    ctx: &egui::Context,
) {
    let r = ui.max_rect();
    hline(ui, theme, ui.cursor().top(), r.left(), r.right());
    ui.add_space(8.0);
    let count = panel
        .graph
        .as_ref()
        .map(|g| format!("({})", g.commits.len()));
    let (response, header) = section_header(
        ui,
        theme,
        panel.graph_open,
        GIT_COMMIT,
        tr(lang, "git.graph.title"),
        count,
    );
    if panel.graph_open {
        let button = Rect::from_center_size(
            Pos2::new(header.right() - 4.0 - 10.0, header.center().y),
            Vec2::splat(20.0),
        );
        if mini_button(
            ui,
            theme,
            button,
            Id::unique("git-graph-refresh"),
            UNDO,
            tr(lang, "git.graph.refresh"),
            true,
            12.0,
        ) {
            panel.load_graph(ctx);
            return;
        }
    }
    if response.clicked() {
        panel.graph_open = !panel.graph_open;
        if panel.graph_open {
            panel.load_graph(ctx);
        } else {
            // A closed graph keeps nothing in memory.
            panel.graph = None;
            panel.detail = None;
        }
    }
    if !panel.graph_open {
        return;
    }

    if panel.detail.is_some() {
        commit_detail(ui, panel, theme, lang);
        return;
    }

    // Search box.
    egui::Frame::new()
        .inner_margin(Margin {
            left: 4,
            right: 4,
            top: 0,
            bottom: 6,
        })
        .show(ui, |ui| {
            egui::Frame::new()
                .fill(theme.panel)
                .stroke(Stroke::new(1.0, theme.border))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(icons::SEARCH)
                                .font(mono(11.0))
                                .color(theme.fg_faint),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut panel.search)
                                .hint_text(
                                    RichText::new(tr(lang, "git.graph.searchPlaceholder"))
                                        .color(theme.fg_faint),
                                )
                                .frame(egui::Frame::NONE)
                                .font(sans(11.0))
                                .desired_width(f32::INFINITY),
                        );
                    });
                });
        });

    if let Some(error) = &panel.graph_error {
        ui.label(RichText::new(error).font(sans(12.0)).color(theme.offline));
    }
    let Some(graph) = &panel.graph else {
        if panel.graph_error.is_none() {
            ui.label(
                RichText::new(tr(lang, "git.graph.loading"))
                    .font(sans(12.0))
                    .color(theme.fg_faint),
            );
        }
        return;
    };
    if graph.commits.is_empty() {
        ui.label(
            RichText::new(tr(lang, "git.graph.empty"))
                .font(sans(12.0))
                .color(theme.fg_faint),
        );
        return;
    }

    // Search never filters rows (that would break the lanes); it only dims the misses.
    let query = panel.search.trim().to_lowercase();
    let matches: Option<HashSet<usize>> = (!query.is_empty()).then(|| {
        graph
            .commits
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.subject.to_lowercase().contains(&query)
                    || c.hash.to_lowercase().contains(&query)
                    || c.author_name.to_lowercase().contains(&query)
                    || c.refs.iter().any(|r| r.to_lowercase().contains(&query))
            })
            .map(|(i, _)| i)
            .collect()
    });
    if matches.as_ref().is_some_and(HashSet::is_empty) {
        ui.label(
            RichText::new(tr(lang, "git.graph.noSearchResults"))
                .font(sans(12.0))
                .color(theme.fg_faint),
        );
    }

    let graph_width = graph.lane_count as f32 * LANE_WIDTH;
    let mut selected: Option<String> = None;
    let mut menu_op: Option<Op> = None;
    let mut branch_prompt: Option<String> = None;
    let mut hard_reset: Option<String> = None;
    let height = ui.available_height().clamp(60.0, GRAPH_MAX_HEIGHT);
    let mut area = egui::ScrollArea::vertical()
        .id_salt("git-graph")
        .max_height(height)
        .auto_shrink([false, true]);
    if panel.restore_offset {
        area = area.vertical_scroll_offset(panel.list_offset);
        panel.restore_offset = false;
    }
    let output = area.show_rows(ui, ROW_HEIGHT, graph.rows.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for i in range {
            let row = &graph.rows[i];
            let commit = &graph.commits[i];
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
            if response.hovered() {
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(4), theme.panel_hover);
            }
            let dim = matches.as_ref().is_some_and(|m| !m.contains(&i));
            paint_graph_row(
                ui,
                theme,
                lang,
                row,
                commit,
                i as u32,
                rect,
                graph_width,
                dim,
            );
            let short: String = commit.hash.chars().take(10).collect();
            let response = response.on_hover_text(format!("{short} — {}", commit.subject));
            if response.clicked() {
                selected = Some(commit.hash.clone());
            }
            response.context_menu(|ui| {
                let hash = commit.hash.clone();
                let item = |ui: &mut Ui, glyph: &str, key: &str| {
                    ui.button(RichText::new(format!("{glyph}  {}", tr(lang, key))).font(sans(12.5)))
                        .clicked()
                };
                if item(ui, COPY, "git.graph.menu.copyHash") {
                    ui.ctx().copy_text(hash.clone());
                    ui.close();
                }
                if item(ui, GIT_BRANCH_CREATE, "git.graph.menu.createBranch") {
                    branch_prompt = Some(hash.clone());
                    ui.close();
                }
                if item(ui, GIT_COMMIT, "git.graph.menu.cherryPick") {
                    menu_op = Some(Op::CherryPick(hash.clone()));
                    ui.close();
                }
                if item(ui, UNDO, "git.graph.menu.revert") {
                    menu_op = Some(Op::Revert(hash.clone()));
                    ui.close();
                }
                ui.separator();
                if item(ui, DISCARD, "git.graph.menu.resetSoft") {
                    menu_op = Some(Op::Reset {
                        hash: hash.clone(),
                        mode: "soft",
                    });
                    ui.close();
                }
                if item(ui, DISCARD, "git.graph.menu.resetMixed") {
                    menu_op = Some(Op::Reset {
                        hash: hash.clone(),
                        mode: "mixed",
                    });
                    ui.close();
                }
                if item(ui, DISCARD, "git.graph.menu.resetHard") {
                    hard_reset = Some(hash.clone());
                    ui.close();
                }
            });
        }
    });
    panel.list_offset = output.state.clamped_offset().y;

    if let Some(hash) = selected {
        panel.restore_offset = true;
        panel.open_detail(ctx, hash);
    }
    if let Some(op) = menu_op {
        panel.run(ctx, op);
    }
    if let Some(hash) = branch_prompt {
        panel.dialog = Some(Dialog::BranchName {
            hash,
            name: String::new(),
        });
    }
    if let Some(hash) = hard_reset {
        panel.dialog = Some(Dialog::Confirm {
            text: tr(lang, "git.graph.menu.resetHardConfirm").to_string(),
            op: Op::Reset { hash, mode: "hard" },
        });
    }
}

/// `GitGraphCommitDetail`: replaces the list in place; dropped (with its data) on "back".
fn commit_detail(ui: &mut Ui, panel: &mut GitPanel, theme: &Theme, lang: Lang) {
    let Some(detail) = &panel.detail else { return };
    let commit = panel
        .graph
        .as_ref()
        .and_then(|g| g.commits.iter().find(|c| c.hash == detail.hash));
    let Some(commit) = commit else {
        panel.detail = None;
        return;
    };
    let mut back = false;
    egui::ScrollArea::vertical()
        .id_salt("git-commit-detail")
        .max_height(ui.available_height().clamp(60.0, GRAPH_MAX_HEIGHT))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin {
                    left: 6,
                    right: 6,
                    top: 4,
                    bottom: 10,
                })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    let label = format!("{ARROW_LEFT}  {}", tr(lang, "common.back"));
                    let galley = ui
                        .painter()
                        .layout_no_wrap(label, sans(11.0), theme.fg_muted);
                    let (rect, response) = ui
                        .allocate_exact_size(galley.size() + Vec2::new(16.0, 6.0), Sense::click());
                    let hovered = response.hovered();
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(4),
                        if hovered {
                            theme.panel_hover
                        } else {
                            theme.panel
                        },
                    );
                    ui.painter().rect_stroke(
                        rect,
                        CornerRadius::same(4),
                        Stroke::new(
                            1.0,
                            if hovered {
                                theme.border_strong
                            } else {
                                theme.border
                            },
                        ),
                        StrokeKind::Inside,
                    );
                    ui.painter().galley(
                        rect.center() - galley.size() / 2.0,
                        galley,
                        if hovered { theme.fg } else { theme.fg_muted },
                    );
                    back = response.clicked();

                    let (rect, _) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 18.0), Sense::hover());
                    let short: String = commit.hash.chars().take(10).collect();
                    let hash = ui.painter().layout_no_wrap(
                        short,
                        FontId::new(12.0, egui::FontFamily::Name("mono-bold".into())),
                        theme.fg,
                    );
                    let hash_width = hash.size().x;
                    ui.painter().galley(
                        Pos2::new(rect.left(), rect.center().y - hash.size().y / 2.0),
                        hash,
                        theme.fg,
                    );
                    ref_badges(
                        ui,
                        theme,
                        &commit.refs,
                        rect.left() + hash_width + 6.0,
                        rect.center().y,
                        rect.width() - hash_width - 6.0,
                    );
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            commit.author_name,
                            relative_time(lang, commit.timestamp)
                        ))
                        .font(sans(10.0))
                        .color(theme.fg_faint),
                    );
                    if let Some(error) = &detail.error {
                        ui.label(RichText::new(error).font(sans(12.0)).color(theme.offline));
                    }
                    egui::Frame::new()
                        .fill(theme.panel)
                        .stroke(Stroke::new(1.0, theme.border))
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(Margin::same(8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let text = detail
                                .message
                                .as_deref()
                                .unwrap_or(tr(lang, "git.graph.detail.loadingMessage"));
                            ui.add(
                                egui::Label::new(
                                    RichText::new(text).font(sans(12.0)).color(theme.fg),
                                )
                                .wrap(),
                            );
                        });
                    ui.label(
                        RichText::new(tr(lang, "git.graph.detail.filesTitle").to_uppercase())
                            .font(bold(11.0))
                            .color(theme.fg_muted),
                    );
                    ui.spacing_mut().item_spacing.y = 2.0;
                    match &detail.files {
                        None => {
                            ui.label(
                                RichText::new(tr(lang, "git.graph.filesLoading"))
                                    .font(sans(11.0))
                                    .color(theme.fg_faint),
                            );
                        }
                        Some(files) if files.is_empty() => {
                            ui.label(
                                RichText::new(tr(lang, "git.graph.filesEmpty"))
                                    .font(sans(11.0))
                                    .color(theme.fg_faint),
                            );
                        }
                        Some(files) => {
                            for file in files {
                                file_line(ui, theme, &file.status, &file.path);
                            }
                        }
                    }
                });
        });
    if back {
        panel.detail = None;
    }
}

// ---------------------------------------------------------------- confirm / prompt dialogs

fn dialogs(ctx: &egui::Context, panel: &mut GitPanel, theme: &Theme, lang: Lang) {
    let Some(dialog) = &mut panel.dialog else {
        return;
    };
    let mut close = false;
    let mut confirmed: Option<Op> = None;
    let modal = egui::Modal::new(Id::unique("git-dialog"))
        .frame(
            egui::Frame::new()
                .fill(theme.bg_elevated)
                .stroke(Stroke::new(1.0, theme.border_strong))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(18)),
        )
        .show(ctx, |ui| {
            ui.set_width(340.0);
            match dialog {
                Dialog::Confirm { text, op } => {
                    ui.add(
                        egui::Label::new(
                            RichText::new(text.as_str())
                                .font(sans(13.0))
                                .color(theme.fg),
                        )
                        .wrap(),
                    );
                    ui.add_space(14.0);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if super::widgets::primary_button(ui, theme, tr(lang, "common.confirm"))
                            .clicked()
                        {
                            confirmed = Some(op.clone());
                        }
                        if super::widgets::ghost_button(ui, theme, tr(lang, "common.cancel"))
                            .clicked()
                        {
                            close = true;
                        }
                    });
                }
                Dialog::BranchName { hash, name } => {
                    ui.label(
                        RichText::new(tr(lang, "git.graph.menu.createBranchPrompt"))
                            .font(bold(13.0))
                            .color(theme.fg),
                    );
                    ui.add_space(8.0);
                    let response = super::widgets::text_input(ui, theme, name, "feature/…", 340.0);
                    response.request_focus();
                    let submit = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    ui.add_space(14.0);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        let ok = super::widgets::primary_button(ui, theme, "OK").clicked();
                        if (ok || submit) && !name.trim().is_empty() {
                            confirmed = Some(Op::CreateBranch {
                                hash: hash.clone(),
                                name: name.trim().to_string(),
                            });
                        }
                        if super::widgets::ghost_button(ui, theme, tr(lang, "common.cancel"))
                            .clicked()
                        {
                            close = true;
                        }
                    });
                }
            }
        });
    if modal.should_close() {
        close = true;
    }
    if let Some(op) = confirmed {
        panel.dialog = None;
        if let Op::Commit { .. } = op {
            panel.message.clear();
        }
        panel.run(ctx, op);
    } else if close {
        panel.dialog = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(hash: &str, parents: &[&str], refs: &[&str]) -> GitCommitEntry {
        GitCommitEntry {
            hash: hash.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author_name: "a".into(),
            author_email: "a@x".into(),
            timestamp: 0,
            subject: hash.into(),
            refs: refs.iter().map(|r| r.to_string()).collect(),
        }
    }

    #[test]
    fn merge_opens_a_second_lane_that_converges_back() {
        // d (merge of b and c) -> b -> a ; c -> a
        let commits = vec![
            commit("d", &["b", "c"], &["HEAD -> main"]),
            commit("c", &["a"], &["feature"]),
            commit("b", &["a"], &[]),
            commit("a", &[], &[]),
        ];
        let (rows, lanes) = build_graph_rows(&commits);
        assert_eq!(lanes, 2);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[0].after, vec![Some(2), Some(1)]);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[2].lane, 0);
        // `a` is awaited by both lanes: lane 1 converges into it.
        assert_eq!(rows[3].before, vec![Some(3), Some(3)]);
        assert!(rows[3].last);
    }

    #[test]
    fn tree_compresses_single_child_folders_and_sorts_dirs_first() {
        let change = |p: &str| GitFileChange {
            path: p.into(),
            original_path: None,
            status: "M".into(),
        };
        let items = vec![
            change("z.txt"),
            change("src/ui/a.rs"),
            change("src/ui/b.rs"),
        ];
        let tree = build_tree(&items);
        match &tree[0] {
            Node::Dir { name, children, .. } => {
                assert_eq!(name, "src/ui");
                assert_eq!(children.len(), 2);
            }
            Node::File { .. } => panic!("directory should come first"),
        }
        assert!(matches!(&tree[1], Node::File { name, .. } if name == "z.txt"));
    }

    #[test]
    fn repo_paths_use_the_root_separator() {
        assert_eq!(
            absolute_repo_path(r"C:\repo", "src/a.rs", false),
            r"C:\repo\src\a.rs"
        );
        assert_eq!(
            absolute_repo_path(r"C:\repo", "src/a.rs", true),
            r"C:\repo\src"
        );
        assert_eq!(absolute_repo_path("/r", "a/b", false), "/r/a/b");
    }
}
