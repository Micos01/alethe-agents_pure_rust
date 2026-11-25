//! Right sidebar: the Todo List panel (`plugins/todos`) with the Pomodoro timer on top.

use super::icons;
use super::widgets::{self, mono, sans};
use crate::app::{Action, AletheApp, RightTab};
use crate::model::TodoItem;
use crate::pomodoro::{countdown, Durations, Phase, Status};
use egui::{
    Align, Align2, CornerRadius, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind,
    Vec2,
};

/// Todo | Agents switcher at the top of the right sidebar.
pub fn tab_strip(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    let tabs = [
        (RightTab::Todos, icons::CHECKLIST, app.t("todo.tab")),
        (
            RightTab::Agents,
            icons::TYPE_HIERARCHY,
            app.t("agentsPanel.title"),
        ),
        (RightTab::Mcp, icons::PLUG, "MCP"),
    ];
    egui::Frame::new()
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (tab, glyph, label) in tabs {
                    let active = app.right_tab == tab;
                    let text = format!("{glyph}  {label}");
                    let width = ui.fonts_mut(|f| {
                        f.layout_no_wrap(text.clone(), widgets::bold(12.5), theme.fg)
                            .size()
                            .x
                    }) + 24.0;
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
                    if active {
                        ui.painter().rect(
                            r,
                            CornerRadius::same(6),
                            theme.panel,
                            Stroke::new(1.0, theme.border_strong),
                            StrokeKind::Inside,
                        );
                    } else if resp.hovered() {
                        ui.painter()
                            .rect_filled(r, CornerRadius::same(6), theme.panel_hover);
                    }
                    ui.painter().text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        text,
                        widgets::bold(12.5),
                        if active { theme.fg } else { theme.fg_muted },
                    );
                    if resp.clicked() {
                        app.right_tab = tab;
                        app.agents_panel.forget_auto_open();
                    }
                }
            });
        });
    rule(ui, &theme);
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum TodoFilter {
    #[default]
    All,
    Active,
    Completed,
}

/// Row being edited in place: id, title, tags.
pub type TodoEdit = (String, String, String);

pub fn show(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    let width = ui.available_width();

    tab_strip(app, ui);

    let open = app.data.todos.iter().filter(|t| !t.completed).count();
    egui::Frame::new()
        .inner_margin(Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icons::CHECKLIST)
                        .font(mono(15.0))
                        .color(theme.fg),
                );
                ui.label(
                    RichText::new(app.t("todo.title"))
                        .font(widgets::bold(14.5))
                        .color(theme.fg),
                );
                ui.label(
                    RichText::new(format!("{open} {}", app.t("todo.open")))
                        .font(sans(12.5))
                        .color(theme.fg_faint),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(ui, &theme, icons::LAYOUT_SIDEBAR_RIGHT, "", 24.0)
                        .clicked()
                    {
                        app.show_right = false;
                    }
                    if widgets::icon_button_active(
                        ui,
                        &theme,
                        icons::GEAR,
                        app.t("todo.openSettings"),
                        24.0,
                        app.todo.settings_open,
                    )
                    .clicked()
                    {
                        app.todo.settings_open = !app.todo.settings_open;
                    }
                });
            });
        });
    rule(ui, &theme);

    if app.todo.settings_open {
        settings(app, ui);
        rule(ui, &theme);
    }

    pomodoro_card(app, ui);
    rule(ui, &theme);

    composer(app, ui, width);
    rule(ui, &theme);

    // ---- filters
    let completed = app.data.todos.len() - open;
    egui::Frame::new()
        .inner_margin(Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (filter, key, count) in [
                    (TodoFilter::All, "todo.all", app.data.todos.len()),
                    (TodoFilter::Active, "todo.active", open),
                    (TodoFilter::Completed, "todo.completed", completed),
                ] {
                    let active = app.todo.filter == filter;
                    let text = RichText::new(format!("{}  {count}", app.t(key)))
                        .font(sans(12.0))
                        .color(if active { theme.fg } else { theme.fg_muted });
                    if ui.selectable_label(active, text).clicked() {
                        app.todo.filter = filter;
                    }
                }
            });
        });

    if app.data.todos.is_empty() {
        empty_state(app, ui, "todo.emptyTitle", Some("todo.emptyDescription"));
        return;
    }

    let rows: Vec<TodoItem> = app.data.todos.clone();
    let (active_rows, done_rows): (Vec<&TodoItem>, Vec<&TodoItem>) =
        rows.iter().partition(|t| !t.completed);
    let mut drop_targets: Vec<(String, Rect)> = Vec::new();
    egui::ScrollArea::vertical()
        .id_salt("todo-list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(10, 4))
                .show(ui, |ui| {
                    if app.todo.filter != TodoFilter::Completed {
                        for todo in &active_rows {
                            todo_row(app, ui, todo, &mut drop_targets);
                        }
                    }
                    match app.todo.filter {
                        TodoFilter::Active => {}
                        TodoFilter::Completed if done_rows.is_empty() => {
                            ui.add_space(20.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new(app.t("todo.emptyCompleted"))
                                        .font(sans(12.5))
                                        .color(theme.fg_faint),
                                );
                            });
                        }
                        filter => {
                            if filter == TodoFilter::All && !done_rows.is_empty() {
                                ui.add_space(8.0);
                                widgets::section_label(ui, &theme, app.t("todo.completed"));
                                ui.add_space(2.0);
                            }
                            for todo in &done_rows {
                                todo_row(app, ui, todo, &mut drop_targets);
                            }
                        }
                    }
                });
        });

    // ---- drag to reorder (`reorderTodoItems`)
    if let Some(dragged) = app.todo.dragging.clone() {
        let pointer = ui.ctx().pointer_latest_pos();
        let target = pointer.and_then(|p| {
            drop_targets
                .iter()
                .find(|(_, r)| r.contains(p))
                .map(|(id, r)| (id.clone(), *r))
        });
        if let Some((_, rect)) = &target {
            ui.painter()
                .hline(rect.x_range(), rect.top(), Stroke::new(2.0, theme.accent));
        }
        if ui.input(|i| i.pointer.any_released()) {
            if let Some((target, _)) = target.filter(|(id, _)| *id != dragged) {
                app.act(Action::ReorderTodo { dragged, target });
            }
            app.todo.dragging = None;
        }
    }
}

