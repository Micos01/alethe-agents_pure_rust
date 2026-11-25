//! `TitleBar`: app menu, sidebar toggle, open-project tabs, memory pill, window controls.

use super::icons;
use super::widgets::{self, icon_button, icon_button_active, mono, sans};
use crate::app::{Action, AletheApp, Modal, View};
use egui::{
    Align, Align2, CornerRadius, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2,
    ViewportCommand,
};
use std::sync::atomic::Ordering;

pub fn show(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    let bar = ui.max_rect();
    // Drag / double-click anywhere not covered by a widget moves / maximizes the window.
    let drag = ui.interact(
        bar,
        egui::Id::unique("titlebar-drag"),
        Sense::click_and_drag(),
    );
    if drag.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
    ui.painter().line_segment(
        [bar.left_bottom(), bar.right_bottom()],
        Stroke::new(1.0, theme.border),
    );

    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let menu = icon_button(ui, &theme, icons::MENU, "", 28.0);
        egui::Popup::menu(&menu).show(|ui| {
            ui.set_min_width(220.0);
            if ui
                .button(format!("{}   Ctrl+,", app.t("menu.preferences")))
                .clicked()
            {
                app.act(Action::ShowModal(Modal::Settings));
            }
            if ui
                .button(format!("{}   Ctrl+B", app.t("menu.toggleSidebar")))
                .clicked()
            {
                app.show_sidebar = !app.show_sidebar;
            }
            if ui.button(app.t("menu.toggleRight")).clicked() {
                app.show_right = !app.show_right;
            }
            ui.separator();
            if ui.button(app.t("menu.about")).clicked() {
                app.act(Action::ShowModal(Modal::About));
            }
            if ui.button(app.t("menu.quit")).clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            }
        });
        if icon_button_active(
            ui,
            &theme,
            icons::LAYOUT_SIDEBAR_LEFT,
            app.t("menu.toggleSidebar"),
            28.0,
            !app.show_sidebar,
        )
        .clicked()
        {
            app.show_sidebar = !app.show_sidebar;
        }
        ui.add(
            egui::Image::from_bytes("bytes://alethe-logo", icons::APP_LOGO)
                .fit_to_exact_size(Vec2::splat(18.0)),
        );
        let title = ui.add(
            egui::Label::new(
                egui::RichText::new("Alethe")
                    .font(widgets::bold(13.0))
                    .color(theme.fg),
            )
            .sense(Sense::click()),
        );
        if title.clicked() {
            app.act(Action::OpenHome);
        }
        ui.add_space(8.0);

        // Right cluster width is fixed; tabs take what is left.
        let right_width = 330.0;
        let tabs_width = (ui.available_width() - right_width).max(60.0);
        let (tabs_rect, _) = ui.allocate_exact_size(Vec2::new(tabs_width, 30.0), Sense::hover());
        let mut tabs_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(tabs_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        tabs_ui.set_clip_rect(tabs_rect);
        project_tabs(app, &mut tabs_ui);

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if window_button(ui, &theme, icons::CHROME_CLOSE, true).clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            }
            let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            if window_button(
                ui,
                &theme,
                if maximized {
                    icons::CHROME_RESTORE
                } else {
                    icons::CHROME_MAXIMIZE
                },
                false,
            )
            .clicked()
            {
                ui.ctx()
                    .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
            }
            if window_button(ui, &theme, icons::CHROME_MINIMIZE, false).clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
            }
            ui.add_space(6.0);
            if icon_button_active(
                ui,
                &theme,
                icons::LAYOUT_SIDEBAR_RIGHT,
                app.t("menu.toggleRight"),
                28.0,
                app.show_right,
            )
            .clicked()
            {
                app.show_right = !app.show_right;
            }
            pomodoro_pill(app, ui);
            usage_pills(app, ui);
            cost_hud(app, ui);
            let mb = app.memory_mb.load(Ordering::Relaxed);
            pill(ui, &theme, &format!("{mb} MB"), None);
            let profile = app.storage.index.active_name();
            pill(ui, &theme, &profile, Some(icons::ACCOUNT));
        });
    });
}

