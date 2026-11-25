//! `OrchestratorPane`: the board of delegated workers. The planner sits above its runs, each run
//! above a row of its workers (`lib/orchestratorGraph.ts`), on a pannable, zoomable dot canvas, with
//! the summary rail (`RunBranch`/`RailRow`) on the right and the worker inspector over the canvas.
//!
//! The board only redraws on a new snapshot from the core, on input, or once a second while a worker
//! runs (for the elapsed clocks). Edges are not animated: a moving dash would repaint every frame.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use egui::{
    pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Layout, Pos2, Rect, RichText, Sense,
    Shape, Stroke, StrokeKind, Vec2,
};
use serde::Deserialize;

use super::icons;
use super::widgets::{self, bold, mono, sans};
use crate::app::{Action, AletheApp};
use crate::orchestrator::ShellControl;
use crate::theme::Theme;

// ----------------------------------------------------------------- snapshot

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct Snapshot {
    jobs: Vec<Job>,
    planners: Vec<PlannerInfo>,
    shells: Vec<ShellInfo>,
    running: usize,
    queued: usize,
    concurrency_limit: usize,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct ShellOwnerInfo {
    id: String,
}

/// A long-running command a planner opened with `alethe_open_shell` (`OrchestratorShell`).
#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct ShellInfo {
    id: String,
    name: String,
    command: String,
    cwd: String,
    owner: Option<ShellOwnerInfo>,
    status: String,
    exit_code: Option<i64>,
    started_at_ms: Option<f64>,
    pty_id: String,
    /// Hangs off the planner node (its own shell) instead of standing alone.
    #[serde(skip)]
    attached: bool,
}

impl ShellInfo {
    fn owner_id(&self) -> Option<&str> {
        self.owner
            .as_ref()
            .map(|owner| owner.id.as_str())
            .filter(|id| !id.is_empty())
    }

    fn running(&self) -> bool {
        self.status == "running"
    }

    fn lane(&self) -> Lane {
        match self.status.as_str() {
            "running" => Lane::Running,
            "exited" if self.exit_code.is_some_and(|code| code != 0) => Lane::Failed,
            _ => Lane::Finished,
        }
    }

    fn status_text(&self, lang: crate::i18n::Lang) -> String {
        let t = |key: &str| crate::i18n::tr(lang, key);
        match self.status.as_str() {
            "running" => t("orchestrator.shell.running").to_string(),
            "exited" => t("orchestrator.shell.exited").replace(
                "{code}",
                &self
                    .exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "—".to_string()),
            ),
            _ => t("orchestrator.shell.stopped").to_string(),
        }
    }

    fn elapsed(&self) -> Option<f64> {
        let started = self.started_at_ms?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis() as f64;
        self.running().then(|| ((now - started) / 1000.0).max(0.0))
    }
}

/// Whether `path` is `root` or inside it, whatever the slashes and case (Windows paths).
fn path_under(path: &str, root: &str) -> bool {
    let norm = |p: &str| p.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let (path, root) = (norm(path), norm(root));
    path == root || path.starts_with(&format!("{root}/"))
}

/// `shellsForBoard`: the active planner's own shells hang off its node; a shell whose planner is
/// not on this board (gone, or never recorded) stands alone, so a `docker compose up` is never
/// dropped because the planner that started it closed.
fn shells_for_board(
    shells: &[ShellInfo],
    active: Option<&str>,
    live: &HashSet<String>,
) -> Vec<ShellInfo> {
    let mut board = Vec::new();
    for shell in shells {
        let owner = shell.owner_id();
        if active.is_some() && owner == active {
            board.push(ShellInfo {
                attached: true,
                ..shell.clone()
            });
        } else if !owner.is_some_and(|id| live.contains(id)) {
            board.push(ShellInfo {
                attached: false,
                ..shell.clone()
            });
        }
    }
    board
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct PlannerInfo {
    id: String,
    label: String,
    agent: String,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct Job {
    id: String,
    planner_id: Option<String>,
    agent: String,
    run_id: String,
    run_label: Option<String>,
    rules: Option<String>,
    spec: String,
    cwd: String,
    status: String,
    outcome: Option<String>,
    seconds: Option<f64>,
    plan: Vec<serde_json::Value>,
    tokens: Option<Tokens>,
    cost_usd: Option<f64>,
    worktree: Option<String>,
    role: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    read_only: bool,
    pending_approval: Option<Approval>,
    superseded_by: Option<String>,
    has_diff: bool,
    summary: String,
    /// One of Claude's own subagents, read from its transcript: Alethe can't stop it.
    #[serde(skip)]
    native: bool,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct Tokens {
    total: Option<TokenCount>,
    last: Option<TokenCount>,
    model_context_window: Option<f64>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct TokenCount {
    total_tokens: Option<f64>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct Approval {
    kind: String,
    command: Option<String>,
    reason: Option<String>,
}

impl Job {
    fn lane(&self) -> Lane {
        match self.status.as_str() {
            "blocked" => Lane::Blocked,
            "running" => Lane::Running,
            "queued" => Lane::Queued,
            "interrupted" => Lane::Interrupted,
            "failed" => Lane::Failed,
            _ => Lane::Finished,
        }
    }
    /// The card's name: the worker id, or a native subagent's description (its id is a
    /// `toolu_…` call id).
    fn label(&self) -> String {
        if !self.native {
            return self.id.clone();
        }
        let text = self.spec.lines().next().unwrap_or_default().trim();
        match text.char_indices().nth(32) {
            Some((cut, _)) => format!("{}…", &text[..cut]),
            None if text.is_empty() => self.id.clone(),
            None => text.to_string(),
        }
    }

    /// Claude's own subagents run inside its process: the core can't stop them.
    fn can_stop(&self) -> bool {
        !self.native && matches!(self.status.as_str(), "running" | "queued" | "blocked")
    }
    fn total_tokens(&self) -> f64 {
        self.tokens
            .as_ref()
            .and_then(|t| t.total.as_ref())
            .and_then(|t| t.total_tokens)
            .unwrap_or(0.0)
    }
    /// How full the model's context window is: the last turn's prompt, not the running total.
    fn context_share(&self) -> Option<u32> {
        let tokens = self.tokens.as_ref()?;
        let used = tokens.last.as_ref()?.total_tokens.filter(|u| *u > 0.0)?;
        let window = tokens.model_context_window.filter(|w| *w > 0.0)?;
        Some(((used / window) * 100.0).round().min(100.0) as u32)
    }
    fn plan_lines(&self) -> Vec<String> {
        self.plan
            .iter()
            .map(|step| match step {
                serde_json::Value::String(s) => s.clone(),
                other => other
                    .get("step")
                    .or_else(|| other.get("text"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| other.to_string()),
            })
            .collect()
    }
}

// ----------------------------------------------------------------- runs (`orchestratorRuns.ts`)

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Lane {
    Blocked,
    Running,
    Queued,
    Interrupted,
    Failed,
    Finished,
}

const LANE_ORDER: [Lane; 6] = [
    Lane::Blocked,
    Lane::Running,
    Lane::Queued,
    Lane::Interrupted,
    Lane::Failed,
    Lane::Finished,
];
// Worst first: a blocked worker holds a slot on a person; a failure outranks live work.
const STATE_PRIORITY: [Lane; 6] = [
    Lane::Blocked,
    Lane::Failed,
    Lane::Interrupted,
    Lane::Running,
    Lane::Queued,
    Lane::Finished,
];
const ATTENTION_PRIORITY: [Lane; 3] = [Lane::Blocked, Lane::Failed, Lane::Interrupted];

impl Lane {
    fn key(self) -> &'static str {
        match self {
            Lane::Blocked => "orchestrator.lane.blocked",
            Lane::Running => "orchestrator.lane.running",
            Lane::Queued => "orchestrator.lane.queued",
            Lane::Interrupted => "orchestrator.lane.interrupted",
            Lane::Failed => "orchestrator.lane.failed",
            Lane::Finished => "orchestrator.lane.finished",
        }
    }
    fn color(self, theme: &Theme) -> Color32 {
        match self {
            Lane::Running => theme.working,
            Lane::Queued | Lane::Interrupted | Lane::Blocked => theme.waiting,
            Lane::Failed => theme.offline,
            Lane::Finished => theme.stopped,
        }
    }
}

#[derive(Default, Clone)]
struct Counts(HashMap<Lane, usize>);

impl Counts {
    fn of(jobs: &[&Job]) -> Self {
        let mut c = Counts::default();
        for job in jobs {
            *c.0.entry(job.lane()).or_default() += 1;
        }
        c
    }
    fn get(&self, lane: Lane) -> usize {
        self.0.get(&lane).copied().unwrap_or(0)
    }
    fn worst(&self) -> Lane {
        STATE_PRIORITY
            .into_iter()
            .find(|l| self.get(*l) > 0)
            .unwrap_or(Lane::Finished)
    }
    fn attention(&self) -> Option<(Lane, usize)> {
        ATTENTION_PRIORITY
            .into_iter()
            .find(|l| self.get(*l) > 0)
            .map(|l| (l, self.get(l)))
    }
}

struct Run<'a> {
    id: String,
    label: String,
    rules: Option<String>,
    jobs: Vec<&'a Job>,
    counts: Counts,
    state: Lane,
}

struct Group<'a> {
    /// The planner id; `None` for work delegated from outside a terminal.
    id: Option<String>,
    label: Option<String>,
    agent: Option<String>,
    runs: Vec<Run<'a>>,
    jobs: Vec<&'a Job>,
    counts: Counts,
    /// Only the active group's: its planner's shells, then the ones standing alone.
    shells: Vec<ShellInfo>,
}

impl Group<'_> {
    fn key(&self) -> String {
        self.id.clone().unwrap_or_default()
    }
}

fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn group_runs<'a>(jobs: &[&'a Job]) -> Vec<Run<'a>> {
    let mut order: Vec<String> = Vec::new();
    let mut grouped: HashMap<String, Vec<&'a Job>> = HashMap::new();
    for job in jobs {
        if !grouped.contains_key(&job.run_id) {
            order.push(job.run_id.clone());
        }
        grouped.entry(job.run_id.clone()).or_default().push(job);
    }
    order
        .into_iter()
        .map(|id| {
            let jobs = grouped.remove(&id).unwrap_or_default();
            let counts = Counts::of(&jobs);
            let label = jobs
                .iter()
                .find_map(|j| clean(j.run_label.as_deref()))
                .unwrap_or_else(|| id.clone());
            let rules = jobs.iter().find_map(|j| clean(j.rules.as_deref()));
            Run {
                state: counts.worst(),
                id,
                label,
                rules,
                jobs,
                counts,
            }
        })
        .collect()
}

fn to_group<'a>(
    id: Option<String>,
    label: Option<String>,
    agent: Option<String>,
    all: Vec<&'a Job>,
) -> Group<'a> {
    let jobs: Vec<&Job> = all
        .into_iter()
        .filter(|j| j.superseded_by.is_none())
        .collect();
    let counts = Counts::of(&jobs);
    Group {
        runs: group_runs(&jobs),
        id,
        label,
        agent,
        jobs,
        counts,
        shells: Vec::new(),
    }
}

/// One group per declared planner (sorted, so tabs do not reshuffle), then planners seen only on a
/// job, then the jobs with no planner at all.
fn group_planners(snapshot: &Snapshot) -> Vec<Group<'_>> {
    let mut buckets: Vec<(Option<String>, Vec<&Job>)> = Vec::new();
    for job in &snapshot.jobs {
        match buckets.iter_mut().find(|(k, _)| *k == job.planner_id) {
            Some((_, bucket)) => bucket.push(job),
            None => buckets.push((job.planner_id.clone(), vec![job])),
        }
    }
    let mut declared: Vec<&PlannerInfo> = snapshot.planners.iter().collect();
    declared.sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
    let mut groups: Vec<Group> = declared
        .iter()
        .map(|p| {
            let jobs = buckets
                .iter()
                .find(|(k, _)| k.as_deref() == Some(p.id.as_str()))
                .map(|(_, b)| b.clone())
                .unwrap_or_default();
            to_group(
                Some(p.id.clone()),
                clean(Some(&p.label)).or_else(|| Some(p.id.clone())),
                clean(Some(&p.agent)),
                jobs,
            )
        })
        .collect();
    for (key, bucket) in &buckets {
        match key {
            Some(k) if !snapshot.planners.iter().any(|p| &p.id == k) => groups.push(to_group(
                Some(k.clone()),
                Some(k.clone()),
                None,
                bucket.clone(),
            )),
            _ => {}
        }
    }
    if let Some((_, orphans)) = buckets.iter().find(|(k, _)| k.is_none()) {
        groups.push(to_group(None, None, None, orphans.clone()));
    }
    groups
}

// ----------------------------------------------------------------- graph (`orchestratorGraph.ts`)

const NODE_WIDTH: f32 = 252.0;
const SIBLING_GAP: f32 = 24.0;
const TREE_GAP: f32 = 120.0;
const LEVEL_GAP: f32 = 56.0;
const CANVAS_PADDING: f32 = 40.0;
const ELBOW_RADIUS: f32 = 8.0;
const DOT_SPACING: f32 = 22.0;
const MIN_SCALE: f32 = 0.35;
const MAX_SCALE: f32 = 1.6;
const ZOOM_STEP: f32 = 1.2;
const PLANNER_H: f32 = 56.0;
const SHELL_GROUP_H: f32 = 56.0;
const SHELL_H: f32 = 82.0;
/// One dash and gap of a running edge, and how long it takes to flow by (`edge-flow`).
const FLOW_PERIOD: f32 = 12.0;
const FLOW_SECONDS: f64 = 0.6;
const RUN_H: f32 = 86.0;
const RAIL_W: f32 = 264.0;
const HEADER_H: f32 = 36.0;
const TABS_H: f32 = 32.0;

#[derive(Clone, Copy)]
enum NodeKind {
    Planner,
    Run(usize),
    Worker(usize, usize),
    /// The shells group: `true` hangs off the planner, `false` stands alone.
    ShellGroup(bool),
    Shell(usize),
}

struct Node {
    kind: NodeKind,
    rect: Rect, // world coordinates
}

struct Edge {
    from: Pos2,
    to: Pos2,
    lane: Lane,
    worker: Option<String>,
}

struct Graph {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    size: Vec2,
}

fn worker_height(job: &Job) -> f32 {
    let mut h = 82.0;
    if job.status == "blocked" && job.pending_approval.is_some() {
        h += 92.0;
    }
    if job.status == "failed" && job.outcome.is_some() {
        h += 26.0;
    }
    h
}

