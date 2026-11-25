//! `WorkspaceView` / `ProjectContainer` / `TerminalPane`: the active project's panes laid out by
//! its `layoutMode`, each with a header (sub-tabs, status, maximize, close) and a live terminal.

use super::icons;
use super::widgets::{self, mono, sans, status_color};
use crate::app::{Action, AletheApp};
use crate::terminal::{view, Status, TermSize};
use egui::{
    Align, Align2, CornerRadius, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2,
};

const HEADER_H: f32 = 30.0;
const GAP: f32 = 6.0;

struct PaneInfo {
    id: String,
    title: String,
    cwd: String,
    is_terminal: bool,
    kind: String,
    file_path: Option<String>,
    tabs: Vec<(String, String, String)>, // (id, agent, name)
    active_tab: String,
}

pub fn show(app: &mut AletheApp, ui: &mut egui::Ui, project_id: &str) {
    let theme = app.theme.clone();
    let Some(project) = app.project(project_id) else {
        app.act(Action::OpenHome);
        return;
    };
    let layout = project.layout_mode.clone();
    let project_cwd = project.cwd();
    let mut panes: Vec<PaneInfo> = project
        .terminals
        .iter()
        .filter(|t| !t.disabled)
        .map(|t| PaneInfo {
            id: t.id.clone(),
            title: app.terminal_title(t),
            cwd: t.cwd.clone(),
            is_terminal: t.is_terminal(),
            kind: t.kind.clone().unwrap_or_else(|| "terminal".into()),
            file_path: t.file_path(),
            tabs: t
                .tabs
                .iter()
                .map(|s| (s.id.clone(), s.agent.clone(), app.tab_title(s)))
                .collect(),
            active_tab: t.active_tab().map(|s| s.id.clone()).unwrap_or_default(),
        })
        .collect();

    let area = ui.max_rect().shrink(GAP);
    if panes.is_empty() {
        empty_state(app, ui, project_id, area);
        return;
    }

    if let Some(max) = app.maximized.clone() {
        if panes.iter().any(|p| p.id == max) {
            panes.retain(|p| p.id == max);
        }
    }

    let focused = app.focused_terminal.clone();
    let focused_index = panes
        .iter()
        .position(|p| Some(&p.id) == focused.as_ref())
        .unwrap_or(0);
    let rects = layout_rects(area, panes.len(), &layout, focused_index);
    let focus_request = app.focus_request.take();

    for (pane, rect) in panes.iter().zip(rects) {
        let is_focused = focused.as_deref() == Some(&pane.id);
        let request = focus_request.as_deref() == Some(&pane.id);
        pane_ui(app, ui, pane, rect, is_focused, request, &project_cwd);
    }
    let _ = &theme;
}

/// Pane rectangles for a layout mode (`gridLayout.ts`, simplified to equal tracks).
fn layout_rects(area: Rect, n: usize, mode: &str, focused: usize) -> Vec<Rect> {
    if n == 1 {
        return vec![area];
    }
    match mode {
        "spotlight" | "sidebar" => {
            let main_w = area.width() * 0.68;
            let side_w = area.width() - main_w - GAP;
            let (main_x, side_x) = if mode == "sidebar" {
                (area.left() + side_w + GAP, area.left())
            } else {
                (area.left(), area.left() + main_w + GAP)
            };
            let others = n - 1;
            let each_h = (area.height() - GAP * (others as f32 - 1.0)) / others as f32;
            let mut out = Vec::with_capacity(n);
            let mut k = 0;
            for i in 0..n {
                if i == focused {
                    out.push(Rect::from_min_size(
                        Pos2::new(main_x, area.top()),
                        Vec2::new(main_w, area.height()),
                    ));
                } else {
                    out.push(Rect::from_min_size(
                        Pos2::new(side_x, area.top() + k as f32 * (each_h + GAP)),
                        Vec2::new(side_w, each_h),
                    ));
                    k += 1;
                }
            }
            out
        }
        _ => {
            // auto / grid: near-square grid; the last row stretches to fill the width.
            let wide = area.width() > area.height() * 1.2;
            let cols = if n == 2 {
                if wide {
                    2
                } else {
                    1
                }
            } else {
                (n as f32).sqrt().ceil() as usize
            };
            let rows = n.div_ceil(cols);
            let cell_h = (area.height() - GAP * (rows as f32 - 1.0)) / rows as f32;
            let mut out = Vec::with_capacity(n);
            for r in 0..rows {
                let in_row = if r == rows - 1 {
                    n - cols * (rows - 1)
                } else {
                    cols
                };
                let cell_w = (area.width() - GAP * (in_row as f32 - 1.0)) / in_row as f32;
                for c in 0..in_row {
                    out.push(Rect::from_min_size(
                        Pos2::new(
                            area.left() + c as f32 * (cell_w + GAP),
                            area.top() + r as f32 * (cell_h + GAP),
                        ),
                        Vec2::new(cell_w, cell_h),
                    ));
                }
            }
            out
        }
    }
}