fn pill(ui: &mut egui::Ui, theme: &crate::theme::Theme, text: &str, glyph: Option<&str>) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), mono(11.5), theme.fg_muted);
    let extra = if glyph.is_some() { 18.0 } else { 0.0 };
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(galley.size().x + 16.0 + extra, 24.0),
        Sense::hover(),
    );
    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        theme.bg,
        Stroke::new(1.0, theme.border),
        StrokeKind::Inside,
    );
    let mut x = rect.left() + 8.0;
    if let Some(g) = glyph {
        ui.painter().text(
            Pos2::new(x + 6.0, rect.center().y),
            Align2::CENTER_CENTER,
            g,
            mono(12.0),
            theme.fg_muted,
        );
        x += extra;
    }
    ui.painter().galley(
        Pos2::new(x, rect.center().y - galley.size().y / 2.0),
        galley,
        theme.fg_muted,
    );
}

fn window_button(
    ui: &mut egui::Ui,
    theme: &crate::theme::Theme,
    glyph: &str,
    danger: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(40.0, 30.0), Sense::click());
    if response.hovered() {
        let fill = if danger {
            egui::Color32::from_rgb(232, 17, 35)
        } else {
            theme.panel_hover
        };
        ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
    }
    let color = if response.hovered() && danger {
        egui::Color32::WHITE
    } else {
        theme.fg_muted
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        mono(13.0),
        color,
    );
    response
}

fn project_tabs(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    ui.spacing_mut().item_spacing.x = 2.0;
    let tabs = app.open_tabs.clone();
    for id in tabs {
        let Some(project) = app.project(&id) else {
            continue;
        };
        let active = app.view == View::Project(id.clone());
        let name = project.name.clone();
        let count = project.terminals.len();
        let color = project
            .color
            .as_deref()
            .and_then(crate::model::parse_hex_color)
            .unwrap_or(theme.accent);
        let working = project
            .terminals
            .iter()
            .any(|t| app.terminal_status(t) == Some(crate::terminal::Status::Working));

        let name_galley = ui.painter().layout_no_wrap(
            name.clone(),
            sans(12.5),
            if active { theme.fg } else { theme.fg_muted },
        );
        let name_w = name_galley.size().x.min(150.0);
        let width = 10.0 + 14.0 + 6.0 + name_w + 8.0 + 16.0 + if active { 20.0 } else { 6.0 };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
        if active || response.hovered() {
            ui.painter().rect(
                rect,
                CornerRadius::same(6),
                if active {
                    theme.panel
                } else {
                    theme.panel_hover
                },
                Stroke::new(
                    1.0,
                    if active {
                        theme.border_strong
                    } else {
                        egui::Color32::TRANSPARENT
                    },
                ),
                StrokeKind::Inside,
            );
        }
        let mut x = rect.left() + 10.0;
        let avatar = Rect::from_center_size(Pos2::new(x + 7.0, rect.center().y), Vec2::splat(14.0));
        widgets::paint_avatar(ui, avatar, color, &name, 1);
        x += 20.0;
        let clip = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(name_w, rect.height()));
        ui.painter().with_clip_rect(clip).galley(
            Pos2::new(x, rect.center().y - name_galley.size().y / 2.0),
            name_galley,
            theme.fg,
        );
        x += name_w + 8.0;
        let badge_color = if working {
            theme.working
        } else {
            theme.fg_faint
        };
        ui.painter().text(
            Pos2::new(x + 6.0, rect.center().y),
            Align2::CENTER_CENTER,
            count.to_string(),
            mono(10.5),
            badge_color,
        );
        if active {
            let close_rect = Rect::from_center_size(
                Pos2::new(rect.right() - 12.0, rect.center().y),
                Vec2::splat(16.0),
            );
            let close = ui.interact(
                close_rect,
                egui::Id::unique(("tab-close", &id)),
                Sense::click(),
            );
            if close.hovered() {
                ui.painter()
                    .rect_filled(close_rect, CornerRadius::same(4), theme.panel_hover);
            }
            ui.painter().text(
                close_rect.center(),
                Align2::CENTER_CENTER,
                icons::CLOSE,
                mono(11.0),
                theme.fg_faint,
            );
            if close.clicked() {
                app.act(Action::CloseTab(id.clone()));
                continue;
            }
        }
        if response.clicked() {
            app.act(Action::OpenProject(id.clone()));
        }
        if response.middle_clicked() {
            app.act(Action::CloseTab(id.clone()));
        }
    }
}

