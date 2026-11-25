//! `HomeView`: greeting, the quick-start composer, recent projects and "start something".

use super::icons;
use super::widgets::{self, mono, sans, section_label};
use crate::app::{Action, AletheApp, Modal};
use chrono::{Datelike, Local, Timelike};
use egui::{
    Align, Align2, Color32, CornerRadius, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, Vec2,
};

const MAX_WIDTH: f32 = 1100.0;

fn greeting_key(hour: u32) -> &'static str {
    match hour {
        5..=11 => "greeting.morning",
        12..=17 => "greeting.afternoon",
        _ => "greeting.evening",
    }
}

/// `formatHomeDate`: "dom, 9 ago · 19:56".
fn home_date(lang: crate::i18n::Lang) -> String {
    let now = Local::now();
    let (weekdays, months): (&[&str; 7], &[&str; 12]) = match lang {
        crate::i18n::Lang::PtBr => (
            &["seg", "ter", "qua", "qui", "sex", "sáb", "dom"],
            &[
                "jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez",
            ],
        ),
        crate::i18n::Lang::En => (
            &["mon", "tue", "wed", "thu", "fri", "sat", "sun"],
            &[
                "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
            ],
        ),
    };
    format!(
        "{}, {} {} · {:02}:{:02}",
        weekdays[now.weekday().num_days_from_monday() as usize],
        now.day(),
        months[now.month0() as usize],
        now.hour(),
        now.minute()
    )
}

pub fn show(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs(30));
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let avail = ui.available_width();
            let width = avail.min(MAX_WIDTH) - 48.0;
            let side = ((avail - width) / 2.0).max(24.0);
            ui.horizontal_top(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.add_space(56.0);
                    hero(app, ui, width);
                    ui.add_space(28.0);
                    composer(app, ui, width);
                    ui.add_space(18.0);
                    ui.horizontal(|ui| {
                        let total = 2.0 * 150.0 + 8.0;
                        ui.add_space((width - total) / 2.0);
                        if widgets::outline_button(
                            ui,
                            &theme,
                            icons::TERMINAL,
                            app.t("home.newTerminal"),
                            None,
                            Some(150.0),
                        )
                        .clicked()
                        {
                            app.act(Action::QuickStart {
                                agent: "shell".into(),
                                folder: app.home.folder.clone(),
                                prompt: String::new(),
                                yolo: false,
                            });
                        }
                        if widgets::outline_button(
                            ui,
                            &theme,
                            icons::NEW_FOLDER,
                            app.t("home.newProject"),
                            None,
                            Some(150.0),
                        )
                        .clicked()
                        {
                            app.act(Action::ShowModal(app.new_project_modal(None)));
                        }
                    });
                    ui.add_space(48.0);
                    recent_projects(app, ui, width);
                    ui.add_space(32.0);
                    start_something(app, ui, width);
                    ui.add_space(48.0);
                });
            });
        });
}

fn hero(app: &AletheApp, ui: &mut egui::Ui, width: f32) {
    let theme = &app.theme;
    let lang = app.lang();
    let name = if app.data.preferences.display_name.trim().is_empty() {
        app.t("greeting.fallbackName").to_string()
    } else {
        app.data.preferences.display_name.trim().to_string()
    };
    ui.vertical_centered(|ui| {
        ui.set_width(width);
        // Avatar trio from the web build (status / profile / agent count).
        let working = app
            .sessions
            .values()
            .filter(|s| s.status() == crate::terminal::Status::Working)
            .count();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(140.0, 44.0), Sense::hover());
        let c = rect.center();
        ui.painter().circle(
            Pos2::new(c.x - 46.0, c.y),
            18.0,
            theme.bg_elevated,
            Stroke::new(1.5, theme.waiting),
        );
        ui.painter().text(
            Pos2::new(c.x - 46.0, c.y),
            Align2::CENTER_CENTER,
            working.to_string(),
            widgets::bold(13.0),
            theme.fg,
        );
        ui.painter()
            .circle(c, 22.0, theme.panel, Stroke::new(1.5, theme.border_strong));
        ui.painter().text(
            c,
            Align2::CENTER_CENTER,
            widgets::initials(&name, 1),
            widgets::bold(18.0),
            theme.fg,
        );
        ui.painter().circle(
            Pos2::new(c.x + 46.0, c.y),
            18.0,
            theme.bg_elevated,
            Stroke::new(1.5, theme.working),
        );
        ui.painter().text(
            Pos2::new(c.x + 46.0, c.y),
            Align2::CENTER_CENTER,
            app.sessions.len().to_string(),
            widgets::bold(13.0),
            theme.fg,
        );

        ui.add_space(14.0);
        let greeting = crate::i18n::tr(lang, greeting_key(Local::now().hour()));
        ui.label(
            RichText::new(format!("{greeting}, {name}."))
                .font(widgets::bold(40.0))
                .color(theme.fg),
        );
        ui.add_space(2.0);
        ui.label(
            RichText::new(home_date(lang))
                .font(widgets::bold(15.0))
                .color(theme.fg_muted),
        );
    });
}