fn layout(runs: &[Run], shells: &[ShellInfo], with_planner: bool) -> Graph {
    // Trees side by side: each run, then the planner's own shells, then the ones standing alone.
    enum Tree {
        Run(usize),
        Shells(bool),
    }
    let mut trees: Vec<(Tree, usize)> = runs
        .iter()
        .enumerate()
        .map(|(ri, run)| (Tree::Run(ri), run.jobs.len()))
        .collect();
    for attached in [true, false] {
        let count = shells.iter().filter(|s| s.attached == attached).count();
        if count > 0 {
            trees.push((Tree::Shells(attached), count));
        }
    }
    let span = |n: usize| {
        let n = n.max(1) as f32;
        n * NODE_WIDTH + (n - 1.0) * SIBLING_GAP
    };
    let run_top = CANVAS_PADDING
        + if with_planner {
            PLANNER_H + LEVEL_GAP
        } else {
            0.0
        };
    let worker_top = run_top + RUN_H + LEVEL_GAP;

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut bottom = run_top + RUN_H;
    // Roots the planner connects to, and every root (to center the planner when it has none).
    let mut heads: Vec<(Rect, Lane)> = Vec::new();
    let mut roots: Vec<Rect> = Vec::new();
    let mut cursor = CANVAS_PADDING;
    for (tree, count) in &trees {
        let left = cursor;
        let width = span(*count);
        cursor += width + TREE_GAP;
        let root_x = (left + (width - NODE_WIDTH) / 2.0).round();
        match tree {
            Tree::Run(ri) => {
                let run = &runs[*ri];
                let root = Rect::from_min_size(pos2(root_x, run_top), vec2(NODE_WIDTH, RUN_H));
                heads.push((root, run.state));
                roots.push(root);
                nodes.push(Node {
                    kind: NodeKind::Run(*ri),
                    rect: root,
                });
                for (wi, job) in run.jobs.iter().enumerate() {
                    let rect = Rect::from_min_size(
                        pos2(left + wi as f32 * (NODE_WIDTH + SIBLING_GAP), worker_top),
                        vec2(NODE_WIDTH, worker_height(job)),
                    );
                    bottom = bottom.max(rect.bottom());
                    edges.push(Edge {
                        from: pos2(root.center().x, root.bottom()),
                        to: pos2(rect.center().x, rect.top()),
                        lane: job.lane(),
                        worker: Some(job.id.clone()),
                    });
                    nodes.push(Node {
                        kind: NodeKind::Worker(*ri, wi),
                        rect,
                    });
                }
            }
            Tree::Shells(attached) => {
                let root =
                    Rect::from_min_size(pos2(root_x, run_top), vec2(NODE_WIDTH, SHELL_GROUP_H));
                roots.push(root);
                let members: Vec<usize> = (0..shells.len())
                    .filter(|&i| shells[i].attached == *attached)
                    .collect();
                let lane = if members.iter().any(|&i| shells[i].running()) {
                    Lane::Running
                } else {
                    Lane::Finished
                };
                if *attached {
                    heads.push((root, lane));
                }
                nodes.push(Node {
                    kind: NodeKind::ShellGroup(*attached),
                    rect: root,
                });
                for (column, &si) in members.iter().enumerate() {
                    let rect = Rect::from_min_size(
                        pos2(
                            left + column as f32 * (NODE_WIDTH + SIBLING_GAP),
                            worker_top,
                        ),
                        vec2(NODE_WIDTH, SHELL_H),
                    );
                    bottom = bottom.max(rect.bottom());
                    edges.push(Edge {
                        from: pos2(root.center().x, root.bottom()),
                        to: pos2(rect.center().x, rect.top()),
                        lane: shells[si].lane(),
                        worker: Some(shells[si].id.clone()),
                    });
                    nodes.push(Node {
                        kind: NodeKind::Shell(si),
                        rect,
                    });
                }
            }
        }
    }
    if with_planner {
        let centering: Vec<Rect> = if heads.is_empty() {
            roots.clone()
        } else {
            heads.iter().map(|(rect, _)| *rect).collect()
        };
        let x = match (centering.first(), centering.last()) {
            (Some(first), Some(last)) => {
                ((first.center().x + last.center().x) / 2.0 - NODE_WIDTH / 2.0).round()
            }
            _ => CANVAS_PADDING,
        };
        let planner = Rect::from_min_size(pos2(x, CANVAS_PADDING), vec2(NODE_WIDTH, PLANNER_H));
        for (root, lane) in heads.iter().rev() {
            edges.insert(
                0,
                Edge {
                    from: pos2(planner.center().x, planner.bottom()),
                    to: pos2(root.center().x, root.top()),
                    lane: *lane,
                    worker: None,
                },
            );
        }
        nodes.insert(
            0,
            Node {
                kind: NodeKind::Planner,
                rect: planner,
            },
        );
    }
    let width = if trees.is_empty() {
        NODE_WIDTH + 2.0 * CANVAS_PADDING
    } else {
        cursor - TREE_GAP + CANVAS_PADDING
    };
    Graph {
        nodes,
        edges,
        size: vec2(width, bottom + CANVAS_PADDING),
    }
}

/// A running edge's dashes flow from parent to child, like the web board's `edge-flow`.
fn flowing_line(painter: &egui::Painter, pts: &[Pos2], stroke: Stroke, s: f32, time: f64) {
    let period = FLOW_PERIOD * s;
    let (dash, gap) = (7.0 * s, 5.0 * s);
    let phase = ((time / FLOW_SECONDS).fract() as f32) * period;
    painter.extend(Shape::dashed_line_with_offset(
        pts,
        stroke,
        &[dash],
        &[gap],
        phase,
    ));
    // The dash entering at the start: the offset leaves room for a whole gap only.
    if phase > gap {
        if let [start, next, ..] = pts {
            let dir = (*next - *start).normalized();
            let len = (phase - gap).min((*next - *start).length());
            painter.line_segment([*start, *start + dir * len], stroke);
        }
    }
}

/// Downward elbow with rounded corners, sampled into points.
fn connector(from: Pos2, to: Pos2) -> Vec<Pos2> {
    let dx = to.x - from.x;
    if dx.abs() < 1.0 {
        return vec![from, to];
    }
    let mid_y = from.y + (to.y - from.y) / 2.0;
    let r = ELBOW_RADIUS
        .min(dx.abs() / 2.0)
        .min((to.y - from.y).abs() / 2.0);
    let step = if dx > 0.0 { r } else { -r };
    let mut pts = vec![from, pos2(from.x, mid_y - r)];
    quad(
        &mut pts,
        pos2(from.x, mid_y - r),
        pos2(from.x, mid_y),
        pos2(from.x + step, mid_y),
    );
    pts.push(pos2(to.x - step, mid_y));
    quad(
        &mut pts,
        pos2(to.x - step, mid_y),
        pos2(to.x, mid_y),
        pos2(to.x, mid_y + r),
    );
    pts.push(to);
    pts
}

fn quad(out: &mut Vec<Pos2>, a: Pos2, c: Pos2, b: Pos2) {
    for i in 1..=6 {
        let t = i as f32 / 6.0;
        let u = 1.0 - t;
        out.push(pos2(
            u * u * a.x + 2.0 * u * t * c.x + t * t * b.x,
            u * u * a.y + 2.0 * u * t * c.y + t * t * b.y,
        ));
    }
}

// ----------------------------------------------------------------- per-board state

#[derive(Clone, Copy, PartialEq)]
struct View {
    scale: f32,
    offset: Vec2,
}

#[derive(Clone, Copy, PartialEq, Default)]
enum InspectorTab {
    #[default]
    Report,
    Diff,
}

type DiffSlot = Arc<Mutex<Option<Result<String, String>>>>;

#[derive(Default)]
pub struct BoardState {
    /// The planner tab was picked by hand, so the board stops following live work.
    pinned: bool,
    revision: u64,
    snapshot: Arc<Snapshot>,
    active: Option<String>,
    selected: Option<String>,
    view: Option<View>,
    moved: bool,
    graph_size: Vec2,
    rail_collapsed: bool,
    closed_runs: HashSet<String>,
    tab: InspectorTab,
    diff: Option<(String, DiffSlot)>,
    message: String,
    last_tick: f64,
    /// Each shell's last output: read at, its status then, and the text.
    shell_tails: HashMap<String, (f64, String, String)>,
}

fn fit(graph: Vec2, viewport: Vec2) -> View {
    if graph.x <= 0.0 || graph.y <= 0.0 || viewport.x <= 0.0 || viewport.y <= 0.0 {
        return View {
            scale: 1.0,
            offset: Vec2::ZERO,
        };
    }
    let scale = (viewport.x / graph.x)
        .min(viewport.y / graph.y)
        .min(1.0)
        .clamp(MIN_SCALE, MAX_SCALE);
    View {
        scale,
        offset: vec2(
            ((viewport.x - graph.x * scale) / 2.0).round(),
            ((viewport.y - graph.y * scale) / 2.0).round(),
        ),
    }
}

