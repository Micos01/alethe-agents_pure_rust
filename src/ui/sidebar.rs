//! `ProjectSidebar` (normal mode): view switcher, search, the group/project/terminal tree,
//! workspace layout footer and the profile row.

use super::icons;
use super::widgets::{
    self, count_badge, icon_button, icon_button_active, mono, sans, section_label, status_color,
};
use crate::app::{Action, AletheApp, Modal, RenameTarget, SidebarTab, View};
use crate::model::Project;
use egui::{
    Align, Align2, Color32, CornerRadius, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};

pub fn show(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let full = ui.max_rect();

    // Top: view switcher (home / projects / files / git).
    egui::Frame::new()
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let home = app.view == View::Home;
                let git_tab = app.sidebar_tab == SidebarTab::Git;
                if icon_button_active(ui, &theme, icons::HOME, app.t("sidebar.home"), 30.0, home)
                    .clicked()
                {
                    app.act(Action::OpenHome);
                }
                if icon_button_active(
                    ui,
                    &theme,
                    icons::GRID,
                    app.t("sidebar.projects"),
                    30.0,
                    !home && !git_tab,
                )
                .clicked()
                {
                    app.sidebar_tab = SidebarTab::Projects;
                    if let Some(id) = app.data.workspace.recent_project_ids.first().cloned() {
                        app.act(Action::OpenProject(id));
                    }
                }
                // Files: no tree of our own; nvim in a pane is the file browser and editor.
                let files_project = app
                    .active_project_id()
                    .map(str::to_string)
                    .or_else(|| app.data.workspace.recent_project_ids.first().cloned());
                ui.add_enabled_ui(files_project.is_some(), |ui| {
                    if icon_button(ui, &theme, icons::FOLDER, app.t("sidebar.files"), 30.0)
                        .clicked()
                    {
                        if let Some(id) = files_project {
                            app.act(Action::OpenFiles(id));
                        }
                    }
                });
                // Source Control: like the web build's contributed `git` tab, it leaves Home.
                let git_label = crate::ui::git_panel::tab_label(lang);
                if icon_button_active(ui, &theme, icons::SOURCE_CONTROL, git_label, 30.0, git_tab)
                    .clicked()
                {
                    app.sidebar_tab = SidebarTab::Git;
                    if home {
                        if let Some(id) = app.data.workspace.recent_project_ids.first().cloned() {
                            app.act(Action::OpenProject(id));
                        }
                    }
                }
            });
        });
    separator(ui, &theme);

    let footer_height = 112.0;
    if app.sidebar_tab == SidebarTab::Git {
        crate::ui::git_panel::sidebar_section(app, ui, full.bottom() - footer_height);
        pinned_footer(app, ui, full, footer_height);
        return;
    }

    // Search.
    egui::Frame::new()
        .inner_margin(Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icons::SEARCH)
                        .font(mono(14.0))
                        .color(theme.fg_faint),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut app.sidebar_search)
                        .hint_text(
                            RichText::new(crate::i18n::tr(lang, "sidebar.search"))
                                .color(theme.fg_faint),
                        )
                        .frame(egui::Frame::NONE)
                        .font(sans(13.5))
                        .desired_width(f32::INFINITY),
                );
            });
        });
    separator(ui, &theme);

    // PROJETOS header.
    egui::Frame::new()
        .inner_margin(Margin {
            left: 14,
            right: 8,
            top: 8,
            bottom: 4,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                section_label(ui, &theme, app.t("sidebar.projects"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let add =
                        icon_button(ui, &theme, icons::ADD, app.t("sidebar.newProject"), 24.0);
                    if add.clicked() {
                        app.act(Action::ShowModal(app.new_project_modal(None)));
                    }
                    if icon_button(
                        ui,
                        &theme,
                        icons::NEW_FOLDER,
                        app.t("sidebar.newProjectFromFolder"),
                        24.0,
                    )
                    .clicked()
                    {
                        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                            let color = crate::model::PROJECT_COLORS
                                [app.data.projects.len() % crate::model::PROJECT_COLORS.len()]
                            .to_string();
                            app.act(Action::CreateProject {
                                name: String::new(),
                                folder: folder.to_string_lossy().to_string(),
                                group_id: None,
                                color,
                            });
                        }
                    }
                    if icon_button(ui, &theme, icons::LAYERS, app.t("sidebar.newGroup"), 24.0)
                        .clicked()
                    {
                        app.act(Action::ShowModal(Modal::NewGroup {
                            name: String::new(),
                            color: 1,
                        }));
                    }
                });
            });
        });

    let tree_height = (full.bottom() - ui.cursor().top() - footer_height).max(40.0);
    egui::ScrollArea::vertical()
        .max_height(tree_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(2.0);
            tree(app, ui);
            ui.add_space(12.0);
        });

    pinned_footer(app, ui, full, footer_height);
}

