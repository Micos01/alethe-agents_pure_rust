//! The Skills view of the MCP panel (`McpPanel` "skills" + the skills tab of `McpManagerModal`).
//!
//! Lists the skills each agent CLI has installed (`~/.claude/skills`, `~/.codex/skills`,
//! OpenCode, Antigravity and the shared `~/.agents/skills` store), grouped by name. A skill opens
//! to its SKILL.md; removing one goes through the vendored `skills` backend, which removes only
//! the link when the folder is a link into the shared store.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::icons;
use super::widgets::{self, mono, sans};
use crate::app::AletheApp;
use crate::backend::skills::{
    self, BundledSkillAgent, BundledSkillEntry, SkillAgentSnapshot, SkillDetail, SkillSummary,
};
use egui::{Align, Align2, CornerRadius, Layout, Margin, Pos2, Rect, RichText, Sense, Vec2};

type Pending = Arc<Mutex<Option<Vec<SkillAgentSnapshot>>>>;

#[derive(Default)]
pub struct SkillsState {
    snapshots: Vec<SkillAgentSnapshot>,
    loaded: bool,
    pending: Option<Pending>,
    search: String,
    /// (agent, name) shown in detail, with its loaded SKILL.md.
    open: Option<(String, String, Result<SkillDetail, String>)>,
    /// Asked once before deleting: the agent whose copy would go.
    confirm_remove: Option<String>,
    message: Option<(String, bool)>,
    /// Alethe's own skills (`OfficialSkills`), for the folder they go to: None = global.
    catalog: Option<(Option<String>, Vec<BundledSkillEntry>)>,
    catalog_in_project: bool,
    catalog_message: Option<(String, bool)>,
}

/// `pendingBundledAgents`: agents without Alethe's current copy.
fn pending_agents(skill: &BundledSkillEntry) -> Vec<String> {
    skill
        .agents
        .iter()
        .filter(|a| a.state == "available" || a.state == "update")
        .map(|a| a.agent.clone())
        .collect()
}

/// `installedBundledAgents`: agents holding Alethe's copy, current or outdated.
fn installed_agents(skill: &BundledSkillEntry) -> Vec<String> {
    skill
        .agents
        .iter()
        .filter(|a| a.state == "installed" || a.state == "update")
        .map(|a| a.agent.clone())
        .collect()
}

enum CatalogAction {
    Install(String, Vec<String>),
    Remove(String, Vec<String>),
}

