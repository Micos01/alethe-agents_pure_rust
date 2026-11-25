//! Modules reused as-is from the Tauri backend (`alethe-agents/src-tauri/src`). They never needed
//! Tauri: only their `#[tauri::command]` IPC wrappers were dropped when vendoring.

#[rustfmt::skip]
pub mod child_env;
#[rustfmt::skip]
pub mod cli_resolver;
#[rustfmt::skip]
pub mod orchestrator_core;
pub mod process;
#[rustfmt::skip]
pub mod git_control;
pub mod git_support;
#[rustfmt::skip]
pub mod mcp_model;
#[rustfmt::skip]
pub mod mcp_agents;
#[rustfmt::skip]
#[allow(dead_code)] // config paths and secret reveal are not wired to the UI yet
pub mod mcp_store;
pub mod mcp_support;
#[rustfmt::skip]
#[allow(dead_code)] // the bundled-skill catalog is not wired to the UI yet
pub mod skills;
