//! Image pane (`ImagePane`): a picture in the workspace, drawn as an egui texture. Kept light: the
//! file is decoded off the UI thread, scaled down to at most `MAX_SIDE` before it reaches the GPU,
//! the decoded copy is dropped once uploaded, and the texture is released whenever the pane is not
//! on screen (`evict`), so an image nobody is looking at costs nothing.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use egui::{
    pos2, vec2, Align, Align2, Color32, CornerRadius, Layout, Rect, RichText, Sense, Stroke,
    StrokeKind, Vec2,
};

use super::icons;
use super::widgets::{self, bold, mono, sans};
use crate::app::{Action, AletheApp};

const HEADER_H: f32 = 34.0;
const POLL: Duration = Duration::from_secs(2);
/// Longest side uploaded to the GPU: enough for a maximized pane, ~16 MB at most.
const MAX_SIDE: u32 = 2048;
/// A pane unseen this long gives its texture back.
const EVICT_AFTER: f64 = 3.0;
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 16.0;
const ZOOM_STEP: f32 = 1.25;

pub const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

struct Decoded {
    image: egui::ColorImage,
    /// The file's own size, before scaling down.
    original: [u32; 2],
}

type Pending = Arc<Mutex<Option<Result<Decoded, String>>>>;

#[derive(Default)]
pub struct ImageState {
    path: String,
    modified: Option<SystemTime>,
    file_bytes: u64,
    pending: Option<Pending>,
    texture: Option<egui::TextureHandle>,
    original: [u32; 2],
    error: Option<String>,
    /// None fits the image to the pane; Some is a fixed scale over the file's own pixels.
    zoom: Option<f32>,
    offset: Vec2,
    last_poll: f64,
    last_seen: f64,
}

impl ImageState {
    fn load(&mut self, ctx: &egui::Context, path: &str) {
        self.path = path.to_string();
        self.modified = mtime(path);
        self.file_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        self.error = None;
        let slot: Pending = Arc::default();
        self.pending = Some(Arc::clone(&slot));
        let path = path.to_string();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            *slot.lock().unwrap() = Some(decode(&path));
            ctx.request_repaint();
        });
    }

    /// Lets the texture go; the next time the pane is drawn the file is decoded again.
    fn release(&mut self) {
        self.texture = None;
        self.pending = None;
    }
}

