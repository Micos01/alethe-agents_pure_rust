//! The `provider_common` helpers the vendored MCP modules use.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn provider_home_dir(segments: &[&str]) -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)?;
    Some(segments.iter().fold(home, |acc, seg| acc.join(seg)))
}

pub(crate) fn file_modified_ms(metadata: &std::fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