fn composer(app: &mut AletheApp, ui: &mut egui::Ui, width: f32) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let card_w = width.min(780.0);
    ui.horizontal(|ui| {
        ui.add_space((width - card_w) / 2.0);
        egui::Frame::new()
            .fill(theme.bg_elevated)
            .stroke(Stroke::new(1.0, theme.border_strong))
            .corner_radius(CornerRadius::same(10))
            .show(ui, |ui| {
                ui.set_width(card_w);
                ui.vertical(|ui| {
                    // Window chrome.
                    egui::Frame::new()
                        .inner_margin(Margin::symmetric(14, 9))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for color in [
                                    Color32::from_rgb(255, 95, 86),
                                    Color32::from_rgb(255, 189, 46),
                                    Color32::from_rgb(39, 201, 63),
                                ] {
                                    let (r, _) =
                                        ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                                    ui.painter().circle_filled(r.center(), 5.0, color);
                                }
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(app.t("home.quickTerminalTitle"))
                                        .font(widgets::bold(12.0))
                                        .color(theme.fg_muted),
                                );
                            });
                        });
                    let line = ui.available_rect_before_wrap();
                    ui.painter().line_segment(
                        [line.left_top(), Pos2::new(line.right(), line.top())],
                        Stroke::new(1.0, theme.border),
                    );

                    // Prompt.
                    let mut send = false;
                    egui::Frame::new()
                        .inner_margin(Margin {
                            left: 18,
                            right: 18,
                            top: 16,
                            bottom: 14,
                        })
                        .show(ui, |ui| {
                            ui.horizontal_top(|ui| {
                                ui.label(
                                    RichText::new(">").font(widgets::bold(15.0)).color(theme.fg),
                                );
                                let edit = egui::TextEdit::multiline(&mut app.home.prompt)
                                    .hint_text(
                                        RichText::new(crate::i18n::tr(
                                            lang,
                                            "home.quickPromptPlaceholder",
                                        ))
                                        .font(mono(14.0))
                                        .color(theme.fg_muted),
                                    )
                                    .frame(egui::Frame::NONE)
                                    .font(mono(14.0))
                                    .desired_rows(1)
                                    .desired_width(f32::INFINITY);
                                let response = ui.add(edit);
                                if response.has_focus()
                                    && ui.input(|i| {
                                        i.key_pressed(egui::Key::Enter) && !i.modifiers.shift
                                    })
                                {
                                    send = true;
                                }
                            });
                        });
                    let line = ui.available_rect_before_wrap();
                    ui.painter().line_segment(
                        [line.left_top(), Pos2::new(line.right(), line.top())],
                        Stroke::new(1.0, theme.border),
                    );

                    // Agent / folder / mode / send.
                    egui::Frame::new()
                        .inner_margin(Margin::symmetric(10, 10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let agent_label = crate::agents::info(&app.home.agent).label;
                                let agent_btn = chip(
                                    ui,
                                    &theme,
                                    Some(&app.home.agent.clone()),
                                    &format!("{}: {}", app.t("home.quickAgentShort"), agent_label),
                                    230.0,
                                );
                                egui::Popup::menu(&agent_btn).show(|ui| {
                                    ui.set_min_width(200.0);
                                    for agent in crate::agents::available() {
                                        if !app.data.preferences.agent_enabled(agent.id) {
                                            continue;
                                        }
                                        if ui
                                            .selectable_label(
                                                app.home.agent == agent.id,
                                                agent.label,
                                            )
                                            .clicked()
                                        {
                                            app.home.agent = agent.id.to_string();
                                            ui.close();
                                        }
                                    }
                                });

                                let folder_label = format!(
                                    "{}: {}",
                                    app.t("home.quickPath"),
                                    short_path(&app.home.folder, 34)
                                );
                                let folder_btn = chip(ui, &theme, None, &folder_label, 300.0);
                                egui::Popup::menu(&folder_btn).show(|ui| {
                                    ui.set_min_width(300.0);
                                    if ui
                                        .button(format!(
                                            "{}  {}",
                                            icons::FOLDER_OPENED,
                                            app.t("dialog.browse")
                                        ))
                                        .clicked()
                                    {
                                        if let Some(dir) = rfd::FileDialog::new()
                                            .set_directory(&app.home.folder)
                                            .pick_folder()
                                        {
                                            app.home.folder = dir.to_string_lossy().to_string();
                                        }
                                        ui.close();
                                    }
                                    ui.separator();
                                    let mut seen: Vec<String> = Vec::new();
                                    let mut projects: Vec<(u64, String)> = app
                                        .data
                                        .projects
                                        .iter()
                                        .map(|p| (p.last_used(), p.cwd()))
                                        .collect();
                                    projects.sort_by(|a, b| b.0.cmp(&a.0));
                                    for (_, cwd) in projects {
                                        if seen.contains(&cwd) || seen.len() >= 8 {
                                            continue;
                                        }
                                        seen.push(cwd.clone());
                                        if ui
                                            .selectable_label(
                                                app.home.folder == cwd,
                                                short_path(&cwd, 48),
                                            )
                                            .clicked()
                                        {
                                            app.home.folder = cwd;
                                            ui.close();
                                        }
                                    }
                                });

                                let mode = if app.home.yolo {
                                    app.t("home.quickUnrestricted")
                                } else {
                                    app.t("home.quickRestricted")
                                };
                                let mode_btn = chip(
                                    ui,
                                    &theme,
                                    None,
                                    &format!("● {}: {}", app.t("home.quickMode"), mode),
                                    150.0,
                                );
                                egui::Popup::menu(&mode_btn).show(|ui| {
                                    if ui
                                        .selectable_label(
                                            !app.home.yolo,
                                            app.t("home.quickRestricted"),
                                        )
                                        .clicked()
                                    {
                                        app.home.yolo = false;
                                        ui.close();
                                    }
                                    if ui
                                        .selectable_label(
                                            app.home.yolo,
                                            app.t("home.quickUnrestricted"),
                                        )
                                        .clicked()
                                    {
                                        app.home.yolo = true;
                                        ui.close();
                                    }
                                });

                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let (rect, response) = ui
                                        .allocate_exact_size(Vec2::new(46.0, 34.0), Sense::click());
                                    let fill = if response.hovered() {
                                        theme.accent_strong
                                    } else {
                                        theme.accent
                                    };
                                    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
                                    ui.painter().text(
                                        rect.center(),
                                        Align2::CENTER_CENTER,
                                        icons::SEND,
                                        mono(15.0),
                                        theme.accent_on,
                                    );
                                    if response.on_hover_text(app.t("home.quickSend")).clicked() {
                                        send = true;
                                    }
                                });
                            });
                        });

                    if send {
                        let prompt = app.home.prompt.trim().to_string();
                        app.act(Action::QuickStart {
                            agent: app.home.agent.clone(),
                            folder: app.home.folder.clone(),
                            prompt,
                            yolo: app.home.yolo,
                        });
                    }
                });
            });
    });
}