fn decode(path: &str) -> Result<Decoded, String> {
    let image = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;
    let original = [image.width(), image.height()];
    let image = if original[0].max(original[1]) > MAX_SIDE {
        image.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let rgba = image.into_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(Decoded {
        image: egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
        original,
    })
}

fn mtime(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn format_bytes(bytes: u64) -> String {
    match bytes {
        b if b >= 1024 * 1024 => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
        b if b >= 1024 => format!("{} KB", b / 1024),
        b => format!("{b} B"),
    }
}

/// Frees the textures of image panes that were not drawn lately (another project, a hidden tab).
pub fn evict(app: &mut AletheApp, now: f64) {
    for state in app.images.values_mut() {
        if state.texture.is_some() && now - state.last_seen > EVICT_AFTER {
            state.release();
        }
    }
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
    let ctx = ui.ctx().clone();

    let state = app.images.entry(pane_id.to_string()).or_default();
    state.last_seen = now;
    if state.path != path
        || state.texture.is_none() && state.pending.is_none() && state.error.is_none()
    {
        state.load(&ctx, path);
        state.last_poll = now;
    }
    // A screenshot an agent rewrites shows up without a reload.
    if now - state.last_poll >= POLL.as_secs_f64() {
        state.last_poll = now;
        if mtime(path) != state.modified {
            state.load(&ctx, path);
        }
    }
    ui.ctx().request_repaint_after(POLL);
    let finished = state
        .pending
        .as_ref()
        .and_then(|slot| slot.lock().unwrap().take());
    if let Some(result) = finished {
        state.pending = None;
        match result {
            Ok(decoded) => {
                state.original = decoded.original;
                // Uploading moves the pixels to the GPU; no copy stays here.
                state.texture = Some(ctx.load_texture(
                    format!("image-pane-{pane_id}"),
                    decoded.image,
                    egui::TextureOptions::LINEAR,
                ));
            }
            Err(error) => {
                state.texture = None;
                state.error = Some(error);
            }
        }
    }

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
    let size = vec2(state.original[0] as f32, state.original[1] as f32);
    let fit_scale = if size.x > 0.0 && size.y > 0.0 {
        let room = body.shrink(16.0).size();
        (room.x / size.x).min(room.y / size.y).min(1.0)
    } else {
        1.0
    };
    let scale = state.zoom.unwrap_or(fit_scale);

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
        RichText::new(icons::FILE_MEDIA)
            .font(mono(14.0))
            .color(theme.accent),
    );
    hui.label(RichText::new(title).font(bold(12.5)).color(theme.fg));
    if state.original[0] > 0 {
        hui.label(
            RichText::new(format!(
                "{}×{} · {}",
                state.original[0],
                state.original[1],
                format_bytes(state.file_bytes)
            ))
            .font(mono(10.5))
            .color(theme.fg_faint),
        );
    }
    let mut actions: Vec<Action> = Vec::new();
    let mut zoom_to: Option<Option<f32>> = None;
    let mut reload = false;
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
        if widgets::icon_button(
            ui,
            &theme,
            icons::LINK_EXTERNAL,
            t("image.openExternal"),
            22.0,
        )
        .clicked()
        {
            // Explorer hands a file to the app the system has for it.
            actions.push(Action::OpenFolder(path.to_string()));
        }
        if widgets::icon_button(ui, &theme, icons::REFRESH, t("markdown.reload"), 22.0).clicked() {
            reload = true;
        }
        ui.add_space(6.0);
        if widgets::icon_button(ui, &theme, icons::ZOOM_IN, t("image.zoomIn"), 22.0).clicked() {
            zoom_to = Some(Some((scale * ZOOM_STEP).clamp(MIN_ZOOM, MAX_ZOOM)));
        }
        let label = if state.zoom.is_none() {
            format!("{} {:.0}%", t("image.fit"), scale * 100.0)
        } else {
            format!("{:.0}%", scale * 100.0)
        };
        if ui
            .add(
                egui::Button::new(RichText::new(label).font(mono(10.5)).color(theme.fg_muted))
                    .frame(false),
            )
            .on_hover_text(t("image.fitHint"))
            .clicked()
        {
            // Fit, or the file's own pixels when it already fits.
            zoom_to = Some(if state.zoom.is_none() {
                Some(1.0)
            } else {
                None
            });
        }
        if widgets::icon_button(ui, &theme, icons::ZOOM_OUT, t("image.zoomOut"), 22.0).clicked() {
            zoom_to = Some(Some((scale / ZOOM_STEP).clamp(MIN_ZOOM, MAX_ZOOM)));
        }
    });
    if reload {
        state.load(&ctx, path);
    }

    // ---- body: drag to pan, Ctrl + scroll (or pinch) to zoom around the pointer.
    let response = ui.interact(
        body,
        egui::Id::unique(("image-pane", pane_id)),
        Sense::click_and_drag(),
    );
    let mut scale = scale;
    if let Some(zoom) = zoom_to {
        state.zoom = zoom;
        state.offset = Vec2::ZERO;
        scale = zoom.unwrap_or(fit_scale);
    }
    if response.hovered() {
        let (pinch, pointer) = ui.input(|i| (i.zoom_delta(), i.pointer.hover_pos()));
        if pinch != 1.0 {
            let next = (scale * pinch).clamp(MIN_ZOOM, MAX_ZOOM);
            // Keep the point under the cursor in place.
            if let Some(p) = pointer {
                let center = body.center() + state.offset;
                state.offset += (p - center) * (1.0 - next / scale);
            }
            scale = next;
            state.zoom = Some(next);
        }
    }
    if response.dragged() {
        state.offset += response.drag_delta();
    }
    if response.double_clicked() {
        state.zoom = None;
        state.offset = Vec2::ZERO;
        scale = fit_scale;
    }

    let painter = ui.painter().with_clip_rect(body);
    if let Some(texture) = &state.texture {
        let shown = size * scale;
        let image_rect = Rect::from_center_size(body.center() + state.offset, shown);
        painter.image(
            texture.id(),
            image_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        painter.rect_stroke(
            image_rect,
            CornerRadius::ZERO,
            Stroke::new(1.0, theme.border),
            StrokeKind::Outside,
        );
        if response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if response.hovered() && (shown.x > body.width() || shown.y > body.height()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    } else if let Some(error) = &state.error {
        painter.text(
            body.center(),
            Align2::CENTER_CENTER,
            format!("{}: {error}", t("image.error")),
            sans(12.5),
            theme.offline,
        );
    } else {
        painter.text(
            body.center(),
            Align2::CENTER_CENTER,
            t("image.loading"),
            sans(12.5),
            theme.fg_faint,
        );
    }

    for action in actions {
        app.act(action);
    }
    if response.clicked() && !focused {
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
