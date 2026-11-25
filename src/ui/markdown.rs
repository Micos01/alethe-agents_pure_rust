//! `MarkdownPane` + `MarkdownRenderer`: a `.md` file as a pane, rendered (GFM: headings, lists,
//! task lists, tables, code, quotes, links) or edited in place, saved back to disk, and reloaded
//! when the file changes outside the app.
//!
//! The file is polled for changes (its mtime, every two seconds while the pane is on screen) rather
//! than watched, so a closed or hidden pane costs nothing.

use std::path::Path;
use std::time::{Duration, SystemTime};

use egui::text::LayoutJob;
use egui::{
    pos2, vec2, Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, Stroke, StrokeKind,
    TextFormat,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use super::icons;
use super::widgets::{self, bold, mono, sans};
use crate::app::{Action, AletheApp};
use crate::theme::Theme;

const HEADER_H: f32 = 34.0;
const POLL: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct MarkdownState {
    path: String,
    content: Option<String>,
    error: Option<String>,
    draft: String,
    editing: bool,
    modified: Option<SystemTime>,
    changed_on_disk: bool,
    last_poll: f64,
    copied_at: Option<f64>,
}

impl MarkdownState {
    fn load(&mut self, path: &str) {
        self.path = path.to_string();
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.content = Some(text);
                self.error = None;
            }
            Err(error) => {
                self.content = None;
                self.error = Some(error.to_string());
            }
        }
        self.modified = mtime(path);
        self.changed_on_disk = false;
    }
}