fn empty_state(app: &AletheApp, ui: &mut egui::Ui, title: &str, body: Option<&str>) {
    let theme = &app.theme;
    ui.add_space(60.0);
    ui.vertical_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::hover());
        ui.painter().rect_stroke(
            r,
            CornerRadius::same(10),
            Stroke::new(1.0, theme.border_strong),
            StrokeKind::Inside,
        );
        ui.painter().text(
            r.center(),
            Align2::CENTER_CENTER,
            icons::CHECKLIST,
            mono(20.0),
            theme.fg_muted,
        );
        ui.add_space(10.0);
        ui.label(
            RichText::new(app.t(title))
                .font(widgets::bold(14.0))
                .color(theme.fg),
        );
        if let Some(body) = body {
            ui.add(
                egui::Label::new(
                    RichText::new(app.t(body))
                        .font(sans(13.0))
                        .color(theme.fg_muted),
                )
                .wrap(),
            );
        }
    });
}

fn composer(app: &mut AletheApp, ui: &mut egui::Ui, width: f32) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let mut submit = false;
    egui::Frame::new()
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                let r = widgets::text_input(
                    ui,
                    &theme,
                    &mut app.todo.input,
                    crate::i18n::tr(lang, "todo.addPlaceholder"),
                    width - 72.0,
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
                let (rect, add) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::same(6),
                    if add.hovered() {
                        theme.panel_hover
                    } else {
                        theme.panel
                    },
                );
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    icons::ADD,
                    mono(15.0),
                    theme.fg,
                );
                if add
                    .on_hover_text(crate::i18n::tr(lang, "todo.add"))
                    .clicked()
                {
                    submit = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icons::TAG)
                        .font(mono(13.0))
                        .color(theme.fg_faint),
                );
                widgets::text_input(
                    ui,
                    &theme,
                    &mut app.todo.tags,
                    crate::i18n::tr(lang, "todo.tags"),
                    width - 50.0,
                );
            });
            let current = app
                .todo
                .project
                .as_deref()
                .and_then(|id| app.project(id))
                .map(|p| p.name.clone())
                .unwrap_or_else(|| app.t("todo.noProject").to_string());
            let projects: Vec<(String, String)> = app
                .data
                .projects
                .iter()
                .map(|p| (p.id.clone(), p.name.clone()))
                .collect();
            egui::ComboBox::from_id_salt("todo-project")
                .width(width - 30.0)
                .selected_text(format!("{}  {current}", icons::FOLDER))
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(app.todo.project.is_none(), app.t("todo.noProject"))
                        .clicked()
                    {
                        app.todo.project = None;
                    }
                    for (id, name) in projects {
                        if ui
                            .selectable_label(app.todo.project.as_deref() == Some(&id), name)
                            .clicked()
                        {
                            app.todo.project = Some(id);
                        }
                    }
                });
        });
    if submit && !app.todo.input.trim().is_empty() {
        app.act(Action::AddTodo {
            title: app.todo.input.clone(),
            tags: split_tags(&app.todo.tags),
            project_id: app.todo.project.clone(),
        });
        app.todo.input.clear();
    }
}