fn short_path(path: &str, max: usize) -> String {
    let count = path.chars().count();
    if count <= max {
        return path.to_string();
    }
    let tail: String = path.chars().skip(count - (max - 1)).collect();
    format!("…{tail}")
}

/// Dropdown-looking chip (`.quickChip`).
fn chip(
    ui: &mut egui::Ui,
    theme: &crate::theme::Theme,
    agent: Option<&str>,
    label: &str,
    max_width: f32,
) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), mono(12.5), theme.fg);
    let lead = if agent.is_some() { 24.0 } else { 0.0 };
    let width = (galley.size().x + lead + 44.0).min(max_width);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::click());
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
    let mut x = rect.left() + 10.0;
    if let Some(agent) = agent {
        if let Some(image) = icons::agent_image(agent, 16.0) {
            image.paint_at(
                ui,
                Rect::from_center_size(Pos2::new(x + 8.0, rect.center().y), Vec2::splat(16.0)),
            );
        }
        x += lead;
    }
    let text_rect = Rect::from_min_max(
        Pos2::new(x, rect.top()),
        Pos2::new(rect.right() - 24.0, rect.bottom()),
    );
    ui.painter().with_clip_rect(text_rect).galley(
        Pos2::new(x, rect.center().y - galley.size().y / 2.0),
        galley,
        theme.fg,
    );
    ui.painter().text(
        Pos2::new(rect.right() - 14.0, rect.center().y),
        Align2::CENTER_CENTER,
        icons::CHEVRON_DOWN,
        mono(11.0),
        theme.fg_faint,
    );
    response
}