fn mtime(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

pub fn pane(
    app: &mut AletheApp,
    ui: &mut egui::Ui,
    pane_id: &str,
    title: &str,
    path: &str,
    rect: Rect,
    focused: bool,
) {
    let theme = app.theme.clone();
    let lang = app.lang();
    let t = |key: &str| crate::i18n::tr(lang, key);
    let maximized = app.maximized.as_deref() == Some(pane_id);
    let now = ui.input(|i| i.time);

    let state = app.markdown.entry(pane_id.to_string()).or_default();
    if state.path != path {
        state.load(path);
        state.last_poll = now;
    }
    // Pick up edits made outside the app. While editing, only flag them: reloading would drop the
    // draft.
    if now - state.last_poll >= POLL.as_secs_f64() {
        state.last_poll = now;
        let current = mtime(path);
        if current != state.modified {
            if state.editing {
                state.changed_on_disk = true;
            } else {
                state.load(path);
            }
        }
    }
    ui.ctx().request_repaint_after(POLL);

    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        theme.bg_sunken,
        Stroke::NONE,
        StrokeKind::Inside,
    );
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H));
    let body = Rect::from_min_max(
        pos2(rect.left() + 1.0, header.bottom()),
        pos2(rect.right() - 1.0, rect.bottom() - 1.0),
    );

    // ---- header
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
            .max_rect(header.shrink2(vec2(10.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    hui.set_clip_rect(header);
    hui.label(
        RichText::new(icons::MARKDOWN)
            .font(mono(14.0))
            .color(theme.accent),
    );
    hui.label(RichText::new(title).font(bold(12.5)).color(theme.fg));
    let dirty = state.editing && state.content.as_deref() != Some(state.draft.as_str());
    if dirty {
        hui.label(
            RichText::new(format!("● {}", t("markdown.unsaved")))
                .font(mono(10.5))
                .color(theme.waiting),
        );
    }
    let mut actions: Vec<Action> = Vec::new();
    let mut save = false;
    let mut toggle_edit = false;
    let mut reload = false;
    let mut copy = false;
    hui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if widgets::icon_button(ui, &theme, icons::CLOSE, t("pane.close"), 22.0).clicked() {
            actions.push(Action::CloseTerminal(pane_id.to_string()));
        }
        let (glyph, tip) = if maximized {
            (icons::SCREEN_NORMAL, t("pane.restore"))
        } else {
            (icons::SCREEN_FULL, t("pane.maximize"))
        };
        if widgets::icon_button(ui, &theme, glyph, tip, 22.0).clicked() {
            actions.push(Action::ToggleMaximize(pane_id.to_string()));
        }
        if widgets::icon_button(
            ui,
            &theme,
            icons::FOLDER_OPENED,
            t("markdown.openFolder"),
            22.0,
        )
        .clicked()
        {
            if let Some(dir) = Path::new(path).parent() {
                actions.push(Action::OpenFolder(dir.to_string_lossy().to_string()));
            }
        }
        if state.editing {
            if widgets::icon_button(ui, &theme, icons::PREVIEW, t("markdown.preview"), 22.0)
                .clicked()
            {
                toggle_edit = true;
            }
            if widgets::icon_button(ui, &theme, icons::SAVE, t("markdown.save"), 22.0).clicked() {
                save = true;
            }
        } else if widgets::icon_button(ui, &theme, icons::EDIT, t("markdown.edit"), 22.0).clicked()
        {
            toggle_edit = true;
        }
        let copied = state.copied_at.is_some_and(|at| now - at < 1.5);
        if widgets::icon_button(
            ui,
            &theme,
            if copied { icons::CHECK } else { icons::COPY },
            t("markdown.copy"),
            22.0,
        )
        .clicked()
        {
            copy = true;
        }
        if widgets::icon_button(ui, &theme, icons::REFRESH, t("markdown.reload"), 22.0).clicked() {
            reload = true;
        }
        ui.add(
            egui::Label::new(RichText::new(path).font(mono(10.5)).color(theme.fg_faint)).truncate(),
        )
        .on_hover_text(path);
    });

    // Ctrl+S saves while editing a focused pane.
    if state.editing
        && focused
        && ui.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::CTRL,
                egui::Key::S,
            ))
        })
    {
        save = true;
    }

    if reload {
        state.load(path);
        state.editing = false;
    }
    if copy {
        let text = if state.editing {
            state.draft.clone()
        } else {
            state.content.clone().unwrap_or_default()
        };
        ui.ctx().copy_text(text);
        state.copied_at = Some(now);
    }
    if toggle_edit {
        if state.editing {
            state.editing = false;
        } else {
            state.draft = state.content.clone().unwrap_or_default();
            state.editing = true;
        }
    }
    if save {
        match std::fs::write(path, &state.draft) {
            Ok(()) => {
                state.content = Some(state.draft.clone());
                state.modified = mtime(path);
                state.changed_on_disk = false;
                state.error = None;
            }
            Err(error) => state.error = Some(error.to_string()),
        }
    }

    // ---- body
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body.shrink2(vec2(18.0, 10.0)))
            .layout(Layout::top_down(Align::Min)),
    );
    child.set_clip_rect(body);
    if state.changed_on_disk {
        child.label(
            RichText::new(format!(
                "{} {}",
                icons::WARNING,
                t("markdown.changedOnDisk")
            ))
            .font(sans(12.0))
            .color(theme.waiting),
        );
        child.add_space(4.0);
    }
    if let Some(error) = &state.error {
        child.label(
            RichText::new(format!("{}: {error}", t("markdown.missing")))
                .font(sans(12.5))
                .color(theme.offline),
        );
    }
    if state.editing {
        egui::ScrollArea::vertical()
            .id_salt(("md-edit", pane_id))
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.add_sized(
                    vec2(ui.available_width(), ui.available_height().max(200.0)),
                    egui::TextEdit::multiline(&mut state.draft)
                        .font(FontId::monospace(12.5))
                        .code_editor()
                        .frame(egui::Frame::NONE),
                );
            });
    } else if let Some(content) = &state.content {
        egui::ScrollArea::vertical()
            .id_salt(("md-view", pane_id))
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.set_max_width(ui.available_width().min(860.0));
                render(ui, &theme, content);
                ui.add_space(24.0);
            });
    }

    for action in actions {
        app.act(action);
    }
    if child.ui_contains_pointer() && ui.input(|i| i.pointer.primary_clicked()) && !focused {
        app.act(Action::FocusTerminal(pane_id.to_string()));
    }
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(
            1.0,
            if focused {
                theme.accent_border
            } else {
                theme.border
            },
        ),
        StrokeKind::Inside,
    );
}

// ----------------------------------------------------------------- renderer

struct Inline {
    bold: u32,
    italic: u32,
    strike: u32,
    code: bool,
    link: Option<String>,
    heading: Option<HeadingLevel>,
    quote: u32,
}

struct Renderer<'a> {
    theme: &'a Theme,
    job: LayoutJob,
    inline: Inline,
    lists: Vec<Option<u64>>,
    pending_item: Option<String>,
    code: Option<(String, String)>,
    table: Option<Table>,
}

#[derive(Default)]
struct Table {
    rows: Vec<Vec<LayoutJob>>,
    head_rows: usize,
    in_head: bool,
}

