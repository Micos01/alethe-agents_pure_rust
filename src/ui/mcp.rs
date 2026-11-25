//! Right sidebar: the MCP panel (`McpPanel` + the servers tab of `McpManagerModal`).
//!
//! Lists the MCP servers every agent CLI has configured (Claude, Codex, Cursor, OpenCode,
//! Antigravity, Kimi), grouped by name, for the user's global config or the active project's.
//! Reading and writing go through the vendored `mcp_store`, which edits each CLI's own file in
//! place (comments and order kept) and backs it up first.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::icons;
use super::widgets::{self, mono, sans};
use crate::app::AletheApp;
use crate::backend::mcp_health::{McpHealth, McpHealthStatus};
use crate::backend::mcp_model::{
    McpAgent, McpAgentSnapshot, McpScope, McpServerRecordView, McpTransportView, ALL_MCP_AGENTS,
};
use crate::backend::mcp_store::{self, McpEnvInput, McpServerInput, McpTransportInput};
use egui::{
    Align, Align2, CornerRadius, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};

type Pending = Arc<Mutex<Option<Vec<McpAgentSnapshot>>>>;
type HealthResult = Result<Vec<McpHealth>, String>;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Transport {
    #[default]
    Stdio,
    Http,
    Sse,
}

#[derive(Default)]
struct AddForm {
    name: String,
    transport: Transport,
    command: String,
    /// One argument per line.
    args: String,
    url: String,
    /// `KEY=value`, one per line.
    env: String,
    agents: Vec<McpAgent>,
}

#[derive(Default)]
pub struct McpPanelState {
    scope: Option<McpScope>,
    repo: Option<String>,
    snapshots: Vec<McpAgentSnapshot>,
    loaded: bool,
    pending: Option<Pending>,
    search: String,
    agent_filter: Vec<McpAgent>,
    selected: Option<String>,
    adding: Option<AddForm>,
    /// The last write's outcome: (text, failed).
    message: Option<(String, bool)>,
    /// Showing installed skills instead of servers.
    show_skills: bool,
    pub skills: super::skills::SkillsState,
    /// Last `<cli> mcp list` result per agent, and the one running now.
    health: Vec<(McpAgent, HealthResult)>,
    checking: Option<(McpAgent, Arc<Mutex<Option<HealthResult>>>)>,
}

/// The agents whose CLI can report server status (`MCP_HEALTH_AGENTS`).
fn health_capable(agent: McpAgent) -> bool {
    matches!(
        agent,
        McpAgent::Claude | McpAgent::Codex | McpAgent::Opencode
    )
}

/// One server name across agents (`groupServersByName`).
struct ServerGroup {
    name: String,
    records: Vec<McpServerRecordView>,
    agents: Vec<McpAgent>,
    missing: Vec<McpAgent>,
    any_disabled: bool,
}

fn agent_label(agent: McpAgent) -> &'static str {
    match agent {
        McpAgent::Kimi => "Kimi",
        other => crate::agents::info(other.as_str()).label,
    }
}

fn transport_summary(transport: &McpTransportView) -> String {
    match transport {
        McpTransportView::Stdio { command, args, .. } => {
            let mut text = command.clone();
            for arg in args {
                text.push(' ');
                text.push_str(arg);
            }
            text
        }
        McpTransportView::Http { url, .. } => format!("http · {url}"),
        McpTransportView::Sse { url, .. } => format!("sse · {url}"),
    }
}

fn group_servers(snapshots: &[McpAgentSnapshot]) -> Vec<ServerGroup> {
    let mut by_name: BTreeMap<String, Vec<McpServerRecordView>> = BTreeMap::new();
    for snapshot in snapshots {
        for record in &snapshot.servers {
            by_name
                .entry(record.server.name.clone())
                .or_default()
                .push(record.clone());
        }
    }
    // Agents that can hold servers in this scope: they have a config source.
    let capable: Vec<McpAgent> = snapshots
        .iter()
        .filter(|s| !s.sources.is_empty())
        .map(|s| s.agent)
        .collect();
    by_name
        .into_iter()
        .map(|(name, records)| {
            let mut agents: Vec<McpAgent> = records.iter().map(|r| r.agent).collect();
            agents.dedup();
            let missing = capable
                .iter()
                .copied()
                .filter(|a| !agents.contains(a))
                .collect();
            let any_disabled = records.iter().any(|r| !r.server.enabled);
            ServerGroup {
                name,
                records,
                agents,
                missing,
                any_disabled,
            }
        })
        .collect()
}