fn recent_projects(app: &mut AletheApp, ui: &mut egui::Ui, width: f32) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let mut projects: Vec<(String, String, Option<String>, usize, u64)> = app
        .data
        .projects
        .iter()
        .filter(|p| p.is_visible())
        .map(|p| {
            (
                p.id.clone(),
                p.name.clone(),
                p.color.clone(),
                p.terminals.len(),
                p.last_used(),
            )
        })
        .collect();
    projects.sort_by(|a, b| b.4.cmp(&a.4));

    ui.horizontal(|ui| {
        section_label(ui, &theme, app.t("home.recentProjects"));
        widgets::count_badge(ui, &theme, projects.len());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if !projects.is_empty() {
                let r = ui.add(
                    egui::Label::new(
                        RichText::new(app.t("home.viewAll").to_uppercase())
                            .font(widgets::bold(11.0))
                            .color(theme.fg_faint),
                    )
                    .sense(Sense::click()),
                );
                if r.clicked() {
                    app.show_sidebar = true;
                }
            }
        });
    });
    ui.add_space(10.0);

    if projects.is_empty() {
        egui::Frame::new()
            .stroke(Stroke::new(1.0, theme.border))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(24))
            .show(ui, |ui| {
                ui.set_width(width - 50.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(app.t("home.projectsEmptyTitle"))
                            .font(widgets::bold(15.0))
                            .color(theme.fg),
                    );
                    ui.label(
                        RichText::new(app.t("home.projectsEmptyDesc"))
                            .font(sans(13.0))
                            .color(theme.fg_muted),
                    );
                    ui.add_space(10.0);
                    if widgets::primary_button(ui, &theme, app.t("home.projectsEmptyAction"))
                        .clicked()
                    {
                        app.act(Action::ShowModal(app.new_project_modal(None)));
                    }
                });
            });
        return;
    }

    let columns = if width > 900.0 { 3 } else { 2 };
    let gap = 10.0;
    let card_w = (width - gap * (columns as f32 - 1.0)) / columns as f32;
    for row in projects.iter().take(6).collect::<Vec<_>>().chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (id, name, color, count, last) in row.iter().copied() {
                let color = color
                    .as_deref()
                    .and_then(crate::model::parse_hex_color)
                    .unwrap_or(theme.accent);
                let (response, _) = widgets::card(ui, &theme, Vec2::new(card_w, 54.0), |ui| {
                    widgets::project_avatar(ui, color, name, 34.0, 1);
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(
                            RichText::new(name)
                                .font(widgets::bold(13.5))
                                .color(theme.fg),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} · {}",
                                widgets::terminal_count_label(lang, *count),
                                widgets::relative_time(lang, *last)
                            ))
                            .font(mono(11.0))
                            .color(theme.fg_faint),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(icons::ARROW_RIGHT)
                                .font(mono(14.0))
                                .color(theme.fg_faint),
                        );
                    });
                });
                if response.clicked() {
                    app.act(Action::OpenProject(id.clone()));
                }
            }
        });
        ui.add_space(gap);
    }
}

fn start_something(app: &mut AletheApp, ui: &mut egui::Ui, width: f32) {
    let theme = app.theme.clone();
    section_label(ui, &theme, app.t("home.startSomething"));
    ui.add_space(10.0);
    let gap = 12.0;
    let w = (width - gap * 2.0) / 3.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        if widgets::outline_button(
            ui,
            &theme,
            icons::TERMINAL,
            app.t("home.newTerminal"),
            Some("Ctrl+T"),
            Some(w),
        )
        .clicked()
        {
            app.act(Action::QuickStart {
                agent: "shell".into(),
                folder: app.home.folder.clone(),
                prompt: String::new(),
                yolo: false,
            });
        }
        if widgets::outline_button(
            ui,
            &theme,
            icons::NEW_FOLDER,
            app.t("home.newProject"),
            Some("Ctrl+Shift+P"),
            Some(w),
        )
        .clicked()
        {
            app.act(Action::ShowModal(app.new_project_modal(None)));
        }
        if widgets::outline_button(
            ui,
            &theme,
            icons::LAYERS,
            app.t("home.newGroup"),
            Some("Ctrl+Shift+G"),
            Some(w),
        )
        .clicked()
        {
            app.act(Action::ShowModal(Modal::NewGroup {
                name: String::new(),
                color: 1,
            }));
        }
    });
}
