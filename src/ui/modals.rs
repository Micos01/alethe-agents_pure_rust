//! Dialogs: new project / group, rename, delete confirmation, preferences and about.

use super::widgets::{self, mono, sans};
use crate::app::{Action, AletheApp, Modal};
use crate::model::PROJECT_COLORS;
use egui::{
    Align, Align2, CornerRadius, Layout, Margin, RichText, Sense, Stroke, StrokeKind, Vec2,
};

pub fn show(app: &mut AletheApp, ctx: &egui::Context) {
    let Some(mut modal) = app.modal.clone() else {
        return;
    };
    let theme = app.theme.clone();
    let mut close = false;
    let width = if matches!(modal, Modal::Settings) {
        620.0
    } else {
        440.0
    };

    let response = egui::Modal::new(egui::Id::unique("alethe-modal"))
        .frame(
            egui::Frame::new()
                .fill(theme.bg_elevated)
                .stroke(Stroke::new(1.0, theme.border_strong))
                .corner_radius(CornerRadius::same(14))
                .inner_margin(Margin::same(20)),
        )
        .show(ctx, |ui| {
            ui.set_width(width);
            match &mut modal {
                Modal::NewProject { name, folder, group_id, color } => {
                    title(ui, &theme, app.t("dialog.newProject"));
                    field(ui, &theme, app.t("dialog.name"));
                    let r = widgets::text_input(ui, &theme, name, "meu-projeto", width);
                    r.request_focus();
                    field(ui, &theme, app.t("dialog.folder"));
                    ui.horizontal(|ui| {
                        widgets::text_input(ui, &theme, folder, &crate::storage::home_dir(), width - 110.0);
                        if widgets::ghost_button(ui, &theme, app.t("dialog.browse")).clicked() {
                            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                *folder = dir.to_string_lossy().to_string();
                                if name.trim().is_empty() {
                                    *name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                }
                            }
                        }
                    });
                    if !app.data.groups.is_empty() {
                        field(ui, &theme, app.t("sidebar.moveToGroup"));
                        let current = group_id
                            .as_deref()
                            .and_then(|g| app.data.groups.iter().find(|x| x.id == g))
                            .map(|g| g.name.clone())
                            .unwrap_or_else(|| app.t("sidebar.ungrouped").to_string());
                        egui::ComboBox::from_id_salt("new-project-group").width(width - 16.0).selected_text(current).show_ui(ui, |ui| {
                            if ui.selectable_label(group_id.is_none(), app.t("sidebar.ungrouped")).clicked() {
                                *group_id = None;
                            }
                            for g in &app.data.groups {
                                if ui.selectable_label(group_id.as_deref() == Some(&g.id), &g.name).clicked() {
                                    *group_id = Some(g.id.clone());
                                }
                            }
                        });
                    }
                    field(ui, &theme, app.t("sidebar.color"));
                    color_picker(ui, &theme, color);
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if footer(ui, &theme, app.t("dialog.create"), app.t("dialog.cancel"), &mut close) || enter {
                        app.act(Action::CreateProject {
                            name: name.clone(),
                            folder: folder.clone(),
                            group_id: group_id.clone(),
                            color: PROJECT_COLORS[*color % PROJECT_COLORS.len()].to_string(),
                        });
                    }
                }
                Modal::NewGroup { name, color } => {
                    title(ui, &theme, app.t("dialog.newGroup"));
                    field(ui, &theme, app.t("dialog.name"));
                    widgets::text_input(ui, &theme, name, "frontend", width).request_focus();
                    field(ui, &theme, app.t("sidebar.color"));
                    color_picker(ui, &theme, color);
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if footer(ui, &theme, app.t("dialog.create"), app.t("dialog.cancel"), &mut close) || enter {
                        app.act(Action::CreateGroup { name: name.clone(), color: PROJECT_COLORS[*color % PROJECT_COLORS.len()].to_string() });
                    }
                }
                Modal::Rename { target, value } => {
                    title(ui, &theme, app.t("dialog.rename"));
                    widgets::text_input(ui, &theme, value, "", width).request_focus();
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if footer(ui, &theme, app.t("dialog.save"), app.t("dialog.cancel"), &mut close) || enter {
                        app.act(Action::Rename { target: target.clone(), value: value.clone() });
                    }
                }
                Modal::DeleteProject { id } => {
                    let name = app.project(id).map(|p| p.name.clone()).unwrap_or_default();
                    title(ui, &theme, app.t("dialog.deleteProject"));
                    ui.label(RichText::new(&name).font(widgets::bold(14.0)).color(theme.fg));
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(RichText::new(app.t("dialog.deleteProjectBody")).font(sans(13.0)).color(theme.fg_muted)).wrap());
                    if footer(ui, &theme, app.t("sidebar.delete"), app.t("dialog.cancel"), &mut close) {
                        app.act(Action::DeleteProject(id.clone()));
                    }
                }
                Modal::Suspend { target } => {
                    let (name, title_key) = match target {
                        crate::app::SuspendTarget::Group(id) => (
                            app.data.groups.iter().find(|g| &g.id == id).map(|g| g.name.clone()),
                            "resources.suspendGroupTitle",
                        ),
                        crate::app::SuspendTarget::Project(id) => {
                            (app.project(id).map(|p| p.name.clone()), "resources.suspendProjectTitle")
                        }
                    };
                    title(ui, &theme, app.t(title_key));
                    ui.label(RichText::new(name.unwrap_or_default()).font(widgets::bold(14.0)).color(theme.fg));
                    ui.add_space(4.0);
                    let body = app
                        .t("resources.suspendBody")
                        .replace("{count}", &app.running_in(target).to_string());
                    ui.add(egui::Label::new(RichText::new(body).font(sans(13.0)).color(theme.fg_muted)).wrap());
                    if footer(ui, &theme, app.t("resources.suspend"), app.t("dialog.cancel"), &mut close) {
                        app.act(Action::Suspend(target.clone()));
                    }
                }
                Modal::Settings => settings(app, ui, width, &mut close),
                Modal::About => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Image::from_bytes("bytes://alethe-logo", super::icons::APP_LOGO).fit_to_exact_size(Vec2::splat(40.0)));
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Alethe Native").font(widgets::bold(18.0)).color(theme.fg));
                            ui.label(RichText::new(format!("v{} · Rust + egui · AGPL-3.0", env!("CARGO_PKG_VERSION"))).font(mono(11.5)).color(theme.fg_faint));
                        });
                    });
                    ui.add_space(10.0);
                    ui.add(egui::Label::new(RichText::new("Versão nativa em Rust puro (sem Tauri/webview) do Alethe — github.com/Kc1t/alethe-agents.").font(sans(13.0)).color(theme.fg_muted)).wrap());
                    ui.add_space(12.0);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if widgets::primary_button(ui, &theme, "OK").clicked() {
                            close = true;
                        }
                    });
                }
            }
        });

    if response.should_close() || close {
        app.act(Action::CloseModal);
    } else if app.modal.is_some() {
        // Keep edits made in this frame unless an action already replaced the modal.
        app.modal = Some(modal);
    }
}