/// The folder project-scoped configs live in: the active project's.
fn active_repo(app: &AletheApp) -> Option<String> {
    let project = match &app.view {
        crate::app::View::Project(id) => app.project(id),
        _ => None,
    }
    .or_else(|| app.data.projects.iter().max_by_key(|p| p.last_used()))?;
    let cwd = project.cwd();
    (!cwd.trim().is_empty()).then_some(cwd)
}

fn default_scope(app: &AletheApp) -> McpScope {
    match app
        .data
        .preferences
        .extra
        .get("mcpDefaultScope")
        .and_then(serde_json::Value::as_str)
    {
        Some("project") => McpScope::Project,
        _ => McpScope::Global,
    }
}

impl McpPanelState {
    fn rescan(&mut self, ctx: &egui::Context) {
        let pending: Pending = Arc::default();
        let slot = Arc::clone(&pending);
        let scope = self.scope.unwrap_or(McpScope::Global);
        let repo = self.repo.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = mcp_store::mcp_scan(scope, repo, None).unwrap_or_default();
            *slot.lock().unwrap() = Some(result);
            ctx.request_repaint();
        });
        self.pending = Some(pending);
    }

    fn poll(&mut self) {
        let done = self.pending.as_ref().and_then(|p| p.lock().unwrap().take());
        if let Some(snapshots) = done {
            self.snapshots = snapshots;
            self.loaded = true;
            self.pending = None;
        }
    }

    fn start_health(&mut self, agent: McpAgent, ctx: &egui::Context) {
        let slot: Arc<Mutex<Option<HealthResult>>> = Arc::default();
        let out = Arc::clone(&slot);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            *out.lock().unwrap() = Some(crate::backend::mcp_health::mcp_health_check(agent));
            ctx.request_repaint();
        });
        self.checking = Some((agent, slot));
    }

    fn poll_health(&mut self) {
        let done = self
            .checking
            .as_ref()
            .and_then(|(agent, slot)| slot.lock().unwrap().take().map(|r| (*agent, r)));
        if let Some((agent, result)) = done {
            self.health.retain(|(a, _)| *a != agent);
            self.health.push((agent, result));
            self.checking = None;
        }
    }

    fn report<T>(&mut self, result: Result<T, String>, ok: String) {
        self.message = Some(match result {
            Ok(_) => (ok, false),
            Err(error) => (error, true),
        });
    }
}