/// Workspace layout + profile rows, pinned at the bottom of the sidebar.
fn pinned_footer(app: &mut AletheApp, ui: &mut egui::Ui, full: Rect, footer_height: f32) {
    let footer = Rect::from_min_max(
        Pos2::new(full.left(), full.bottom() - footer_height),
        full.right_bottom(),
    );
    let mut fui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(footer)
            .layout(Layout::top_down(Align::Min)),
    );
    footer_ui(app, &mut fui);
}

fn separator(ui: &mut egui::Ui, theme: &crate::theme::Theme) {
    let rect = ui.available_rect_before_wrap();
    ui.painter().line_segment(
        [
            Pos2::new(rect.left(), rect.top()),
            Pos2::new(rect.right(), rect.top()),
        ],
        Stroke::new(1.0, theme.border),
    );
}

fn matches_search(app: &AletheApp, project: &Project) -> bool {
    let q = app.sidebar_search.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    project.name.to_lowercase().contains(&q)
        || project
            .terminals
            .iter()
            .any(|t| app.terminal_title(t).to_lowercase().contains(&q) || t.agent().contains(&q))
}

fn tree(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    let groups: Vec<(String, String, String, bool, Vec<String>)> = app
        .data
        .groups
        .iter()
        .filter(|g| !g.archived.unwrap_or(false))
        .map(|g| {
            (
                g.id.clone(),
                g.name.clone(),
                g.color.clone(),
                g.collapsed,
                g.project_ids.clone(),
            )
        })
        .collect();
    let searching = !app.sidebar_search.trim().is_empty();
    let mut listed: Vec<String> = Vec::new();
    let mut anything = false;

    for (group_id, name, color, collapsed, project_ids) in groups {
        let projects: Vec<String> = project_ids
            .iter()
            .filter(|id| {
                app.project(id)
                    .is_some_and(|p| p.is_visible() && matches_search(app, p))
            })
            .cloned()
            .collect();
        listed.extend(project_ids.iter().cloned());
        if searching && projects.is_empty() {
            continue;
        }
        anything = true;
        let color = crate::model::parse_hex_color(&color).unwrap_or(theme.accent);
        egui::Frame::new()
            .stroke(Stroke::new(1.0, theme.border))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(2, 4))
            .outer_margin(Margin::symmetric(8, 4))
            .show(ui, |ui| {
                group_header(app, ui, &group_id, &name, color, collapsed, projects.len());
                if !collapsed || searching {
                    for project_id in &projects {
                        project_node(app, ui, project_id);
                    }
                }
            });
    }

    // Ungrouped, in `ungroupedOrder`, then anything the order misses.
    let mut ungrouped: Vec<String> = app
        .data
        .ungrouped_order
        .iter()
        .filter(|id| !listed.contains(id))
        .cloned()
        .collect();
    for project in &app.data.projects {
        if !listed.contains(&project.id) && !ungrouped.contains(&project.id) {
            ungrouped.push(project.id.clone());
        }
    }
    let ungrouped: Vec<String> = ungrouped
        .into_iter()
        .filter(|id| {
            app.project(id)
                .is_some_and(|p| p.is_visible() && matches_search(app, p))
        })
        .collect();
    if !ungrouped.is_empty() {
        anything = true;
        if !app.data.groups.is_empty() {
            egui::Frame::new()
                .inner_margin(Margin {
                    left: 16,
                    right: 8,
                    top: 10,
                    bottom: 4,
                })
                .show(ui, |ui| {
                    section_label(ui, &theme, app.t("sidebar.ungrouped"));
                });
        }
        egui::Frame::new()
            .inner_margin(Margin::symmetric(8, 0))
            .show(ui, |ui| {
                for id in &ungrouped {
                    project_node(app, ui, id);
                }
            });
    }

    if !anything {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            let key = if searching {
                "sidebar.noResults"
            } else {
                "home.projectsEmptyTitle"
            };
            ui.label(
                RichText::new(app.t(key))
                    .color(theme.fg_faint)
                    .font(sans(12.5)),
            );
        });
    }
}