fn zoom_at(view: View, factor: f32, point: Vec2) -> View {
    let scale = (view.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
    if scale == view.scale {
        return view;
    }
    let ratio = scale / view.scale;
    View {
        scale,
        offset: point - (point - view.offset) * ratio,
    }
}

// ----------------------------------------------------------------- formatting (`nodeFormat.tsx`)

fn format_elapsed(seconds: Option<f64>) -> Option<String> {
    let whole = seconds?.floor() as u64;
    Some(if whole < 60 {
        format!("{whole}s")
    } else {
        format!("{}m {:02}s", whole / 60, whole % 60)
    })
}

fn format_tokens(total: f64) -> Option<String> {
    if total <= 0.0 {
        return None;
    }
    Some(if total < 1000.0 {
        format!("{total:.0}")
    } else if total < 10_000.0 {
        format!("{:.1}k", total / 1000.0)
    } else {
        format!("{:.0}k", total / 1000.0)
    })
}

fn fmt_usd(value: f64) -> String {
    if value < 0.01 && value > 0.0 {
        format!("${value:.4}")
    } else {
        format!("${value:.2}")
    }
}

fn latest_line(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

fn fill(template: &str, pairs: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (k, v) in pairs {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

// ----------------------------------------------------------------- the pane

enum Intent {
    Select(Option<String>),
    Answer(String, &'static str),
    Stop(String),
    Restart(String),
    RevealPlanner(String),
    ToggleRun(String),
    Shell(String, ShellControl),
}

pub fn pane(app: &mut AletheApp, ui: &mut egui::Ui, pane_id: &str, rect: Rect, focused: bool) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let t = |key: &str| crate::i18n::tr(lang, key);
    let enabled = app.feature_enabled("orchestrator");
    let term_palette = crate::theme::term_palette(&app.terminal_theme_id());
    let term_font = app.data.preferences.terminal_font_size.clamp(8.0, 28.0);
    if enabled && !app.orchestrator.is_started() {
        if let Err(error) = app.orchestrator.ensure_started() {
            app.spawn_errors.insert(pane_id.to_string(), error);
        }
        app.orchestrator
            .apply_settings(app.data.preferences.extra.get("orchestration"));
    }

    // Claude's own subagents join the jobs, under the Claude tab that started them.
    let native = native_subagent_jobs(app);
    if native.iter().any(|j| j.status == "running") {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(1000));
    }
    // Fresh snapshot only when the core published a new one.
    let orchestrator = Arc::clone(&app.orchestrator);
    let state = app.boards.entry(pane_id.to_string()).or_default();
    let revision = orchestrator.revision();
    if revision != state.revision {
        state.revision = revision;
        state.snapshot = Arc::new(parse_snapshot(orchestrator.board()));
    }
    let snapshot = Arc::clone(&state.snapshot);
    // Elapsed clocks tick once a second while something runs; nothing repaints otherwise.
    if snapshot.running > 0 {
        let now = ui.input(|i| i.time);
        if now - state.last_tick >= 1.0 {
            state.last_tick = now;
            orchestrator.refresh();
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(1000));
    }

    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        theme.bg_sunken,
        Stroke::NONE,
        StrokeKind::Inside,
    );
    // A board shows this project's planners only, and the jobs they delegated; a job with no
    // planner belongs to the project whose folder it ran in (`OrchestratorPane` jobs filter).
    let (project_tabs, project_cwd): (HashSet<String>, Option<String>) = app
        .data
        .projects
        .iter()
        .find(|p| p.terminals.iter().any(|t| t.id == pane_id))
        .map(|p| {
            let tabs = p
                .terminals
                .iter()
                .flat_map(|t| t.tabs.iter().map(|tab| tab.id.clone()))
                .collect();
            (tabs, p.default_cwd.clone())
        })
        .unwrap_or_default();
    let snapshot = Snapshot {
        jobs: snapshot
            .jobs
            .iter()
            .cloned()
            .chain(native)
            .filter(|job| match (&job.planner_id, &project_cwd) {
                (Some(planner), _) => project_tabs.contains(planner),
                (None, Some(cwd)) => job.cwd.starts_with(cwd.as_str()),
                (None, None) => true,
            })
            .collect(),
        planners: snapshot
            .planners
            .iter()
            .filter(|p| project_tabs.contains(&p.id))
            .cloned()
            .collect(),
        // A shell shows on its planner's project, or, with no planner here, where its folder is.
        shells: snapshot
            .shells
            .iter()
            .filter(|shell| match (shell.owner_id(), &project_cwd) {
                (Some(owner), _) if project_tabs.contains(owner) => true,
                (_, Some(cwd)) => path_under(&shell.cwd, cwd),
                (_, None) => true,
            })
            .cloned()
            .collect(),
        running: snapshot.running,
        queued: snapshot.queued,
        concurrency_limit: snapshot.concurrency_limit,
    };
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H));
    let mut groups = group_planners(&snapshot);
    // A Claude tab that only has subagents is no declared planner: name it like its tab.
    for group in &mut groups {
        if let Some(id) = &group.id {
            if group.label.as_deref() == Some(id.as_str()) {
                if let Some(tab) = app.find_tab(id) {
                    group.label = Some(app.tab_title(tab));
                    group.agent = Some(tab.agent.clone());
                }
            }
        }
    }
    // Shells with no planner on this board still get one.
    if groups.is_empty() && !snapshot.shells.is_empty() {
        groups.push(to_group(None, None, None, Vec::new()));
    }
    let own_running_shells = |id: &Option<String>| {
        snapshot
            .shells
            .iter()
            .filter(|shell| shell.running() && shell.owner_id() == id.as_deref())
            .count()
    };

    let mut close = false;
    header_ui(app, ui, header, &snapshot, &groups, &mut close);
    if close {
        app.act(Action::CloseTerminal(pane_id.to_string()));
    }
    let body = Rect::from_min_max(
        pos2(rect.left() + 1.0, header.bottom()),
        pos2(rect.right() - 1.0, rect.bottom() - 1.0),
    );

    let mut intents: Vec<Intent> = Vec::new();
    if !enabled || groups.is_empty() {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(body.shrink(32.0))
                .layout(Layout::top_down(Align::Center)),
        );
        child.add_space(body.height() * 0.28);
        child.label(
            RichText::new(icons::TYPE_HIERARCHY)
                .font(mono(30.0))
                .color(theme.fg_faint),
        );
        child.add_space(10.0);
        if enabled {
            child.label(
                RichText::new(t("orchestrator.emptyTitle"))
                    .font(bold(15.0))
                    .color(theme.fg),
            );
            child.add_space(4.0);
            child.add(
                egui::Label::new(
                    RichText::new(t("orchestrator.emptyBody"))
                        .font(sans(12.5))
                        .color(theme.fg_muted),
                )
                .wrap(),
            );
        } else {
            child.add(
                egui::Label::new(
                    RichText::new(t("orchestrator.disabled"))
                        .font(sans(12.5))
                        .color(theme.fg_muted),
                )
                .wrap(),
            );
            child.add_space(12.0);
            if widgets::primary_button(&mut child, &theme, t("settings.orchestrator")).clicked() {
                app.set_feature("orchestrator", true);
            }
        }
        border(ui, rect, &theme, focused);
        return;
    }

    let state = app.boards.get_mut(pane_id).expect("board state");
    // Until a tab is picked by hand, follow the planner with live work, then any with work.
    let preferred = || {
        groups
            .iter()
            .find(|g| g.jobs.iter().any(|j| j.can_stop()) || own_running_shells(&g.id) > 0)
            .or_else(|| groups.iter().find(|g| !g.jobs.is_empty()))
            .unwrap_or(&groups[0])
            .key()
    };
    let active_key = state
        .active
        .clone()
        .filter(|k| state.pinned && groups.iter().any(|g| &g.key() == k))
        .unwrap_or_else(preferred);
    state.active = Some(active_key.clone());
    let live: HashSet<String> = groups.iter().filter_map(|g| g.id.clone()).collect();
    let board_shells = shells_for_board(
        &snapshot.shells,
        Some(active_key.as_str()).filter(|key| !key.is_empty()),
        &live,
    );
    if let Some(active) = groups.iter_mut().find(|g| g.key() == active_key) {
        active.shells = board_shells;
    }

    // ---- planner tabs
    let tabs = Rect::from_min_size(body.min, vec2(body.width(), TABS_H));
    ui.painter().rect_filled(tabs, CornerRadius::ZERO, theme.bg);
    ui.painter().line_segment(
        [tabs.left_bottom(), tabs.right_bottom()],
        Stroke::new(1.0, theme.border),
    );
    let mut tui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(tabs.shrink2(vec2(8.0, 4.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    tui.set_clip_rect(tabs);
    for group in &groups {
        let selected = group.key() == active_key;
        let name = group
            .label
            .clone()
            .unwrap_or_else(|| t("orchestrator.noPlanner").to_string());
        let galley = tui
            .painter()
            .layout_no_wrap(name.clone(), sans(12.0), theme.fg);
        let w = galley.size().x.clamp(60.0, 200.0) + 58.0;
        let (r, resp) = tui.allocate_exact_size(vec2(w, 24.0), Sense::click());
        if selected || resp.hovered() {
            tui.painter().rect_filled(
                r,
                CornerRadius::same(5),
                if selected {
                    theme.panel
                } else {
                    theme.panel_hover
                },
            );
        }
        let icon = Rect::from_center_size(pos2(r.left() + 13.0, r.center().y), Vec2::splat(13.0));
        agent_glyph(&tui, &theme, group.agent.as_deref(), icon);
        let text = Rect::from_min_max(
            pos2(r.left() + 25.0, r.top()),
            pos2(r.right() - 30.0, r.bottom()),
        );
        tui.painter().with_clip_rect(text).text(
            pos2(text.left(), r.center().y),
            Align2::LEFT_CENTER,
            &name,
            sans(12.0),
            if selected { theme.fg } else { theme.fg_muted },
        );
        let alert = if selected {
            None
        } else {
            group.counts.attention()
        };
        let (badge, badge_color) = match alert {
            Some((lane, n)) => (n.to_string(), lane.color(&theme)),
            None => (
                (group.jobs.len() + own_running_shells(&group.id)).to_string(),
                theme.fg_faint,
            ),
        };
        tui.painter().text(
            pos2(r.right() - 10.0, r.center().y),
            Align2::RIGHT_CENTER,
            badge,
            mono(10.5),
            badge_color,
        );
        if resp.clicked() {
            state.active = Some(group.key());
            state.pinned = true;
            state.selected = None;
            state.moved = false;
            state.view = None;
        }
    }

    let group = groups
        .iter()
        .find(|g| g.key() == active_key)
        .unwrap_or(&groups[0]);
    let split = Rect::from_min_max(pos2(body.left(), tabs.bottom()), body.max);
    let rail_w = if state.rail_collapsed {
        30.0
    } else {
        RAIL_W.min(split.width() * 0.4)
    };
    let canvas = Rect::from_min_max(split.min, pos2(split.right() - rail_w, split.bottom()));
    let rail = Rect::from_min_max(pos2(canvas.right(), split.top()), split.max);

    // Each shell card's last line: read every couple of seconds while it runs, once otherwise.
    let now = ui.input(|i| i.time);
    for shell in &group.shells {
        let stale = state
            .shell_tails
            .get(&shell.id)
            .is_none_or(|(at, status, _)| {
                *status != shell.status || shell.running() && now - at >= 2.0
            });
        if stale {
            let tail = orchestrator.shell_tail(&shell.id, 3).unwrap_or_default();
            state
                .shell_tails
                .insert(shell.id.clone(), (now, shell.status.clone(), tail));
        }
    }
    if group.shells.iter().any(ShellInfo::running) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(2));
    }

    canvas_ui(
        ui,
        &theme,
        lang,
        pane_id,
        canvas,
        group,
        state,
        &mut intents,
    );
    rail_ui(
        ui,
        &theme,
        lang,
        rail,
        &snapshot,
        &groups,
        group,
        state,
        &mut intents,
    );

    let selected_job = state
        .selected
        .as_ref()
        .and_then(|id| snapshot.jobs.iter().find(|j| &j.id == id))
        .cloned();
    let selected_shell = state
        .selected
        .as_ref()
        .and_then(|id| group.shells.iter().find(|shell| &shell.id == id))
        .cloned();
    if let Some(job) = selected_job {
        inspector_ui(
            ui,
            &theme,
            lang,
            canvas,
            &job,
            &orchestrator,
            state,
            &mut intents,
        );
    } else if let Some(shell) = selected_shell {
        shell_inspector(
            ui,
            &theme,
            lang,
            canvas,
            &shell,
            &orchestrator,
            term_palette,
            term_font,
            &mut intents,
        );
    }

    // ---- apply
    for intent in intents {
        match intent {
            Intent::Select(id) => {
                let state = app.boards.get_mut(pane_id).expect("board state");
                if state.selected != id {
                    state.tab = InspectorTab::Report;
                    state.diff = None;
                    state.message.clear();
                }
                state.selected = id;
            }
            // Native subagents are only shown; the core doesn't know them.
            Intent::Answer(job, _) | Intent::Stop(job) | Intent::Restart(job)
                if job.starts_with("subagent:") => {}
            Intent::Answer(job, decision) => orchestrator.answer(job, decision),
            Intent::Stop(job) => orchestrator.cancel(job),
            Intent::Restart(job) => orchestrator.restart(job),
            Intent::Shell(shell, control) => orchestrator.shell_control(shell, control),
            Intent::ToggleRun(run) => {
                let state = app.boards.get_mut(pane_id).expect("board state");
                if !state.closed_runs.remove(&run) {
                    state.closed_runs.insert(run);
                }
            }
            Intent::RevealPlanner(planner) => {
                let found = app.data.projects.iter().find_map(|p| {
                    p.terminals
                        .iter()
                        .find(|term| term.tabs.iter().any(|tab| tab.id == planner))
                        .map(|term| (p.id.clone(), term.id.clone(), planner.clone()))
                });
                if let Some((project, terminal, tab)) = found {
                    app.act(Action::OpenProject(project));
                    app.act(Action::SelectSubTab {
                        terminal_id: terminal.clone(),
                        tab_id: tab,
                    });
                    app.act(Action::FocusTerminal(terminal));
                }
            }
        }
    }
    if let Some(error) = orchestrator.take_shell_error() {
        app.toasts.push((
            format!("{}: {error}", t("orchestrator.shell.failed")),
            std::time::Instant::now(),
        ));
    }
    border(ui, rect, &theme, focused);
}

fn border(ui: &egui::Ui, rect: Rect, theme: &Theme, focused: bool) {
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(
            1.0,
            if focused {
                theme.accent_border
            } else {
                theme.border
            },
        ),
        StrokeKind::Inside,
    );
}

fn agent_glyph(ui: &egui::Ui, theme: &Theme, agent: Option<&str>, rect: Rect) {
    match agent.and_then(|a| icons::agent_image(a, rect.width())) {
        Some(image) => image.paint_at(ui, rect),
        None => {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                icons::TYPE_HIERARCHY,
                mono(rect.width() * 0.85),
                theme.fg_muted,
            );
        }
    }
}

