//! Views. Layout mirrors `App.tsx`: title bar on top, project sidebar on the left, the Home or
//! Workspace view in the middle and the Todo panel on the right.

pub mod git_panel;
pub mod home;
pub mod icons;
pub mod markdown;
pub mod modals;
pub mod orchestrator;
pub mod sidebar;
pub mod titlebar;
pub mod todos;
pub mod widgets;
pub mod workspace;

use crate::app::{AletheApp, View};
use egui::{
    CornerRadius, CursorIcon, Frame, Margin, Pos2, Rect, ResizeDirection, Sense, Stroke,
    ViewportCommand,
};

pub fn root(app: &mut AletheApp, ui: &mut egui::Ui) {
    let theme = app.theme.clone();

    egui::Panel::top("titlebar")
        .exact_size(36.0)
        .frame(
            Frame::new()
                .fill(theme.bg)
                .inner_margin(Margin::symmetric(6, 0))
                .stroke(Stroke::NONE),
        )
        .show(ui, |ui| titlebar::show(app, ui));

    if app.show_sidebar {
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(272.0)
            .size_range(220.0..=440.0)
            .frame(
                Frame::new()
                    .fill(theme.bg)
                    .stroke(Stroke::new(1.0, theme.border)),
            )
            .show(ui, |ui| sidebar::show(app, ui));
    }

    orchestrator::auto_open_agents(app);
    if app.show_right {
        egui::Panel::right("right-sidebar")
            .resizable(true)
            .default_size(320.0)
            .size_range(260.0..=480.0)
            .frame(
                Frame::new()
                    .fill(theme.bg)
                    .stroke(Stroke::new(1.0, theme.border)),
            )
            .show(ui, |ui| match app.right_tab {
                crate::app::RightTab::Todos => todos::show(app, ui),
                crate::app::RightTab::Agents => orchestrator::agents_panel(app, ui),
            });
    }

    egui::CentralPanel::default()
        .frame(Frame::new().fill(theme.bg))
        .show(ui, |ui| match app.view.clone() {
            View::Home => home::show(app, ui),
            View::Project(id) => workspace::show(app, ui, &id),
        });

    modals::show(app, ui.ctx());
    toasts(app, ui.ctx());
    window_resize_edges(ui.ctx());
}

fn toasts(app: &AletheApp, ctx: &egui::Context) {
    if app.toasts.is_empty() {
        return;
    }
    let theme = &app.theme;
    egui::Area::new(egui::Id::unique("toasts"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            for (text, _) in &app.toasts {
                Frame::new()
                    .fill(theme.bg_elevated)
                    .stroke(Stroke::new(1.0, theme.border_strong))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(text)
                                .color(theme.fg)
                                .font(widgets::sans(13.0)),
                        );
                    });
                ui.add_space(6.0);
            }
        });
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
}

/// The window has no OS frame (like the Tauri build), so the edges start native resizes.
fn window_resize_edges(ctx: &egui::Context) {
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    if maximized {
        return;
    }
    let screen = ctx.content_rect();
    let edge = 5.0;
    let zones = [
        (
            ResizeDirection::NorthWest,
            Rect::from_min_size(screen.left_top(), egui::vec2(edge * 2.0, edge * 2.0)),
            CursorIcon::ResizeNorthWest,
        ),
        (
            ResizeDirection::NorthEast,
            Rect::from_min_size(
                Pos2::new(screen.right() - edge * 2.0, screen.top()),
                egui::vec2(edge * 2.0, edge * 2.0),
            ),
            CursorIcon::ResizeNorthEast,
        ),
        (
            ResizeDirection::SouthWest,
            Rect::from_min_size(
                Pos2::new(screen.left(), screen.bottom() - edge * 2.0),
                egui::vec2(edge * 2.0, edge * 2.0),
            ),
            CursorIcon::ResizeSouthWest,
        ),
        (
            ResizeDirection::SouthEast,
            Rect::from_min_size(
                Pos2::new(screen.right() - edge * 2.0, screen.bottom() - edge * 2.0),
                egui::vec2(edge * 2.0, edge * 2.0),
            ),
            CursorIcon::ResizeSouthEast,
        ),
        (
            ResizeDirection::North,
            Rect::from_min_max(
                screen.left_top(),
                Pos2::new(screen.right(), screen.top() + edge),
            ),
            CursorIcon::ResizeVertical,
        ),
        (
            ResizeDirection::South,
            Rect::from_min_max(
                Pos2::new(screen.left(), screen.bottom() - edge),
                screen.right_bottom(),
            ),
            CursorIcon::ResizeVertical,
        ),
        (
            ResizeDirection::West,
            Rect::from_min_max(
                screen.left_top(),
                Pos2::new(screen.left() + edge, screen.bottom()),
            ),
            CursorIcon::ResizeHorizontal,
        ),
        (
            ResizeDirection::East,
            Rect::from_min_max(
                Pos2::new(screen.right() - edge, screen.top()),
                screen.right_bottom(),
            ),
            CursorIcon::ResizeHorizontal,
        ),
    ];
    egui::Area::new(egui::Id::unique("resize-edges"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.left_top())
        .interactable(true)
        .show(ctx, |ui| {
            for (i, (direction, rect, cursor)) in zones.into_iter().enumerate() {
                let response =
                    ui.interact(rect, egui::Id::unique(("resize-edge", i)), Sense::drag());
                if response.hovered() || response.dragged() {
                    ctx.set_cursor_icon(cursor);
                }
                if response.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
                }
            }
        });
}