pub fn panel(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let t = |key: &str| crate::i18n::tr(lang, key);
    super::todos::tab_strip(app, ui);

    let repo = active_repo(app);
    let scope_pref = default_scope(app);
    let state = &mut app.mcp_panel;
    if state.scope.is_none() {
        state.scope = Some(scope_pref);
        state.show_skills = std::env::var("ALETHE_NATIVE_SMOKE").as_deref() == Ok("@skills");
        // `ALETHE_NATIVE_SMOKE_MCP=<server>` opens with that server's detail showing.
        state.selected = std::env::var("ALETHE_NATIVE_SMOKE_MCP").ok();
    }
    if !state.loaded && state.pending.is_none() || state.repo != repo {
        state.repo = repo.clone();
        state.rescan(ui.ctx());
    }
    state.poll();
    let scope = state.scope.unwrap_or(McpScope::Global);
    let loading = state.pending.is_some();
    let groups = group_servers(&state.snapshots);
    let mut rescan = false;
    let mut new_scope = None;

    // ---- header
    egui::Frame::new()
        .inner_margin(Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let show_skills = app.mcp_panel.show_skills;
                for (skills, key) in [(false, "mcp.tabServers"), (true, "mcp.tabSkills")] {
                    if ui
                        .selectable_label(
                            show_skills == skills,
                            RichText::new(t(key)).font(widgets::bold(13.0)),
                        )
                        .clicked()
                    {
                        app.mcp_panel.show_skills = skills;
                    }
                }
                ui.add_space(8.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(ui, &theme, icons::LAYOUT_SIDEBAR_RIGHT, "", 24.0)
                        .clicked()
                    {
                        app.show_right = false;
                    }
                    let state = &mut app.mcp_panel;
                    if widgets::icon_button(ui, &theme, icons::REFRESH, t("mcp.refresh"), 24.0)
                        .clicked()
                    {
                        if show_skills {
                            state.skills.refresh(ui.ctx());
                        } else {
                            rescan = true;
                        }
                    }
                    if loading || (show_skills && state.skills.loading()) {
                        ui.add(egui::Spinner::new().size(12.0));
                    }
                    if show_skills {
                        return;
                    }
                    // Scope switch.
                    for (value, key) in [
                        (McpScope::Project, "mcp.scopeProject"),
                        (McpScope::Global, "mcp.scopeGlobal"),
                    ] {
                        let enabled = value == McpScope::Global || repo.is_some();
                        let selected = scope == value;
                        let resp = ui.add_enabled(
                            enabled,
                            egui::Button::selectable(
                                selected,
                                RichText::new(t(key)).font(sans(11.5)),
                            ),
                        );
                        if resp.clicked() && !selected {
                            new_scope = Some(value);
                        }
                    }
                    let _ = state;
                });
            });
        });
    rule(ui, &theme);

    if let Some(value) = new_scope {
        app.data.preferences.extra.insert(
            "mcpDefaultScope".into(),
            serde_json::json!(if value == McpScope::Project {
                "project"
            } else {
                "global"
            }),
        );
        app.mark_dirty();
        let state = &mut app.mcp_panel;
        state.scope = Some(value);
        state.selected = None;
        rescan = true;
    }
    if rescan {
        app.mcp_panel.rescan(ui.ctx());
    }
    if app.mcp_panel.show_skills {
        super::skills::view(app, ui);
        return;
    }

    if app.mcp_panel.adding.is_some() {
        add_form(app, ui, scope);
        return;
    }
    if let Some(name) = app.mcp_panel.selected.clone() {
        if let Some(group) = groups.iter().find(|g| g.name == name) {
            detail(app, ui, group, scope);
            return;
        }
        // Only once a scan has settled: a server just added isn't in the old list yet.
        if app.mcp_panel.loaded && app.mcp_panel.pending.is_none() {
            app.mcp_panel.selected = None;
        }
    }

    // ---- search + agent filter
    egui::Frame::new()
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            let width = ui.available_width();
            widgets::text_input(
                ui,
                &theme,
                &mut app.mcp_panel.search,
                t("mcp.search"),
                width,
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for agent in ALL_MCP_AGENTS {
                    let on = app.mcp_panel.agent_filter.contains(&agent);
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
                    if on || resp.hovered() {
                        ui.painter().rect_filled(
                            r,
                            CornerRadius::same(5),
                            if on { theme.panel } else { theme.panel_hover },
                        );
                    }
                    if on {
                        ui.painter().rect_stroke(
                            r,
                            CornerRadius::same(5),
                            Stroke::new(1.0, theme.border_strong),
                            egui::StrokeKind::Inside,
                        );
                    }
                    paint_agent(
                        ui,
                        agent,
                        r.shrink(5.0),
                        on || app.mcp_panel.agent_filter.is_empty(),
                    );
                    if resp.on_hover_text(agent_label(agent)).clicked() {
                        let filter = &mut app.mcp_panel.agent_filter;
                        if on {
                            filter.retain(|a| *a != agent);
                        } else {
                            filter.push(agent);
                        }
                    }
                }
            });
        });

    let state = &app.mcp_panel;
    let query = state.search.trim().to_lowercase();
    let visible: Vec<&ServerGroup> = groups
        .iter()
        .filter(|g| {
            query.is_empty()
                || g.name.to_lowercase().contains(&query)
                || g.records.iter().any(|r| {
                    transport_summary(&r.server.transport)
                        .to_lowercase()
                        .contains(&query)
                })
        })
        .filter(|g| {
            state.agent_filter.is_empty() || state.agent_filter.iter().any(|a| g.agents.contains(a))
        })
        .collect();

    egui::Frame::new()
        .inner_margin(Margin::symmetric(14, 2))
        .show(ui, |ui| {
            let mut text = format!("{} {}", visible.len(), t("mcp.statServers"));
            if visible.len() != groups.len() {
                text.push_str(&format!(" · {} {}", groups.len(), t("mcp.total")));
            }
            ui.label(RichText::new(text).font(mono(11.0)).color(theme.fg_faint));
        });

    let mut open: Option<String> = None;
    let mut add = false;
    egui::ScrollArea::vertical()
        .id_salt("mcp-list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    if visible.is_empty() && state.loaded {
                        ui.add_space(30.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(icons::PLUG)
                                    .font(mono(22.0))
                                    .color(theme.fg_faint),
                            );
                            ui.label(
                                RichText::new(t(if groups.is_empty() {
                                    "mcp.emptyTitle"
                                } else {
                                    "mcp.noMatch"
                                }))
                                .font(widgets::bold(13.0))
                                .color(theme.fg),
                            );
                            if groups.is_empty() {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(t("mcp.emptyDescription"))
                                            .font(sans(12.0))
                                            .color(theme.fg_muted),
                                    )
                                    .wrap(),
                                );
                            }
                        });
                    }
                    for group in &visible {
                        if server_row(ui, &theme, group, lang) {
                            open = Some(group.name.clone());
                        }
                    }
                    ui.add_space(8.0);
                    if widgets::ghost_button(
                        ui,
                        &theme,
                        &format!("{}  {}", icons::ADD, t("mcp.addMore")),
                    )
                    .clicked()
                    {
                        add = true;
                    }
                    // Diagnostics: agents whose config can't be read or written.
                    let diagnostics: Vec<(McpAgent, &str)> = state
                        .snapshots
                        .iter()
                        .filter_map(|s| {
                            let key = if s.sources.is_empty() {
                                "mcp.diagUnsupported"
                            } else if s.sources.iter().any(|src| src.parse_error.is_some()) {
                                "mcp.diagUnreadable"
                            } else if s.sources.iter().all(|src| !src.exists) {
                                "mcp.diagMissing"
                            } else if s.sources.iter().any(|src| src.exists && !src.writable) {
                                "mcp.diagReadOnly"
                            } else {
                                return None;
                            };
                            Some((s.agent, key))
                        })
                        .collect();
                    if !diagnostics.is_empty() {
                        ui.add_space(10.0);
                        widgets::section_label(ui, &theme, t("mcp.diagTitle"));
                        for (agent, key) in diagnostics {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(agent_label(agent))
                                        .font(widgets::bold(11.5))
                                        .color(theme.fg_muted),
                                );
                                ui.label(
                                    RichText::new(t(key)).font(sans(11.5)).color(theme.fg_faint),
                                );
                            });
                        }
                    }
                });
        });
    if let Some(name) = open {
        app.mcp_panel.selected = Some(name);
        app.mcp_panel.message = None;
    }
    if add {
        app.mcp_panel.adding = Some(AddForm {
            agents: vec![McpAgent::Claude],
            ..Default::default()
        });
        app.mcp_panel.message = None;
    }
}

