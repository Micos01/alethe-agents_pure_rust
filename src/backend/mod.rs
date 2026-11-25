//! Modules reused as-is from the Tauri backend (`alethe-agents/src-tauri/src`). They never needed
//! Tauri: only their `#[tauri::command]` IPC wrappers were dropped when vendoring.

#[rustfmt::skip]
pub mod child_env;
#[rustfmt::skip]
pub mod cli_resolver;
pub mod process;