/// Renders GitHub-flavoured markdown into `ui`.
pub fn render(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let mut r = Renderer {
        theme,
        job: LayoutJob::default(),
        inline: Inline {
            bold: 0,
            italic: 0,
            strike: 0,
            code: false,
            link: None,
            heading: None,
            quote: 0,
        },
        lists: Vec::new(),
        pending_item: None,
        code: None,
        table: None,
    };
    ui.spacing_mut().item_spacing.y = 4.0;
    for event in Parser::new_ext(text, options) {
        r.event(ui, event);
    }
    r.flush(ui);
}

impl Renderer<'_> {
    fn base_size(&self) -> f32 {
        match self.inline.heading {
            Some(HeadingLevel::H1) => 22.0,
            Some(HeadingLevel::H2) => 18.5,
            Some(HeadingLevel::H3) => 16.0,
            Some(_) => 14.0,
            None => 13.5,
        }
    }

    fn format(&self) -> TextFormat {
        let size = self.base_size();
        let theme = self.theme;
        let strong = self.inline.bold > 0 || self.inline.heading.is_some();
        let font = if self.inline.code {
            FontId::monospace(size - 1.0)
        } else if strong {
            bold(size)
        } else {
            sans(size)
        };
        let mut color = if self.inline.quote > 0 {
            theme.fg_muted
        } else {
            theme.fg
        };
        if self.inline.link.is_some() {
            color = theme.accent;
        }
        TextFormat {
            font_id: font,
            color,
            italics: self.inline.italic > 0,
            background: if self.inline.code {
                theme.panel
            } else {
                Color32::TRANSPARENT
            },
            strikethrough: if self.inline.strike > 0 {
                Stroke::new(1.0, color)
            } else {
                Stroke::NONE
            },
            underline: if self.inline.link.is_some() {
                Stroke::new(1.0, color.gamma_multiply(0.5))
            } else {
                Stroke::NONE
            },
            ..Default::default()
        }
    }

    fn push_text(&mut self, text: &str) {
        if let Some(prefix) = self.pending_item.take() {
            let format = TextFormat::simple(sans(13.5), self.theme.fg_muted);
            self.job.append(&prefix, 0.0, format);
        }
        let format = self.format();
        self.job.append(text, 0.0, format);
    }

    fn flush(&mut self, ui: &mut egui::Ui) {
        if self.job.text.is_empty() {
            return;
        }
        let mut job = std::mem::take(&mut self.job);
        let indent =
            self.lists.len().saturating_sub(1) as f32 * 18.0 + self.inline.quote as f32 * 14.0;
        job.wrap.max_width =
            (ui.available_width() - indent - if self.lists.is_empty() { 0.0 } else { 18.0 })
                .max(80.0);
        if let Some(table) = &mut self.table {
            if let Some(row) = table.rows.last_mut() {
                row.push(job);
            }
            return;
        }
        ui.horizontal(|ui| {
            if indent > 0.0 {
                ui.add_space(indent);
            }
            if self.inline.quote > 0 {
                let (r, _) = ui.allocate_exact_size(vec2(3.0, 18.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(r, CornerRadius::same(1), self.theme.border_strong);
            }
            ui.add(egui::Label::new(job).wrap());
        });
    }

    fn event(&mut self, ui: &mut egui::Ui, event: Event) {
        match event {
            Event::Start(tag) => self.start(ui, tag),
            Event::End(tag) => self.end(ui, tag),
            Event::Text(text) => {
                if let Some((_, code)) = &mut self.code {
                    code.push_str(&text);
                } else {
                    self.push_text(&text);
                }
            }
            Event::Code(text) => {
                self.inline.code = true;
                self.push_text(&text);
                self.inline.code = false;
            }
            Event::SoftBreak => self.push_text(" "),
            Event::HardBreak => self.push_text("\n"),
            Event::Rule => {
                self.flush(ui);
                ui.add_space(4.0);
                let (r, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(r, CornerRadius::ZERO, self.theme.border);
                ui.add_space(4.0);
            }
            Event::TaskListMarker(done) => {
                self.pending_item = None;
                let format = TextFormat::simple(
                    mono(12.5),
                    if done {
                        self.theme.working
                    } else {
                        self.theme.fg_faint
                    },
                );
                self.job
                    .append(if done { "\u{f0132} " } else { "\u{f0131} " }, 0.0, format);
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                let format = TextFormat::simple(FontId::monospace(12.0), self.theme.fg_faint);
                self.job.append(&html, 0.0, format);
            }
            Event::FootnoteReference(name) => self.push_text(&format!("[{name}]")),
            _ => {}
        }
    }

    fn start(&mut self, ui: &mut egui::Ui, tag: Tag) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush(ui);
                ui.add_space(if level == HeadingLevel::H1 { 10.0 } else { 6.0 });
                self.inline.heading = Some(level);
            }
            Tag::BlockQuote(_) => {
                self.flush(ui);
                self.inline.quote += 1;
            }
            Tag::CodeBlock(kind) => {
                self.flush(ui);
                let lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(start) => {
                self.flush(ui);
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush(ui);
                let bullet = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let s = format!("{n}. ");
                        *n += 1;
                        s
                    }
                    _ => "•  ".to_string(),
                };
                self.pending_item = Some(bullet);
            }
            Tag::Emphasis => self.inline.italic += 1,
            Tag::Strong => self.inline.bold += 1,
            Tag::Strikethrough => self.inline.strike += 1,
            Tag::Link { dest_url, .. } => self.inline.link = Some(dest_url.to_string()),
            Tag::Image { dest_url, .. } => self.push_text(&format!("[{dest_url}]")),
            Tag::Table(_) => {
                self.flush(ui);
                self.table = Some(Table::default());
            }
            Tag::TableHead => {
                if let Some(table) = &mut self.table {
                    table.in_head = true;
                    table.rows.push(Vec::new());
                }
                self.inline.bold += 1;
            }
            Tag::TableRow => {
                if let Some(table) = &mut self.table {
                    table.rows.push(Vec::new());
                }
            }
            Tag::TableCell => {}
            _ => {}
        }
    }

    fn end(&mut self, ui: &mut egui::Ui, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush(ui);
                if self.lists.is_empty() {
                    ui.add_space(4.0);
                }
            }
            TagEnd::Heading(level) => {
                self.flush(ui);
                self.inline.heading = None;
                if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
                    let (r, _) = ui
                        .allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(r, CornerRadius::ZERO, self.theme.border);
                }
                ui.add_space(4.0);
            }
            TagEnd::BlockQuote(_) => {
                self.flush(ui);
                self.inline.quote = self.inline.quote.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                if let Some((lang, code)) = self.code.take() {
                    code_block(ui, self.theme, &lang, code.trim_end_matches('\n'));
                }
            }
            TagEnd::List(_) => {
                self.flush(ui);
                self.lists.pop();
                if self.lists.is_empty() {
                    ui.add_space(4.0);
                }
            }
            TagEnd::Item => self.flush(ui),
            TagEnd::Emphasis => self.inline.italic = self.inline.italic.saturating_sub(1),
            TagEnd::Strong => self.inline.bold = self.inline.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.inline.strike = self.inline.strike.saturating_sub(1),
            TagEnd::Link => self.inline.link = None,
            TagEnd::TableCell => self.flush(ui),
            TagEnd::TableHead => {
                self.inline.bold = self.inline.bold.saturating_sub(1);
                if let Some(table) = &mut self.table {
                    table.in_head = false;
                    table.head_rows = table.rows.len();
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    table_view(ui, self.theme, table);
                }
            }
            _ => {}
        }
    }
}

fn code_block(ui: &mut egui::Ui, theme: &Theme, lang: &str, code: &str) {
    egui::Frame::new()
        .fill(theme.bg_elevated)
        .stroke(Stroke::new(1.0, theme.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !lang.is_empty() {
                ui.label(RichText::new(lang).font(mono(10.0)).color(theme.fg_faint));
            }
            egui::ScrollArea::horizontal()
                .id_salt(code.len() ^ lang.len())
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(code)
                                .font(FontId::monospace(12.0))
                                .color(theme.fg),
                        )
                        .extend(),
                    );
                });
        });
    ui.add_space(4.0);
}

fn table_view(ui: &mut egui::Ui, theme: &Theme, table: Table) {
    let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 {
        return;
    }
    egui::Frame::new()
        .stroke(Stroke::new(1.0, theme.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            egui::Grid::new(ui.next_auto_id())
                .striped(true)
                .spacing(vec2(16.0, 6.0))
                .show(ui, |ui| {
                    for row in table.rows {
                        for mut cell in row {
                            cell.wrap.max_width = f32::INFINITY;
                            ui.add(egui::Label::new(cell).extend());
                        }
                        ui.end_row();
                    }
                });
        });
    ui.add_space(4.0);
}