/// The official skills card: install into every agent's global folder or into this project's,
/// where they travel with the repository.
fn catalog_section(
    ui: &mut egui::Ui,
    theme: &crate::theme::Theme,
    lang: crate::i18n::Lang,
    state: &mut SkillsState,
    repo: Option<&str>,
) {
    let t = |key: &str| crate::i18n::tr(lang, key);
    if repo.is_none() {
        state.catalog_in_project = false;
    }
    let target = state
        .catalog_in_project
        .then(|| repo.map(str::to_string))
        .flatten();
    if state.catalog.as_ref().is_none_or(|(at, _)| *at != target) {
        let entries = skills::skills_catalog(target.clone()).unwrap_or_default();
        state.catalog = Some((target.clone(), entries));
    }
    let mut action: Option<CatalogAction> = None;
    let mut switch: Option<bool> = None;
    egui::Frame::new()
        .fill(theme.panel)
        .stroke(egui::Stroke::new(1.0, theme.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(t("skills.officialTitle"))
                        .font(widgets::bold(12.5))
                        .color(theme.fg),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    for (project, key) in [(true, "mcp.scopeProject"), (false, "mcp.scopeGlobal")] {
                        let on = state.catalog_in_project == project;
                        if ui
                            .add_enabled(
                                !project || repo.is_some(),
                                egui::Button::selectable(
                                    on,
                                    RichText::new(t(key)).font(sans(11.0)),
                                ),
                            )
                            .clicked()
                            && !on
                        {
                            switch = Some(project);
                        }
                    }
                });
            });
            ui.label(
                RichText::new(t(if target.is_some() {
                    "skills.projectDescription"
                } else {
                    "skills.globalDescription"
                }))
                .font(sans(11.0))
                .color(theme.fg_faint),
            );
            let Some((_, entries)) = &state.catalog else {
                return;
            };
            for skill in entries {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&skill.name)
                            .font(widgets::bold(12.5))
                            .color(theme.fg),
                    );
                    ui.label(
                        RichText::new(format!("v{}", skill.version))
                            .font(mono(10.5))
                            .color(theme.fg_faint),
                    );
                    if !skill.repository.is_empty() {
                        ui.hyperlink_to(
                            RichText::new(t("skills.sourceAction")).font(sans(11.0)),
                            &skill.repository,
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let installed = installed_agents(skill);
                        if !installed.is_empty()
                            && ui
                                .button(
                                    RichText::new(t("skills.removeAction"))
                                        .font(sans(11.0))
                                        .color(theme.offline),
                                )
                                .clicked()
                        {
                            action = Some(CatalogAction::Remove(skill.name.clone(), installed));
                        }
                        let pending = pending_agents(skill);
                        if !pending.is_empty() {
                            let update = skill.agents.iter().any(|a| a.state == "update");
                            if ui
                                .button(
                                    RichText::new(t(if update {
                                        "skills.updateAction"
                                    } else {
                                        "skills.installAction"
                                    }))
                                    .font(sans(11.0)),
                                )
                                .clicked()
                            {
                                action = Some(CatalogAction::Install(skill.name.clone(), pending));
                            }
                        }
                    });
                });
                if !skill.description.is_empty() {
                    let short = match skill.description.char_indices().nth(180) {
                        Some((cut, _)) => format!("{}…", skill.description[..cut].trim_end()),
                        None => skill.description.clone(),
                    };
                    ui.add(
                        egui::Label::new(
                            RichText::new(short).font(sans(11.5)).color(theme.fg_muted),
                        )
                        .wrap(),
                    )
                    .on_hover_text(&skill.description);
                }
                for entry in &skill.agents {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(agent_label(&entry.agent))
                                .font(sans(11.5))
                                .color(theme.fg),
                        );
                        let color = match entry.state.as_str() {
                            "installed" => theme.working,
                            "update" => theme.waiting,
                            "conflict" => theme.offline,
                            _ => theme.fg_faint,
                        };
                        ui.label(
                            RichText::new(t(&format!("skills.state.{}", entry.state)))
                                .font(mono(10.5))
                                .color(color),
                        )
                        .on_hover_text(entry.path.clone().unwrap_or_default());
                    });
                }
                if skill.agents.iter().any(|a| a.state == "conflict") {
                    ui.add(
                        egui::Label::new(
                            RichText::new(t("skills.conflictNote"))
                                .font(sans(11.0))
                                .color(theme.fg_faint),
                        )
                        .wrap(),
                    );
                }
            }
            if let Some((text, failed)) = &state.catalog_message {
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(RichText::new(text).font(sans(11.5)).color(if *failed {
                        theme.offline
                    } else {
                        theme.working
                    }))
                    .wrap(),
                );
            }
        });
    if let Some(project) = switch {
        state.catalog_in_project = project;
        state.catalog_message = None;
    }
    let Some(action) = action else {
        return;
    };
    let failed = |report: &[BundledSkillAgent], ok: &dyn Fn(&BundledSkillAgent) -> bool| {
        report
            .iter()
            .filter(|e| !ok(e))
            .map(|e| {
                format!(
                    "{}: {}",
                    agent_label(&e.agent),
                    e.error
                        .clone()
                        .unwrap_or_else(|| t("skills.conflictNote").into())
                )
            })
            .collect::<Vec<_>>()
    };
    state.catalog_message = Some(match action {
        CatalogAction::Install(name, agents) => {
            match skills::skills_install_bundled(name.clone(), agents, target.clone()) {
                Ok(report) => {
                    let errors = failed(&report, &|e| e.state == "installed");
                    if errors.is_empty() {
                        (t("skills.installed").replace("{name}", &name), false)
                    } else {
                        (errors.join("\n"), true)
                    }
                }
                Err(error) => (error, true),
            }
        }
        CatalogAction::Remove(name, agents) => {
            match skills::skills_remove_bundled(name.clone(), agents, target.clone()) {
                Ok(report) => {
                    let errors =
                        failed(&report, &|e| e.state != "installed" && e.state != "update");
                    if errors.is_empty() {
                        (t("skills.removedOfficial").replace("{name}", &name), false)
                    } else {
                        (errors.join("\n"), true)
                    }
                }
                Err(error) => (error, true),
            }
        }
    });
    // Reload the states, and the installed list the install just changed.
    state.catalog = None;
    state.loaded = false;
}

struct SkillGroup {
    name: String,
    description: String,
    copies: Vec<SkillSummary>,
}

const AGENTS: [&str; 5] = ["claude", "codex", "opencode", "antigravity", "shared"];

fn agent_label(agent: &str) -> &'static str {
    match agent {
        "shared" => "~/.agents",
        other => crate::agents::info(other).label,
    }
}

