//! Small building blocks shared by every view, styled after the web build's CSS modules.

use crate::theme::Theme;
use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Pos2, Rect, Response, RichText, Sense,
    Stroke, StrokeKind, Ui, Vec2,
};

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn sans(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// Square icon button (`.iconBtn`): transparent until hovered.
pub fn icon_button(ui: &mut Ui, theme: &Theme, glyph: &str, tooltip: &str, size: f32) -> Response {
    icon_button_active(ui, theme, glyph, tooltip, size, false)
}

pub fn icon_button_active(
    ui: &mut Ui,
    theme: &Theme,
    glyph: &str,
    tooltip: &str,
    size: f32,
    active: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let hovered = response.hovered();
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
    let color = if active || hovered {
        theme.fg
    } else {
        theme.fg_faint
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        mono((size * 0.56).round()),
        color,
    );
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

/// Uppercase, letter-spaced section label (`PROJETOS`, `PROJETOS RECENTES`).
pub fn section_label(ui: &mut Ui, theme: &Theme, text: &str) -> Response {
    let spaced: String = text
        .to_uppercase()
        .chars()
        .flat_map(|c| [c, '\u{200a}'])
        .collect();
    ui.label(RichText::new(spaced).font(bold(11.0)).color(theme.fg_faint))
}

/// Small counter pill (`1`, `3`).
pub fn count_badge(ui: &mut Ui, theme: &Theme, count: usize) {
    let text = count.to_string();
    let galley = ui
        .painter()
        .layout_no_wrap(text, mono(10.5), theme.fg_faint);
    let size = Vec2::new((galley.size().x + 10.0).max(18.0), 16.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(8), theme.panel);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, theme.fg_faint);
}

pub fn status_color(theme: &Theme, status: Option<crate::terminal::Status>) -> Color32 {
    use crate::terminal::Status;
    match status {
        Some(Status::Working) => theme.working,
        Some(Status::Waiting) => theme.waiting,
        Some(Status::Exited) => theme.offline,
        None => theme.stopped,
    }
}

pub fn status_dot(ui: &mut Ui, color: Color32, pulse: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    let t = ui.input(|i| i.time) as f32;
    let alpha = if pulse {
        0.55 + 0.45 * (t * 4.0).sin().abs()
    } else {
        1.0
    };
    ui.painter()
        .circle_filled(rect.center(), 3.5, color.gamma_multiply(alpha));
    if pulse {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(80));
    }
}

/// Agent logo, or a colored terminal glyph for shells / unknown agents.
pub fn agent_icon(ui: &mut Ui, theme: &Theme, agent: &str, size: f32) {
    if let Some(image) = super::icons::agent_image(agent, size) {
        ui.add(image);
    } else {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            super::icons::TERMINAL,
            mono(size * 0.9),
            theme.agent_color(agent),
        );
    }
}

/// Rounded square with the project's initials (`da`, `rp`).
pub fn project_avatar(
    ui: &mut Ui,
    color: Color32,
    name: &str,
    size: f32,
    letters: usize,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    paint_avatar(ui, rect, color, name, letters);
    response
}

pub fn initials(name: &str, letters: usize) -> String {
    let words: Vec<&str> = name
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_')
        .filter(|w| !w.is_empty())
        .collect();
    let mut out = String::new();
    if words.len() >= 2 && letters >= 2 {
        for w in words.iter().take(letters) {
            out.extend(w.chars().next());
        }
    } else {
        out.extend(name.chars().filter(|c| c.is_alphanumeric()).take(letters));
    }
    if letters == 1 {
        out.to_uppercase()
    } else {
        out.to_lowercase()
    }
}

pub fn paint_avatar(ui: &Ui, rect: Rect, color: Color32, name: &str, letters: usize) {
    let radius = (rect.width() * 0.22).round() as u8;
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(radius),
        color.gamma_multiply(if letters == 1 { 1.0 } else { 0.85 }),
    );
    let text_color = if (color.r() as u32 + color.g() as u32 + color.b() as u32) > 560 {
        Color32::from_rgb(20, 20, 20)
    } else {
        Color32::WHITE
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        initials(name, letters),
        bold(rect.height() * 0.42),
        text_color,
    );
}