fn group_header(
    app: &mut AletheApp,
    ui: &mut egui::Ui,
    id: &str,
    name: &str,
    color: Color32,
    collapsed: bool,
    count: usize,
) {
    let theme = app.theme.clone();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();
    let mut x = rect.left() + 8.0;
    painter.text(
        Pos2::new(x + 5.0, rect.center().y),
        Align2::CENTER_CENTER,
        if collapsed {
            icons::CHEVRON_RIGHT
        } else {
            icons::CHEVRON_DOWN
        },
        mono(11.0),
        theme.fg_faint,
    );
    x += 16.0;
    painter.circle_filled(Pos2::new(x + 3.0, rect.center().y), 3.5, color);
    x += 12.0;
    let spaced: String = name
        .to_uppercase()
        .chars()
        .flat_map(|c| [c, '\u{200a}'])
        .collect();
    painter.text(
        Pos2::new(x, rect.center().y),
        Align2::LEFT_CENTER,
        spaced,
        widgets::bold(11.0),
        theme.fg_muted,
    );
    painter.text(
        Pos2::new(rect.right() - 40.0, rect.center().y),
        Align2::CENTER_CENTER,
        count.to_string(),
        mono(10.5),
        theme.fg_faint,
    );

    let add_rect = Rect::from_center_size(
        Pos2::new(rect.right() - 16.0, rect.center().y),
        Vec2::splat(20.0),
    );
    let add = ui.interact(
        add_rect,
        egui::Id::unique(("group-add", id)),
        Sense::click(),
    );
    if hovered || add.hovered() {
        if add.hovered() {
            ui.painter()
                .rect_filled(add_rect, CornerRadius::same(4), theme.panel_hover);
        }
        ui.painter().text(
            add_rect.center(),
            Align2::CENTER_CENTER,
            icons::ADD,
            mono(12.0),
            theme.fg_muted,
        );
    }
    if add.clicked() {
        app.act(Action::ShowModal(
            app.new_project_modal(Some(id.to_string())),
        ));
    } else if response.clicked() {
        app.act(Action::ToggleGroupCollapsed(id.to_string()));
    }
    response.context_menu(|ui| {
        if ui.button(app.t("sidebar.newProject")).clicked() {
            app.act(Action::ShowModal(
                app.new_project_modal(Some(id.to_string())),
            ));
            ui.close();
        }
        let target = crate::app::SuspendTarget::Group(id.to_string());
        let suspended = app
            .data
            .groups
            .iter()
            .any(|g| g.id == id && g.suspended == Some(true));
        if suspended {
            if ui.button(app.t("resources.reactivateGroup")).clicked() {
                app.act(Action::Reactivate(target));
                ui.close();
            }
        } else if ui.button(app.t("resources.suspendGroup")).clicked() {
            app.act(Action::ShowModal(Modal::Suspend { target }));
            ui.close();
        }
        if ui.button(app.t("sidebar.rename")).clicked() {
            app.act(Action::ShowModal(Modal::Rename {
                target: RenameTarget::Group(id.to_string()),
                value: name.to_string(),
            }));
            ui.close();
        }
        ui.separator();
        if ui
            .button(RichText::new(app.t("sidebar.delete")).color(theme.offline))
            .clicked()
        {
            app.act(Action::DeleteGroup(id.to_string()));
            ui.close();
        }
    });
}

fn agent_menu(app: &AletheApp, ui: &mut egui::Ui, on_pick: impl Fn(&str) -> Action) {
    let theme = app.theme.clone();
    ui.set_min_width(200.0);
    for agent in crate::agents::available() {
        if !app.data.preferences.agent_enabled(agent.id) {
            continue;
        }
        let clicked = ui
            .horizontal(|ui| {
                widgets::agent_icon(ui, &theme, agent.id, 16.0);
                ui.add(egui::Button::new(agent.label).frame(false))
                    .clicked()
            })
            .inner;
        if clicked {
            app.act(on_pick(agent.id));
            ui.close();
        }
    }
}