fn split_tags(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for tag in text.split(',') {
        let tag = tag.trim().trim_start_matches('#').to_lowercase();
        if !tag.is_empty() && !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

fn todo_row(
    app: &mut AletheApp,
    ui: &mut egui::Ui,
    todo: &TodoItem,
    drop_targets: &mut Vec<(String, Rect)>,
) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let t = |key: &str| crate::i18n::tr(lang, key);

    // In-place edit.
    if app.todo.editing.as_ref().is_some_and(|e| e.0 == todo.id) {
        let mut save = false;
        let mut cancel = false;
        egui::Frame::new()
            .fill(theme.panel)
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                let edit = app.todo.editing.as_mut().expect("editing");
                let title = ui.add(
                    egui::TextEdit::singleline(&mut edit.1)
                        .desired_width(f32::INFINITY)
                        .font(sans(13.0)),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut edit.2)
                        .desired_width(f32::INFINITY)
                        .hint_text(t("todo.editTags"))
                        .font(mono(11.5)),
                );
                if title.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    save = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
                ui.horizontal(|ui| {
                    if widgets::primary_button(ui, &theme, t("todo.saveEdit")).clicked() {
                        save = true;
                    }
                    if widgets::ghost_button(ui, &theme, t("common.cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if save {
            if let Some((id, title, tags)) = app.todo.editing.take() {
                app.act(Action::UpdateTodo {
                    id,
                    title,
                    tags: split_tags(&tags),
                });
            }
        } else if cancel {
            app.todo.editing = None;
        }
        ui.add_space(4.0);
        return;
    }

    let mut meta: Vec<String> = Vec::new();
    if let Some(name) = todo
        .project_id
        .as_deref()
        .and_then(|p| app.project(p))
        .map(|p| p.name.clone())
    {
        meta.push(name);
    }
    meta.extend(todo.tags.iter().map(|t| format!("#{t}")));
    let height = if meta.is_empty() { 32.0 } else { 42.0 };
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        Sense::click_and_drag(),
    );
    drop_targets.push((todo.id.clone(), rect));
    let dragging = app.todo.dragging.as_deref() == Some(todo.id.as_str());
    let focused = app.pomodoro.focus_todo_id.as_deref() == Some(todo.id.as_str());
    let hovered = response.hovered() || dragging;
    if hovered || focused {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(6),
            if dragging {
                theme.panel
            } else if focused && !hovered {
                theme.accent_soft
            } else {
                theme.panel_hover
            },
        );
    }
    if response.drag_started() {
        app.todo.dragging = Some(todo.id.clone());
    }
    if hovered {
        ui.painter().text(
            Pos2::new(rect.left() + 5.0, rect.center().y),
            Align2::CENTER_CENTER,
            icons::GRIP,
            mono(12.0),
            theme.fg_faint,
        );
    }

    // Checkbox.
    let check = Rect::from_center_size(
        Pos2::new(rect.left() + 20.0, rect.top() + 16.0),
        Vec2::splat(16.0),
    );
    let check_r = ui.interact(
        check.expand(3.0),
        ui.scope_id().with(("todo-check", &todo.id)),
        Sense::click(),
    );
    ui.painter().rect(
        check,
        CornerRadius::same(4),
        if todo.completed {
            theme.accent
        } else {
            theme.bg
        },
        Stroke::new(
            1.0,
            if check_r.hovered() {
                theme.accent
            } else {
                theme.border_strong
            },
        ),
        StrokeKind::Inside,
    );
    if todo.completed {
        ui.painter().text(
            check.center(),
            Align2::CENTER_CENTER,
            icons::CHECK,
            mono(11.0),
            theme.accent_on,
        );
    }
    if check_r.on_hover_text(t("todo.complete")).clicked() {
        app.act(Action::ToggleTodo(todo.id.clone()));
    }

    // Hover actions, right to left: delete, edit, focus.
    let mut right = rect.right() - 4.0;
    let mut action = |glyph: &str, tip: &str, salt: &str, active: bool, danger: bool| -> bool {
        let r = Rect::from_center_size(
            Pos2::new(right - 10.0, rect.top() + 16.0),
            Vec2::splat(20.0),
        );
        right -= 22.0;
        let resp = ui.interact(r, ui.scope_id().with((salt, &todo.id)), Sense::click());
        if !(hovered || active) {
            return false;
        }
        let color = if resp.hovered() && danger {
            theme.offline
        } else if resp.hovered() || active {
            theme.accent
        } else {
            theme.fg_faint
        };
        ui.painter()
            .text(r.center(), Align2::CENTER_CENTER, glyph, mono(12.0), color);
        resp.on_hover_text(tip).clicked()
    };
    if action(icons::TRASH, t("todo.delete"), "todo-del", false, true) {
        app.act(Action::DeleteTodo(todo.id.clone()));
    }
    if action(icons::EDIT, t("todo.edit"), "todo-edit", false, false) {
        app.todo.editing = Some((todo.id.clone(), todo.title.clone(), todo.tags.join(", ")));
    }
    if !todo.completed
        && action(
            icons::FOCUS,
            t("pomodoro.focusOnLabel"),
            "todo-focus",
            focused,
            false,
        )
    {
        let id = (!focused).then(|| todo.id.clone());
        app.pomodoro_update(|p, _, _| p.focus_todo_id = id);
    }

    let color = if todo.completed {
        theme.fg_faint
    } else {
        theme.fg
    };
    let text_left = rect.left() + 36.0;
    let clip = Rect::from_min_max(
        Pos2::new(text_left, rect.top()),
        Pos2::new(right, rect.bottom()),
    );
    let galley = ui
        .painter()
        .layout_no_wrap(todo.title.clone(), sans(13.0), color);
    let title_pos = Pos2::new(text_left, rect.top() + 16.0 - galley.size().y / 2.0);
    let title_width = galley.size().x;
    ui.painter()
        .with_clip_rect(clip)
        .galley(title_pos, galley, color);
    if todo.completed {
        let y = rect.top() + 16.0;
        ui.painter().with_clip_rect(clip).hline(
            text_left..=text_left + title_width,
            y,
            Stroke::new(1.0, theme.fg_faint),
        );
    }
    if !meta.is_empty() {
        ui.painter().with_clip_rect(clip).text(
            Pos2::new(text_left, rect.top() + 32.0),
            Align2::LEFT_CENTER,
            meta.join(" · "),
            mono(10.5),
            theme.fg_faint,
        );
    }
    let response = response.on_hover_text(&todo.title);
    if response.double_clicked() {
        app.todo.editing = Some((todo.id.clone(), todo.title.clone(), todo.tags.join(", ")));
    }
}

fn settings(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    egui::Frame::new()
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.label(
                RichText::new(app.t("todo.settingsTitle"))
                    .font(widgets::bold(12.5))
                    .color(theme.fg),
            );
            ui.add_space(4.0);
            for (key, label, default) in [
                ("pomodoroWorkMinutes", "pomodoro.settingsWorkMinutes", 25u64),
                (
                    "pomodoroShortBreakMinutes",
                    "pomodoro.settingsShortBreakMinutes",
                    5,
                ),
                (
                    "pomodoroLongBreakMinutes",
                    "pomodoro.settingsLongBreakMinutes",
                    15,
                ),
            ] {
                let mut minutes = app
                    .data
                    .preferences
                    .extra
                    .get(key)
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(default);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(app.t(label))
                            .font(sans(12.0))
                            .color(theme.fg_muted),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(egui::DragValue::new(&mut minutes).range(1..=180).speed(0.2))
                            .changed()
                        {
                            app.data
                                .preferences
                                .extra
                                .insert(key.into(), serde_json::json!(minutes));
                            app.mark_dirty();
                        }
                    });
                });
            }
        });
}