fn header_ui(
    app: &AletheApp,
    ui: &mut egui::Ui,
    header: Rect,
    snapshot: &Snapshot,
    groups: &[Group],
    close: &mut bool,
) {
    let theme = &app.theme;
    let lang = app.lang();
    let t = |key: &str| crate::i18n::tr(lang, key);
    ui.painter().rect_filled(
        header,
        CornerRadius {
            nw: 6,
            ne: 6,
            sw: 0,
            se: 0,
        },
        theme.bg_elevated,
    );
    ui.painter().line_segment(
        [header.left_bottom(), header.right_bottom()],
        Stroke::new(1.0, theme.border),
    );
    let mut hui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(header.shrink2(vec2(10.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    hui.set_clip_rect(header);
    let (icon, _) = hui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
    hui.painter()
        .rect_filled(icon, CornerRadius::same(5), theme.accent_soft);
    hui.painter().text(
        icon.center(),
        Align2::CENTER_CENTER,
        icons::TYPE_HIERARCHY,
        mono(13.0),
        theme.accent,
    );
    hui.label(
        RichText::new(t("orchestrator.title"))
            .font(bold(13.0))
            .color(theme.fg),
    );

    hui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if widgets::icon_button(ui, theme, icons::CLOSE, t("pane.close"), 22.0).clicked() {
            *close = true;
        }
        let chip = |ui: &mut egui::Ui, text: String, color: Color32| {
            ui.label(RichText::new(text).font(mono(11.0)).color(color));
        };
        chip(
            ui,
            fill(
                t("orchestrator.limit"),
                &[("count", snapshot.concurrency_limit.to_string())],
            ),
            theme.fg_faint,
        );
        chip(
            ui,
            fill(
                t("orchestrator.queued"),
                &[("count", snapshot.queued.to_string())],
            ),
            theme.fg_faint,
        );
        chip(
            ui,
            fill(
                t("orchestrator.running"),
                &[("count", snapshot.running.to_string())],
            ),
            if snapshot.running > 0 {
                theme.working
            } else {
                theme.fg_faint
            },
        );
        let all: Vec<&Job> = groups.iter().flat_map(|g| g.jobs.iter().copied()).collect();
        let counts = Counts::of(&all);
        if counts.get(Lane::Interrupted) > 0 {
            chip(
                ui,
                fill(
                    t("orchestrator.runInterrupted"),
                    &[("count", counts.get(Lane::Interrupted).to_string())],
                ),
                theme.waiting,
            );
        }
        if counts.get(Lane::Blocked) > 0 {
            chip(
                ui,
                fill(
                    t("orchestrator.runBlocked"),
                    &[("count", counts.get(Lane::Blocked).to_string())],
                ),
                theme.waiting,
            );
        }
        // Spend per worker provider, in the order first seen.
        let mut spend: Vec<(String, f64, f64, usize)> = Vec::new();
        for job in &snapshot.jobs {
            let tokens = job.total_tokens();
            if tokens <= 0.0 && job.cost_usd.is_none() {
                continue;
            }
            let entry = match spend.iter_mut().position(|s| s.0 == job.agent) {
                Some(i) => &mut spend[i],
                None => {
                    spend.push((job.agent.clone(), 0.0, 0.0, 0));
                    spend.last_mut().unwrap()
                }
            };
            entry.1 += tokens;
            if let Some(cost) = job.cost_usd {
                entry.2 += cost;
                entry.3 += 1;
            }
        }
        for (agent, tokens, cost, priced) in spend.iter().rev() {
            let price = if *priced > 0 {
                fmt_usd(*cost)
            } else {
                t("orchestrator.noPrice").to_string()
            };
            ui.label(RichText::new(price).font(bold(11.0)).color(theme.fg))
                .on_hover_text(format!(
                    "{agent}: {}",
                    format_tokens(*tokens).unwrap_or_else(|| "0".into())
                ));
            ui.label(RichText::new(agent).font(mono(11.0)).color(theme.fg_muted));
            let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
            agent_glyph(ui, theme, Some(agent), r);
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn canvas_ui(
    ui: &mut egui::Ui,
    theme: &Theme,
    lang: crate::i18n::Lang,
    pane_id: &str,
    canvas: Rect,
    group: &Group,
    state: &mut BoardState,
    intents: &mut Vec<Intent>,
) {
    let t = |key: &str| crate::i18n::tr(lang, key);
    let graph = layout(&group.runs, &group.shells, group.id.is_some());

    // Auto-fit until the person pans or zooms; refit whenever the board changes size.
    if !state.moved || state.view.is_none() || state.graph_size != graph.size && !state.moved {
        state.view = Some(fit(graph.size, canvas.size()));
    }
    state.graph_size = graph.size;
    let mut view = state.view.unwrap_or(View {
        scale: 1.0,
        offset: Vec2::ZERO,
    });

    let response = ui.interact(
        canvas,
        egui::Id::unique(("orch-canvas", pane_id)),
        Sense::click_and_drag(),
    );
    if response.dragged() {
        view.offset += response.drag_delta();
        state.moved = true;
    }
    if response.hovered() {
        let (zoom, scroll, pointer) =
            ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos()));
        if zoom != 1.0 {
            let at = pointer
                .map(|p| p - canvas.min)
                .unwrap_or(canvas.size() / 2.0);
            view = zoom_at(view, zoom, at);
            state.moved = true;
        } else if scroll != Vec2::ZERO {
            view.offset += scroll;
            state.moved = true;
        }
    }
    if response.clicked() {
        intents.push(Intent::Select(None));
    }
    state.view = Some(view);

    let painter = ui.painter().with_clip_rect(canvas);
    painter.rect_filled(canvas, CornerRadius::ZERO, theme.bg_sunken);
    // Dot texture, kept well under the weight of a real edge.
    let spacing = DOT_SPACING * view.scale;
    if spacing >= 6.0 {
        let dot = theme.border_strong.gamma_multiply(0.4);
        let start_x = canvas.left() + view.offset.x.rem_euclid(spacing);
        let start_y = canvas.top() + view.offset.y.rem_euclid(spacing);
        let mut shapes = Vec::new();
        let mut y = start_y;
        while y < canvas.bottom() {
            let mut x = start_x;
            while x < canvas.right() {
                shapes.push(Shape::circle_filled(pos2(x, y), 1.0, dot));
                x += spacing;
            }
            y += spacing;
        }
        painter.extend(shapes);
    }

    let origin = canvas.min + view.offset;
    let s = view.scale;
    let to_screen = |p: Pos2| origin + p.to_vec2() * s;
    let rect_screen = |r: Rect| Rect::from_min_max(to_screen(r.min), to_screen(r.max));

    if group.runs.is_empty() && group.shells.is_empty() {
        painter.text(
            canvas.center(),
            Align2::CENTER_CENTER,
            t("orchestrator.emptyPlanner"),
            sans(12.5),
            theme.fg_faint,
        );
    }

    // Edges under the nodes.
    let time = ui.input(|i| i.time);
    let mut flowing = false;
    for edge in &graph.edges {
        let pts: Vec<Pos2> = connector(edge.from, edge.to)
            .into_iter()
            .map(to_screen)
            .collect();
        let selected = edge.worker.is_some() && edge.worker == state.selected;
        let color = match edge.lane {
            Lane::Finished => theme.border_strong,
            lane => lane.color(theme),
        };
        let width = if selected || edge.lane == Lane::Blocked {
            2.0
        } else if edge.lane == Lane::Running {
            1.6
        } else {
            1.0
        };
        let stroke = Stroke::new(width * s.max(0.6), color);
        match edge.lane {
            Lane::Queued => painter.extend(Shape::dashed_line(&pts, stroke, 3.0 * s, 4.0 * s)),
            Lane::Interrupted => {
                painter.extend(Shape::dotted_line(&pts, color, 4.0 * s, width * s * 0.6))
            }
            Lane::Running => {
                flowing |= pts.iter().any(|p| canvas.contains(*p));
                flowing_line(&painter, &pts, stroke, s, time);
            }
            _ => {
                painter.add(Shape::line(pts, stroke));
            }
        }
    }
    // About 30 frames a second, and only while a running edge is on screen.
    if flowing {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }

    for node in &graph.nodes {
        let r = rect_screen(node.rect);
        if !r.intersects(canvas) {
            continue;
        }
        match node.kind {
            NodeKind::Planner => {
                planner_node(ui, &painter, theme, lang, pane_id, r, s, group, intents)
            }
            NodeKind::Run(ri) => run_node(&painter, theme, lang, r, s, &group.runs[ri]),
            NodeKind::Worker(ri, wi) => {
                let job = group.runs[ri].jobs[wi];
                worker_node(
                    ui,
                    &painter,
                    theme,
                    lang,
                    pane_id,
                    r,
                    s,
                    job,
                    state.selected.as_deref() == Some(&job.id),
                    intents,
                );
            }
            NodeKind::ShellGroup(attached) => {
                let members: Vec<&ShellInfo> = group
                    .shells
                    .iter()
                    .filter(|shell| shell.attached == attached)
                    .collect();
                shell_group_node(&painter, theme, lang, r, s, attached, &members);
            }
            NodeKind::Shell(si) => {
                let shell = &group.shells[si];
                shell_node(
                    ui,
                    &painter,
                    theme,
                    lang,
                    pane_id,
                    r,
                    s,
                    shell,
                    state.shell_tails.get(&shell.id).map(|t| t.2.as_str()),
                    state.selected.as_deref() == Some(&shell.id),
                    intents,
                );
            }
        }
    }

    // ---- overlays: hint + zoom control
    painter.text(
        pos2(canvas.left() + 14.0, canvas.bottom() - 14.0),
        Align2::LEFT_BOTTOM,
        t("orchestrator.canvasHint"),
        sans(11.0),
        theme.fg_faint,
    );
    let ctl = Rect::from_min_size(
        pos2(canvas.right() - 150.0, canvas.bottom() - 36.0),
        vec2(138.0, 26.0),
    );
    painter.rect(
        ctl,
        CornerRadius::same(6),
        theme.bg_elevated,
        Stroke::new(1.0, theme.border),
        StrokeKind::Inside,
    );
    let mut cui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(ctl.shrink2(vec2(4.0, 2.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    cui.spacing_mut().item_spacing.x = 2.0;
    let zoom_by = |factor: f32, state: &mut BoardState| {
        let v = state.view.unwrap_or(view);
        state.view = Some(zoom_at(v, factor, canvas.size() / 2.0));
        state.moved = true;
    };
    if widgets::icon_button(
        &mut cui,
        theme,
        icons::ZOOM_OUT,
        t("orchestrator.zoomOut"),
        20.0,
    )
    .clicked()
    {
        zoom_by(1.0 / ZOOM_STEP, state);
    }
    if cui
        .add(
            egui::Button::new(
                RichText::new(t("orchestrator.zoomFit"))
                    .font(sans(11.0))
                    .color(theme.fg_muted),
            )
            .frame(false),
        )
        .clicked()
    {
        state.moved = false;
        state.view = Some(fit(graph.size, canvas.size()));
    }
    cui.label(
        RichText::new(format!("{:.0}%", view.scale * 100.0))
            .font(mono(10.5))
            .color(theme.fg_faint),
    );
    if widgets::icon_button(
        &mut cui,
        theme,
        icons::ZOOM_IN,
        t("orchestrator.zoomIn"),
        20.0,
    )
    .clicked()
    {
        zoom_by(ZOOM_STEP, state);
    }
}

fn card_frame(
    painter: &egui::Painter,
    theme: &Theme,
    r: Rect,
    s: f32,
    fill: Color32,
    stroke: Color32,
) {
    painter.rect(
        r,
        CornerRadius::same((8.0 * s).round().clamp(2.0, 12.0) as u8),
        fill,
        Stroke::new(1.0, stroke),
        StrokeKind::Inside,
    );
    let _ = theme;
}

fn dot(painter: &egui::Painter, center: Pos2, s: f32, lane: Lane, theme: &Theme) {
    let radius = 3.0 * s.max(0.7);
    match lane {
        Lane::Interrupted => {
            painter.circle_stroke(center, radius - 0.5, Stroke::new(1.5, theme.waiting));
        }
        lane => {
            painter.circle_filled(center, radius, lane.color(theme));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn planner_node(
    ui: &egui::Ui,
    painter: &egui::Painter,
    theme: &Theme,
    lang: crate::i18n::Lang,
    pane_id: &str,
    r: Rect,
    s: f32,
    group: &Group,
    intents: &mut Vec<Intent>,
) {
    let t = |key: &str| crate::i18n::tr(lang, key);
    let resp = ui.interact(
        r,
        egui::Id::unique(("orch-planner", pane_id)),
        Sense::click(),
    );
    card_frame(
        painter,
        theme,
        r,
        s,
        if resp.hovered() {
            theme.panel_hover
        } else {
            theme.bg_elevated
        },
        theme.border_strong,
    );
    let pad = 12.0 * s;
    let icon = Rect::from_center_size(
        pos2(r.left() + pad + 9.0 * s, r.center().y),
        Vec2::splat(17.0 * s),
    );
    agent_glyph(ui, theme, group.agent.as_deref(), icon);
    let x = icon.right() + 10.0 * s;
    painter.text(
        pos2(x, r.center().y - 2.0 * s),
        Align2::LEFT_BOTTOM,
        t("orchestrator.plannerEyebrow").to_uppercase(),
        mono(9.5 * s),
        theme.fg_faint,
    );
    let name = group
        .label
        .clone()
        .unwrap_or_else(|| t("orchestrator.noPlanner").to_string());
    painter
        .with_clip_rect(Rect::from_min_max(
            pos2(x, r.top()),
            pos2(r.right() - 70.0 * s, r.bottom()),
        ))
        .text(
            pos2(x, r.center().y + 1.0 * s),
            Align2::LEFT_TOP,
            name,
            bold(13.0 * s),
            theme.fg,
        );
    painter.text(
        pos2(r.right() - pad, r.center().y),
        Align2::RIGHT_CENTER,
        fill(
            t("orchestrator.runCount"),
            &[("count", group.runs.len().to_string())],
        ),
        mono(10.5 * s),
        theme.fg_muted,
    );
    if resp.clicked() {
        if let Some(id) = &group.id {
            intents.push(Intent::RevealPlanner(id.clone()));
        }
    }
}

fn run_node(
    painter: &egui::Painter,
    theme: &Theme,
    lang: crate::i18n::Lang,
    r: Rect,
    s: f32,
    run: &Run,
) {
    let t = |key: &str| crate::i18n::tr(lang, key);
    let fill_color = match run.state {
        Lane::Failed => blend(theme.offline, theme.bg_elevated, 0.14),
        Lane::Blocked => blend(theme.waiting, theme.bg_elevated, 0.16),
        _ => theme.bg_elevated,
    };
    card_frame(painter, theme, r, s, fill_color, theme.border_strong);
    let pad = 13.0 * s;
    let mut y = r.top() + 11.0 * s;
    dot(
        painter,
        pos2(r.left() + pad + 3.0 * s, y + 6.0 * s),
        s,
        run.state,
        theme,
    );
    painter.text(
        pos2(r.left() + pad + 12.0 * s, y),
        Align2::LEFT_TOP,
        t("orchestrator.runEyebrow").to_uppercase(),
        mono(9.5 * s),
        theme.fg_faint,
    );
    let state_color = match run.state {
        Lane::Finished => theme.fg_faint,
        lane => lane.color(theme),
    };
    painter.text(
        pos2(r.right() - pad, y),
        Align2::RIGHT_TOP,
        t(run.state.key()),
        mono(10.0 * s),
        state_color,
    );
    y += 19.0 * s;
    painter.with_clip_rect(r.shrink(pad)).text(
        pos2(r.left() + pad, y),
        Align2::LEFT_TOP,
        &run.label,
        bold(13.0 * s),
        theme.fg,
    );
    y += 24.0 * s;
    let total = run.jobs.len();
    let mut foot = fill(
        t("orchestrator.workerCount"),
        &[("count", total.to_string())],
    );
    if let Some(rules) = &run.rules {
        foot.push_str(&format!(" · {rules}"));
    }
    if run.counts.get(Lane::Blocked) > 0 {
        foot.push_str(&format!(
            " · {}",
            fill(
                t("orchestrator.runBlocked"),
                &[("count", run.counts.get(Lane::Blocked).to_string())]
            )
        ));
    }
    painter
        .with_clip_rect(Rect::from_min_max(
            r.min,
            pos2(r.right() - 80.0 * s, r.bottom()),
        ))
        .text(
            pos2(r.left() + pad, y),
            Align2::LEFT_TOP,
            foot,
            mono(10.5 * s),
            theme.fg_muted,
        );
    painter.text(
        pos2(r.right() - pad, y),
        Align2::RIGHT_TOP,
        fill(
            t("orchestrator.runDone"),
            &[
                ("done", run.counts.get(Lane::Finished).to_string()),
                ("total", total.to_string()),
            ],
        ),
        bold(10.5 * s),
        theme.fg,
    );
}

fn shell_group_node(
    painter: &egui::Painter,
    theme: &Theme,
    lang: crate::i18n::Lang,
    r: Rect,
    s: f32,
    attached: bool,
    members: &[&ShellInfo],
) {
    let t = |key: &str| crate::i18n::tr(lang, key);
    card_frame(painter, theme, r, s, theme.bg_elevated, theme.border_strong);
    let pad = 13.0 * s;
    let y = r.top() + 11.0 * s;
    let lane = if members.iter().any(|shell| shell.running()) {
        Lane::Running
    } else {
        Lane::Finished
    };
    dot(
        painter,
        pos2(r.left() + pad + 3.0 * s, y + 6.0 * s),
        s,
        lane,
        theme,
    );
    painter.text(
        pos2(r.left() + pad + 12.0 * s, y),
        Align2::LEFT_TOP,
        t("orchestrator.shellsEyebrow").to_uppercase(),
        mono(9.5 * s),
        theme.fg_faint,
    );
    let count = members.len();
    painter.text(
        pos2(r.right() - pad, y),
        Align2::RIGHT_TOP,
        t(if count == 1 {
            "orchestrator.shellCount"
        } else {
            "orchestrator.shellCountPlural"
        })
        .replace("{count}", &count.to_string()),
        mono(10.0 * s),
        theme.fg_muted,
    );
    painter.with_clip_rect(r.shrink(pad * 0.5)).text(
        pos2(r.left() + pad, y + 19.0 * s),
        Align2::LEFT_TOP,
        t(if attached {
            "orchestrator.shellsLabel"
        } else {
            "orchestrator.shellsOrphanLabel"
        }),
        bold(13.0 * s),
        theme.fg,
    );
}

/// The controls a shell offers (`shellControls`): a finished one has no process left to stop.
fn shell_controls(shell: &ShellInfo) -> &'static [(&'static str, &'static str, ShellControl)] {
    if shell.running() {
        &[
            (
                "orchestrator.shell.stop",
                icons::DEBUG_STOP,
                ShellControl::Stop,
            ),
            (
                "orchestrator.shell.restart",
                icons::DEBUG_RESTART,
                ShellControl::Restart,
            ),
        ]
    } else {
        &[
            (
                "orchestrator.shell.play",
                icons::PLAY,
                ShellControl::Restart,
            ),
            (
                "orchestrator.shell.remove",
                icons::TRASH,
                ShellControl::Remove,
            ),
        ]
    }
}

/// The canvas card for a shell (`ShellNode`): same footprint as a worker, its last output line
/// read every couple of seconds while it runs.
#[allow(clippy::too_many_arguments)]
fn shell_node(
    ui: &egui::Ui,
    painter: &egui::Painter,
    theme: &Theme,
    lang: crate::i18n::Lang,
    pane_id: &str,
    r: Rect,
    s: f32,
    shell: &ShellInfo,
    tail: Option<&str>,
    selected: bool,
    intents: &mut Vec<Intent>,
) {
    let t = |key: &str| crate::i18n::tr(lang, key);
    let lane = shell.lane();
    let resp = ui.interact(
        r,
        egui::Id::unique(("orch-shell", pane_id, &shell.id)),
        Sense::click(),
    );
    let dim = !shell.running() && !selected && !resp.hovered();
    let fade = |c: Color32| if dim { c.gamma_multiply(0.58) } else { c };
    let stroke = if selected {
        theme.accent_border
    } else if resp.hovered() {
        theme.border_strong
    } else {
        theme.border
    };
    card_frame(painter, theme, r, s, fade(theme.bg_elevated), fade(stroke));
    if selected {
        painter.rect_stroke(
            r.expand(1.0),
            CornerRadius::same((9.0 * s) as u8),
            Stroke::new(1.0, theme.accent.gamma_multiply(0.5)),
            StrokeKind::Outside,
        );
    }
    let pad = 11.0 * s;
    let head_y = r.top() + 17.0 * s;
    painter.text(
        pos2(r.left() + pad, head_y),
        Align2::LEFT_CENTER,
        icons::TERMINAL,
        mono(12.0 * s),
        fade(theme.fg_muted),
    );
    dot(
        painter,
        pos2(r.left() + pad + 22.0 * s, head_y),
        s,
        lane,
        theme,
    );

    // Controls, right to left.
    let controls = shell_controls(shell);
    let mut x = r.right() - pad - 9.0 * s;
    for (key, icon, control) in controls.iter().rev() {
        let button = Rect::from_center_size(pos2(x, head_y), Vec2::splat(18.0 * s));
        let button_resp = ui.interact(
            button,
            egui::Id::unique(("orch-shell-ctl", pane_id, &shell.id, *key)),
            Sense::click(),
        );
        if button_resp.hovered() {
            painter.rect_filled(button, CornerRadius::same(4), theme.panel_hover);
        }
        painter.text(
            button.center(),
            Align2::CENTER_CENTER,
            *icon,
            mono(11.0 * s),
            theme.fg_muted,
        );
        if button_resp.on_hover_text(t(key)).clicked() {
            intents.push(Intent::Shell(shell.id.clone(), *control));
        }
        x -= 22.0 * s;
    }
    painter
        .with_clip_rect(Rect::from_min_max(r.min, pos2(x - 4.0 * s, r.bottom())))
        .text(
            pos2(r.left() + pad + 30.0 * s, head_y),
            Align2::LEFT_CENTER,
            &shell.name,
            bold(11.5 * s),
            fade(theme.fg),
        );

    let line = |y: f32, text: &str, font: egui::FontId, color: Color32| {
        painter
            .with_clip_rect(Rect::from_min_max(
                pos2(r.left() + pad, y),
                pos2(r.right() - pad, y + 16.0 * s),
            ))
            .text(pos2(r.left() + pad, y), Align2::LEFT_TOP, text, font, color);
    };
    let mut y = r.top() + 31.0 * s;
    line(y, &shell.command, mono(10.5 * s), fade(theme.fg_muted));
    y += 18.0 * s;
    if let Some(last) = tail.and_then(|text| text.lines().rev().find(|l| !l.trim().is_empty())) {
        line(y, last.trim(), mono(10.0 * s), fade(theme.fg_faint));
    }
    y += 16.0 * s;
    let mut meta = shell.status_text(lang);
    if let Some(elapsed) = format_elapsed(shell.elapsed()) {
        meta.push_str(&format!("  ·  {elapsed}"));
    }
    line(
        y,
        &meta,
        mono(10.0 * s),
        match lane {
            Lane::Finished => fade(theme.fg_faint),
            lane => lane.color(theme),
        },
    );

    let resp = resp.on_hover_text(format!("{}\n{}", shell.command, shell.cwd));
    if resp.clicked() {
        intents.push(Intent::Select(Some(shell.id.clone())));
    }
    let id = shell.id.clone();
    resp.context_menu(|ui| {
        if ui.button(t("orchestrator.menuOpen")).clicked() {
            intents.push(Intent::Select(Some(id.clone())));
            ui.close();
        }
        for (key, _, control) in controls {
            if ui.button(t(key)).clicked() {
                intents.push(Intent::Shell(id.clone(), *control));
                ui.close();
            }
        }
    });
}

/// A shell in the inspector: its live terminal while the process (or its last screen) is around,
/// which also takes typing; the kept tail once it was stopped.
#[allow(clippy::too_many_arguments)]
fn shell_inspector(
    ui: &mut egui::Ui,
    theme: &Theme,
    lang: crate::i18n::Lang,
    canvas: Rect,
    shell: &ShellInfo,
    orchestrator: &Arc<crate::orchestrator::Orchestrator>,
    palette: &crate::theme::TermPalette,
    font_size: f32,
    intents: &mut Vec<Intent>,
) {
    let t = |key: &str| crate::i18n::tr(lang, key);
    let width = (canvas.width() * 0.66).clamp(300.0, 620.0);
    let panel = Rect::from_min_max(
        pos2(canvas.right() - width - 8.0, canvas.top() + 8.0),
        pos2(canvas.right() - 8.0, canvas.bottom() - 8.0),
    );
    let _ = ui.interact(
        panel,
        egui::Id::unique(("orch-shell-inspector", &shell.id)),
        Sense::click_and_drag(),
    );
    ui.painter().rect(
        panel,
        CornerRadius::same(10),
        theme.bg_elevated,
        Stroke::new(1.0, theme.border_strong),
        StrokeKind::Inside,
    );
    let mut pui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(panel.shrink(12.0))
            .layout(Layout::top_down(Align::Min)),
    );
    pui.set_clip_rect(panel);
    pui.horizontal(|ui| {
        ui.label(
            RichText::new(icons::TERMINAL)
                .font(mono(13.0))
                .color(theme.fg_muted),
        );
        ui.label(RichText::new(&shell.name).font(bold(13.0)).color(theme.fg));
        ui.label(
            RichText::new(shell.status_text(lang))
                .font(mono(11.0))
                .color(match shell.lane() {
                    Lane::Finished => theme.fg_faint,
                    lane => lane.color(theme),
                }),
        );
        if let Some(elapsed) = format_elapsed(shell.elapsed()) {
            ui.label(
                RichText::new(elapsed)
                    .font(mono(11.0))
                    .color(theme.fg_faint),
            );
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::icon_button(
                ui,
                theme,
                icons::CLOSE,
                t("orchestrator.inspectorClose"),
                22.0,
            )
            .clicked()
            {
                intents.push(Intent::Select(None));
            }
            for (key, icon, control) in shell_controls(shell).iter().rev() {
                if widgets::icon_button(ui, theme, icon, t(key), 22.0).clicked() {
                    intents.push(Intent::Shell(shell.id.clone(), *control));
                }
            }
        });
    });
    pui.label(
        RichText::new(&shell.command)
            .font(mono(11.0))
            .color(theme.fg),
    );
    pui.label(
        RichText::new(t("orchestrator.shellCwd").replace("{path}", &shell.cwd))
            .font(mono(10.5))
            .color(theme.fg_faint),
    );
    pui.add_space(6.0);
    let body = Rect::from_min_max(
        pos2(panel.left() + 8.0, pui.cursor().top()),
        pos2(panel.right() - 8.0, panel.bottom() - 8.0),
    );
    if body.height() < 40.0 {
        return;
    }
    match orchestrator.shell_session(&shell.pty_id) {
        Some(session) => {
            ui.painter()
                .rect_filled(body, CornerRadius::same(6), palette.background.c());
            let (cell_w, cell_h) = crate::terminal::view::cell_metrics(ui.ctx(), font_size);
            let inner = body.shrink(crate::terminal::view::PADDING);
            session.resize(
                crate::terminal::TermSize {
                    cols: ((inner.width() / cell_w).floor() as usize).max(20),
                    lines: ((inner.height() / cell_h).floor() as usize).max(4),
                },
                cell_w,
                cell_h,
            );
            crate::terminal::view::show(
                ui,
                body,
                egui::Id::unique(("orch-shell-term", &shell.id)),
                &session,
                palette,
                font_size,
                false,
            );
        }
        None => {
            let text = orchestrator.shell_tail(&shell.id, 200).unwrap_or_default();
            let mut tui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(body)
                    .layout(Layout::top_down(Align::Min)),
            );
            tui.set_clip_rect(body);
            egui::ScrollArea::vertical()
                .id_salt(("orch-shell-tail", &shell.id))
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(&mut tui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(if text.is_empty() {
                                t("orchestrator.shell.noOutput").to_string()
                            } else {
                                text
                            })
                            .font(mono(11.0))
                            .color(theme.fg_muted),
                        )
                        .wrap(),
                    );
                });
        }
    }
}

fn blend(a: Color32, b: Color32, amount: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 * amount + y as f32 * (1.0 - amount)).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

#[allow(clippy::too_many_arguments)]
fn worker_node(
    ui: &egui::Ui,
    painter: &egui::Painter,
    theme: &Theme,
    lang: crate::i18n::Lang,
    pane_id: &str,
    r: Rect,
    s: f32,
    job: &Job,
    selected: bool,
    intents: &mut Vec<Intent>,
) {
    let t = |key: &str| crate::i18n::tr(lang, key);
    let lane = job.lane();
    let resp = ui.interact(
        r,
        egui::Id::unique(("orch-worker", pane_id, &job.id)),
        Sense::click(),
    );
    // Finished workers fade back until hovered or selected.
    let dim = lane == Lane::Finished && !selected && !resp.hovered();
    let fade = |c: Color32| if dim { c.gamma_multiply(0.58) } else { c };
    let stroke = if selected {
        theme.accent_border
    } else if resp.hovered() {
        theme.border_strong
    } else {
        theme.border
    };
    card_frame(painter, theme, r, s, fade(theme.bg_elevated), fade(stroke));
    if selected {
        painter.rect_stroke(
            r.expand(1.0),
            CornerRadius::same((9.0 * s) as u8),
            Stroke::new(1.0, theme.accent.gamma_multiply(0.5)),
            StrokeKind::Outside,
        );
    }
    let pad = 11.0 * s;
    let mut y = r.top() + 10.0 * s;

    // Head: glyph, status dot, id, elapsed.
    let icon = Rect::from_min_size(pos2(r.left() + pad, y), Vec2::splat(14.0 * s));
    agent_glyph(ui, theme, Some(&job.agent), icon);
    dot(
        painter,
        pos2(icon.right() + 8.0 * s, icon.center().y),
        s,
        lane,
        theme,
    );
    painter.text(
        pos2(icon.right() + 16.0 * s, icon.center().y),
        Align2::LEFT_CENTER,
        job.label(),
        bold(11.5 * s),
        fade(theme.fg),
    );
    if let Some(elapsed) = format_elapsed(job.seconds) {
        let x = if job.can_stop() {
            r.right() - pad - 22.0 * s
        } else {
            r.right() - pad
        };
        painter.text(
            pos2(x, icon.center().y),
            Align2::RIGHT_CENTER,
            elapsed,
            mono(10.5 * s),
            fade(theme.fg_faint),
        );
    }
    // Stop control.
    if job.can_stop() {
        let stop = Rect::from_center_size(
            pos2(r.right() - pad - 7.0 * s, icon.center().y),
            Vec2::splat(18.0 * s),
        );
        let stop_resp = ui.interact(
            stop,
            egui::Id::unique(("orch-stop", pane_id, &job.id)),
            Sense::click(),
        );
        if stop_resp.hovered() {
            painter.rect_filled(stop, CornerRadius::same(4), theme.panel_hover);
        }
        painter.text(
            stop.center(),
            Align2::CENTER_CENTER,
            icons::DEBUG_STOP,
            mono(11.0 * s),
            theme.fg_muted,
        );
        if stop_resp
            .on_hover_text(t("orchestrator.stopWorker"))
            .clicked()
        {
            intents.push(Intent::Stop(job.id.clone()));
        }
    }
    y += 22.0 * s;

    // Live line.
    if let Some(live) = latest_line(&job.summary).or_else(|| latest_line(&job.spec)) {
        painter
            .with_clip_rect(Rect::from_min_max(
                pos2(r.left() + pad, y),
                pos2(r.right() - pad, y + 18.0 * s),
            ))
            .text(
                pos2(r.left() + pad, y),
                Align2::LEFT_TOP,
                live,
                sans(12.0 * s),
                fade(theme.fg_muted),
            );
    }
    y += 22.0 * s;

    // Meta chips.
    let mut meta: Vec<String> = vec![t(&format!("orchestrator.status.{}", job.status)).to_string()];
    if let Some(role) = &job.role {
        meta.push(role.clone());
    }
    meta.push(format!(
        "{} · {}",
        job.model
            .clone()
            .unwrap_or_else(|| t("orchestrator.cliDefault").to_string()),
        job.effort
            .clone()
            .unwrap_or_else(|| t("orchestrator.cliDefault").to_string())
    ));
    if job.read_only {
        meta.push(t("orchestrator.readOnly").to_string());
    }
    if let Some(share) = job.context_share() {
        meta.push(fill(
            t("orchestrator.contextChip"),
            &[("value", share.to_string())],
        ));
    }
    if let Some(tokens) = format_tokens(job.total_tokens()) {
        meta.push(tokens);
        meta.push(
            job.cost_usd
                .map(fmt_usd)
                .unwrap_or_else(|| t("orchestrator.noPrice").to_string()),
        );
    }
    if job.worktree.is_some() {
        meta.push(format!(
            "{} {}",
            icons::GIT_BRANCH,
            t("orchestrator.isolated")
        ));
    }
    if job.has_diff {
        meta.push(t("orchestrator.hasDiff").to_string());
    }
    let status_color = match lane {
        Lane::Finished => theme.fg_faint,
        lane => lane.color(theme),
    };
    let meta_rect =
        Rect::from_min_max(pos2(r.left() + pad, y), pos2(r.right() - pad, y + 16.0 * s));
    let mut job_text = egui::text::LayoutJob::default();
    for (i, chip) in meta.iter().enumerate() {
        if i > 0 {
            job_text.append(
                "  ·  ",
                0.0,
                egui::TextFormat::simple(mono(10.0 * s), fade(theme.fg_faint)),
            );
        }
        let color = if i == 0 { status_color } else { theme.fg_faint };
        job_text.append(
            chip,
            0.0,
            egui::TextFormat::simple(mono(10.0 * s), fade(color)),
        );
    }
    job_text.wrap.max_width = meta_rect.width();
    job_text.wrap.max_rows = 1;
    job_text.wrap.break_anywhere = true;
    let galley = ui.fonts_mut(|f| f.layout_job(job_text));
    painter
        .with_clip_rect(meta_rect)
        .galley(meta_rect.min, galley, theme.fg_faint);
    y += 24.0 * s;

    // Approval ask (`ApprovalAsk`).
    if job.status == "blocked" {
        if let Some(ask) = &job.pending_approval {
            let block = Rect::from_min_max(
                pos2(r.left() + 6.0 * s, y),
                pos2(r.right() - 6.0 * s, y + 86.0 * s),
            );
            painter.rect_filled(
                block,
                CornerRadius::same(5),
                blend(theme.waiting, theme.bg_elevated, 0.12),
            );
            let mut yy = block.top() + 7.0 * s;
            let question = if ask.kind == "fileChange" {
                t("orchestrator.askFileChange")
            } else {
                t("orchestrator.askCommand")
            };
            painter.text(
                pos2(block.left() + 8.0 * s, yy),
                Align2::LEFT_TOP,
                question,
                sans(11.5 * s),
                theme.waiting,
            );
            yy += 17.0 * s;
            if let Some(command) = ask.command.as_ref().or(ask.reason.as_ref()) {
                painter.with_clip_rect(block.shrink(6.0 * s)).text(
                    pos2(block.left() + 8.0 * s, yy),
                    Align2::LEFT_TOP,
                    command,
                    mono(10.5 * s),
                    theme.fg,
                );
            }
            yy += 22.0 * s;
            let decisions: [(&str, &'static str); 4] = [
                ("orchestrator.answerAccept", "accept"),
                ("orchestrator.answerSession", "acceptForSession"),
                ("orchestrator.answerDecline", "decline"),
                ("orchestrator.answerAbort", "abort"),
            ];
            let bw = (block.width() - 16.0 * s - 3.0 * 4.0 * s) / 4.0;
            for (i, (label, decision)) in decisions.iter().enumerate() {
                let b = Rect::from_min_size(
                    pos2(block.left() + 8.0 * s + i as f32 * (bw + 4.0 * s), yy),
                    vec2(bw, 24.0 * s),
                );
                let br = ui.interact(
                    b,
                    egui::Id::unique(("orch-answer", pane_id, &job.id, i)),
                    Sense::click(),
                );
                let primary = i == 0;
                painter.rect(
                    b,
                    CornerRadius::same(4),
                    if primary {
                        theme.accent
                    } else if br.hovered() {
                        theme.panel_hover
                    } else {
                        theme.panel
                    },
                    Stroke::new(1.0, theme.border),
                    StrokeKind::Inside,
                );
                painter.with_clip_rect(b).text(
                    b.center(),
                    Align2::CENTER_CENTER,
                    t(label),
                    sans(10.5 * s),
                    if primary { theme.accent_on } else { theme.fg },
                );
                if br.clicked() {
                    intents.push(Intent::Answer(job.id.clone(), decision));
                }
            }
            y = block.bottom() + 6.0 * s;
        }
    }

    // Failure bar.
    if job.status == "failed" {
        if let Some(outcome) = &job.outcome {
            let bar = Rect::from_min_max(
                pos2(r.left() + 6.0 * s, y),
                pos2(r.right() - 6.0 * s, y + 20.0 * s),
            );
            painter.rect_filled(
                bar,
                CornerRadius::same(4),
                blend(theme.offline, theme.bg_elevated, 0.14),
            );
            painter.with_clip_rect(bar.shrink(4.0)).text(
                pos2(bar.left() + 6.0 * s, bar.center().y),
                Align2::LEFT_CENTER,
                outcome.lines().next().unwrap_or_default(),
                mono(10.5 * s),
                theme.offline,
            );
        }
    }

    // Context track.
    if let Some(share) = job.context_share() {
        let track = Rect::from_min_max(
            pos2(r.left() + 1.0, r.bottom() - 3.0),
            pos2(r.right() - 1.0, r.bottom() - 1.0),
        );
        painter.rect_filled(track, CornerRadius::ZERO, theme.border);
        let filled = Rect::from_min_max(
            track.min,
            pos2(
                track.left() + track.width() * share as f32 / 100.0,
                track.bottom(),
            ),
        );
        painter.rect_filled(
            filled,
            CornerRadius::ZERO,
            if share >= 80 {
                theme.waiting
            } else {
                theme.accent
            },
        );
    }

    let resp = resp.on_hover_text(&job.spec);
    if resp.clicked() {
        intents.push(Intent::Select(Some(job.id.clone())));
    }
    let id = job.id.clone();
    let can_stop = job.can_stop();
    resp.context_menu(|ui| {
        if ui.button(t("orchestrator.menuOpen")).clicked() {
            intents.push(Intent::Select(Some(id.clone())));
            ui.close();
        }
        if ui
            .add_enabled(can_stop, egui::Button::new(t("orchestrator.menuStop")))
            .clicked()
        {
            intents.push(Intent::Stop(id.clone()));
            ui.close();
        }
        if ui.button(t("orchestrator.menuRestart")).clicked() {
            intents.push(Intent::Restart(id.clone()));
            ui.close();
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn rail_ui(
    ui: &mut egui::Ui,
    theme: &Theme,
    lang: crate::i18n::Lang,
    rail: Rect,
    snapshot: &Snapshot,
    groups: &[Group],
    group: &Group,
    state: &mut BoardState,
    intents: &mut Vec<Intent>,
) {
    let t = |key: &str| crate::i18n::tr(lang, key);
    ui.painter().rect_filled(rail, CornerRadius::ZERO, theme.bg);
    ui.painter().line_segment(
        [rail.left_top(), rail.left_bottom()],
        Stroke::new(1.0, theme.border),
    );
    let mut rui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rail.shrink2(vec2(8.0, 6.0)))
            .layout(Layout::top_down(Align::Min)),
    );
    rui.set_clip_rect(rail);

    // Head: collapse toggle, "summary", planner name.
    let head = rui.horizontal(|ui| {
        let glyph = if state.rail_collapsed {
            icons::CHEVRON_RIGHT
        } else {
            icons::CHEVRON_DOWN
        };
        let r = ui.add(
            egui::Button::new(RichText::new(glyph).font(mono(11.0)).color(theme.fg_muted))
                .frame(false),
        );
        if !state.rail_collapsed {
            ui.label(
                RichText::new(t("orchestrator.summary").to_uppercase())
                    .font(mono(10.0))
                    .color(theme.fg_faint),
            );
            let name = group
                .label
                .clone()
                .unwrap_or_else(|| t("orchestrator.noPlanner").to_string());
            ui.add(
                egui::Label::new(RichText::new(name).font(bold(12.0)).color(theme.fg)).truncate(),
            );
        }
        r
    });
    if head.inner.clicked() {
        state.rail_collapsed = !state.rail_collapsed;
    }
    if state.rail_collapsed {
        return;
    }
    rui.add_space(6.0);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(&mut rui, |ui| {
            // Headline + lane counts.
            let total = group.jobs.len();
            ui.label(
                RichText::new(fill(
                    t("orchestrator.workerCount"),
                    &[("count", total.to_string())],
                ))
                .font(bold(15.0))
                .color(theme.fg),
            );
            ui.add_space(4.0);
            for lane in LANE_ORDER {
                let n = group.counts.get(lane);
                if n == 0 {
                    continue;
                }
                kv(ui, theme, t(lane.key()), &n.to_string(), lane.color(theme));
            }
            kv(
                ui,
                theme,
                t("orchestrator.slots"),
                &format!("{}/{}", snapshot.running, snapshot.concurrency_limit),
                theme.fg_muted,
            );
            ui.add_space(10.0);

            // Runs tree.
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(t("orchestrator.runsLabel").to_uppercase())
                        .font(mono(10.0))
                        .color(theme.fg_faint),
                );
                ui.label(
                    RichText::new(group.runs.len().to_string())
                        .font(mono(10.0))
                        .color(theme.fg_faint),
                );
            });
            if group.runs.is_empty() {
                ui.label(
                    RichText::new(t("orchestrator.noWorkers"))
                        .font(sans(12.0))
                        .color(theme.fg_faint),
                );
            }
            for run in &group.runs {
                let open = !state.closed_runs.contains(&run.id);
                let row = rail_row(
                    ui,
                    theme,
                    0.0,
                    if open {
                        icons::CHEVRON_DOWN
                    } else {
                        icons::CHEVRON_RIGHT
                    },
                    run.state,
                    None,
                    &run.label,
                    &run.jobs.len().to_string(),
                    t(run.state.key()),
                    false,
                );
                if row.clicked() {
                    intents.push(Intent::ToggleRun(run.id.clone()));
                }
                if open {
                    for job in &run.jobs {
                        let lane = job.lane();
                        let value = if lane == Lane::Blocked {
                            t(Lane::Blocked.key()).to_string()
                        } else {
                            format_elapsed(job.seconds).unwrap_or_else(|| t(lane.key()).to_string())
                        };
                        let selected = state.selected.as_deref() == Some(&job.id);
                        if rail_row(
                            ui,
                            theme,
                            14.0,
                            "",
                            lane,
                            Some(&job.agent),
                            &job.label(),
                            "",
                            &value,
                            selected,
                        )
                        .clicked()
                        {
                            intents.push(Intent::Select(Some(job.id.clone())));
                        }
                    }
                }
            }

            // Other planners that need the person.
            let attention: Vec<(&Group, Lane, usize)> = groups
                .iter()
                .filter(|g| g.key() != group.key())
                .filter_map(|g| g.counts.attention().map(|(lane, n)| (g, lane, n)))
                .collect();
            if !attention.is_empty() {
                ui.add_space(10.0);
                ui.label(
                    RichText::new(t("orchestrator.attentionLabel").to_uppercase())
                        .font(mono(10.0))
                        .color(theme.fg_faint),
                );
                for (g, lane, n) in attention {
                    let name = g
                        .label
                        .clone()
                        .unwrap_or_else(|| t("orchestrator.noPlanner").to_string());
                    if rail_row(
                        ui,
                        theme,
                        0.0,
                        "",
                        lane,
                        g.agent.as_deref(),
                        &name,
                        "",
                        &n.to_string(),
                        false,
                    )
                    .clicked()
                    {
                        state.active = Some(g.key());
                        state.pinned = true;
                        state.selected = None;
                        state.moved = false;
                    }
                }
            }
        });
}

fn kv(ui: &mut egui::Ui, theme: &Theme, key: &str, value: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).font(sans(12.0)).color(theme.fg_muted));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).font(mono(11.5)).color(color));
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn rail_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    indent: f32,
    chevron: &str,
    lane: Lane,
    agent: Option<&str>,
    name: &str,
    count: &str,
    value: &str,
    selected: bool,
) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    if selected || resp.hovered() {
        ui.painter().rect_filled(
            r,
            CornerRadius::same(4),
            if selected {
                theme.panel
            } else {
                theme.panel_hover
            },
        );
    }
    let mut x = r.left() + 6.0 + indent;
    if !chevron.is_empty() {
        ui.painter().text(
            pos2(x + 5.0, r.center().y),
            Align2::CENTER_CENTER,
            chevron,
            mono(10.0),
            theme.fg_faint,
        );
        x += 14.0;
    }
    let painter = ui.painter();
    dot(painter, pos2(x + 3.0, r.center().y), 1.0, lane, theme);
    x += 11.0;
    if let Some(agent) = agent {
        agent_glyph(
            ui,
            theme,
            Some(agent),
            Rect::from_min_size(pos2(x, r.center().y - 6.0), Vec2::splat(12.0)),
        );
        x += 17.0;
    }
    let value_w = ui.fonts_mut(|f| {
        f.layout_no_wrap(value.to_string(), mono(10.5), theme.fg)
            .size()
            .x
    });
    let name_rect = Rect::from_min_max(
        pos2(x, r.top()),
        pos2(r.right() - value_w - 30.0, r.bottom()),
    );
    ui.painter().with_clip_rect(name_rect).text(
        pos2(x, r.center().y),
        Align2::LEFT_CENTER,
        name,
        sans(12.0),
        theme.fg,
    );
    if !count.is_empty() {
        ui.painter().text(
            pos2(r.right() - value_w - 14.0, r.center().y),
            Align2::RIGHT_CENTER,
            count,
            mono(10.0),
            theme.fg_faint,
        );
    }
    let value_color = match lane {
        Lane::Finished => theme.fg_faint,
        lane => lane.color(theme),
    };
    ui.painter().text(
        pos2(r.right() - 6.0, r.center().y),
        Align2::RIGHT_CENTER,
        value,
        mono(10.5),
        value_color,
    );
    resp
}

