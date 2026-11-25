//! Right sidebar: the Todo List panel (`todos` in `projects.json`).

use super::icons;
use super::widgets::{self, mono, sans};
use crate::app::{Action, AletheApp, RightTab};
use egui::{
    Align, Align2, CornerRadius, Layout, Margin, Pos2, RichText, Sense, Stroke, StrokeKind, Vec2,
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

pub fn show(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    let lang = app.lang();
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
                });
            });
        });
    rule(ui, &theme);

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
                if add.clicked() {
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
    if submit {
        let tags = app
            .todo
            .tags
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        app.act(Action::AddTodo {
            title: app.todo.input.clone(),
            tags,
            project_id: app.todo.project.clone(),
        });
        app.todo.input.clear();
    }
    rule(ui, &theme);

    if app.data.todos.is_empty() {
        ui.add_space(80.0);
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
                RichText::new(app.t("todo.emptyTitle"))
                    .font(widgets::bold(14.0))
                    .color(theme.fg),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(app.t("todo.emptyDescription"))
                        .font(sans(13.0))
                        .color(theme.fg_muted),
                )
                .wrap(),
            );
        });
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    let todos: Vec<(String, String, bool, Vec<String>, Option<String>)> = app
                        .data
                        .todos
                        .iter()
                        .map(|t| {
                            (
                                t.id.clone(),
                                t.title.clone(),
                                t.completed,
                                t.tags.clone(),
                                t.project_id.clone(),
                            )
                        })
                        .collect();
                    for (id, title, done, tags, project) in todos {
                        let (rect, response) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 40.0),
                            Sense::click(),
                        );
                        if response.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(6),
                                theme.panel_hover,
                            );
                        }
                        let check = egui::Rect::from_center_size(
                            Pos2::new(rect.left() + 16.0, rect.center().y),
                            Vec2::splat(16.0),
                        );
                        ui.painter().rect(
                            check,
                            CornerRadius::same(4),
                            if done { theme.accent } else { theme.bg },
                            Stroke::new(1.0, theme.border_strong),
                            StrokeKind::Inside,
                        );
                        if done {
                            ui.painter().text(
                                check.center(),
                                Align2::CENTER_CENTER,
                                icons::CHECK,
                                mono(11.0),
                                theme.accent_on,
                            );
                        }
                        let color = if done { theme.fg_faint } else { theme.fg };
                        ui.painter().text(
                            Pos2::new(rect.left() + 34.0, rect.top() + 12.0),
                            Align2::LEFT_CENTER,
                            &title,
                            sans(13.0),
                            color,
                        );
                        let mut meta: Vec<String> = tags.iter().map(|t| format!("#{t}")).collect();
                        if let Some(name) = project
                            .as_deref()
                            .and_then(|p| app.project(p))
                            .map(|p| p.name.clone())
                        {
                            meta.insert(0, name);
                        }
                        if !meta.is_empty() {
                            ui.painter().text(
                                Pos2::new(rect.left() + 34.0, rect.top() + 29.0),
                                Align2::LEFT_CENTER,
                                meta.join(" · "),
                                mono(10.5),
                                theme.fg_faint,
                            );
                        }
                        let trash = egui::Rect::from_center_size(
                            Pos2::new(rect.right() - 16.0, rect.center().y),
                            Vec2::splat(20.0),
                        );
                        let trash_r =
                            ui.interact(trash, egui::Id::unique(("todo-del", &id)), Sense::click());
                        if response.hovered() || trash_r.hovered() {
                            ui.painter().text(
                                trash.center(),
                                Align2::CENTER_CENTER,
                                icons::TRASH,
                                mono(12.0),
                                if trash_r.hovered() {
                                    theme.offline
                                } else {
                                    theme.fg_faint
                                },
                            );
                        }
                        if trash_r.clicked() {
                            app.act(Action::DeleteTodo(id.clone()));
                        } else if response.clicked() {
                            app.act(Action::ToggleTodo(id.clone()));
                        }
                    }
                });
        });
}

fn rule(ui: &mut egui::Ui, theme: &crate::theme::Theme) {
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment(
        [r.left_top(), Pos2::new(r.right(), r.top())],
        Stroke::new(1.0, theme.border),
    );
}