/// `PomodoroTitleBarPill`: ring + countdown (opens the Todo panel) and a pause/resume toggle.
fn pomodoro_pill(app: &mut AletheApp, ui: &mut egui::Ui) {
    use crate::pomodoro::Status;
    let p = app.pomodoro.clone();
    if p.status == Status::Idle {
        return;
    }
    let theme = app.theme.clone();
    let lang = app.lang();
    let now = crate::model::now_ms();
    let durations = crate::pomodoro::Durations::from_prefs(&app.data.preferences.extra);
    let color = if p.is_break() {
        theme.waiting
    } else {
        theme.working
    };
    let label = crate::pomodoro::countdown(p.remaining_ms(now));
    let galley = ui.painter().layout_no_wrap(label, mono(11.5), theme.fg);
    let main_w = 8.0 + 18.0 + 6.0 + galley.size().x + 8.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(main_w + 26.0, 24.0), Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        theme.bg,
        Stroke::new(1.0, theme.border),
        StrokeKind::Inside,
    );
    // The toggle sits on the right of the pill.
    let toggle = Rect::from_min_max(Pos2::new(rect.right() - 26.0, rect.top()), rect.max);
    let main = Rect::from_min_max(rect.min, Pos2::new(toggle.left(), rect.bottom()));
    let main_r = ui.interact(main, ui.scope_id().with("pomodoro-pill"), Sense::click());
    let toggle_r = ui.interact(
        toggle,
        ui.scope_id().with("pomodoro-toggle"),
        Sense::click(),
    );
    if main_r.hovered() {
        ui.painter()
            .rect_filled(main.shrink(1.0), CornerRadius::same(5), theme.panel_hover);
    }
    if toggle_r.hovered() {
        ui.painter()
            .rect_filled(toggle.shrink(1.0), CornerRadius::same(5), theme.panel_hover);
    }
    let ring_center = Pos2::new(main.left() + 8.0 + 9.0, rect.center().y);
    super::todos::ring(
        ui.painter(),
        ring_center,
        8.0,
        p.progress(durations, now),
        color,
        theme.border_strong,
        2.0,
    );
    ui.painter().text(
        ring_center,
        Align2::CENTER_CENTER,
        if p.is_break() {
            icons::COFFEE
        } else {
            icons::TIMER
        },
        mono(8.0),
        color,
    );
    ui.painter().galley(
        Pos2::new(
            ring_center.x + 15.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        theme.fg,
    );
    ui.painter().vline(
        toggle.left(),
        rect.top() + 5.0..=rect.bottom() - 5.0,
        Stroke::new(1.0, theme.border),
    );
    let running = p.status == Status::Running;
    ui.painter().text(
        toggle.center(),
        Align2::CENTER_CENTER,
        if running { icons::PAUSE } else { icons::PLAY },
        mono(10.0),
        theme.fg_muted,
    );
    let phase = super::todos::phase_label(lang, p.phase);
    if main_r
        .on_hover_text(format!(
            "{phase} · {}",
            crate::i18n::tr(lang, "pomodoro.titlebarOpen")
        ))
        .clicked()
    {
        app.show_right = true;
        app.right_tab = crate::app::RightTab::Todos;
        app.agents_panel.forget_auto_open();
    }
    let tip = crate::i18n::tr(
        lang,
        if running {
            "pomodoro.pause"
        } else {
            "pomodoro.resume"
        },
    );
    if toggle_r.on_hover_text(tip).clicked() {
        app.pomodoro_update(|p, d, now| match p.status {
            Status::Running => p.pause(now),
            Status::Paused => p.resume(now),
            _ => p.start(None, d, now),
        });
    }
}