impl SkillsState {
    fn rescan(&mut self, ctx: &egui::Context) {
        let pending: Pending = Arc::default();
        let slot = Arc::clone(&pending);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            *slot.lock().unwrap() = Some(skills::skills_scan().unwrap_or_default());
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

    pub fn loading(&self) -> bool {
        self.pending.is_some()
    }

    pub fn refresh(&mut self, ctx: &egui::Context) {
        self.open = None;
        self.catalog = None;
        self.rescan(ctx);
    }
}

fn groups(snapshots: &[SkillAgentSnapshot]) -> Vec<SkillGroup> {
    let mut by_name: BTreeMap<String, Vec<SkillSummary>> = BTreeMap::new();
    for snapshot in snapshots {
        for skill in &snapshot.skills {
            by_name
                .entry(skill.name.clone())
                .or_default()
                .push(skill.clone());
        }
    }
    by_name
        .into_iter()
        .map(|(name, copies)| SkillGroup {
            description: copies
                .iter()
                .map(|c| c.description.clone())
                .find(|d| !d.trim().is_empty())
                .unwrap_or_default(),
            name,
            copies,
        })
        .collect()
}

pub fn view(app: &mut AletheApp, ui: &mut egui::Ui, repo: Option<String>) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let t = |key: &str| crate::i18n::tr(lang, key);
    let state = &mut app.mcp_panel.skills;
    if !state.loaded && state.pending.is_none() {
        state.rescan(ui.ctx());
    }
    state.poll();

    if state.open.is_some() {
        detail(app, ui);
        return;
    }

    let all = groups(&state.snapshots);
    egui::Frame::new()
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            let width = ui.available_width();
            widgets::text_input(ui, &theme, &mut state.search, t("skills.search"), width);
        });
    let query = state.search.trim().to_lowercase();
    let visible: Vec<&SkillGroup> = all
        .iter()
        .filter(|g| {
            query.is_empty()
                || g.name.to_lowercase().contains(&query)
                || g.description.to_lowercase().contains(&query)
        })
        .collect();
    egui::Frame::new()
        .inner_margin(Margin::symmetric(14, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("{} skills", visible.len()))
                    .font(mono(11.0))
                    .color(theme.fg_faint),
            );
        });

    let mut open: Option<(String, String)> = None;
    egui::ScrollArea::vertical()
        .id_salt("skills-list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    if query.is_empty() {
                        catalog_section(ui, &theme, lang, state, repo.as_deref());
                        ui.add_space(8.0);
                    }
                    if visible.is_empty() && state.loaded {
                        ui.add_space(30.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(t(if all.is_empty() {
                                    "skills.emptyTitle"
                                } else {
                                    "mcp.noMatch"
                                }))
                                .font(widgets::bold(13.0))
                                .color(theme.fg),
                            );
                        });
                    }
                    for group in visible {
                        let (r, resp) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 42.0),
                            Sense::click(),
                        );
                        if resp.hovered() {
                            ui.painter()
                                .rect_filled(r, CornerRadius::same(6), theme.panel_hover);
                        }
                        let p = ui.painter();
                        p.with_clip_rect(r.shrink2(Vec2::new(8.0, 0.0))).text(
                            Pos2::new(r.left() + 8.0, r.top() + 13.0),
                            Align2::LEFT_CENTER,
                            &group.name,
                            widgets::bold(12.5),
                            theme.fg,
                        );
                        let mut right = r.right() - 6.0;
                        for agent in AGENTS.iter().rev() {
                            if !group.copies.iter().any(|c| c.agent == *agent) {
                                continue;
                            }
                            let icon = Rect::from_min_size(
                                Pos2::new(right - 14.0, r.top() + 6.0),
                                Vec2::splat(14.0),
                            );
                            if let Some(image) = icons::agent_image(agent, 14.0) {
                                image.paint_at(ui, icon);
                            } else {
                                ui.painter().text(
                                    icon.center(),
                                    Align2::CENTER_CENTER,
                                    icons::LINK,
                                    mono(11.0),
                                    theme.fg_muted,
                                );
                            }
                            right -= 18.0;
                        }
                        ui.painter()
                            .with_clip_rect(r.shrink2(Vec2::new(8.0, 0.0)))
                            .text(
                                Pos2::new(r.left() + 8.0, r.top() + 31.0),
                                Align2::LEFT_CENTER,
                                group.description.lines().next().unwrap_or_default(),
                                sans(11.0),
                                theme.fg_faint,
                            );
                        let agents: Vec<&str> =
                            group.copies.iter().map(|c| agent_label(&c.agent)).collect();
                        if resp.on_hover_text(agents.join(", ")).clicked() {
                            if let Some(first) = group.copies.first() {
                                open = Some((first.agent.clone(), group.name.clone()));
                            }
                        }
                    }
                });
        });
    if let Some((agent, name)) = open {
        let detail = skills::skills_detail(agent.clone(), name.clone());
        state.open = Some((agent, name, detail));
        state.confirm_remove = None;
        state.message = None;
    }
}