fn paint_agent(ui: &egui::Ui, agent: McpAgent, rect: Rect, bright: bool) {
    if let Some(image) = icons::agent_image(agent.as_str(), rect.width()) {
        let image = if bright {
            image
        } else {
            image.tint(egui::Color32::from_white_alpha(90))
        };
        image.paint_at(ui, rect);
    } else {
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            &agent_label(agent)[..1],
            mono(rect.height()),
            if bright {
                egui::Color32::WHITE
            } else {
                egui::Color32::from_white_alpha(90)
            },
        );
    }
}

/// `ServerRow`: name, badges, the agents that have it (dim: could have it but don't).
fn server_row(
    ui: &mut egui::Ui,
    theme: &crate::theme::Theme,
    group: &ServerGroup,
    lang: crate::i18n::Lang,
) -> bool {
    let t = |key: &str| crate::i18n::tr(lang, key);
    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(r, CornerRadius::same(6), theme.panel_hover);
    }
    let p = ui.painter();
    let mut x = r.left() + 8.0;
    let name = p.layout_no_wrap(group.name.clone(), widgets::bold(12.5), theme.fg);
    let name_w = name.size().x.min(r.width() * 0.5);
    p.with_clip_rect(Rect::from_min_size(
        Pos2::new(x, r.top()),
        Vec2::new(name_w, 24.0),
    ))
    .galley(
        Pos2::new(x, r.top() + 13.0 - name.size().y / 2.0),
        name,
        theme.fg,
    );
    x += name_w + 6.0;
    if group.any_disabled {
        let badge = p.layout_no_wrap(
            t("mcp.badgeDisabled").to_string(),
            mono(10.0),
            theme.fg_faint,
        );
        let b = Rect::from_min_size(
            Pos2::new(x, r.top() + 5.0),
            badge.size() + Vec2::new(8.0, 4.0),
        );
        p.rect_stroke(
            b,
            CornerRadius::same(4),
            Stroke::new(1.0, theme.border),
            egui::StrokeKind::Inside,
        );
        p.galley(b.min + Vec2::new(4.0, 2.0), badge, theme.fg_faint);
    }
    // Agent icons, right aligned: present bright, missing dim.
    let mut right = r.right() - 6.0;
    for agent in ALL_MCP_AGENTS.iter().rev() {
        let present = group.agents.contains(agent);
        if !present && !group.missing.contains(agent) {
            continue;
        }
        let icon = Rect::from_min_size(Pos2::new(right - 14.0, r.top() + 6.0), Vec2::splat(14.0));
        paint_agent(ui, *agent, icon, present);
        right -= 18.0;
    }
    let summary = group
        .records
        .first()
        .map(|rec| transport_summary(&rec.server.transport))
        .unwrap_or_default();
    ui.painter()
        .with_clip_rect(r.shrink2(Vec2::new(8.0, 0.0)))
        .text(
            Pos2::new(r.left() + 8.0, r.top() + 31.0),
            Align2::LEFT_CENTER,
            summary,
            mono(10.5),
            theme.fg_faint,
        );
    let present: Vec<&str> = group.agents.iter().map(|a| agent_label(*a)).collect();
    resp.on_hover_text(format!("{}: {}", t("mcp.presentOn"), present.join(", ")))
        .clicked()
}

