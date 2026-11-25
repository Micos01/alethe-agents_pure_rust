//! Icons. The web build uses lucide SVGs; here the same shapes come from the Codicons set that
//! ships inside the embedded Caskaydia Cove Nerd Font, plus the agent logos as images.

pub const HOME: &str = "\u{eb06}";
pub const TERMINAL: &str = "\u{ea85}";
pub const FOLDER: &str = "\u{ea83}";
pub const FOLDER_OPENED: &str = "\u{eaf7}";
pub const NEW_FOLDER: &str = "\u{ea80}";
pub const ADD: &str = "\u{ea60}";
pub const CLOSE: &str = "\u{ea76}";
pub const CHEVRON_DOWN: &str = "\u{eab4}";
pub const CHEVRON_RIGHT: &str = "\u{eab6}";
pub const ARROW_RIGHT: &str = "\u{ea9c}";
pub const MENU: &str = "\u{eb94}";
pub const SEARCH: &str = "\u{ea6d}";
pub const GEAR: &str = "\u{eb51}";
pub const SOURCE_CONTROL: &str = "\u{ea68}";
pub const GRID: &str = "\u{ebeb}";
pub const ELLIPSIS: &str = "\u{ea7c}";
pub const LAYOUT_SIDEBAR_LEFT: &str = "\u{ebf3}";
pub const LAYOUT_SIDEBAR_RIGHT: &str = "\u{ebf2}";
pub const SCREEN_FULL: &str = "\u{eb4c}";
pub const SCREEN_NORMAL: &str = "\u{eb4d}";
pub const CHROME_MINIMIZE: &str = "\u{eaba}";
pub const CHROME_MAXIMIZE: &str = "\u{eab9}";
pub const CHROME_RESTORE: &str = "\u{eabb}";
pub const CHROME_CLOSE: &str = "\u{eab8}";
pub const CHECKLIST: &str = "\u{eab3}";
pub const MARKDOWN: &str = "\u{eb1d}";
pub const LAYERS: &str = "\u{ebd2}";
pub const ACCOUNT: &str = "\u{eb99}";
pub const TRASH: &str = "\u{ea81}";
pub const DEBUG_RESTART: &str = "\u{ead2}";
pub const SEND: &str = "\u{f1d8}";
pub const TAG: &str = "\u{ea66}";
pub const CHECK: &str = "\u{eab2}";
pub const SPLIT: &str = "\u{eb56}";
pub const TYPE_HIERARCHY: &str = "\u{ebb9}";
pub const DEBUG_STOP: &str = "\u{ead7}";
pub const GIT_BRANCH: &str = "\u{ea68}";
pub const ZOOM_IN: &str = "\u{eb81}";
pub const ZOOM_OUT: &str = "\u{eb82}";
pub const EDIT: &str = "\u{ea73}";
pub const SAVE: &str = "\u{eb4b}";
pub const REFRESH: &str = "\u{eb37}";
pub const COPY: &str = "\u{ebcc}";
pub const PREVIEW: &str = "\u{eb2f}";
pub const WARNING: &str = "\u{ea6c}";

pub fn agent_icon_bytes(file: &str) -> Option<&'static [u8]> {
    Some(match file {
        "claude-code.png" => include_bytes!("../../assets/icons/claude-code.png"),
        "codex.png" => include_bytes!("../../assets/icons/codex.png"),
        "copilot.svg" => include_bytes!("../../assets/icons/copilot.svg"),
        "cursor.svg" => include_bytes!("../../assets/icons/cursor.svg"),
        "antigravity-official.svg" => include_bytes!("../../assets/icons/antigravity-official.svg"),
        "opencode.png" => include_bytes!("../../assets/icons/opencode.png"),
        "mimo.svg" => include_bytes!("../../assets/icons/mimo.svg"),
        "freebuff.png" => include_bytes!("../../assets/icons/freebuff.png"),
        "kiro.svg" => include_bytes!("../../assets/icons/kiro.svg"),
        "kimi.svg" => include_bytes!("../../assets/icons/kimi.svg"),
        "grok.svg" => include_bytes!("../../assets/icons/grok.svg"),
        "codewhale.svg" => include_bytes!("../../assets/icons/codewhale.svg"),
        "wsl.svg" => include_bytes!("../../assets/icons/wsl.svg"),
        _ => return None,
    })
}

pub const APP_LOGO: &[u8] = include_bytes!("../../assets/icons/alethe.png");

/// Agent logo as an `egui::Image`, or `None` for agents drawn with a glyph (the shell).
pub fn agent_image(agent: &str, size: f32) -> Option<egui::Image<'static>> {
    let info = crate::agents::info(agent);
    let bytes = agent_icon_bytes(info.icon)?;
    Some(
        egui::Image::from_bytes(format!("bytes://agent/{}", info.icon), bytes)
            .fit_to_exact_size(egui::vec2(size, size))
            .corner_radius(3),
    )
}
pub const GRIP: &str = "\u{f01dd}";
pub const FOCUS: &str = "\u{f05b}";
pub const PLAY: &str = "\u{f04b}";
pub const PAUSE: &str = "\u{f04c}";
pub const COFFEE: &str = "\u{f0f4}";
pub const TIMER: &str = "\u{f017}";
pub const RESET: &str = "\u{f0e2}";
