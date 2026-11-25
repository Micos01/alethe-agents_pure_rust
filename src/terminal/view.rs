//! Paints a `TerminalSession` and routes keyboard/mouse input to it (the xterm.js view).

use super::input::{key_to_bytes, mouse_report};
use super::{TermSize, TerminalSession};
use crate::theme::TermPalette;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use egui::{
    Align2, Color32, CornerRadius, Event, EventFilter, FontFamily, FontId, MouseWheelUnit,
    PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};

pub const PADDING: f32 = 6.0;

pub fn mono_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn bold_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("mono-bold".into()))
}

#[derive(Default)]
pub struct ViewOutput {
    pub focused: bool,
    pub clicked: bool,
}

/// Per-view state kept in egui memory.
#[derive(Clone, Copy, Default)]
struct DragState {
    selecting: bool,
    mouse_button_down: Option<u8>,
    wheel_accum: f32,
}

fn c32(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn indexed_color(i: u8, palette: &TermPalette) -> Color32 {
    match i {
        0..=15 => palette.ansi[i as usize].c(),
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            c32(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            c32(v, v, v)
        }
    }
}

fn resolve(color: Color, colors: &Colors, palette: &TermPalette, bold: bool) -> Color32 {
    match color {
        Color::Spec(rgb) => c32(rgb.r, rgb.g, rgb.b),
        Color::Indexed(i) => {
            let i = if bold && i < 8 { i + 8 } else { i };
            match colors[i as usize] {
                Some(rgb) => c32(rgb.r, rgb.g, rgb.b),
                None => indexed_color(i, palette),
            }
        }
        Color::Named(named) => {
            let idx = named as usize;
            if let Some(rgb) = colors[idx] {
                return c32(rgb.r, rgb.g, rgb.b);
            }
            match named {
                NamedColor::Foreground | NamedColor::BrightForeground => palette.foreground.c(),
                NamedColor::DimForeground => palette.foreground.c().gamma_multiply(0.66),
                NamedColor::Background => palette.background.c(),
                NamedColor::Cursor => palette.cursor.c(),
                _ if idx < 16 => {
                    let idx = if bold && idx < 8 { idx + 8 } else { idx };
                    palette.ansi[idx].c()
                }
                _ => {
                    // Dim variants: DimBlack (259) ..= DimWhite (266).
                    let base = idx.saturating_sub(NamedColor::DimBlack as usize).min(7);
                    let c = palette.ansi[base].c();
                    c32(
                        (c.r() as f32 * 0.66) as u8,
                        (c.g() as f32 * 0.66) as u8,
                        (c.b() as f32 * 0.66) as u8,
                    )
                }
            }
        }
    }
}

pub fn cell_metrics(ctx: &egui::Context, font_size: f32) -> (f32, f32) {
    let font = mono_font(font_size);
    ctx.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font).ceil()))
}