/// One server: where it is configured, per agent toggles, copy to the agents missing it.
fn detail(app: &mut AletheApp, ui: &mut egui::Ui, group: &ServerGroup, scope: McpScope) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let t = |key: &str| crate::i18n::tr(lang, key);
    let repo = app.mcp_panel.repo.clone();
    app.mcp_panel.poll_health();
    let mut back = false;
    let mut changed = false;
    let mut check: Option<McpAgent> = None;
    egui::ScrollArea::vertical()
        .id_salt("mcp-detail")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if widgets::icon_button(ui, &theme, icons::ARROW_LEFT, t("mcp.back"), 22.0)
                            .clicked()
                        {
                            back = true;
                        }
                        ui.label(
                            RichText::new(&group.name)
                                .font(widgets::bold(14.0))
                                .color(theme.fg),
                        );
                    });
                    ui.add_space(6.0);
                    let first = &group.records[0].server;
                    widgets::section_label(ui, &theme, t("mcp.transport"));
                    ui.add(
                        egui::Label::new(
                            RichText::new(transport_summary(&first.transport))
                                .font(mono(11.5))
                                .color(theme.fg_muted),
                        )
                        .wrap(),
                    );
                    if !first.env.is_empty() {
                        ui.add_space(6.0);
                        widgets::section_label(ui, &theme, t("mcp.env"));
                        for (key, value) in &first.env {
                            let shown = value
                                .literal
                                .as_ref()
                                .map(|l| l.preview.clone())
                                .or_else(|| {
                                    value.passthrough_from.as_ref().map(|v| format!("${v}"))
                                })
                                .unwrap_or_default();
                            ui.label(
                                RichText::new(format!("{key} = {shown}"))
                                    .font(mono(11.0))
                                    .color(theme.fg_muted),
                            );
                        }
                    }
                    ui.add_space(10.0);
                    widgets::section_label(ui, &theme, t("mcp.agents"));
                    for record in &group.records {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                            paint_agent(ui, record.agent, r, record.server.enabled);
                            ui.label(
                                RichText::new(agent_label(record.agent))
                                    .font(sans(12.5))
                                    .color(theme.fg),
                            );
                            ui.label(
                                RichText::new(record.source_kind.as_str())
                                    .font(mono(10.5))
                                    .color(theme.fg_faint),
                            )
                            .on_hover_text(&record.source_path);
                            let probed = app
                                .mcp_panel
                                .health
                                .iter()
                                .find(|(a, _)| *a == record.agent)
                                .map(|(_, r)| r);
                            match probed {
                                Some(Ok(list)) => {
                                    let status = list
                                        .iter()
                                        .find(|h| h.name == group.name)
                                        .map(|h| h.status);
                                    let (key, color) = match status {
                                        Some(McpHealthStatus::Connected) => {
                                            ("mcp.health.connected", theme.working)
                                        }
                                        Some(McpHealthStatus::Failed) => {
                                            ("mcp.health.failed", theme.offline)
                                        }
                                        Some(McpHealthStatus::NeedsAuth) => {
                                            ("mcp.health.needsAuth", theme.waiting)
                                        }
                                        Some(McpHealthStatus::Disabled) => {
                                            ("mcp.health.disabled", theme.fg_faint)
                                        }
                                        Some(McpHealthStatus::Unknown) => {
                                            ("mcp.health.unknown", theme.fg_faint)
                                        }
                                        None => ("mcp.health.absent", theme.fg_faint),
                                    };
                                    ui.label(RichText::new(t(key)).font(mono(10.5)).color(color));
                                }
                                Some(Err(error)) => {
                                    let key = format!("mcp.healthError.{error}");
                                    let text = crate::i18n::tr(lang, &key);
                                    ui.label(
                                        RichText::new(if text == key {
                                            error.as_str()
                                        } else {
                                            text
                                        })
                                        .font(mono(10.5))
                                        .color(theme.offline),
                                    );
                                }
                                None => {}
                            }
                            if health_capable(record.agent) {
                                let running = app.mcp_panel.checking.as_ref().map(|(a, _)| *a);
                                if running == Some(record.agent) {
                                    ui.add(egui::Spinner::new().size(11.0));
                                } else if ui
                                    .add_enabled(
                                        running.is_none(),
                                        egui::Button::new(
                                            RichText::new(t("mcp.healthCheck")).font(sans(11.0)),
                                        )
                                        .small(),
                                    )
                                    .on_hover_text(t("mcp.healthCheckHint"))
                                    .clicked()
                                {
                                    check = Some(record.agent);
                                }
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if widgets::icon_button(
                                    ui,
                                    &theme,
                                    icons::TRASH,
                                    t("mcp.remove"),
                                    22.0,
                                )
                                .clicked()
                                {
                                    let result = mcp_store::mcp_remove(
                                        record.agent.as_str().into(),
                                        scope,
                                        repo.clone(),
                                        group.name.clone(),
                                        Some(record.source_kind.as_str().into()),
                                    );
                                    app.mcp_panel.report(
                                        result,
                                        t("mcp.removed")
                                            .replace("{agent}", agent_label(record.agent)),
                                    );
                                    changed = true;
                                }
                                let mut enabled = record.server.enabled;
                                if crate::backend::mcp_model::capability(record.agent).enabled_flag
                                    && ui
                                        .checkbox(
                                            &mut enabled,
                                            RichText::new(t("mcp.enabled")).font(sans(11.5)),
                                        )
                                        .changed()
                                {
                                    let result = mcp_store::mcp_set_enabled(
                                        record.agent.as_str().into(),
                                        scope,
                                        repo.clone(),
                                        group.name.clone(),
                                        enabled,
                                        Some(record.source_kind.as_str().into()),
                                    );
                                    app.mcp_panel.report(result, t("mcp.saved").into());
                                    changed = true;
                                }
                            });
                        });
                    }
                    if !group.missing.is_empty() {
                        ui.add_space(10.0);
                        widgets::section_label(ui, &theme, t("mcp.copyTo"));
                        ui.horizontal_wrapped(|ui| {
                            for agent in &group.missing {
                                if widgets::ghost_button(ui, &theme, agent_label(*agent)).clicked()
                                {
                                    let result = mcp_store::mcp_sync(
                                        group.records[0].agent.as_str().into(),
                                        vec![agent.as_str().into()],
                                        scope,
                                        repo.clone(),
                                        group.name.clone(),
                                        Some(false),
                                    )
                                    .and_then(|outcomes| {
                                        match outcomes.into_iter().find(|o| o.status != "written") {
                                            Some(o) if o.status == "blocked" => Err(format!(
                                                "{}: {}",
                                                t("mcp.syncBlocked"),
                                                o.unsupported
                                                    .iter()
                                                    .map(|u| u.field.clone())
                                                    .collect::<Vec<_>>()
                                                    .join(", ")
                                            )),
                                            Some(o) => {
                                                Err(o.error.unwrap_or_else(|| o.status.to_string()))
                                            }
                                            None => Ok(()),
                                        }
                                    });
                                    app.mcp_panel.report(
                                        result,
                                        t("mcp.copied").replace("{agent}", agent_label(*agent)),
                                    );
                                    changed = true;
                                }
                            }
                        });
                    }
                    message(ui, &theme, &app.mcp_panel.message);
                });
        });
    if back {
        app.mcp_panel.selected = None;
        app.mcp_panel.message = None;
    }
    if changed {
        app.mcp_panel.rescan(ui.ctx());
    }
    if let Some(agent) = check {
        app.mcp_panel.start_health(agent, ui.ctx());
    }
}

