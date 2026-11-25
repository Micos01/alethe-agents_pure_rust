//! The two helpers `git_control` borrows from the Tauri backend's `filesystem.rs` and `wsl.rs`,
//! copied verbatim. AGPL-3.0-or-later. The native build has no WSL integration toggle, so
//! `wsl_target` behaves as upstream's default (integration on).

#[allow(dead_code)] // only its presence matters to git_control (safe.directory for WSL paths)
pub struct WslPath {
    pub distro: String,
    pub linux_path: String,
}

pub fn strip_extended_prefix(path: &str) -> String {
    path.strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| path.strip_prefix(r"\\?\").map(str::to_string))
        .unwrap_or_else(|| path.to_string())
}

pub fn wsl_target(cwd: &str) -> Option<WslPath> {
    parse_wsl_unc(cwd)
}

pub fn parse_wsl_unc(path: &str) -> Option<WslPath> {
    let normalized = path.trim().replace('\\', "/");
    let rest = normalized.strip_prefix("//")?;

    let mut segments = rest.split('/').filter(|segment| !segment.is_empty());
    let host = segments.next()?;
    if !host.eq_ignore_ascii_case("wsl.localhost") && !host.eq_ignore_ascii_case("wsl$") {
        return None;
    }

    let distro = segments.next()?;
    let tail: Vec<&str> = segments.collect();
    let linux_path = if tail.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", tail.join("/"))
    };

    Some(WslPath {
        distro: distro.to_string(),
        linux_path,
    })
}
