//! UI and terminal palettes, ported 1:1 from the web build's CSS custom properties.
//! `generated.rs` is produced from `src/styles/theme.css` (see `tools/gen_themes.py`).

mod generated;

use egui::{Color32, CornerRadius, Stroke, Visuals};

/// Raw, non-premultiplied RGBA exactly as written in the CSS.
#[derive(Clone, Copy, Debug)]
pub struct Rgba8(pub [u8; 4]);

pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Rgba8 {
    Rgba8([r, g, b, a])
}

impl Rgba8 {
    pub fn c(self) -> Color32 {
        let [r, g, b, a] = self.0;
        Color32::from_rgba_unmultiplied(r, g, b, a)
    }
}

/// The subset of `--*` tokens the native UI consumes.
#[derive(Clone, Copy, Debug)]
pub struct UiTokens {
    pub bg: Rgba8,
    pub bg_elevated: Rgba8,
    pub bg_sunken: Rgba8,
    pub panel: Rgba8,
    pub panel_hover: Rgba8,
    pub border: Rgba8,
    pub border_strong: Rgba8,
    pub fg: Rgba8,
    pub fg_muted: Rgba8,
    pub fg_faint: Rgba8,
    pub text_quaternary: Rgba8,
    pub accent: Rgba8,
    pub accent_strong: Rgba8,
    pub accent_on: Rgba8,
    pub accent_soft: Rgba8,
    pub accent_border: Rgba8,
    pub status_working: Rgba8,
    pub status_waiting: Rgba8,
    pub status_stopped: Rgba8,
    pub status_offline: Rgba8,
    pub agent_shell: Rgba8,
    pub agent_claude: Rgba8,
    pub agent_codex: Rgba8,
    pub agent_opencode: Rgba8,
}

/// xterm.js `ITheme` equivalent.
#[derive(Clone, Copy, Debug)]
pub struct TermPalette {
    pub background: Rgba8,
    pub foreground: Rgba8,
    pub cursor: Rgba8,
    pub selection_background: Rgba8,
    pub ansi: [Rgba8; 16],
}

/// Resolved colors for one frame.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Theme {
    pub id: String,
    pub light: bool,
    pub bg: Color32,
    pub bg_elevated: Color32,
    pub bg_sunken: Color32,
    pub panel: Color32,
    pub panel_hover: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub fg: Color32,
    pub fg_muted: Color32,
    pub fg_faint: Color32,
    pub fg_quaternary: Color32,
    pub accent: Color32,
    pub accent_strong: Color32,
    pub accent_on: Color32,
    pub accent_soft: Color32,
    pub accent_border: Color32,
    pub working: Color32,
    pub waiting: Color32,
    pub stopped: Color32,
    pub offline: Color32,
    pub agent_shell: Color32,
    pub agent_claude: Color32,
    pub agent_codex: Color32,
    pub agent_opencode: Color32,
}

/// Swatches shown in the theme picker (`BUILTIN_THEME_OPTIONS` in `lib/themes.ts`).
pub const THEME_OPTIONS: &[(&str, &str, [u32; 3])] = &[
    (
        "elite-original",
        "Elite Original",
        [0xfbfafd, 0xede8f7, 0x6157f0],
    ),
    (
        "elite-pure-black",
        "Elite Pure Black",
        [0x000000, 0x171717, 0xffffff],
    ),
    (
        "elite-indigo",
        "Elite Indigo",
        [0x0c0c0c, 0x1c1c2e, 0x7d72ff],
    ),
    ("elite-blush", "Elite Blush", [0xfff7f2, 0xf3e2d6, 0x7a4a3a]),
    ("dark", "Dark", [0x101114, 0x2a2d33, 0xf3f4f6]),
    ("light", "Light", [0xf6f7fb, 0xffffff, 0x18181b]),
    ("dracula", "Dracula", [0x282a36, 0xbd93f9, 0xff79c6]),
    ("nord", "Nord", [0x2e3440, 0x88c0d0, 0xa3be8c]),
    ("gruvbox", "Gruvbox", [0x282828, 0xfabd2f, 0xb8bb26]),
    ("solarized", "Solarized", [0x002b36, 0x268bd2, 0xb58900]),
    ("tokyo-night", "Tokyo Night", [0x1a1b26, 0x7aa2f7, 0xbb9af7]),
    ("vscode", "VS Code", [0x1e1e1e, 0x007acc, 0xcccccc]),
    ("min-dark", "Min Dark", [0x1f1f1f, 0xfafafa, 0x888888]),
    ("min-light", "Min Light", [0xffffff, 0x1976d2, 0x6f42c1]),
    (
        "catppuccin-frappe",
        "Catppuccin Frappé",
        [0x303446, 0xf4b8e4, 0xb5bfe2],
    ),
    (
        "gruvbox-material",
        "Gruvbox Material",
        [0x1d2021, 0xe78a4e, 0xd4be98],
    ),
    ("dark-lemon", "Dark Lemon", [0x141414, 0xffff50, 0xc792ea]),
];