fn message(ui: &mut egui::Ui, theme: &crate::theme::Theme, message: &Option<(String, bool)>) {
    if let Some((text, failed)) = message {
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(RichText::new(text).font(sans(12.0)).color(if *failed {
                theme.offline
            } else {
                theme.working
            }))
            .wrap(),
        );
    }
}

/// "Add server": stdio command or remote URL, written to every chosen agent's config.
fn add_form(app: &mut AletheApp, ui: &mut egui::Ui, scope: McpScope) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let t = |key: &str| crate::i18n::tr(lang, key);
    let repo = app.mcp_panel.repo.clone();
    let mut cancel = false;
    let mut submit = false;
    egui::ScrollArea::vertical()
        .id_salt("mcp-add")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    let width = ui.available_width();
                    let form = app.mcp_panel.adding.as_mut().expect("adding");
                    ui.label(
                        RichText::new(t("mcp.addTitle"))
                            .font(widgets::bold(14.0))
                            .color(theme.fg),
                    );
                    ui.label(
                        RichText::new(t(if scope == McpScope::Project {
                            "mcp.scopeProjectHint"
                        } else {
                            "mcp.scopeGlobalHint"
                        }))
                        .font(sans(11.5))
                        .color(theme.fg_faint),
                    );
                    ui.add_space(8.0);
                    widgets::section_label(ui, &theme, t("mcp.name"));
                    widgets::text_input(ui, &theme, &mut form.name, "github", width);
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        for (value, label) in [
                            (Transport::Stdio, "stdio"),
                            (Transport::Http, "http"),
                            (Transport::Sse, "sse"),
                        ] {
                            if ui
                                .selectable_label(
                                    form.transport == value,
                                    RichText::new(label).font(mono(11.5)),
                                )
                                .clicked()
                            {
                                form.transport = value;
                            }
                        }
                    });
                    ui.add_space(4.0);
                    if form.transport == Transport::Stdio {
                        widgets::section_label(ui, &theme, t("mcp.command"));
                        widgets::text_input(ui, &theme, &mut form.command, "npx", width);
                        widgets::section_label(ui, &theme, t("mcp.args"));
                        ui.add(
                            egui::TextEdit::multiline(&mut form.args)
                                .desired_rows(3)
                                .desired_width(width)
                                .hint_text("-y\n@modelcontextprotocol/server-github")
                                .font(mono(11.5)),
                        );
                    } else {
                        widgets::section_label(ui, &theme, "URL");
                        widgets::text_input(ui, &theme, &mut form.url, "https://", width);
                    }
                    widgets::section_label(ui, &theme, t("mcp.env"));
                    ui.add(
                        egui::TextEdit::multiline(&mut form.env)
                            .desired_rows(2)
                            .desired_width(width)
                            .hint_text("GITHUB_TOKEN=…")
                            .font(mono(11.5)),
                    );
                    ui.add_space(6.0);
                    widgets::section_label(ui, &theme, t("mcp.agents"));
                    ui.horizontal_wrapped(|ui| {
                        for agent in ALL_MCP_AGENTS {
                            let mut on = form.agents.contains(&agent);
                            if ui.checkbox(&mut on, agent_label(agent)).changed() {
                                if on {
                                    form.agents.push(agent);
                                } else {
                                    form.agents.retain(|a| *a != agent);
                                }
                            }
                        }
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if widgets::primary_button(ui, &theme, t("mcp.add")).clicked() {
                            submit = true;
                        }
                        if widgets::ghost_button(ui, &theme, t("dialog.cancel")).clicked() {
                            cancel = true;
                        }
                    });
                    message(ui, &theme, &app.mcp_panel.message);
                });
        });
    if cancel {
        app.mcp_panel.adding = None;
        app.mcp_panel.message = None;
        return;
    }
    if !submit {
        return;
    }
    let form = app.mcp_panel.adding.as_ref().expect("adding");
    let name = form.name.trim().to_string();
    if name.is_empty() || form.agents.is_empty() {
        app.mcp_panel.message = Some((t("mcp.addMissing").into(), true));
        return;
    }
    let env: Vec<McpEnvInput> = form
        .env
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| McpEnvInput {
            key: k.trim().to_string(),
            value: Some(v.trim().to_string()),
            passthrough_from: None,
        })
        .collect();
    let transport = match form.transport {
        Transport::Stdio => McpTransportInput::Stdio {
            command: form.command.trim().to_string(),
            args: form
                .args
                .lines()
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(str::to_string)
                .collect(),
            cwd: None,
        },
        Transport::Http => McpTransportInput::Http {
            url: form.url.trim().to_string(),
            headers: vec![],
        },
        Transport::Sse => McpTransportInput::Sse {
            url: form.url.trim().to_string(),
            headers: vec![],
        },
    };
    let mut errors = Vec::new();
    for agent in form.agents.clone() {
        let input = McpServerInput {
            name: name.clone(),
            transport: transport.clone(),
            env: env.clone(),
            enabled: true,
            timeouts: Default::default(),
            bearer_token_env_var: None,
        };
        if let Err(error) =
            mcp_store::mcp_upsert(agent.as_str().into(), scope, repo.clone(), input, None)
        {
            errors.push(format!("{}: {error}", agent_label(agent)));
        }
    }
    if errors.is_empty() {
        app.mcp_panel.adding = None;
        app.mcp_panel.selected = Some(name);
        app.mcp_panel.message = Some((t("mcp.saved").into(), false));
    } else {
        app.mcp_panel.message = Some((errors.join("\n"), true));
    }
    app.mcp_panel.rescan(ui.ctx());
}

fn rule(ui: &mut egui::Ui, theme: &crate::theme::Theme) {
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment(
        [r.left_top(), Pos2::new(r.right(), r.top())],
        Stroke::new(1.0, theme.border),
    );
}