fn project_node(app: &mut AletheApp, ui: &mut egui::Ui, project_id: &str) {
    let theme = app.theme.clone();
    let Some(project) = app.project(project_id) else {
        return;
    };
    let name = project.name.clone();
    let collapsed = project.collapsed && app.sidebar_search.trim().is_empty();
    let color = project
        .color
        .as_deref()
        .and_then(crate::model::parse_hex_color)
        .unwrap_or(theme.accent);
    let terminals: Vec<(
        String,
        String,
        String,
        Option<&'static str>,
        Option<crate::terminal::Status>,
    )> = project
        .terminals
        .iter()
        .map(|t| {
            (
                t.id.clone(),
                app.terminal_title(t),
                t.agent(),
                pane_glyph(t.kind.as_deref()),
                app.terminal_status(t),
            )
        })
        .collect();
    let cwd = project.cwd();
    let group_id = project.group_id.clone();
    let suspended = project.suspended();
    let active = app.view == View::Project(project_id.to_string());

    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
    let hovered = response.hovered() || ui.rect_contains_pointer(rect);
    if active || hovered {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(6),
            if active {
                theme.panel
            } else {
                theme.panel_hover
            },
        );
    }
    let painter = ui.painter();
    let mut x = rect.left() + 6.0;
    let any_working = terminals
        .iter()
        .any(|t| t.4 == Some(crate::terminal::Status::Working));
    let dot = if any_working {
        theme.working
    } else if terminals.iter().any(|t| t.4.is_some()) {
        theme.waiting
    } else {
        theme.stopped.gamma_multiply(0.5)
    };
    painter.circle_filled(Pos2::new(x + 3.0, rect.center().y), 3.0, dot);
    x += 12.0;
    let avatar = Rect::from_center_size(Pos2::new(x + 10.0, rect.center().y), Vec2::splat(20.0));
    widgets::paint_avatar(ui, avatar, color, &name, 2);
    x += 28.0;
    let name_font = if active {
        widgets::bold(13.5)
    } else {
        sans(13.5)
    };
    let max_name = rect.right() - 64.0 - x;
    let name_color = if suspended { theme.fg_faint } else { theme.fg };
    let galley = painter.layout_no_wrap(name.clone(), name_font, name_color);
    painter
        .with_clip_rect(Rect::from_min_max(
            Pos2::new(x, rect.top()),
            Pos2::new(x + max_name.max(10.0), rect.bottom()),
        ))
        .galley(
            Pos2::new(x, rect.center().y - galley.size().y / 2.0),
            galley,
            name_color,
        );

    // Count, or hover actions (+ / …).
    let plus_rect = Rect::from_center_size(
        Pos2::new(rect.right() - 40.0, rect.center().y),
        Vec2::splat(20.0),
    );
    let more_rect = Rect::from_center_size(
        Pos2::new(rect.right() - 16.0, rect.center().y),
        Vec2::splat(20.0),
    );
    let plus = ui.interact(
        plus_rect,
        egui::Id::unique(("project-plus", project_id)),
        Sense::click(),
    );
    let more = ui.interact(
        more_rect,
        egui::Id::unique(("project-more", project_id)),
        Sense::click(),
    );
    if hovered || plus.hovered() || more.hovered() {
        for (r, glyph, resp) in [
            (plus_rect, icons::ADD, &plus),
            (more_rect, icons::ELLIPSIS, &more),
        ] {
            if resp.hovered() {
                ui.painter()
                    .rect_filled(r, CornerRadius::same(4), theme.panel_hover);
            }
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                glyph,
                mono(12.0),
                theme.fg_muted,
            );
        }
    } else if suspended {
        ui.painter().text(
            Pos2::new(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            app.t("resources.suspended"),
            mono(10.5),
            theme.fg_faint,
        );
    } else {
        ui.painter().text(
            Pos2::new(rect.right() - 16.0, rect.center().y),
            Align2::CENTER_CENTER,
            terminals.len().to_string(),
            mono(11.0),
            theme.fg_faint,
        );
    }

    let pid = project_id.to_string();
    egui::Popup::menu(&plus).show(|ui| {
        agent_menu(app, ui, |agent| Action::NewTerminal {
            project_id: pid.clone(),
            agent: agent.to_string(),
            cwd: None,
            extra_args: vec![],
            initial_input: None,
        });
        ui.separator();
        if ui
            .button(format!(
                "{}  {}",
                super::icons::TYPE_HIERARCHY,
                app.t("sidebar.newOrchestration")
            ))
            .clicked()
        {
            app.act(Action::NewOrchestration(pid.clone()));
            ui.close();
        }
        if ui
            .button(format!(
                "{}  {}",
                super::icons::MARKDOWN,
                app.t("sidebar.openMarkdown")
            ))
            .clicked()
        {
            app.act(Action::OpenMarkdown(pid.clone()));
            ui.close();
        }
        if ui
            .button(format!(
                "{}  {}",
                super::icons::FILE_MEDIA,
                app.t("sidebar.openImage")
            ))
            .clicked()
        {
            app.act(Action::OpenImage(pid.clone()));
            ui.close();
        }
    });
    let menu_contents = |app: &mut AletheApp, ui: &mut egui::Ui| {
        ui.menu_button(app.t("sidebar.newTerminal"), |ui| {
            agent_menu(app, ui, |agent| Action::NewTerminal {
                project_id: pid.clone(),
                agent: agent.to_string(),
                cwd: None,
                extra_args: vec![],
                initial_input: None,
            })
        });
        if ui.button(app.t("sidebar.rename")).clicked() {
            app.act(Action::ShowModal(Modal::Rename {
                target: RenameTarget::Project(pid.clone()),
                value: name.clone(),
            }));
            ui.close();
        }
        if ui.button(app.t("sidebar.openFolder")).clicked() {
            app.act(Action::OpenFolder(cwd.clone()));
            ui.close();
        }
        let groups: Vec<(String, String)> = app
            .data
            .groups
            .iter()
            .map(|g| (g.id.clone(), g.name.clone()))
            .collect();
        if !groups.is_empty() {
            ui.menu_button(app.t("sidebar.moveToGroup"), |ui| {
                for (gid, gname) in &groups {
                    if ui
                        .add_enabled(group_id.as_deref() != Some(gid), egui::Button::new(gname))
                        .clicked()
                    {
                        app.act(Action::MoveProjectToGroup {
                            project_id: pid.clone(),
                            group_id: Some(gid.clone()),
                        });
                        ui.close();
                    }
                }
                if group_id.is_some() && ui.button(app.t("sidebar.removeFromGroup")).clicked() {
                    app.act(Action::MoveProjectToGroup {
                        project_id: pid.clone(),
                        group_id: None,
                    });
                    ui.close();
                }
            });
        }
        let target = crate::app::SuspendTarget::Project(pid.clone());
        if suspended {
            if ui.button(app.t("resources.reactivateProject")).clicked() {
                app.act(Action::Reactivate(target));
                ui.close();
            }
        } else if ui.button(app.t("resources.suspendProject")).clicked() {
            app.act(Action::ShowModal(Modal::Suspend { target }));
            ui.close();
        }
        ui.separator();
        if ui
            .button(RichText::new(app.t("sidebar.delete")).color(theme.offline))
            .clicked()
        {
            app.act(Action::ShowModal(Modal::DeleteProject { id: pid.clone() }));
            ui.close();
        }
    };
    egui::Popup::menu(&more).show(|ui| menu_contents(app, ui));
    response.context_menu(|ui| menu_contents(app, ui));

    if response.clicked() && !plus.clicked() && !more.clicked() {
        if active {
            app.act(Action::ToggleProjectCollapsed(project_id.to_string()));
        } else {
            app.act(Action::OpenProject(project_id.to_string()));
        }
    }

    if !collapsed && !suspended {
        for (terminal_id, title, agent, glyph, status) in terminals {
            terminal_row(app, ui, &terminal_id, &title, &agent, glyph, status);
        }
    }
}