fn detail(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let t = |key: &str| crate::i18n::tr(lang, key);
    let all = groups(&app.mcp_panel.skills.snapshots);
    let state = &mut app.mcp_panel.skills;
    let Some((agent, name, loaded)) = &state.open else {
        return;
    };
    let (agent, name) = (agent.clone(), name.clone());
    let copies: Vec<SkillSummary> = all
        .iter()
        .find(|g| g.name == name)
        .map(|g| g.copies.clone())
        .unwrap_or_default();
    let mut back = false;
    let mut switch_to: Option<String> = None;
    let mut remove: Option<String> = None;
    egui::ScrollArea::vertical()
        .id_salt("skill-detail")
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
                            RichText::new(&name)
                                .font(widgets::bold(14.0))
                                .color(theme.fg),
                        );
                    });
                    ui.add_space(6.0);
                    widgets::section_label(ui, &theme, t("skills.installedIn"));
                    for copy in &copies {
                        ui.horizontal(|ui| {
                            let shown = copy.agent == agent;
                            if ui
                                .selectable_label(
                                    shown,
                                    RichText::new(agent_label(&copy.agent)).font(sans(12.0)),
                                )
                                .on_hover_text(&copy.path)
                                .clicked()
                                && !shown
                            {
                                switch_to = Some(copy.agent.clone());
                            }
                            let mut tags = Vec::new();
                            if copy.linked {
                                tags.push(t("skills.linked"));
                            }
                            if copy.bundled {
                                tags.push(t("skills.bundled"));
                            }
                            ui.label(
                                RichText::new(tags.join(" · "))
                                    .font(mono(10.5))
                                    .color(theme.fg_faint),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if copy.bundled {
                                    return;
                                }
                                let confirming =
                                    state.confirm_remove.as_deref() == Some(copy.agent.as_str());
                                if confirming {
                                    if ui
                                        .button(
                                            RichText::new(t("skills.confirmRemove"))
                                                .color(theme.offline),
                                        )
                                        .clicked()
                                    {
                                        remove = Some(copy.agent.clone());
                                    }
                                } else if widgets::icon_button(
                                    ui,
                                    &theme,
                                    icons::TRASH,
                                    t("skills.remove"),
                                    22.0,
                                )
                                .clicked()
                                {
                                    state.confirm_remove = Some(copy.agent.clone());
                                }
                            });
                        });
                    }
                    if let Some((text, failed)) = &state.message {
                        ui.label(RichText::new(text).font(sans(12.0)).color(if *failed {
                            theme.offline
                        } else {
                            theme.working
                        }));
                    }
                    ui.add_space(8.0);
                    match loaded {
                        Ok(detail) => {
                            for (key, value) in &detail.frontmatter {
                                if key == "name" || key == "description" {
                                    continue;
                                }
                                ui.label(
                                    RichText::new(format!("{key}: {value}"))
                                        .font(mono(10.5))
                                        .color(theme.fg_faint),
                                );
                            }
                            if let Some(description) = detail.frontmatter.get("description") {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(description)
                                            .font(sans(12.5))
                                            .color(theme.fg_muted),
                                    )
                                    .wrap(),
                                );
                            }
                            ui.separator();
                            super::markdown::render(ui, &theme, &detail.body);
                        }
                        Err(error) => {
                            ui.label(RichText::new(error).color(theme.offline));
                        }
                    }
                });
        });
    if back {
        state.open = None;
        state.message = None;
        return;
    }
    if let Some(agent) = switch_to {
        let detail = skills::skills_detail(agent.clone(), name.clone());
        state.open = Some((agent, name.clone(), detail));
        state.confirm_remove = None;
    }
    if let Some(target) = remove {
        state.confirm_remove = None;
        match skills::skills_uninstall(target.clone(), name.clone()) {
            Ok(_) => {
                state.message = Some((
                    t("skills.removed").replace("{agent}", agent_label(&target)),
                    false,
                ));
                let left: Vec<&SkillSummary> =
                    copies.iter().filter(|c| c.agent != target).collect();
                match left.first() {
                    Some(next) if target == agent => {
                        let detail = skills::skills_detail(next.agent.clone(), name.clone());
                        state.open = Some((next.agent.clone(), name, detail));
                    }
                    Some(_) => {}
                    None => state.open = None,
                }
            }
            Err(error) => state.message = Some((error, true)),
        }
        state.rescan(ui.ctx());
    }
}