#[allow(clippy::too_many_arguments)]
fn inspector_ui(
    ui: &mut egui::Ui,
    theme: &Theme,
    lang: crate::i18n::Lang,
    canvas: Rect,
    job: &Job,
    orchestrator: &Arc<crate::orchestrator::Orchestrator>,
    state: &mut BoardState,
    intents: &mut Vec<Intent>,
) {
    let t = |key: &str| crate::i18n::tr(lang, key);
    let width = (canvas.width() * 0.62).clamp(280.0, 440.0);
    let panel = Rect::from_min_max(
        pos2(canvas.right() - width - 8.0, canvas.top() + 8.0),
        pos2(canvas.right() - 8.0, canvas.bottom() - 8.0),
    );
    // Swallow clicks so they do not reach the canvas underneath.
    let _ = ui.interact(
        panel,
        egui::Id::unique(("orch-inspector", &job.id)),
        Sense::click_and_drag(),
    );
    ui.painter().rect(
        panel,
        CornerRadius::same(10),
        theme.bg_elevated,
        Stroke::new(1.0, theme.border_strong),
        StrokeKind::Inside,
    );
    let mut pui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(panel.shrink(12.0))
            .layout(Layout::top_down(Align::Min)),
    );
    pui.set_clip_rect(panel);

    // Head.
    pui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        agent_glyph(ui, theme, Some(&job.agent), r);
        ui.label(RichText::new(job.label()).font(bold(13.0)).color(theme.fg));
        let lane = job.lane();
        ui.label(
            RichText::new(t(&format!("orchestrator.status.{}", job.status)))
                .font(mono(11.0))
                .color(match lane {
                    Lane::Finished => theme.fg_faint,
                    lane => lane.color(theme),
                }),
        );
        if let Some(elapsed) = format_elapsed(job.seconds) {
            ui.label(
                RichText::new(elapsed)
                    .font(mono(11.0))
                    .color(theme.fg_faint),
            );
        }
        if let Some(tokens) = format_tokens(job.total_tokens()) {
            ui.label(RichText::new(tokens).font(mono(11.0)).color(theme.fg_faint));
        }
        if let Some(cost) = job.cost_usd {
            ui.label(
                RichText::new(fmt_usd(cost))
                    .font(mono(11.0))
                    .color(theme.fg_faint),
            );
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::icon_button(
                ui,
                theme,
                icons::CLOSE,
                t("orchestrator.inspectorClose"),
                22.0,
            )
            .clicked()
            {
                intents.push(Intent::Select(None));
            }
            if widgets::icon_button(
                ui,
                theme,
                icons::DEBUG_RESTART,
                t("orchestrator.menuRestart"),
                22.0,
            )
            .clicked()
            {
                intents.push(Intent::Restart(job.id.clone()));
            }
            if job.can_stop()
                && widgets::icon_button(
                    ui,
                    theme,
                    icons::DEBUG_STOP,
                    t("orchestrator.stopWorker"),
                    22.0,
                )
                .clicked()
            {
                intents.push(Intent::Stop(job.id.clone()));
            }
        });
    });
    if let Some(worktree) = &job.worktree {
        pui.label(
            RichText::new(format!("{} {worktree}", icons::GIT_BRANCH))
                .font(mono(10.5))
                .color(theme.fg_faint),
        );
    }
    pui.add_space(6.0);

    // Tabs.
    pui.horizontal(|ui| {
        for (tab, key) in [
            (InspectorTab::Report, "orchestrator.reportTab"),
            (InspectorTab::Diff, "orchestrator.diffTab"),
        ] {
            let active = state.tab == tab;
            let enabled = tab == InspectorTab::Report || job.has_diff;
            let text = RichText::new(t(key))
                .font(if active { bold(12.0) } else { sans(12.0) })
                .color(if active { theme.fg } else { theme.fg_muted });
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(text).selected(active).frame(active),
                )
                .clicked()
            {
                state.tab = tab;
            }
        }
    });
    pui.separator();

    // Message box pinned at the bottom.
    let footer_h = 34.0;
    let body_h = (pui.available_height() - footer_h - 8.0).max(60.0);
    egui::ScrollArea::vertical()
        .max_height(body_h)
        .auto_shrink([false, false])
        .show(&mut pui, |ui| match state.tab {
            InspectorTab::Report => {
                section(ui, theme, t("orchestrator.specLabel"));
                ui.add(
                    egui::Label::new(RichText::new(&job.spec).font(sans(12.5)).color(theme.fg))
                        .wrap(),
                );
                let plan = job.plan_lines();
                if !plan.is_empty() {
                    section(ui, theme, t("orchestrator.planLabel"));
                    for step in plan {
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("• {step}"))
                                    .font(sans(12.0))
                                    .color(theme.fg_muted),
                            )
                            .wrap(),
                        );
                    }
                }
                section(ui, theme, t("orchestrator.summaryLabel"));
                let report = if !job.summary.trim().is_empty() {
                    job.summary.clone()
                } else {
                    job.outcome
                        .clone()
                        .unwrap_or_else(|| t("orchestrator.noReport").to_string())
                };
                super::markdown::render(ui, theme, &report);
                if job.status == "failed" {
                    if let Some(outcome) = &job.outcome {
                        ui.add_space(6.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(outcome).font(mono(11.5)).color(theme.offline),
                            )
                            .wrap(),
                        );
                    }
                }
            }
            InspectorTab::Diff => {
                let slot = match &state.diff {
                    Some((id, slot)) if id == &job.id => Arc::clone(slot),
                    _ => {
                        let slot: DiffSlot = Arc::new(Mutex::new(None));
                        orchestrator.diff(job.id.clone(), Arc::clone(&slot));
                        state.diff = Some((job.id.clone(), Arc::clone(&slot)));
                        slot
                    }
                };
                let result = slot.lock().unwrap().clone();
                match result {
                    None => {
                        ui.label(
                            RichText::new(t("orchestrator.diffLoading"))
                                .font(sans(12.0))
                                .color(theme.fg_faint),
                        );
                    }
                    Some(Err(error)) => {
                        ui.label(
                            RichText::new(format!("{}: {error}", t("orchestrator.diffFailed")))
                                .font(sans(12.0))
                                .color(theme.offline),
                        );
                    }
                    Some(Ok(diff)) => diff_view(ui, theme, &diff),
                }
            }
        });

    // Message (steer when it is mid-turn, a new turn otherwise).
    pui.add_space(6.0);
    pui.horizontal(|ui| {
        let send_w = 70.0;
        let edit = ui.add_sized(
            vec2(ui.available_width() - send_w - 6.0, 26.0),
            egui::TextEdit::singleline(&mut state.message)
                .hint_text(t("orchestrator.messagePlaceholder"))
                .font(sans(12.5)),
        );
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let send = widgets::primary_button(ui, theme, t("orchestrator.messageSend")).clicked();
        if (enter || send) && !state.message.trim().is_empty() {
            let steer = job.status == "running";
            orchestrator.message(job.id.clone(), std::mem::take(&mut state.message), steer);
        }
    });
}