/// Renders the session in `rect` and handles input. `focus` asks for keyboard focus.
pub fn show(
    ui: &mut egui::Ui,
    rect: Rect,
    id: egui::Id,
    session: &TerminalSession,
    palette: &TermPalette,
    font_size: f32,
    focus: bool,
) -> ViewOutput {
    let ctx = ui.ctx().clone();
    let response = ui.interact(rect, id, Sense::click_and_drag());
    if focus || response.clicked() || response.drag_started() {
        response.request_focus();
    }
    let has_focus = response.has_focus();
    if has_focus {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
    }

    let (cell_w, cell_h) = cell_metrics(&ctx, font_size);
    let inner = rect.shrink(PADDING);
    let cols = ((inner.width() / cell_w).floor() as usize).max(2);
    let lines = ((inner.height() / cell_h).floor() as usize).max(1);
    session.resize(TermSize { cols, lines }, cell_w, cell_h);

    let mut drag: DragState = ui.data(|d| d.get_temp(id).unwrap_or_default());
    let mut out = ViewOutput {
        focused: has_focus,
        clicked: response.clicked(),
    };

    // ---------- input ----------
    let cell_at = |pos: Pos2, display_offset: usize| -> (Point, Side, usize, usize) {
        let x = ((pos.x - inner.left()) / cell_w).max(0.0);
        let y = ((pos.y - inner.top()) / cell_h).max(0.0);
        let col = (x.floor() as usize).min(cols - 1);
        let line = (y.floor() as usize).min(lines - 1);
        let side = if x.fract() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        (
            Point::new(Line(line as i32 - display_offset as i32), Column(col)),
            side,
            col,
            line,
        )
    };

    let events = if has_focus || response.hovered() {
        ui.input(|i| i.events.clone())
    } else {
        Vec::new()
    };
    let mut to_write: Vec<u8> = Vec::new();
    let mut paste: Option<String> = None;
    let mut copy_selection = false;
    let mut scroll_lines: i32 = 0;
    let mut suppress_text = false;
    {
        let mut term = session.term.lock().unwrap();
        let mode = *term.mode();
        let display_offset = term.grid().display_offset();
        let mouse_mode = mode.intersects(TermMode::MOUSE_MODE);

        for event in &events {
            match event {
                Event::Text(text) if has_focus => {
                    if !suppress_text {
                        to_write.extend_from_slice(text.as_bytes());
                    }
                    suppress_text = false;
                }
                Event::Ime(egui::ImeEvent::Commit(text)) if has_focus => {
                    to_write.extend_from_slice(text.as_bytes())
                }
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if has_focus => {
                    if modifiers.shift
                        && !modifiers.ctrl
                        && matches!(key, egui::Key::PageUp | egui::Key::PageDown)
                        && !mode.contains(TermMode::ALT_SCREEN)
                    {
                        let page = lines as i32 - 1;
                        scroll_lines += if *key == egui::Key::PageUp {
                            page
                        } else {
                            -page
                        };
                        continue;
                    }
                    if modifiers.ctrl && modifiers.shift && *key == egui::Key::C {
                        copy_selection = true;
                        continue;
                    }
                    if let Some(bytes) = key_to_bytes(*key, modifiers, mode) {
                        suppress_text = modifiers.alt && !modifiers.ctrl;
                        to_write.extend_from_slice(&bytes);
                    }
                }
                Event::Copy if has_focus => {
                    if term.selection.as_ref().is_some_and(|s| !s.is_empty()) {
                        copy_selection = true;
                    } else {
                        to_write.push(0x03);
                    }
                }
                Event::Cut if has_focus => to_write.push(0x18),
                Event::Paste(text) if has_focus => paste = Some(text.clone()),
                // Let the agent read the image from the clipboard itself (Claude Code: Ctrl+V).
                Event::PasteImage(_) if has_focus => to_write.push(0x16),
                Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } if response.hovered() => {
                    let step = match unit {
                        MouseWheelUnit::Point => delta.y / cell_h,
                        MouseWheelUnit::Line => delta.y,
                        MouseWheelUnit::Page => delta.y * lines as f32,
                    };
                    drag.wheel_accum += step;
                    let whole = drag.wheel_accum.trunc() as i32;
                    if whole == 0 {
                        continue;
                    }
                    drag.wheel_accum -= whole as f32;
                    if mouse_mode && !modifiers.shift {
                        if let Some(pos) = response.hover_pos() {
                            let (_, _, col, line) = cell_at(pos, display_offset);
                            let button = if whole > 0 { 64 } else { 65 };
                            for _ in 0..whole.abs() {
                                if let Some(b) =
                                    mouse_report(mode, button, true, col, line, modifiers)
                                {
                                    to_write.extend_from_slice(&b);
                                }
                            }
                        }
                    } else if mode.contains(TermMode::ALT_SCREEN)
                        && mode.contains(TermMode::ALTERNATE_SCROLL)
                    {
                        let seq: &[u8] = if whole > 0 { b"\x1bOA" } else { b"\x1bOB" };
                        for _ in 0..whole.abs() * 3 {
                            to_write.extend_from_slice(seq);
                        }
                    } else {
                        scroll_lines += whole * 3;
                    }
                }
                _ => {}
            }
        }

        // Mouse: selection, or reports when the application grabbed the mouse.
        let modifiers = ui.input(|i| i.modifiers);
        let report_mouse = mouse_mode && !modifiers.shift;
        if let Some(pos) = response.interact_pointer_pos().or(response.hover_pos()) {
            let (point, side, col, line) = cell_at(pos, display_offset);
            if report_mouse {
                let pressed = ui.input(|i| i.pointer.button_pressed(PointerButton::Primary));
                let released = ui.input(|i| i.pointer.button_released(PointerButton::Primary));
                if pressed && response.hovered() {
                    if let Some(b) = mouse_report(mode, 0, true, col, line, &modifiers) {
                        to_write.extend_from_slice(&b);
                    }
                    drag.mouse_button_down = Some(0);
                } else if released && drag.mouse_button_down.is_some() {
                    if let Some(b) = mouse_report(mode, 0, false, col, line, &modifiers) {
                        to_write.extend_from_slice(&b);
                    }
                    drag.mouse_button_down = None;
                } else if drag.mouse_button_down.is_some()
                    && response.dragged()
                    && mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
                {
                    if let Some(b) = mouse_report(mode, 32, true, col, line, &modifiers) {
                        to_write.extend_from_slice(&b);
                    }
                }
            } else {
                if response.double_clicked() {
                    term.selection = Some(Selection::new(SelectionType::Semantic, point, side));
                    if let Some(sel) = term.selection.as_mut() {
                        sel.update(point, side);
                    }
                } else if response.triple_clicked() {
                    term.selection = Some(Selection::new(SelectionType::Lines, point, side));
                } else if response.drag_started_by(PointerButton::Primary) {
                    term.selection = Some(Selection::new(SelectionType::Simple, point, side));
                    drag.selecting = true;
                } else if drag.selecting && response.dragged_by(PointerButton::Primary) {
                    if let Some(sel) = term.selection.as_mut() {
                        sel.update(point, side);
                    }
                    // Auto-scroll while dragging past the edges.
                    if pos.y < inner.top() {
                        scroll_lines += 1;
                    } else if pos.y > inner.bottom() {
                        scroll_lines -= 1;
                    }
                } else if response.clicked() {
                    term.selection = None;
                }
                if response.drag_stopped() {
                    drag.selecting = false;
                }
            }
        }

        if copy_selection {
            if let Some(text) = term.selection_to_string().filter(|t| !t.is_empty()) {
                ctx.copy_text(text);
            }
        }
        if scroll_lines != 0 {
            term.scroll_display(Scroll::Delta(scroll_lines));
        }
        if !to_write.is_empty() || paste.is_some() {
            term.scroll_display(Scroll::Bottom);
            if !to_write.is_empty() {
                term.selection = None;
            }
        }
    }

    // Context menu.
    response.context_menu(|ui| {
        let lang = crate::i18n::Lang::from_code(&ui.ctx().data(|d| {
            d.get_temp::<String>(egui::Id::unique("lang"))
                .unwrap_or_default()
        }));
        if ui.button(crate::i18n::tr(lang, "pane.copy")).clicked() {
            if let Some(text) = session.term.lock().unwrap().selection_to_string() {
                ui.ctx().copy_text(text);
            }
            ui.close();
        }
        if ui.button(crate::i18n::tr(lang, "pane.paste")).clicked() {
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                if let Ok(text) = clipboard.get_text() {
                    session.paste(&text);
                }
            }
            ui.close();
        }
        if ui.button(crate::i18n::tr(lang, "pane.clear")).clicked() {
            // Ctrl+L: every shell and agent TUI redraws on it.
            session.write(b"\x0c");
            ui.close();
        }
    });

    if !to_write.is_empty() {
        session.write(&to_write);
    }
    if let Some(text) = paste {
        session.paste(&text);
    }
    if let Some(text) = session.take_clipboard_store() {
        ctx.copy_text(text);
    }
    ui.data_mut(|d| d.insert_temp(id, drag));
    if response.clicked() {
        out.clicked = true;
    }

    paint(
        ui, rect, inner, session, palette, font_size, cell_w, cell_h, has_focus,
    );
    out
}