/// Claude and Codex usage pills (5 h window), with the other windows on hover.
fn usage_pills(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let t = |key: &str| crate::i18n::tr(lang, key);
    let now = crate::model::now_ms();
    let shown = |key: &str| {
        app.data
            .preferences
            .extra
            .get(key)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true)
    };
    type Row = (
        &'static str,
        f64,
        bool,
        Vec<(String, String)>,
        Option<String>,
    );
    let mut rows: Vec<Row> = Vec::new();
    let fmt = |p: f64| format!("{p:.0}%");
    // Right to left: Codex first so Claude ends up leftmost, as in the web build.
    if shown("topbarShowCodexUsage") {
        let reading = app.usage.codex();
        if let Some(u) = reading.value {
            let mut lines = Vec::new();
            let headline = if u.primary_minutes > 0 {
                lines.push((t("usage.window5h").to_string(), fmt(u.primary.percent)));
                lines.push((t("usage.week").to_string(), fmt(u.secondary.percent)));
                u.primary.percent
            } else {
                lines.push((t("usage.week").to_string(), fmt(u.secondary.percent)));
                u.secondary.percent
            };
            lines.push((
                t("usage.status").to_string(),
                t(if u.rate_limited {
                    "usage.limited"
                } else {
                    "usage.ok"
                })
                .to_string(),
            ));
            if !u.plan.is_empty() {
                lines.push((t("usage.plan").to_string(), u.plan.clone()));
            }
            if u.reset_credits > 0 {
                lines.push((
                    t("usage.resetCredits").to_string(),
                    u.reset_credits.to_string(),
                ));
            }
            let reset = crate::usage::reset_in(
                if u.primary_minutes > 0 {
                    u.primary.resets_at_ms
                } else {
                    u.secondary.resets_at_ms
                },
                now,
            );
            rows.push(("codex", headline, reading.stale, lines, reset));
        }
    }
    if shown("topbarShowClaudeUsage") {
        let reading = app.usage.claude();
        if let Some(u) = reading.value {
            let mut lines = vec![
                (t("usage.window5h").to_string(), fmt(u.five_hour.percent)),
                (t("usage.week").to_string(), fmt(u.seven_day.percent)),
                (t("usage.opus").to_string(), fmt(u.seven_day_opus.percent)),
            ];
            for (model, w) in &u.model_limits {
                lines.push((
                    format!("{} ({})", t("usage.week"), model.to_lowercase()),
                    fmt(w.percent),
                ));
            }
            let reset = crate::usage::reset_in(u.five_hour.resets_at_ms, now);
            rows.push(("claude", u.five_hour.percent, reading.stale, lines, reset));
        }
    }
    for (agent, percent, stale, lines, reset) in rows {
        let color = match crate::usage::level(percent) {
            2 => theme.offline,
            1 => theme.waiting,
            _ => theme.working,
        };
        let label = format!("{percent:.0}%");
        let galley = ui.painter().layout_no_wrap(label, mono(11.5), color);
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(8.0 + 13.0 + 5.0 + galley.size().x + 8.0, 24.0),
            Sense::click(),
        );
        ui.painter().rect(
            rect,
            CornerRadius::same(6),
            if response.hovered() {
                theme.panel_hover
            } else {
                theme.bg
            },
            Stroke::new(
                1.0,
                if stale {
                    theme.border
                } else {
                    color.gamma_multiply(0.45)
                },
            ),
            StrokeKind::Inside,
        );
        let icon = Rect::from_center_size(
            Pos2::new(rect.left() + 8.0 + 6.5, rect.center().y),
            Vec2::splat(13.0),
        );
        if let Some(image) = icons::agent_image(agent, 13.0) {
            image.paint_at(ui, icon);
        }
        ui.painter().galley(
            Pos2::new(icon.right() + 5.0, rect.center().y - galley.size().y / 2.0),
            galley,
            if stale { theme.fg_faint } else { color },
        );
        let title = crate::agents::info(agent).label;
        let response = response.on_hover_ui(|ui| {
            ui.set_min_width(170.0);
            ui.label(
                RichText::new(title)
                    .font(super::widgets::bold(12.5))
                    .color(theme.fg),
            );
            for (name, value) in &lines {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).font(sans(12.0)).color(theme.fg_muted));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(value).font(mono(12.0)).color(theme.fg));
                    });
                });
            }
            if let Some(reset) = &reset {
                ui.label(
                    RichText::new(format!("{} · {reset}", t("usage.resetsIn")))
                        .font(sans(11.0))
                        .color(theme.fg_faint),
                );
            }
            if stale {
                ui.label(
                    RichText::new(t("usage.stale"))
                        .font(sans(11.0))
                        .color(theme.waiting),
                );
            }
        });
        if response.clicked() {
            app.usage.refresh();
        }
    }
}