fn section(ui: &mut egui::Ui, theme: &Theme, label: &str) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(label.to_uppercase())
            .font(mono(10.0))
            .color(theme.fg_faint),
    );
    ui.add_space(2.0);
}

/// Diff lines colored like `diffLineClass`. Capped so a huge diff cannot balloon the frame.
fn diff_view(ui: &mut egui::Ui, theme: &Theme, diff: &str) {
    const MAX_LINES: usize = 4000;
    let font = FontId::monospace(11.5);
    for line in diff.lines().take(MAX_LINES) {
        let color = if line.starts_with("+++")
            || line.starts_with("---")
            || line.starts_with("diff ")
            || line.starts_with("index ")
        {
            theme.fg_faint
        } else if line.starts_with('+') {
            theme.working
        } else if line.starts_with('-') {
            theme.offline
        } else if line.starts_with("@@ ") {
            theme.accent
        } else {
            theme.fg_muted
        };
        ui.add(egui::Label::new(RichText::new(line).font(font.clone()).color(color)).extend());
    }
    let total = diff.lines().count();
    if total > MAX_LINES {
        ui.label(
            RichText::new(format!("… +{}", total - MAX_LINES))
                .font(font)
                .color(theme.fg_faint),
        );
    }
}

// ----------------------------------------------------------------- tolerant snapshot parsing

