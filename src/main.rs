#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Alethe Native — the Alethe multi-agent workspace rebuilt as a pure Rust desktop app
//! (eframe/egui + alacritty_terminal + portable-pty). No Tauri, no webview.

mod agents;
mod app;
mod backend;
mod codex_sessions;
mod i18n;
mod launch;
mod model;
mod notify;
mod orchestrator;
mod pomodoro;
mod resources;
mod storage;
mod terminal;
mod theme;
mod transcripts;
mod ui;
mod usage;

use eframe::egui;
use std::sync::Arc;

fn load_system_font(candidates: &[&str]) -> Option<Vec<u8>> {
    candidates.iter().find_map(|path| std::fs::read(path).ok())
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "caskaydia".into(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/CaskaydiaCoveNerdFontMono-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "caskaydia-bold".into(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/CaskaydiaCoveNerdFontMono-Bold.ttf"
        ))),
    );

    // `--font-sans`: Inter, system-ui, 'Segoe UI', sans-serif. Use the platform UI font when present.
    let sans = load_system_font(&[
        "C:\\Windows\\Fonts\\segoeui.ttf",
        "/System/Library/Fonts/SFNS.ttf",
        "/usr/share/fonts/truetype/inter/Inter-Regular.ttf",
        "/usr/share/fonts/opentype/inter/Inter-Regular.otf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]);
    let sans_bold = load_system_font(&[
        "C:\\Windows\\Fonts\\seguisb.ttf",
        "C:\\Windows\\Fonts\\segoeuib.ttf",
        "/usr/share/fonts/truetype/inter/Inter-SemiBold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    ]);

    let proportional = fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default();
    if let Some(bytes) = sans {
        fonts
            .font_data
            .insert("sans".into(), Arc::new(egui::FontData::from_owned(bytes)));
        proportional.insert(0, "sans".into());
    }
    // Icons (Codicons/Nerd glyphs) resolve through the monospace font as a fallback.
    proportional.push("caskaydia".into());

    let mono = fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default();
    mono.insert(0, "caskaydia".into());

    let mut bold_family = vec!["caskaydia-bold".to_string()];
    bold_family.extend(
        fonts.families[&egui::FontFamily::Monospace]
            .iter()
            .skip(1)
            .cloned(),
    );
    fonts
        .families
        .insert(egui::FontFamily::Name("mono-bold".into()), bold_family);

    let mut ui_bold = Vec::new();
    if let Some(bytes) = sans_bold {
        fonts.font_data.insert(
            "sans-bold".into(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        ui_bold.push("sans-bold".to_string());
    }
    ui_bold.extend(
        fonts.families[&egui::FontFamily::Proportional]
            .iter()
            .cloned(),
    );
    fonts
        .families
        .insert(egui::FontFamily::Name("bold".into()), ui_bold);

    ctx.set_fonts(fonts);
}

fn app_icon() -> Option<egui::IconData> {
    let image = image::load_from_memory(include_bytes!("../assets/icons/app-icon.png"))
        .ok()?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

/// Markers Claude Code sets for its own session. When the app is started from inside one, a Claude
/// launched in our terminals would inherit them and run as that session's child (no transcript
/// saving, nested-session checks). User settings such as `CLAUDE_CODE_USE_BEDROCK` are kept.
const CLAUDE_SESSION_MARKERS: [&str; 9] = [
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_CHILD_NOTES_DIR",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
];

fn main() -> eframe::Result<()> {
    // Single-threaded here: nothing else reads the environment yet.
    for key in CLAUDE_SESSION_MARKERS {
        std::env::remove_var(key);
    }
    backend::process::install_kill_on_close_guard();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Alethe")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([760.0, 480.0])
        .with_decorations(false)
        .with_transparent(false);
    if let Some(icon) = app_icon() {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Alethe",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            install_fonts(&cc.egui_ctx);
            Ok(Box::new(app::AletheApp::new(cc)))
        }),
    )
}