/// The glyph a pane that is not a terminal shows in place of an agent icon.
fn pane_glyph(kind: Option<&str>) -> Option<&'static str> {
    match kind? {
        "markdown" => Some(super::icons::MARKDOWN),
        "image" => Some(super::icons::FILE_MEDIA),
        "orchestrator" => Some(super::icons::TYPE_HIERARCHY),
        _ => None,
    }
}

fn terminal_row(
    app: &mut AletheApp,
    ui: &mut egui::Ui,
    id: &str,
    title: &str,
    agent: &str,
    glyph: Option<&'static str>,
    status: Option<crate::terminal::Status>,
) {
    let theme = app.theme.clone();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
    let focused = app.focused_terminal.as_deref() == Some(id);
    if focused || response.hovered() {
        ui.painter().rect_filled(
            rect.shrink2(Vec2::new(0.0, 1.0)),
            CornerRadius::same(5),
            if focused {
                theme.accent_soft
            } else {
                theme.panel_hover
            },
        );
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(4.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.add_space(18.0);
    widgets::status_dot(
        &mut child,
        status_color(&theme, status),
        status == Some(crate::terminal::Status::Working),
    );
    match glyph {
        Some(glyph) => {
            let (r, _) = child.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
            child.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                glyph,
                mono(12.5),
                theme.fg_muted,
            );
        }
        None => widgets::agent_icon(&mut child, &theme, agent, 14.0),
    }
    child.label(RichText::new(title).font(sans(12.5)).color(if focused {
        theme.fg
    } else {
        theme.fg_muted
    }));

    if response.clicked() {
        app.act(Action::FocusTerminal(id.to_string()));
    }
    response.context_menu(|ui| {
        if ui.button(app.t("sidebar.rename")).clicked() {
            app.act(Action::ShowModal(Modal::Rename {
                target: RenameTarget::Terminal(id.to_string()),
                value: title.to_string(),
            }));
            ui.close();
        }
        if ui.button(app.t("pane.maximize")).clicked() {
            app.act(Action::FocusTerminal(id.to_string()));
            app.act(Action::ToggleMaximize(id.to_string()));
            ui.close();
        }
        ui.separator();
        if ui
            .button(RichText::new(app.t("sidebar.close")).color(theme.offline))
            .clicked()
        {
            app.act(Action::CloseTerminal(id.to_string()));
            ui.close();
        }
    });
}

fn footer_ui(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, CornerRadius::ZERO, theme.bg);
    ui.painter().line_segment(
        [rect.left_top(), rect.right_top()],
        Stroke::new(1.0, theme.border),
    );

    egui::Frame::new()
        .inner_margin(Margin {
            left: 14,
            right: 10,
            top: 8,
            bottom: 6,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                section_label(ui, &theme, app.t("sidebar.workspace"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let project_id = app.active_project_id().map(str::to_string);
                    let current = project_id
                        .as_deref()
                        .and_then(|id| app.project(id))
                        .map(|p| p.layout_mode.clone())
                        .unwrap_or_default();
                    ui.add_enabled_ui(project_id.is_some(), |ui| {
                        for (mode, glyph, key) in [
                            ("sidebar", icons::LAYOUT_SIDEBAR_LEFT, "layout.sidebar"),
                            ("spotlight", icons::SCREEN_FULL, "layout.spotlight"),
                            ("grid", icons::GRID, "layout.grid"),
                            ("auto", icons::SPLIT, "layout.auto"),
                        ] {
                            if icon_button_active(
                                ui,
                                &theme,
                                glyph,
                                app.t(key),
                                24.0,
                                current == mode,
                            )
                            .clicked()
                            {
                                if let Some(pid) = project_id.clone() {
                                    app.act(Action::SetLayout {
                                        project_id: pid,
                                        mode: mode.into(),
                                    });
                                }
                            }
                        }
                    });
                });
            });
        });

    let sep = ui.available_rect_before_wrap();
    ui.painter().line_segment(
        [sep.left_top(), Pos2::new(sep.right(), sep.top())],
        Stroke::new(1.0, theme.border),
    );
    egui::Frame::new()
        .inner_margin(Margin {
            left: 12,
            right: 10,
            top: 10,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let name = if app.data.preferences.display_name.trim().is_empty() {
                    app.t("greeting.fallbackName").to_string()
                } else {
                    app.data.preferences.display_name.clone()
                };
                let (avatar, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                ui.painter()
                    .circle_filled(avatar.center(), 15.0, theme.panel);
                ui.painter().text(
                    avatar.center(),
                    Align2::CENTER_CENTER,
                    widgets::initials(&name, 1),
                    widgets::bold(13.0),
                    theme.fg,
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(
                        RichText::new(&name)
                            .font(widgets::bold(13.0))
                            .color(theme.fg),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            app.t("sidebar.local"),
                            app.storage.index.active_name()
                        ))
                        .font(sans(11.0))
                        .color(theme.fg_faint),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icon_button(ui, &theme, icons::GEAR, app.t("menu.preferences"), 26.0)
                        .clicked()
                    {
                        app.act(Action::ShowModal(Modal::Settings));
                    }
                });
            });
        });
    let _ = count_badge;
}