fn pane_ui(
    app: &mut AletheApp,
    ui: &mut egui::Ui,
    pane: &PaneInfo,
    rect: Rect,
    focused: bool,
    focus_request: bool,
    project_cwd: &str,
) {
    match (pane.kind.as_str(), &pane.file_path) {
        ("orchestrator", _) => return super::orchestrator::pane(app, ui, &pane.id, rect, focused),
        ("markdown", Some(path)) => {
            return super::markdown::pane(app, ui, &pane.id, &pane.title, path, rect, focused)
        }
        _ => {}
    }
    let theme = app.theme.clone();
    let border = if focused {
        theme.accent_border
    } else {
        theme.border
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        theme.bg_sunken,
        Stroke::NONE,
        StrokeKind::Inside,
    );

    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEADER_H));
    let body = Rect::from_min_max(
        Pos2::new(rect.left() + 1.0, header.bottom()),
        Pos2::new(rect.right() - 1.0, rect.bottom() - 1.0),
    );

    // ---------- body ----------
    let tab_id = pane.active_tab.clone();
    let agent = pane
        .tabs
        .iter()
        .find(|t| t.0 == tab_id)
        .map(|t| t.1.clone())
        .unwrap_or_else(|| "shell".into());
    let mut status: Option<Status> = None;
    if !pane.is_terminal {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(body)
                .layout(Layout::centered_and_justified(egui::Direction::TopDown)),
        );
        child.label(
            RichText::new(format!("{}\n\n({})", app.t("pane.unsupported"), pane.kind))
                .color(theme.fg_faint)
                .font(sans(13.0)),
        );
    } else {
        let font_size = app.data.preferences.terminal_font_size.clamp(8.0, 28.0);
        let (cell_w, cell_h) = view::cell_metrics(ui.ctx(), font_size);
        let inner = body.shrink(view::PADDING);
        let size = TermSize {
            cols: ((inner.width() / cell_w).floor() as usize).max(2),
            lines: ((inner.height() / cell_h).floor() as usize).max(1),
        };
        match app.ensure_session(&pane.id, size) {
            Some(session) => {
                status = Some(session.status());
                let palette = crate::theme::term_palette(&app.terminal_theme_id());
                let out = view::show(
                    ui,
                    body,
                    egui::Id::unique(("term", &tab_id)),
                    &session,
                    palette,
                    font_size,
                    focus_request,
                );
                if (out.clicked || (out.focused && !focused)) && !focused {
                    app.act(Action::FocusTerminal(pane.id.clone()));
                }
                if session.is_exited() {
                    exited_overlay(app, ui, body, &tab_id, session.exit_code());
                }
            }
            None => {
                let error = app.spawn_errors.get(&tab_id).cloned().unwrap_or_default();
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(body.shrink(24.0))
                        .layout(Layout::top_down(Align::Center)),
                );
                child.add_space(body.height() * 0.3);
                child.label(
                    RichText::new(icons::TERMINAL)
                        .font(mono(28.0))
                        .color(theme.offline),
                );
                child.label(RichText::new(&error).color(theme.fg_muted).font(mono(12.0)));
                child.add_space(8.0);
                if widgets::ghost_button(&mut child, &theme, app.t("pane.restart")).clicked() {
                    app.act(Action::RestartTab(tab_id.clone()));
                }
            }
        }
    }

    // ---------- header ----------
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
            .max_rect(header.shrink2(Vec2::new(8.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    hui.set_clip_rect(header);
    hui.spacing_mut().item_spacing.x = 4.0;

    widgets::status_dot(
        &mut hui,
        status_color(&theme, status),
        status == Some(Status::Working),
    );
    // Sub-tabs (`SubTabsLane`).
    let many = pane.tabs.len() > 1;
    for (sub_id, sub_agent, sub_name) in &pane.tabs {
        let active = *sub_id == pane.active_tab;
        let label = if many {
            sub_name.clone()
        } else {
            pane.title.clone()
        };
        let galley = hui.painter().layout_no_wrap(
            label,
            if active {
                widgets::bold(12.5)
            } else {
                sans(12.5)
            },
            theme.fg,
        );
        let w = galley.size().x.min(180.0) + 30.0 + if many { 16.0 } else { 0.0 };
        let (r, response) = hui.allocate_exact_size(Vec2::new(w, 22.0), Sense::click());
        if many && (active || response.hovered()) {
            hui.painter().rect_filled(
                r,
                CornerRadius::same(5),
                if active {
                    theme.panel
                } else {
                    theme.panel_hover
                },
            );
        }
        if let Some(image) = icons::agent_image(sub_agent, 14.0) {
            image.paint_at(
                &hui,
                Rect::from_center_size(Pos2::new(r.left() + 13.0, r.center().y), Vec2::splat(14.0)),
            );
        } else {
            hui.painter().text(
                Pos2::new(r.left() + 13.0, r.center().y),
                Align2::CENTER_CENTER,
                icons::TERMINAL,
                mono(12.0),
                theme.agent_color(sub_agent),
            );
        }
        let text_rect = Rect::from_min_max(
            Pos2::new(r.left() + 24.0, r.top()),
            Pos2::new(r.right() - if many { 18.0 } else { 4.0 }, r.bottom()),
        );
        let color = if active { theme.fg } else { theme.fg_muted };
        hui.painter().with_clip_rect(text_rect).galley(
            Pos2::new(text_rect.left(), r.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
        if many {
            let close_rect = Rect::from_center_size(
                Pos2::new(r.right() - 10.0, r.center().y),
                Vec2::splat(14.0),
            );
            let close = hui.interact(
                close_rect,
                egui::Id::unique(("subtab-close", sub_id)),
                Sense::click(),
            );
            if response.hovered() || close.hovered() || active {
                hui.painter().text(
                    close_rect.center(),
                    Align2::CENTER_CENTER,
                    icons::CLOSE,
                    mono(10.0),
                    theme.fg_faint,
                );
            }
            if close.clicked() {
                app.act(Action::CloseSubTab {
                    terminal_id: pane.id.clone(),
                    tab_id: sub_id.clone(),
                });
                continue;
            }
        }
        if response.clicked() {
            app.act(Action::SelectSubTab {
                terminal_id: pane.id.clone(),
                tab_id: sub_id.clone(),
            });
            app.act(Action::FocusTerminal(pane.id.clone()));
        }
    }
    let add = widgets::icon_button(&mut hui, &theme, icons::ADD, app.t("pane.newTab"), 22.0);
    let pane_id = pane.id.clone();
    egui::Popup::menu(&add).show(|ui| {
        ui.set_min_width(200.0);
        for info in crate::agents::available() {
            if !app.data.preferences.agent_enabled(info.id) {
                continue;
            }
            let clicked = ui
                .horizontal(|ui| {
                    widgets::agent_icon(ui, &theme, info.id, 16.0);
                    ui.add(egui::Button::new(info.label).frame(false)).clicked()
                })
                .inner;
            if clicked {
                app.act(Action::AddSubTab {
                    terminal_id: pane_id.clone(),
                    agent: info.id.to_string(),
                });
                ui.close();
            }
        }
    });

    hui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if widgets::icon_button(ui, &theme, icons::CLOSE, app.t("pane.close"), 22.0).clicked() {
            app.act(Action::CloseTerminal(pane.id.clone()));
        }
        let maximized = app.maximized.as_deref() == Some(&pane.id);
        let (glyph, tip) = if maximized {
            (icons::SCREEN_NORMAL, app.t("pane.restore"))
        } else {
            (icons::SCREEN_FULL, app.t("pane.maximize"))
        };
        if widgets::icon_button(ui, &theme, glyph, tip, 22.0).clicked() {
            app.act(Action::ToggleMaximize(pane.id.clone()));
        }
        let cwd = if pane.cwd.is_empty() {
            project_cwd
        } else {
            &pane.cwd
        };
        let short = std::path::Path::new(cwd)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| cwd.to_string());
        ui.label(
            RichText::new(format!("{} {short}", icons::FOLDER))
                .font(mono(11.0))
                .color(theme.fg_faint),
        )
        .on_hover_text(cwd);
        // Session cost (`TokenHud`, per terminal).
        if let Some(stats) = app.transcripts.stats(&pane.active_tab) {
            if stats.total_tokens > 0 {
                let text = match stats.cost_usd {
                    Some(usd) => crate::transcripts::fmt_usd(usd),
                    None => crate::transcripts::fmt_tokens(stats.total_tokens),
                };
                ui.label(
                    RichText::new(text)
                        .font(mono(11.0))
                        .color(cost_color(&theme, stats.cost_usd)),
                )
                .on_hover_text(format!(
                    "{} · {} {}",
                    stats.model.as_deref().unwrap_or("claude"),
                    crate::transcripts::fmt_tokens(stats.total_tokens),
                    app.t("hud.tokens")
                ));
            }
        }
        if let Some(s) = status {
            let key = match s {
                Status::Working => "status.working",
                Status::Waiting => "status.waiting",
                Status::Exited => "status.offline",
            };
            ui.label(
                RichText::new(app.t(key))
                    .font(mono(11.0))
                    .color(status_color(&theme, Some(s))),
            );
        }
        let _ = &agent;
    });

    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
}

fn exited_overlay(app: &AletheApp, ui: &mut egui::Ui, body: Rect, tab_id: &str, code: Option<i64>) {
    let theme = &app.theme;
    let bar = Rect::from_min_size(
        Pos2::new(body.left() + 12.0, body.bottom() - 46.0),
        Vec2::new(body.width() - 24.0, 36.0),
    );
    ui.painter().rect(
        bar,
        CornerRadius::same(6),
        theme.bg_elevated,
        Stroke::new(1.0, theme.border_strong),
        StrokeKind::Inside,
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar.shrink2(Vec2::new(12.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    let text = match code {
        Some(code) => format!("{} ({code})", app.t("pane.exited")),
        None => app.t("pane.exited").to_string(),
    };
    child.label(RichText::new(text).font(mono(12.0)).color(theme.fg_muted));
    child.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if ui
            .add(egui::Button::new(
                RichText::new(format!(
                    "{} {}",
                    icons::DEBUG_RESTART,
                    app.t("pane.restart")
                ))
                .font(mono(12.0)),
            ))
            .clicked()
        {
            app.act(Action::RestartTab(tab_id.to_string()));
        }
    });
}

fn empty_state(app: &mut AletheApp, ui: &mut egui::Ui, project_id: &str, area: Rect) {
    let theme = app.theme.clone();
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(area)
            .layout(Layout::top_down(Align::Center)),
    );
    child.add_space(area.height() * 0.22);
    child.label(
        RichText::new(icons::TERMINAL)
            .font(mono(36.0))
            .color(theme.fg_faint),
    );
    child.add_space(8.0);
    child.label(
        RichText::new(app.t("workspace.empty"))
            .font(widgets::bold(18.0))
            .color(theme.fg),
    );
    child.label(
        RichText::new(app.t("workspace.emptyDesc"))
            .font(sans(13.0))
            .color(theme.fg_muted),
    );
    child.add_space(18.0);
    let agents: Vec<&crate::agents::AgentInfo> = crate::agents::available()
        .filter(|a| app.data.preferences.agent_enabled(a.id))
        .collect();
    for row in agents.chunks(4) {
        let total = row.len() as f32 * 170.0 + (row.len() as f32 - 1.0) * 8.0;
        child.horizontal(|ui| {
            ui.add_space((area.width() - total) / 2.0);
            for info in row {
                let (r, response) = ui.allocate_exact_size(Vec2::new(170.0, 40.0), Sense::click());
                let hovered = response.hovered();
                ui.painter().rect(
                    r,
                    CornerRadius::same(8),
                    if hovered {
                        theme.panel_hover
                    } else {
                        theme.bg_elevated
                    },
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
                let icon_rect = Rect::from_center_size(
                    Pos2::new(r.left() + 20.0, r.center().y),
                    Vec2::splat(18.0),
                );
                match icons::agent_image(info.id, 18.0) {
                    Some(image) => image.paint_at(ui, icon_rect),
                    None => {
                        ui.painter().text(
                            icon_rect.center(),
                            Align2::CENTER_CENTER,
                            icons::TERMINAL,
                            mono(15.0),
                            theme.agent_color(info.id),
                        );
                    }
                }
                ui.painter().text(
                    Pos2::new(r.left() + 38.0, r.center().y),
                    Align2::LEFT_CENTER,
                    info.label,
                    sans(13.0),
                    theme.fg,
                );
                if response.clicked() {
                    app.act(Action::NewTerminal {
                        project_id: project_id.to_string(),
                        agent: info.id.to_string(),
                        cwd: None,
                        extra_args: vec![],
                        initial_input: None,
                    });
                }
            }
        });
        child.add_space(8.0);
    }
}

/// `costLevel`: green under $1, amber under $5, red above; faint without a price.
pub fn cost_color(theme: &crate::theme::Theme, usd: Option<f64>) -> egui::Color32 {
    match usd {
        None => theme.fg_faint,
        Some(v) if v >= 5.0 => theme.offline,
        Some(v) if v >= 1.0 => theme.waiting,
        Some(_) => theme.working,
    }
}