const LIGHT_THEMES: &[&str] = &["light", "min-light", "elite-original", "elite-blush"];

pub fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn ui_tokens(id: &str) -> (&'static str, &'static UiTokens) {
    generated::UI_THEMES
        .iter()
        .find(|(name, _)| *name == id)
        .or_else(|| {
            generated::UI_THEMES
                .iter()
                .find(|(name, _)| *name == "dark")
        })
        .map(|(name, tokens)| (*name, tokens))
        .expect("dark theme is always generated")
}

pub fn term_palette(id: &str) -> &'static TermPalette {
    generated::TERM_PALETTES
        .iter()
        .find(|(name, _)| *name == id)
        .or_else(|| {
            generated::TERM_PALETTES
                .iter()
                .find(|(name, _)| *name == "dark")
        })
        .map(|(_, palette)| palette)
        .expect("dark palette is always generated")
}

impl Theme {
    pub fn resolve(id: &str) -> Self {
        let (name, t) = ui_tokens(id);
        Self {
            id: name.to_string(),
            light: LIGHT_THEMES.contains(&name),
            bg: t.bg.c(),
            bg_elevated: t.bg_elevated.c(),
            bg_sunken: t.bg_sunken.c(),
            panel: t.panel.c(),
            panel_hover: t.panel_hover.c(),
            border: t.border.c(),
            border_strong: t.border_strong.c(),
            fg: t.fg.c(),
            fg_muted: t.fg_muted.c(),
            fg_faint: t.fg_faint.c(),
            fg_quaternary: t.text_quaternary.c(),
            accent: t.accent.c(),
            accent_strong: t.accent_strong.c(),
            accent_on: t.accent_on.c(),
            accent_soft: t.accent_soft.c(),
            accent_border: t.accent_border.c(),
            working: t.status_working.c(),
            waiting: t.status_waiting.c(),
            stopped: t.status_stopped.c(),
            offline: t.status_offline.c(),
            agent_shell: t.agent_shell.c(),
            agent_claude: t.agent_claude.c(),
            agent_codex: t.agent_codex.c(),
            agent_opencode: t.agent_opencode.c(),
        }
    }

    /// Mirrors `reset.css` + `theme.css` onto egui's style system.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = if self.light {
            Visuals::light()
        } else {
            Visuals::dark()
        };
        visuals.override_text_color = Some(self.fg);
        visuals.panel_fill = self.bg;
        visuals.window_fill = self.bg_elevated;
        visuals.extreme_bg_color = self.bg_sunken;
        visuals.faint_bg_color = self.bg_elevated;
        visuals.code_bg_color = self.panel;
        visuals.window_stroke = Stroke::new(1.0, self.border);
        visuals.window_corner_radius = CornerRadius::same(14);
        visuals.menu_corner_radius = CornerRadius::same(8);
        visuals.hyperlink_color = self.accent;
        visuals.selection.bg_fill = self.accent_soft.gamma_multiply(2.5);
        visuals.selection.stroke = Stroke::new(1.0, self.fg);
        visuals.text_cursor.stroke = Stroke::new(1.5, self.accent);

        let radius = CornerRadius::same(6);
        let w = &mut visuals.widgets;
        w.noninteractive.bg_fill = self.bg;
        w.noninteractive.weak_bg_fill = self.bg;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.fg_muted);
        w.noninteractive.corner_radius = radius;

        w.inactive.bg_fill = self.panel;
        w.inactive.weak_bg_fill = Color32::TRANSPARENT;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, self.fg_muted);
        w.inactive.corner_radius = radius;

        w.hovered.bg_fill = self.panel_hover;
        w.hovered.weak_bg_fill = self.accent_soft;
        w.hovered.bg_stroke = Stroke::new(1.0, self.border_strong);
        w.hovered.fg_stroke = Stroke::new(1.0, self.fg);
        w.hovered.corner_radius = radius;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = self.panel_hover;
        w.active.weak_bg_fill = self.accent_soft;
        w.active.bg_stroke = Stroke::new(1.0, self.accent_border);
        w.active.fg_stroke = Stroke::new(1.0, self.fg);
        w.active.corner_radius = radius;
        w.active.expansion = 0.0;

        w.open = w.active;

        ctx.set_visuals(visuals);
        ctx.global_style_mut(|style| {
            style.spacing.item_spacing = egui::vec2(6.0, 4.0);
            style.spacing.button_padding = egui::vec2(8.0, 4.0);
            style.spacing.interact_size.y = 22.0;
            style.spacing.scroll.bar_width = 6.0;
            style.spacing.scroll.floating = true;
            style.interaction.selectable_labels = false;
        });
    }

    pub fn agent_color(&self, agent: &str) -> Color32 {
        match agent {
            "claude" => self.agent_claude,
            "codex" => self.agent_codex,
            "shell" | "wsl" => self.agent_shell,
            "opencode" => self.agent_opencode,
            other => crate::agents::info(other).color,
        }
    }
}