fn title(ui: &mut egui::Ui, theme: &crate::theme::Theme, text: &str) {
    ui.label(
        RichText::new(text)
            .font(widgets::bold(17.0))
            .color(theme.fg),
    );
    ui.add_space(10.0);
}

fn field(ui: &mut egui::Ui, theme: &crate::theme::Theme, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).font(sans(12.0)).color(theme.fg_faint));
    ui.add_space(2.0);
}

/// Returns true when the primary button was clicked.
fn footer(
    ui: &mut egui::Ui,
    theme: &crate::theme::Theme,
    primary: &str,
    secondary: &str,
    close: &mut bool,
) -> bool {
    ui.add_space(16.0);
    let mut confirmed = false;
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        if widgets::primary_button(ui, theme, primary).clicked() {
            confirmed = true;
        }
        if widgets::ghost_button(ui, theme, secondary).clicked() {
            *close = true;
        }
    });
    confirmed
}

fn color_picker(ui: &mut egui::Ui, theme: &crate::theme::Theme, selected: &mut usize) {
    ui.horizontal(|ui| {
        for (i, hex) in PROJECT_COLORS.iter().enumerate() {
            let color = crate::model::parse_hex_color(hex).unwrap_or(theme.accent);
            let (r, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
            ui.painter().circle_filled(r.center(), 9.0, color);
            if *selected == i {
                ui.painter()
                    .circle_stroke(r.center(), 11.5, Stroke::new(2.0, theme.fg));
            }
            if response.clicked() {
                *selected = i;
            }
        }
    });
}

fn settings(app: &mut AletheApp, ui: &mut egui::Ui, width: f32, close: &mut bool) {
    let theme = app.theme.clone();
    title(ui, &theme, app.t("settings.title"));
    egui::ScrollArea::vertical()
        .max_height(560.0)
        .show(ui, |ui| {
            ui.set_width(width - 8.0);
            section(ui, &theme, app.t("settings.appearance"));
            field(ui, &theme, app.t("settings.theme"));
            let columns = 4;
            let tile_w = (width - 8.0 - (columns as f32 - 1.0) * 8.0) / columns as f32;
            for row in crate::theme::THEME_OPTIONS.chunks(columns) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for (id, label, swatch) in row {
                        let selected = app.data.preferences.ui_theme == *id;
                        let (r, response) =
                            ui.allocate_exact_size(Vec2::new(tile_w, 58.0), Sense::click());
                        ui.painter().rect(
                            r,
                            CornerRadius::same(8),
                            crate::theme::hex(swatch[0]),
                            Stroke::new(
                                if selected { 2.0 } else { 1.0 },
                                if selected { theme.fg } else { theme.border },
                            ),
                            StrokeKind::Inside,
                        );
                        for (k, c) in swatch.iter().enumerate().skip(1) {
                            ui.painter().circle_filled(
                                egui::pos2(r.left() + 14.0 + (k - 1) as f32 * 16.0, r.top() + 16.0),
                                6.0,
                                crate::theme::hex(*c),
                            );
                        }
                        let text_color = if (swatch[0] >> 16) > 0x80 {
                            egui::Color32::from_rgb(30, 30, 30)
                        } else {
                            egui::Color32::from_rgb(235, 235, 235)
                        };
                        ui.painter().text(
                            egui::pos2(r.left() + 10.0, r.bottom() - 14.0),
                            Align2::LEFT_CENTER,
                            *label,
                            sans(12.0),
                            text_color,
                        );
                        if response.clicked() {
                            app.act(Action::SetTheme(id.to_string()));
                        }
                    }
                });
                ui.add_space(8.0);
            }

            field(ui, &theme, app.t("settings.terminalTheme"));
            let current = app.data.preferences.terminal_theme.clone();
            let current_label = current
                .as_deref()
                .and_then(|id| {
                    crate::theme::THEME_OPTIONS
                        .iter()
                        .find(|t| t.0 == id)
                        .map(|t| t.1)
                })
                .unwrap_or(app.t("settings.sameAsUi"));
            egui::ComboBox::from_id_salt("terminal-theme")
                .width(260.0)
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(current.is_none(), app.t("settings.sameAsUi"))
                        .clicked()
                    {
                        app.act(Action::SetTerminalTheme(None));
                    }
                    for (id, label, _) in crate::theme::THEME_OPTIONS {
                        if ui
                            .selectable_label(current.as_deref() == Some(*id), *label)
                            .clicked()
                        {
                            app.act(Action::SetTerminalTheme(Some(id.to_string())));
                        }
                    }
                });

            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    field(ui, &theme, app.t("settings.zoom"));
                    let mut zoom = app.data.preferences.ui_zoom;
                    if ui
                        .add(egui::Slider::new(&mut zoom, 0.7..=1.5).step_by(0.05))
                        .changed()
                    {
                        app.data.preferences.ui_zoom = zoom;
                        app.mark_dirty();
                    }
                });
                ui.add_space(24.0);
                ui.vertical(|ui| {
                    field(ui, &theme, app.t("settings.fontSize"));
                    let mut size = app.data.preferences.terminal_font_size;
                    if ui
                        .add(egui::Slider::new(&mut size, 9.0..=22.0).step_by(0.5))
                        .changed()
                    {
                        app.data.preferences.terminal_font_size = size;
                        app.mark_dirty();
                    }
                });
            });

            field(ui, &theme, app.t("settings.language"));
            ui.horizontal(|ui| {
                for (code, label) in [("pt-BR", "Português (Brasil)"), ("en", "English")] {
                    if ui
                        .selectable_label(app.data.preferences.language == code, label)
                        .clicked()
                    {
                        app.act(Action::SetLanguage(code.into()));
                    }
                }
            });

            ui.add_space(12.0);
            section(ui, &theme, app.t("settings.profile"));
            field(ui, &theme, app.t("settings.displayName"));
            let mut name = app.data.preferences.display_name.clone();
            if widgets::text_input(ui, &theme, &mut name, "dev", 300.0).changed() {
                app.data.preferences.display_name = name;
                app.mark_dirty();
            }
            field(ui, &theme, app.t("settings.shell"));
            let mut shell = app.data.preferences.shell_path.clone().unwrap_or_default();
            ui.horizontal(|ui| {
                if widgets::text_input(
                    ui,
                    &theme,
                    &mut shell,
                    &crate::backend::cli_resolver::default_shell(),
                    400.0,
                )
                .changed()
                {
                    app.data.preferences.shell_path =
                        (!shell.trim().is_empty()).then_some(shell.clone());
                    app.mark_dirty();
                }
                if widgets::ghost_button(ui, &theme, app.t("dialog.browse")).clicked() {
                    if let Some(file) = rfd::FileDialog::new().pick_file() {
                        app.data.preferences.shell_path = Some(file.to_string_lossy().to_string());
                        app.mark_dirty();
                    }
                }
            });

            ui.add_space(12.0);
            section(ui, &theme, app.t("settings.agents"));
            let agents: Vec<&crate::agents::AgentInfo> = crate::agents::available().collect();
            for row in agents.chunks(3) {
                ui.horizontal(|ui| {
                    for info in row {
                        ui.allocate_ui(Vec2::new((width - 20.0) / 3.0, 26.0), |ui| {
                            ui.horizontal(|ui| {
                                let mut enabled = app.data.preferences.agent_enabled(info.id);
                                if ui.checkbox(&mut enabled, "").changed() {
                                    app.data.preferences.enabled_agents.insert(
                                        info.id.to_string(),
                                        serde_json::Value::Bool(enabled),
                                    );
                                    app.mark_dirty();
                                }
                                widgets::agent_icon(ui, &theme, info.id, 16.0);
                                let found = crate::agents::installed(info.id);
                                let label = RichText::new(info.label)
                                    .font(sans(12.5))
                                    .color(if found { theme.fg } else { theme.fg_faint });
                                let r = ui.label(label);
                                if !found {
                                    r.on_hover_text(app.t("settings.notFound"));
                                }
                            });
                        });
                    }
                });
            }

            ui.add_space(12.0);
            section(ui, &theme, app.t("settings.features"));
            let mut orchestrator_on = app.feature_enabled("orchestrator");
            if ui
                .checkbox(
                    &mut orchestrator_on,
                    RichText::new(app.t("settings.orchestrator"))
                        .font(sans(12.5))
                        .color(theme.fg),
                )
                .changed()
            {
                app.set_feature("orchestrator", orchestrator_on);
            }
            ui.add(
                egui::Label::new(
                    RichText::new(app.t("settings.orchestratorHint"))
                        .font(sans(11.5))
                        .color(theme.fg_faint),
                )
                .wrap(),
            );
            if orchestrator_on {
                let mut max = app
                    .data
                    .preferences
                    .extra
                    .get("orchestration")
                    .and_then(|o| o.get("maxConcurrent"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(4) as usize;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(app.t("settings.maxWorkers"))
                            .font(sans(12.0))
                            .color(theme.fg_muted),
                    );
                    if ui.add(egui::Slider::new(&mut max, 1..=12)).changed() {
                        let orchestration = app
                            .data
                            .preferences
                            .extra
                            .entry("orchestration")
                            .or_insert(serde_json::Value::Null);
                        if !orchestration.is_object() {
                            // Same defaults as `DEFAULT_PREFERENCES.orchestration` in the web build.
                            *orchestration = serde_json::json!({
                                "roles": [],
                                "maxConcurrent": 4,
                                "defaultTimeoutSeconds": 900,
                                "workerDisabledPlugins": []
                            });
                        }
                        orchestration["maxConcurrent"] = serde_json::json!(max);
                        app.orchestrator
                            .apply_settings(app.data.preferences.extra.get("orchestration"));
                        app.mark_dirty();
                    }
                });
            }
            ui.add_space(6.0);
            for (provider, key) in [
                ("claude", "settings.usageClaude"),
                ("codex", "settings.usageCodex"),
            ] {
                let mut on = app.usage_access(provider);
                if ui
                    .checkbox(
                        &mut on,
                        RichText::new(app.t(key)).font(sans(12.5)).color(theme.fg),
                    )
                    .changed()
                {
                    app.set_usage_access(provider, on);
                }
            }
            ui.add(
                egui::Label::new(
                    RichText::new(app.t("settings.usageHint"))
                        .font(sans(11.5))
                        .color(theme.fg_faint),
                )
                .wrap(),
            );
            ui.add_space(6.0);
            section(ui, &theme, app.t("settings.data"));
            field(ui, &theme, app.t("settings.dataFile"));
            let path = app.storage.projects_path().to_string_lossy().to_string();
            ui.horizontal(|ui| {
                if ui.small_button(super::icons::FOLDER_OPENED).clicked() {
                    app.act(Action::OpenFolder(
                        app.storage.profile_dir().to_string_lossy().to_string(),
                    ));
                }
                // Long paths would otherwise widen the scroll area, and the modal with it.
                ui.add(
                    egui::Label::new(RichText::new(&path).font(mono(11.5)).color(theme.fg_muted))
                        .truncate(),
                )
                .on_hover_text(&path);
            });
            ui.add_space(6.0);
            let tauri = crate::storage::Storage::tauri_projects_path();
            ui.add_enabled_ui(tauri.is_some(), |ui| {
                if widgets::ghost_button(ui, &theme, app.t("settings.reimport")).clicked() {
                    app.act(Action::Reimport);
                    *close = true;
                }
            });
            ui.label(
                RichText::new(app.t("settings.reimportHint"))
                    .font(sans(11.5))
                    .color(theme.fg_faint),
            );
        });
    ui.add_space(12.0);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        if widgets::primary_button(ui, &theme, app.t("settings.done")).clicked() {
            *close = true;
        }
    });
}

fn section(ui: &mut egui::Ui, theme: &crate::theme::Theme, text: &str) {
    widgets::section_label(ui, theme, text);
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment(
        [
            egui::pos2(r.left(), r.top() + 2.0),
            egui::pos2(r.right(), r.top() + 2.0),
        ],
        Stroke::new(1.0, theme.border),
    );
    ui.add_space(6.0);
}