/// Bordered card (`.card`), with hover lift. Returns the click response of the whole card.
pub fn card<R>(
    ui: &mut Ui,
    theme: &Theme,
    size: Vec2,
    add: impl FnOnce(&mut Ui) -> R,
) -> (Response, R) {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    ui.painter().rect(
        rect,
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
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(14.0, 10.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let inner = add(&mut child);
    (response, inner)
}

/// Outlined button with an optional leading glyph and trailing shortcut (`novo terminal  Ctrl+T`).
pub fn outline_button(
    ui: &mut Ui,
    theme: &Theme,
    glyph: &str,
    label: &str,
    shortcut: Option<&str>,
    width: Option<f32>,
) -> Response {
    let font = mono(12.5);
    let label_galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font.clone(), theme.fg);
    let shortcut_galley = shortcut.map(|s| {
        ui.painter()
            .layout_no_wrap(s.to_string(), mono(11.0), theme.fg_faint)
    });
    let natural = 12.0
        + if glyph.is_empty() { 0.0 } else { 22.0 }
        + label_galley.size().x
        + shortcut_galley.as_ref().map_or(0.0, |g| g.size().x + 24.0)
        + 12.0;
    let size = Vec2::new(width.unwrap_or(natural).max(natural), 34.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    ui.painter().rect(
        rect,
        CornerRadius::same(6),
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
    let mut x = rect.left() + 12.0;
    if !glyph.is_empty() {
        ui.painter().text(
            Pos2::new(x + 7.0, rect.center().y),
            Align2::CENTER_CENTER,
            glyph,
            mono(14.0),
            theme.fg_muted,
        );
        x += 22.0;
    }
    ui.painter().galley(
        Pos2::new(x, rect.center().y - label_galley.size().y / 2.0),
        label_galley,
        theme.fg,
    );
    if let Some(g) = shortcut_galley {
        let pill = Rect::from_min_size(
            Pos2::new(rect.right() - g.size().x - 22.0, rect.center().y - 10.0),
            Vec2::new(g.size().x + 12.0, 20.0),
        );
        ui.painter().rect(
            pill,
            CornerRadius::same(4),
            theme.bg,
            Stroke::new(1.0, theme.border),
            StrokeKind::Inside,
        );
        ui.painter()
            .galley(pill.center() - g.size() / 2.0, g, theme.fg_faint);
    }
    response
}

/// Filled primary button (`.primaryBtn`).
pub fn primary_button(ui: &mut Ui, theme: &Theme, label: &str) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), bold(13.0), theme.accent_on);
    let size = Vec2::new(galley.size().x + 28.0, 32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.hovered() {
        theme.accent_strong
    } else {
        theme.accent
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, theme.accent_on);
    response
}

pub fn ghost_button(ui: &mut Ui, theme: &Theme, label: &str) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), sans(13.0), theme.fg_muted);
    let size = Vec2::new(galley.size().x + 24.0, 32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), theme.panel_hover);
    }
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(1.0, theme.border),
        StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, theme.fg_muted);
    response
}

/// Single-line input styled like the web `<input>` (sunken, bordered).
pub fn text_input(
    ui: &mut Ui,
    theme: &Theme,
    value: &mut String,
    hint: &str,
    width: f32,
) -> Response {
    egui::Frame::new()
        .fill(theme.bg_sunken)
        .stroke(Stroke::new(1.0, theme.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(value)
                    .hint_text(RichText::new(hint).color(theme.fg_faint))
                    .frame(egui::Frame::NONE)
                    .desired_width(width - 22.0)
                    .font(sans(13.0)),
            )
        })
        .inner
}

pub fn relative_time(lang: crate::i18n::Lang, ts: u64) -> String {
    use crate::i18n::tr;
    if ts == 0 {
        return "—".into();
    }
    let diff = crate::model::now_ms().saturating_sub(ts) / 60_000;
    if diff < 1 {
        return tr(lang, "time.now").into();
    }
    if diff < 60 {
        return format!("{diff}min");
    }
    let hours = diff / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    if days == 1 {
        return tr(lang, "time.yesterday").into();
    }
    format!("{days} {}", tr(lang, "time.days"))
}

pub fn terminal_count_label(lang: crate::i18n::Lang, n: usize) -> String {
    let word = if n == 1 {
        crate::i18n::tr(lang, "home.terminalsOne")
    } else {
        crate::i18n::tr(lang, "home.terminalsMany")
    };
    format!("{n} {word}")
}
