//! Built-in agent providers (`BuiltinAgentType`, `AGENT_TYPE_LABELS`, `BUILTIN_CLI_COMMANDS`).

use egui::Color32;

#[allow(dead_code)]
pub struct AgentInfo {
    pub id: &'static str,
    pub label: &'static str,
    /// Name shown on a sub-tab.
    pub short: &'static str,
    /// CLI binary; `None` opens the user's shell.
    pub command: Option<&'static str>,
    pub color: Color32,
    /// Icon file in `assets/icons`.
    pub icon: &'static str,
    /// Flag that resumes a previous session (`--resume <id>` style), when the CLI has one.
    pub resume_flag: Option<&'static str>,
}

const fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const AGENTS: &[AgentInfo] = &[
    AgentInfo {
        id: "claude",
        label: "Claude Code",
        short: "claude",
        command: Some("claude"),
        color: rgb(0xf59e0b),
        icon: "claude-code.png",
        resume_flag: Some("--resume"),
    },
    AgentInfo {
        id: "codex",
        label: "Codex",
        short: "codex",
        command: Some("codex"),
        color: rgb(0x38bdf8),
        icon: "codex.png",
        resume_flag: None,
    },
    AgentInfo {
        id: "copilot",
        label: "GitHub Copilot",
        short: "copilot",
        command: Some("copilot"),
        color: rgb(0xa78bfa),
        icon: "copilot.svg",
        resume_flag: None,
    },
    AgentInfo {
        id: "cursor",
        label: "Cursor",
        short: "cursor",
        command: Some("cursor-agent"),
        color: rgb(0xe5e7eb),
        icon: "cursor.svg",
        resume_flag: None,
    },
    AgentInfo {
        id: "antigravity",
        label: "Antigravity",
        short: "agy",
        command: Some("agy"),
        color: rgb(0x60a5fa),
        icon: "antigravity-official.svg",
        resume_flag: None,
    },
    AgentInfo {
        id: "opencode",
        label: "OpenCode",
        short: "opencode",
        command: Some("opencode"),
        color: rgb(0xa1a1aa),
        icon: "opencode.png",
        resume_flag: None,
    },
    AgentInfo {
        id: "mimo",
        label: "Mimo",
        short: "mimo",
        command: Some("mimo"),
        color: rgb(0xfb923c),
        icon: "mimo.svg",
        resume_flag: None,
    },
    AgentInfo {
        id: "freebuff",
        label: "Freebuff",
        short: "freebuff",
        command: Some("freebuff"),
        color: rgb(0x4ade80),
        icon: "freebuff.png",
        resume_flag: None,
    },
    AgentInfo {
        id: "kiro",
        label: "Kiro CLI",
        short: "kiro",
        command: Some("kiro-cli"),
        color: rgb(0x8b5cf6),
        icon: "kiro.svg",
        resume_flag: None,
    },
    AgentInfo {
        id: "kimi",
        label: "Kimi Code",
        short: "kimi",
        command: Some("kimi"),
        color: rgb(0x60a5fa),
        icon: "kimi.svg",
        resume_flag: None,
    },
    AgentInfo {
        id: "grok",
        label: "Grok Build",
        short: "grok",
        command: Some("grok"),
        color: rgb(0xe5e7eb),
        icon: "grok.svg",
        resume_flag: None,
    },
    AgentInfo {
        id: "codewhale",
        label: "Codewhale",
        short: "codewhale",
        command: Some("codewhale"),
        color: rgb(0x22d3ee),
        icon: "codewhale.svg",
        resume_flag: None,
    },
    AgentInfo {
        id: "shell",
        label: "Shell",
        short: "shell",
        command: None,
        color: rgb(0x22c55e),
        icon: "shell.svg",
        resume_flag: None,
    },
    AgentInfo {
        id: "wsl",
        label: "WSL",
        short: "wsl",
        command: Some("wsl.exe"),
        color: rgb(0x22c55e),
        icon: "wsl.svg",
        resume_flag: None,
    },
];

static UNKNOWN: AgentInfo = AgentInfo {
    id: "unknown",
    label: "Agent",
    short: "agent",
    command: None,
    color: rgb(0x8a8a8a),
    icon: "terminal.svg",
    resume_flag: None,
};

/// `UNRESTRICTED_FLAG` from `lib/types.ts`: the "YOLO" permission mode.
pub fn unrestricted_flag(id: &str) -> Option<&'static str> {
    match id {
        "claude" | "opencode" | "antigravity" => Some("--dangerously-skip-permissions"),
        "codex" => Some("--dangerously-bypass-approvals-and-sandbox"),
        "copilot" => Some("--allow-all"),
        "cursor" => Some("--force"),
        "kiro" => Some("--trust-all-tools"),
        "kimi" | "grok" => Some("--yolo"),
        _ => None,
    }
}

pub fn info(id: &str) -> &'static AgentInfo {
    AGENTS.iter().find(|a| a.id == id).unwrap_or(&UNKNOWN)
}

/// Agents offered in pickers on this platform.
pub fn available() -> impl Iterator<Item = &'static AgentInfo> {
    AGENTS.iter().filter(|a| cfg!(windows) || a.id != "wsl")
}

/// Whether the agent's CLI resolves on PATH. Looked up once per agent per run: the resolver
/// walks PATH and the npm/registry fallbacks, far too slow to repeat every frame.
pub fn installed(id: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(found) = cache.lock().unwrap().get(id) {
        return *found;
    }
    let found = info(id)
        .command
        .is_none_or(|c| crate::backend::cli_resolver::find_windows_cli_launcher(c).is_some());
    cache.lock().unwrap().insert(id.to_string(), found);
    found
}