/// `TokenHud`: the running Claude sessions' total cost, one row per terminal on hover.
fn cost_hud(app: &mut AletheApp, ui: &mut egui::Ui) {
    let rows = app.transcripts.all();
    if rows.is_empty() {
        return;
    }
    let theme = app.theme.clone();
    let total: f64 = rows.iter().filter_map(|(_, s)| s.cost_usd).sum();
    let priced = rows.iter().any(|(_, s)| s.cost_usd.is_some());
    let color = super::workspace::cost_color(&theme, priced.then_some(total));
    let label = if priced {
        crate::transcripts::fmt_usd(total)
    } else {
        crate::transcripts::fmt_tokens(rows.iter().map(|(_, s)| s.total_tokens).sum())
    };
    let galley = ui.painter().layout_no_wrap(label, mono(11.5), color);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(galley.size().x + 16.0, 24.0), Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        if response.hovered() {
            theme.panel_hover
        } else {
            theme.bg
        },
        Stroke::new(1.0, theme.border),
        StrokeKind::Inside,
    );
    ui.painter().galley(
        Pos2::new(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    response.on_hover_ui(|ui| {
        ui.set_min_width(240.0);
        ui.label(
            RichText::new(app.t("hud.title"))
                .font(widgets::bold(12.5))
                .color(theme.fg),
        );
        for (tab_id, stats) in &rows {
            let place = std::path::Path::new(&stats.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| stats.cwd.clone());
            let title = app
                .find_tab(tab_id)
                .map(|tab| app.tab_title(tab))
                .unwrap_or_default();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(&title).font(sans(12.0)).color(theme.fg));
                    ui.label(
                        RichText::new(format!(
                            "{place} · {} · {} {}",
                            stats.model.as_deref().unwrap_or("claude"),
                            crate::transcripts::fmt_tokens(stats.total_tokens),
                            app.t("hud.tokens")
                        ))
                        .font(mono(10.5))
                        .color(theme.fg_faint),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let text = stats
                        .cost_usd
                        .map(crate::transcripts::fmt_usd)
                        .unwrap_or_else(|| app.t("hud.noCost").to_string());
                    ui.label(
                        RichText::new(text)
                            .font(mono(12.0))
                            .color(super::workspace::cost_color(&theme, stats.cost_usd)),
                    );
                });
            });
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(app.t("hud.total"))
                    .font(sans(12.0))
                    .color(theme.fg_muted),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(crate::transcripts::fmt_usd(total))
                        .font(mono(12.0))
                        .color(color),
                );
            });
        });
    });
}