#[allow(clippy::too_many_arguments)]
fn paint(
    ui: &egui::Ui,
    rect: Rect,
    inner: Rect,
    session: &TerminalSession,
    palette: &TermPalette,
    font_size: f32,
    cell_w: f32,
    cell_h: f32,
    focused: bool,
) {
    let painter = ui.painter_at(rect);
    let bg_default = palette.background.c();
    painter.rect_filled(rect, CornerRadius::ZERO, bg_default);

    let regular = mono_font(font_size);
    let bold = bold_font(font_size);
    let selection_bg = palette.selection_background.c();

    let term = session.term.lock().unwrap();
    let content = term.renderable_content();
    let display_offset = content.display_offset as i32;
    let colors = content.colors;
    let selection = content.selection;
    let cursor = content.cursor;
    let screen_lines = term.screen_lines();

    struct Run {
        col: usize,
        text: String,
        fg: Color32,
        bold: bool,
        underline: bool,
        strike: bool,
    }
    let mut runs: Vec<(usize, Run)> = Vec::new();
    let mut current: Option<(usize, Run)> = None;
    let flush = |current: &mut Option<(usize, Run)>, runs: &mut Vec<(usize, Run)>| {
        if let Some(run) = current.take() {
            if !run.1.text.trim().is_empty() || run.1.underline || run.1.strike {
                runs.push(run);
            }
        }
    };

    for indexed in content.display_iter {
        let row = indexed.point.line.0 + display_offset;
        if row < 0 || row as usize >= screen_lines {
            continue;
        }
        let row = row as usize;
        let col = indexed.point.column.0;
        let cell = indexed.cell;
        let flags = cell.flags;
        if flags.contains(Flags::WIDE_CHAR_SPACER)
            || flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        let is_bold = flags.contains(Flags::BOLD);
        let mut fg = resolve(cell.fg, colors, palette, is_bold);
        let mut bg = resolve(cell.bg, colors, palette, false);
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::DIM) {
            fg = fg.gamma_multiply(0.66);
        }
        if flags.contains(Flags::HIDDEN) {
            fg = bg;
        }
        let selected = selection.is_some_and(|s| s.contains(indexed.point));
        if selected {
            bg = selection_bg;
        }
        let width_cells = if flags.contains(Flags::WIDE_CHAR) {
            2.0
        } else {
            1.0
        };
        let x = inner.left() + col as f32 * cell_w;
        let y = inner.top() + row as f32 * cell_h;
        if bg != bg_default || selected {
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(x, y), Vec2::new(cell_w * width_cells, cell_h)),
                CornerRadius::ZERO,
                bg,
            );
        }

        let c = cell.c;
        let underline = flags.intersects(Flags::ALL_UNDERLINES);
        let strike = flags.contains(Flags::STRIKEOUT);
        let simple = c.is_ascii() && !flags.contains(Flags::WIDE_CHAR);
        let continues = current.as_ref().is_some_and(|(r, run)| {
            *r == row
                && simple
                && run.col + run.text.chars().count() == col
                && run.fg == fg
                && run.bold == is_bold
                && run.underline == underline
                && run.strike == strike
        });
        if continues {
            current.as_mut().unwrap().1.text.push(c);
            continue;
        }
        flush(&mut current, &mut runs);
        let mut text = String::new();
        text.push(c);
        if let Some(extra) = cell.zerowidth() {
            text.extend(extra.iter());
        }
        let run = Run {
            col,
            text,
            fg,
            bold: is_bold,
            underline,
            strike,
        };
        if simple {
            current = Some((row, run));
        } else {
            runs.push((row, run));
        }
    }
    flush(&mut current, &mut runs);

    for (row, run) in &runs {
        let pos = Pos2::new(
            inner.left() + run.col as f32 * cell_w,
            inner.top() + *row as f32 * cell_h,
        );
        let font = if run.bold {
            bold.clone()
        } else {
            regular.clone()
        };
        painter.text(pos, Align2::LEFT_TOP, &run.text, font, run.fg);
        let width = run.text.chars().count() as f32 * cell_w;
        if run.underline {
            let y = pos.y + cell_h - 1.5;
            painter.line_segment(
                [Pos2::new(pos.x, y), Pos2::new(pos.x + width, y)],
                Stroke::new(1.0, run.fg),
            );
        }
        if run.strike {
            let y = pos.y + cell_h * 0.5;
            painter.line_segment(
                [Pos2::new(pos.x, y), Pos2::new(pos.x + width, y)],
                Stroke::new(1.0, run.fg),
            );
        }
    }

    // Cursor.
    let cursor_row = cursor.point.line.0 + display_offset;
    if cursor.shape != CursorShape::Hidden
        && cursor_row >= 0
        && (cursor_row as usize) < screen_lines
    {
        let x = inner.left() + cursor.point.column.0 as f32 * cell_w;
        let y = inner.top() + cursor_row as f32 * cell_h;
        let cell_rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(cell_w, cell_h));
        let color = palette.cursor.c();
        if !focused {
            painter.rect_stroke(
                cell_rect.shrink(0.5),
                CornerRadius::ZERO,
                Stroke::new(1.0, color),
                StrokeKind::Inside,
            );
        } else {
            match cursor.shape {
                CursorShape::Beam => {
                    painter.rect_filled(
                        Rect::from_min_size(cell_rect.min, Vec2::new(2.0, cell_h)),
                        CornerRadius::ZERO,
                        color,
                    );
                }
                CursorShape::Underline => {
                    painter.rect_filled(
                        Rect::from_min_size(Pos2::new(x, y + cell_h - 2.0), Vec2::new(cell_w, 2.0)),
                        CornerRadius::ZERO,
                        color,
                    );
                }
                CursorShape::HollowBlock => {
                    painter.rect_stroke(
                        cell_rect.shrink(0.5),
                        CornerRadius::ZERO,
                        Stroke::new(1.0, color),
                        StrokeKind::Inside,
                    );
                }
                _ => {
                    painter.rect_filled(cell_rect, CornerRadius::ZERO, color);
                    let grid = term.grid();
                    let under = &grid[cursor.point];
                    if under.c != ' ' {
                        painter.text(
                            cell_rect.min,
                            Align2::LEFT_TOP,
                            under.c,
                            regular.clone(),
                            palette.background.c(),
                        );
                    }
                }
            }
        }
    }

    // Scrollback indicator.
    if display_offset > 0 {
        let history = term.grid().history_size().max(1) as f32;
        let track = rect.shrink2(Vec2::new(2.0, 4.0));
        let visible = screen_lines as f32 / (history + screen_lines as f32);
        let h = (track.height() * visible).max(18.0);
        let progress = 1.0 - display_offset as f32 / history;
        let top = track.top() + (track.height() - h) * progress;
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(track.right() - 4.0, top), Vec2::new(4.0, h)),
            CornerRadius::same(2),
            palette.foreground.c().gamma_multiply(0.35),
        );
    }
}