/// Reads the core's snapshot one job at a time, after dropping nulls, so one job with a field
/// shaped differently than expected costs that job, not the whole board.
fn parse_snapshot(mut value: serde_json::Value) -> Snapshot {
    fn strip_nulls(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                map.retain(|_, v| !v.is_null());
                map.values_mut().for_each(strip_nulls);
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(strip_nulls),
            _ => {}
        }
    }
    fn list<T: serde::de::DeserializeOwned>(value: &serde_json::Value, key: &str) -> Vec<T> {
        value
            .get(key)
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| match serde_json::from_value(item.clone()) {
                        Ok(parsed) => Some(parsed),
                        Err(error) => {
                            eprintln!("[orchestrator] skipped a {key} entry: {error}");
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    strip_nulls(&mut value);
    let count = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize
    };
    Snapshot {
        jobs: list(&value, "jobs"),
        planners: list(&value, "planners"),
        shells: list(&value, "shells"),
        running: count("running"),
        queued: count("queued"),
        concurrency_limit: count("concurrencyLimit"),
    }
}

// ----------------------------------------------------------------- right sidebar: Agents (`AgentsPanel`)

#[derive(Default)]
pub struct AgentsPanelState {
    revision: u64,
    snapshot: Arc<Snapshot>,
    show_all: bool,
    scoped_to: Option<String>,
    collapsed: HashSet<String>,
    finished_open: HashSet<String>,
    last_tick: f64,
    on_planner: bool,
    prev: Option<(bool, crate::app::RightTab)>,
}

impl AgentsPanelState {
    /// A manual tab change wins over the automatic one: focus leaving the planner no longer
    /// restores the old tab.
    pub fn forget_auto_open(&mut self) {
        self.prev = None;
    }
}

/// Focusing a planner terminal swaps the right sidebar to Agents; focusing anything else puts back
/// the tab and visibility from before (`useAgentsSidebarAutoOpen`). Only focus transitions act.
pub fn auto_open_agents(app: &mut AletheApp) {
    let on_planner = app.focused_planner().is_some();
    if on_planner == app.agents_panel.on_planner {
        return;
    }
    app.agents_panel.on_planner = on_planner;
    if on_planner {
        app.agents_panel.prev = Some((app.show_right, app.right_tab));
        app.show_right = true;
        app.right_tab = crate::app::RightTab::Agents;
    } else if let Some((visible, tab)) = app.agents_panel.prev.take() {
        app.show_right = visible;
        app.right_tab = tab;
    }
}

struct PlannerBlock<'a> {
    key: String,
    name: String,
    agent: Option<String>,
    terminal: Option<(String, String, String)>, // project, terminal, tab
    active: Vec<&'a Job>,
    finished: Vec<&'a Job>,
}

struct ProjectGroup<'a> {
    key: String,
    project: Option<(String, String, Color32)>, // id, name, color
    planners: Vec<PlannerBlock<'a>>,
    running: usize,
    waiting: usize,
    cost: Option<f64>,
}

fn sum_cost<'a>(jobs: impl Iterator<Item = &'a Job>) -> Option<f64> {
    jobs.filter_map(|j| j.cost_usd)
        .fold(None, |acc, c| Some(acc.unwrap_or(0.0) + c))
}

fn job_title(job: &Job) -> String {
    job.spec
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
        .or_else(|| clean(job.run_label.as_deref()))
        .unwrap_or_else(|| job.id.clone())
}

enum PanelIntent {
    ToggleProject(String),
    ToggleFinished(String),
    ShowAll(bool),
    Reveal(String, String, String),
    OpenBoard(String),
    HidePanel,
    Stop(String),
    Answer(String, &'static str),
    Enable,
}

/// Claude's own subagents as jobs under their tab, the planner (`nativeSubagentJobs`).
fn native_subagent_jobs(app: &AletheApp) -> Vec<Job> {
    use crate::transcripts::SubStatus;
    let now = crate::model::now_ms();
    app.transcripts
        .all()
        .into_iter()
        .flat_map(|(tab_id, stats)| {
            stats.subagents.into_iter().map(move |sub| {
                let seconds = sub.started_ms.map(|start| {
                    (sub.ended_ms.unwrap_or(now).saturating_sub(start)) as f64 / 1000.0
                });
                Job {
                    id: format!("subagent:{}", sub.tool_use_id),
                    planner_id: Some(tab_id.clone()),
                    agent: "claude".into(),
                    run_id: format!("native-subagents:{tab_id}"),
                    run_label: Some("Subagents".into()),
                    spec: if sub.description.is_empty() {
                        sub.prompt.clone()
                    } else {
                        sub.description.clone()
                    },
                    cwd: stats.cwd.clone(),
                    status: match sub.status {
                        SubStatus::Running => "running",
                        SubStatus::Done => "done",
                        SubStatus::Failed => "failed",
                        SubStatus::Killed => "interrupted",
                    }
                    .into(),
                    outcome: sub.result.clone(),
                    seconds,
                    tokens: (sub.total_tokens > 0).then(|| Tokens {
                        total: Some(TokenCount {
                            total_tokens: Some(sub.total_tokens as f64),
                        }),
                        ..Default::default()
                    }),
                    cost_usd: sub.cost_usd,
                    role: Some(sub.agent_type.clone()),
                    model: sub.model.clone(),
                    summary: sub.result.clone().unwrap_or_else(|| sub.prompt.clone()),
                    native: true,
                    ..Default::default()
                }
            })
        })
        .collect()
}

pub fn agents_panel(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let t = |key: &str| crate::i18n::tr(lang, key);
    super::todos::tab_strip(app, ui);
    let native = native_subagent_jobs(app);

    let enabled = app.feature_enabled("orchestrator");
    if enabled && !app.orchestrator.is_started() {
        let _ = app.orchestrator.ensure_started();
        app.orchestrator
            .apply_settings(app.data.preferences.extra.get("orchestration"));
    }
    let orchestrator = Arc::clone(&app.orchestrator);
    let planner = app.focused_planner();
    let now = ui.input(|i| i.time);
    let state = &mut app.agents_panel;
    if state.scoped_to != planner {
        state.scoped_to = planner.clone();
        state.show_all = false;
    }
    let revision = orchestrator.revision();
    if revision != state.revision {
        state.revision = revision;
        state.snapshot = Arc::new(parse_snapshot(orchestrator.board()));
    }
    let snapshot = Arc::clone(&state.snapshot);
    if snapshot.running > 0 {
        if now - state.last_tick >= 1.0 {
            state.last_tick = now;
            orchestrator.refresh();
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(1000));
    }
    let show_all = state.show_all;
    let scoped = planner.is_some() && !show_all;
    let jobs: Vec<&Job> = snapshot
        .jobs
        .iter()
        .chain(native.iter())
        .filter(|j| j.superseded_by.is_none())
        .filter(|j| !scoped || j.planner_id == planner)
        .collect();

    // ---- group: project -> planner -> jobs (`groupAgentJobs`)
    let active_project = match &app.view {
        crate::app::View::Project(id) => Some(id.clone()),
        _ => None,
    };
    let mut groups: Vec<ProjectGroup> = Vec::new();
    for job in &jobs {
        let found = job.planner_id.as_ref().and_then(|pid| {
            app.data.projects.iter().find_map(|p| {
                p.terminals.iter().find_map(|term| {
                    term.tabs.iter().find(|tab| &tab.id == pid).map(|tab| {
                        let name = term.name.clone();
                        (p, term.id.clone(), tab.id.clone(), name)
                    })
                })
            })
        });
        let project_key = found.as_ref().map(|f| f.0.id.clone()).unwrap_or_default();
        let gi = match groups.iter().position(|g| g.key == project_key) {
            Some(i) => i,
            None => {
                groups.push(ProjectGroup {
                    key: project_key.clone(),
                    project: found.as_ref().map(|f| {
                        let color =
                            f.0.color
                                .as_deref()
                                .and_then(|c| Color32::from_hex(c).ok())
                                .unwrap_or(theme.accent);
                        (f.0.id.clone(), f.0.name.clone(), color)
                    }),
                    planners: Vec::new(),
                    running: 0,
                    waiting: 0,
                    cost: None,
                });
                groups.len() - 1
            }
        };
        let group = &mut groups[gi];
        let planner_key = job.planner_id.clone().unwrap_or_default();
        let bi = match group.planners.iter().position(|b| b.key == planner_key) {
            Some(i) => i,
            None => {
                let declared = snapshot
                    .planners
                    .iter()
                    .find(|p| Some(&p.id) == job.planner_id.as_ref());
                let name = match (&found, declared, &job.planner_id) {
                    (Some(f), _, _) => f.3.clone(),
                    (None, Some(p), _) => {
                        format!("{} · {}", p.label, t("agentsPanel.plannerClosed"))
                    }
                    (None, None, Some(_)) => t("agentsPanel.plannerClosed").to_string(),
                    _ => t("orchestrator.noPlanner").to_string(),
                };
                group.planners.push(PlannerBlock {
                    key: planner_key,
                    name,
                    agent: declared.map(|p| p.agent.clone()).filter(|a| !a.is_empty()),
                    terminal: found
                        .as_ref()
                        .map(|f| (f.0.id.clone(), f.1.clone(), f.2.clone())),
                    active: Vec::new(),
                    finished: Vec::new(),
                });
                group.planners.len() - 1
            }
        };
        let block = &mut group.planners[bi];
        if job.can_stop() {
            block.active.push(job);
        } else {
            block.finished.push(job);
        }
        match job.status.as_str() {
            "running" => group.running += 1,
            "blocked" => group.waiting += 1,
            _ => {}
        }
        if let Some(cost) = job.cost_usd {
            group.cost = Some(group.cost.unwrap_or(0.0) + cost);
        }
    }
    groups.sort_by_key(|g| {
        (
            g.project.is_none(),
            Some(&g.key) != active_project.as_ref(),
            g.project.as_ref().map(|p| p.1.to_lowercase()),
        )
    });

    let mut intents: Vec<PanelIntent> = Vec::new();
    let state = &app.agents_panel;

    // ---- header
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icons::TYPE_HIERARCHY)
                        .font(mono(15.0))
                        .color(theme.fg),
                );
                ui.label(
                    RichText::new(t("agentsPanel.title"))
                        .font(bold(14.5))
                        .color(theme.fg),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(ui, &theme, icons::LAYOUT_SIDEBAR_RIGHT, "", 24.0)
                        .clicked()
                    {
                        intents.push(PanelIntent::HidePanel);
                    }
                    if let Some(project) = &active_project {
                        if widgets::icon_button(
                            ui,
                            &theme,
                            icons::SCREEN_FULL,
                            t("agentsPanel.expand"),
                            24.0,
                        )
                        .clicked()
                        {
                            intents.push(PanelIntent::OpenBoard(project.clone()));
                        }
                    }
                    if planner.is_some() {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if ui
                            .selectable_label(
                                show_all,
                                RichText::new(t("agentsPanel.scopeAll")).font(sans(11.5)),
                            )
                            .clicked()
                        {
                            intents.push(PanelIntent::ShowAll(true));
                        }
                        if ui
                            .selectable_label(
                                !show_all,
                                RichText::new(t("agentsPanel.scopeFocused")).font(sans(11.5)),
                            )
                            .clicked()
                        {
                            intents.push(PanelIntent::ShowAll(false));
                        }
                    }
                });
            });
            if !jobs.is_empty() {
                let count = |s: &str| jobs.iter().filter(|j| j.status == s).count();
                let (running, waiting, queued) =
                    (count("running"), count("blocked"), count("queued"));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let chip = |ui: &mut egui::Ui, text: String, color: Color32| {
                        ui.label(RichText::new(text).font(mono(11.0)).color(color));
                    };
                    if waiting > 0 {
                        chip(
                            ui,
                            fill(t("agentsPanel.waiting"), &[("count", waiting.to_string())]),
                            theme.waiting,
                        );
                    }
                    if running > 0 {
                        chip(
                            ui,
                            fill(t("orchestrator.running"), &[("count", running.to_string())]),
                            theme.working,
                        );
                    }
                    if queued > 0 {
                        chip(
                            ui,
                            fill(t("orchestrator.queued"), &[("count", queued.to_string())]),
                            theme.fg_muted,
                        );
                    }
                    if running + waiting + queued == 0 {
                        chip(ui, t("agentsPanel.allSettled").to_string(), theme.fg_muted);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let text = sum_cost(jobs.iter().copied())
                            .map(fmt_usd)
                            .unwrap_or_else(|| t("orchestrator.noPrice").to_string());
                        ui.label(RichText::new(text).font(bold(12.0)).color(theme.fg))
                            .on_hover_text(t("agentsPanel.costTitle"));
                    });
                });
            }
        });
    let y = ui.cursor().top();
    ui.painter()
        .hline(ui.max_rect().x_range(), y, Stroke::new(1.0, theme.border));

    // ---- body
    if jobs.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(icons::TYPE_HIERARCHY)
                    .font(mono(24.0))
                    .color(theme.fg_faint),
            );
            ui.add_space(6.0);
            if enabled {
                ui.label(
                    RichText::new(t("agentsPanel.emptyTitle"))
                        .font(bold(13.0))
                        .color(theme.fg),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(t("agentsPanel.emptyBody"))
                            .font(sans(12.0))
                            .color(theme.fg_muted),
                    )
                    .wrap(),
                );
            } else {
                ui.add(
                    egui::Label::new(
                        RichText::new(t("orchestrator.disabled"))
                            .font(sans(12.0))
                            .color(theme.fg_muted),
                    )
                    .wrap(),
                );
                ui.add_space(8.0);
                if widgets::primary_button(ui, &theme, t("settings.orchestrator")).clicked() {
                    intents.push(PanelIntent::Enable);
                }
            }
        });
    } else {
        egui::ScrollArea::vertical()
            .id_salt("agents-panel")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(4.0);
                for group in &groups {
                    let collapsed = state.collapsed.contains(&group.key);
                    project_header(ui, &theme, lang, group, collapsed, &mut intents);
                    if collapsed {
                        continue;
                    }
                    for block in &group.planners {
                        planner_header(ui, &theme, block, &mut intents);
                        for job in &block.active {
                            agent_job_row(ui, &theme, lang, job, &mut intents);
                        }
                        if !block.finished.is_empty() {
                            let open = state.finished_open.contains(&block.key);
                            let label = fill(
                                t("agentsPanel.finished"),
                                &[("count", block.finished.len().to_string())],
                            );
                            let chevron = if open {
                                icons::CHEVRON_DOWN
                            } else {
                                icons::CHEVRON_RIGHT
                            };
                            let resp = ui.add(
                                egui::Button::new(
                                    RichText::new(format!("{chevron}  {label}"))
                                        .font(sans(11.5))
                                        .color(theme.fg_faint),
                                )
                                .frame(false),
                            );
                            if resp.clicked() {
                                intents.push(PanelIntent::ToggleFinished(block.key.clone()));
                            }
                            if open {
                                for job in &block.finished {
                                    agent_job_row(ui, &theme, lang, job, &mut intents);
                                }
                            }
                        }
                        ui.add_space(6.0);
                    }
                }
            });
    }

    for intent in intents {
        match intent {
            PanelIntent::ToggleProject(key) => {
                if !app.agents_panel.collapsed.remove(&key) {
                    app.agents_panel.collapsed.insert(key);
                }
            }
            PanelIntent::ToggleFinished(key) => {
                if !app.agents_panel.finished_open.remove(&key) {
                    app.agents_panel.finished_open.insert(key);
                }
            }
            PanelIntent::ShowAll(all) => app.agents_panel.show_all = all,
            PanelIntent::Reveal(project, terminal, tab) => {
                app.act(Action::OpenProject(project));
                app.act(Action::SelectSubTab {
                    terminal_id: terminal.clone(),
                    tab_id: tab,
                });
                app.act(Action::FocusTerminal(terminal));
            }
            PanelIntent::HidePanel => {
                app.show_right = false;
                app.agents_panel.forget_auto_open();
            }
            PanelIntent::OpenBoard(project) => {
                let existing = app
                    .project(&project)
                    .and_then(|p| {
                        p.terminals
                            .iter()
                            .find(|term| term.kind.as_deref() == Some("orchestrator"))
                    })
                    .map(|term| term.id.clone());
                match existing {
                    Some(id) => {
                        app.act(Action::OpenProject(project));
                        app.act(Action::FocusTerminal(id));
                    }
                    None => app.act(Action::NewOrchestration(project)),
                }
            }
            PanelIntent::Stop(job) => orchestrator.cancel(job),
            PanelIntent::Answer(job, decision) => orchestrator.answer(job, decision),
            PanelIntent::Enable => app.set_feature("orchestrator", true),
        }
    }
}