// ----------------------------------------------------------------- Pomodoro

/// Progress ring: a faint track with an arc from 12 o'clock, clockwise.
pub fn ring(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    progress: f32,
    color: egui::Color32,
    track: egui::Color32,
    width: f32,
) {
    painter.circle_stroke(center, radius, Stroke::new(width, track));
    let progress = progress.clamp(0.0, 1.0);
    if progress <= 0.0 {
        return;
    }
    let steps = ((64.0 * progress).ceil() as usize).max(2);
    let points: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let angle = -std::f32::consts::FRAC_PI_2
                + std::f32::consts::TAU * progress * i as f32 / steps as f32;
            center + Vec2::new(angle.cos(), angle.sin()) * radius
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(width, color)));
}

pub fn phase_label(lang: crate::i18n::Lang, phase: Phase) -> &'static str {
    crate::i18n::tr(
        lang,
        match phase {
            Phase::Idle => "pomodoro.phaseIdle",
            Phase::Work => "pomodoro.phaseWork",
            Phase::ShortBreak => "pomodoro.phaseShortBreak",
            Phase::LongBreak => "pomodoro.phaseLongBreak",
        },
    )
}

fn pomodoro_card(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let t = |key: &str| crate::i18n::tr(lang, key);
    let now = crate::model::now_ms();
    let durations = Durations::from_prefs(&app.data.preferences.extra);
    let p = app.pomodoro.clone();
    let color = if p.is_break() {
        theme.waiting
    } else {
        theme.working
    };
    egui::Frame::new()
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                let progress = if p.status == Status::Idle {
                    0.0
                } else {
                    p.progress(durations, now)
                };
                ring(
                    ui.painter(),
                    r.center(),
                    19.0,
                    progress,
                    color,
                    theme.border_strong,
                    3.0,
                );
                ui.painter().text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    if p.is_break() {
                        icons::COFFEE
                    } else {
                        icons::TIMER
                    },
                    mono(14.0),
                    if p.status == Status::Idle {
                        theme.fg_muted
                    } else {
                        color
                    },
                );
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let label = if p.status == Status::Idle {
                        t("pomodoro.phaseIdle").to_string()
                    } else {
                        phase_label(lang, p.phase).to_string()
                    };
                    ui.label(
                        RichText::new(label.to_uppercase())
                            .font(mono(10.0))
                            .color(theme.fg_faint),
                    );
                    let remaining = match p.status {
                        Status::Idle => durations.work * 60_000,
                        _ => p.remaining_ms(now),
                    };
                    ui.label(
                        RichText::new(countdown(remaining))
                            .font(widgets::bold(20.0))
                            .color(if p.status == Status::Paused {
                                theme.fg_muted
                            } else {
                                theme.fg
                            }),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if p.status != Status::Idle
                        && widgets::icon_button(ui, &theme, icons::RESET, t("pomodoro.reset"), 26.0)
                            .clicked()
                    {
                        app.pomodoro_update(|p, _, _| p.reset());
                    }
                    let (glyph, tip) = match p.status {
                        Status::Running => (icons::PAUSE, t("pomodoro.pause")),
                        Status::Paused => (icons::PLAY, t("pomodoro.resume")),
                        _ => (icons::PLAY, t("pomodoro.start")),
                    };
                    if widgets::icon_button_active(ui, &theme, glyph, tip, 30.0, true).clicked() {
                        app.pomodoro_update(|p, d, now| match p.status {
                            Status::Running => p.pause(now),
                            Status::Paused => p.resume(now),
                            _ => p.start(None, d, now),
                        });
                    }
                });
            });
            // Focus target and the cycle dots.
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{}:", t("pomodoro.focusOnLabel")))
                        .font(sans(11.5))
                        .color(theme.fg_faint),
                );
                let focus = p
                    .focus_todo_id
                    .as_deref()
                    .and_then(|id| app.data.todos.iter().find(|t| t.id == id))
                    .map(|t| t.title.clone());
                ui.add(
                    egui::Label::new(
                        RichText::new(focus.unwrap_or_else(|| t("pomodoro.focusOnNone").into()))
                            .font(sans(11.5))
                            .color(theme.fg_muted),
                    )
                    .truncate(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let done = p.cycles_completed % 4;
                    let full = p.cycles_completed > 0 && done == 0;
                    for i in (0..4).rev() {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                        let filled = full || i < done;
                        if filled {
                            ui.painter().circle_filled(r.center(), 3.0, theme.working);
                        } else {
                            ui.painter().circle_stroke(
                                r.center(),
                                2.5,
                                Stroke::new(1.0, theme.border_strong),
                            );
                        }
                    }
                });
            });
        });
    if p.status == Status::Running {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(1000));
    }
}

fn rule(ui: &mut egui::Ui, theme: &crate::theme::Theme) {
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment(
        [r.left_top(), Pos2::new(r.right(), r.top())],
        Stroke::new(1.0, theme.border),
    );
}