fn project_header(
    ui: &mut egui::Ui,
    theme: &Theme,
    lang: crate::i18n::Lang,
    group: &ProjectGroup,
    collapsed: bool,
    intents: &mut Vec<PanelIntent>,
) {
    let t = |key: &str| crate::i18n::tr(lang, key);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(r, CornerRadius::same(4), theme.panel_hover);
    }
    let mut x = r.left() + 8.0;
    let chevron = if collapsed {
        icons::CHEVRON_RIGHT
    } else {
        icons::CHEVRON_DOWN
    };
    ui.painter().text(
        pos2(x + 4.0, r.center().y),
        Align2::CENTER_CENTER,
        chevron,
        mono(10.0),
        theme.fg_faint,
    );
    x += 14.0;
    let (name, color) = match &group.project {
        Some((_, name, color)) => (name.clone(), *color),
        None => (t("agentsPanel.noProject").to_string(), theme.fg_faint),
    };
    let badge = Rect::from_min_size(pos2(x, r.center().y - 8.0), Vec2::splat(16.0));
    ui.painter()
        .rect_filled(badge, CornerRadius::same(4), color.gamma_multiply(0.85));
    let initial = name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    ui.painter().text(
        badge.center(),
        Align2::CENTER_CENTER,
        initial,
        bold(10.0),
        Color32::WHITE,
    );
    x += 22.0;
    // Right side, right to left: open board, cost, running, waiting.
    let mut right = r.right() - 6.0;
    if let Some((project_id, _, _)) = &group.project {
        let b = Rect::from_center_size(pos2(right - 10.0, r.center().y), Vec2::splat(20.0));
        let br = ui.interact(
            b,
            ui.scope_id().with(("agents-board", project_id)),
            Sense::click(),
        );
        if br.hovered() {
            ui.painter()
                .rect_filled(b, CornerRadius::same(4), theme.panel);
        }
        ui.painter().text(
            b.center(),
            Align2::CENTER_CENTER,
            icons::SCREEN_FULL,
            mono(11.0),
            theme.fg_muted,
        );
        if br.on_hover_text(t("agentsPanel.openBoard")).clicked() {
            intents.push(PanelIntent::OpenBoard(project_id.clone()));
        }
        right -= 26.0;
    }
    let p = ui.painter();
    let put = |text: String, color: Color32, right: &mut f32| {
        let g = p.layout_no_wrap(text, mono(11.0), color);
        *right -= g.size().x;
        p.galley(pos2(*right, r.center().y - g.size().y / 2.0), g, color);
        *right -= 8.0;
    };
    if let Some(cost) = group.cost {
        put(fmt_usd(cost), theme.fg_muted, &mut right);
    }
    if group.running > 0 {
        put(group.running.to_string(), theme.working, &mut right);
        p.circle_filled(pos2(right - 1.0, r.center().y), 3.0, theme.working);
        right -= 12.0;
    }
    if group.waiting > 0 {
        put(group.waiting.to_string(), theme.waiting, &mut right);
    }
    p.with_clip_rect(Rect::from_min_max(
        pos2(x, r.top()),
        pos2(right, r.bottom()),
    ))
    .text(
        pos2(x, r.center().y),
        Align2::LEFT_CENTER,
        name,
        bold(12.5),
        theme.fg,
    );
    if resp.clicked() {
        intents.push(PanelIntent::ToggleProject(group.key.clone()));
    }
}

fn planner_header(
    ui: &mut egui::Ui,
    theme: &Theme,
    block: &PlannerBlock,
    intents: &mut Vec<PanelIntent>,
) {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    let can_reveal = block.terminal.is_some();
    if can_reveal && resp.hovered() {
        ui.painter()
            .rect_filled(r, CornerRadius::same(4), theme.panel_hover);
    }
    let icon = Rect::from_min_size(pos2(r.left() + 20.0, r.center().y - 6.5), Vec2::splat(13.0));
    agent_glyph(ui, theme, block.agent.as_deref(), icon);
    let total = (block.active.len() + block.finished.len()).to_string();
    let p = ui.painter();
    p.text(
        pos2(r.right() - 8.0, r.center().y),
        Align2::RIGHT_CENTER,
        &total,
        mono(10.5),
        theme.fg_faint,
    );
    p.with_clip_rect(Rect::from_min_max(
        pos2(icon.right() + 7.0, r.top()),
        pos2(r.right() - 30.0, r.bottom()),
    ))
    .text(
        pos2(icon.right() + 7.0, r.center().y),
        Align2::LEFT_CENTER,
        &block.name,
        sans(12.0),
        if can_reveal { theme.fg } else { theme.fg_muted },
    );
    if resp.clicked() {
        if let Some((project, terminal, tab)) = &block.terminal {
            intents.push(PanelIntent::Reveal(
                project.clone(),
                terminal.clone(),
                tab.clone(),
            ));
        }
    }
}

fn agent_job_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    lang: crate::i18n::Lang,
    job: &Job,
    intents: &mut Vec<PanelIntent>,
) {
    let t = |key: &str| crate::i18n::tr(lang, key);
    let lane = job.lane();
    let ask = job
        .pending_approval
        .as_ref()
        .filter(|_| job.status == "blocked");
    let height = 40.0 + if ask.is_some() { 64.0 } else { 0.0 };
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    if resp.hovered() {
        ui.painter()
            .rect_filled(r, CornerRadius::same(5), theme.panel_hover);
    }
    let left = r.left() + 26.0;
    let top = r.top() + 5.0;
    let dot_at = pos2(left + 4.0, top + 8.0);
    if lane == Lane::Running {
        let phase = (ui.input(|i| i.time) as f32 * 3.0).sin() * 0.5 + 0.5;
        ui.painter()
            .circle_filled(dot_at, 3.0 + phase * 2.0, theme.working.gamma_multiply(0.3));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(120));
    }
    dot(ui.painter(), dot_at, 1.0, lane, theme);
    let icon = Rect::from_min_size(pos2(left + 13.0, top + 1.0), Vec2::splat(14.0));
    agent_glyph(ui, theme, Some(&job.agent), icon);
    let text_left = icon.right() + 7.0;
    let mut right = r.right() - 6.0;
    if job.can_stop() && !job.native {
        let b = Rect::from_center_size(pos2(right - 9.0, top + 8.0), Vec2::splat(18.0));
        let br = ui.interact(
            b,
            ui.scope_id().with(("agents-stop", &job.id)),
            Sense::click(),
        );
        if br.hovered() {
            ui.painter()
                .rect_filled(b, CornerRadius::same(4), theme.panel);
        }
        ui.painter().text(
            b.center(),
            Align2::CENTER_CENTER,
            icons::DEBUG_STOP,
            mono(11.0),
            theme.fg_muted,
        );
        if br.on_hover_text(t("orchestrator.menuStop")).clicked() {
            intents.push(PanelIntent::Stop(job.id.clone()));
        }
        right -= 24.0;
    }
    let p = ui.painter();
    if let Some(elapsed) = format_elapsed(job.seconds) {
        let g = p.layout_no_wrap(elapsed, mono(10.5), theme.fg_faint);
        right -= g.size().x;
        p.galley(pos2(right, top + 8.0 - g.size().y / 2.0), g, theme.fg_faint);
        right -= 8.0;
    }
    p.with_clip_rect(Rect::from_min_max(
        pos2(text_left, r.top()),
        pos2(right, r.bottom()),
    ))
    .text(
        pos2(text_left, top + 8.0),
        Align2::LEFT_CENTER,
        job_title(job),
        sans(12.5),
        theme.fg,
    );

    // Meta: model · effort · role, status (when not running), cost (or tokens until it is known).
    let meta_y = top + 25.0;
    let mut meta_right = r.right() - 6.0;
    let put = |text: String, color: Color32, right: &mut f32| {
        let g = p.layout_no_wrap(text, mono(10.5), color);
        *right -= g.size().x;
        p.galley(pos2(*right, meta_y - g.size().y / 2.0), g, color);
        *right -= 8.0;
    };
    if let Some(cost) = job
        .cost_usd
        .map(fmt_usd)
        .or_else(|| format_tokens(job.total_tokens()))
    {
        put(cost, theme.fg_muted, &mut meta_right);
    }
    if lane != Lane::Running {
        let color = match lane {
            Lane::Finished => theme.fg_faint,
            lane => lane.color(theme),
        };
        put(
            t(&format!("orchestrator.status.{}", job.status)).to_string(),
            color,
            &mut meta_right,
        );
    }
    let details: Vec<String> = [job.model.clone(), job.effort.clone(), job.role.clone()]
        .into_iter()
        .flatten()
        .collect();
    if !details.is_empty() {
        p.with_clip_rect(Rect::from_min_max(
            pos2(text_left, r.top()),
            pos2(meta_right, r.bottom()),
        ))
        .text(
            pos2(text_left, meta_y),
            Align2::LEFT_CENTER,
            details.join(" · "),
            mono(10.5),
            theme.fg_faint,
        );
    }

    if let Some(ask) = ask {
        let block = Rect::from_min_max(
            pos2(text_left - 4.0, r.top() + 38.0),
            pos2(r.right() - 6.0, r.bottom() - 4.0),
        );
        p.rect_filled(
            block,
            CornerRadius::same(5),
            blend(theme.waiting, theme.bg, 0.12),
        );
        let question = if ask.kind == "fileChange" {
            t("orchestrator.askFileChange")
        } else {
            t("orchestrator.askCommand")
        };
        p.text(
            pos2(block.left() + 6.0, block.top() + 10.0),
            Align2::LEFT_CENTER,
            question,
            sans(11.5),
            theme.waiting,
        );
        if let Some(command) = ask.command.as_ref().or(ask.reason.as_ref()) {
            p.with_clip_rect(block.shrink(4.0)).text(
                pos2(block.left() + 6.0, block.top() + 26.0),
                Align2::LEFT_CENTER,
                command,
                mono(10.5),
                theme.fg,
            );
        }
        let buttons: [(&str, &'static str); 2] = [
            ("orchestrator.answerAccept", "accept"),
            ("orchestrator.answerDecline", "decline"),
        ];
        let bw = 84.0;
        for (i, (label, decision)) in buttons.iter().enumerate() {
            let b = Rect::from_min_size(
                pos2(
                    block.left() + 6.0 + i as f32 * (bw + 6.0),
                    block.bottom() - 22.0,
                ),
                vec2(bw, 18.0),
            );
            let br = ui.interact(
                b,
                ui.scope_id().with(("agents-answer", &job.id, i)),
                Sense::click(),
            );
            let primary = i == 0;
            ui.painter().rect(
                b,
                CornerRadius::same(4),
                if primary {
                    theme.accent
                } else if br.hovered() {
                    theme.panel_hover
                } else {
                    theme.panel
                },
                Stroke::new(1.0, theme.border),
                StrokeKind::Inside,
            );
            ui.painter().text(
                b.center(),
                Align2::CENTER_CENTER,
                t(label),
                sans(11.0),
                if primary { theme.accent_on } else { theme.fg },
            );
            if br.clicked() {
                intents.push(PanelIntent::Answer(job.id.clone(), decision));
            }
        }
    }
    resp.on_hover_text(&job.spec);
}
