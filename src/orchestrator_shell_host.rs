//! The app side of orchestrator shells (`orchestrator_shell_host.rs` in the Tauri build): each one
//! is a terminal session with no pane of its own, which the board controls and shows live in the
//! inspector. The core (`backend::orchestrator_core`) keeps the records; this only runs them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::backend::orchestrator_core::{Core, ShellHost};
use crate::terminal::{SpawnSpec, TermSize, TerminalSession};

/// How long Ctrl+C gets before the process tree is killed.
const STOP_GRACE: Duration = Duration::from_secs(5);
/// How long to wait, after a hard kill, for the process to be reported gone.
const EXIT_DRAIN: Duration = Duration::from_secs(3);
const EXIT_POLL: Duration = Duration::from_millis(200);
pub const HEADLESS_SIZE: TermSize = TermSize {
    cols: 120,
    lines: 30,
};

pub struct PtyShellHost {
    core: Core,
    ctx: egui::Context,
    sessions: Mutex<HashMap<String, Arc<TerminalSession>>>,
}

impl PtyShellHost {
    pub fn new(core: Core, ctx: egui::Context) -> Self {
        Self {
            core,
            ctx,
            sessions: Mutex::default(),
        }
    }

    /// The live session behind a shell's PTY id, for the inspector to draw and type into.
    pub fn session(&self, pty_id: &str) -> Option<Arc<TerminalSession>> {
        self.sessions.lock().unwrap().get(pty_id).cloned()
    }

    /// On app exit: a shell's process dies with the app that owns it, as in the Tauri build.
    pub fn kill_all(&self) {
        for session in self.sessions.lock().unwrap().values() {
            session.kill_blocking();
        }
    }
}

fn shell_id_of(pty_id: &str) -> &str {
    pty_id.strip_prefix("orchestrator-").unwrap_or(pty_id)
}

fn wait_exit(session: &TerminalSession, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if session.is_exited() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    session.is_exited()
}

impl ShellHost for PtyShellHost {
    fn open(&self, pty_id: &str, run: u64, command_line: &str, cwd: &str) -> Result<(), String> {
        // The line runs through the system shell and the PTY ends with it, so a service that
        // exited reads apart from one still running.
        let builder = crate::backend::cli_resolver::command_builder_for_terminal(
            None,
            None,
            &[],
            Some(command_line),
            false,
        )?;
        let session = TerminalSession::spawn(
            SpawnSpec {
                program_label: "shell".to_string(),
                builder,
                cwd: cwd.to_string(),
                size: HEADLESS_SIZE,
                initial_input: None,
            },
            crate::theme::term_palette("default"),
            self.ctx.clone(),
        )?;
        self.sessions
            .lock()
            .unwrap()
            .insert(pty_id.to_string(), Arc::clone(&session));

        // Reports this run's exit. A restart starts a new run, and the core ignores a late report
        // from the old one, as it does for one the person stopped.
        let core = self.core.clone();
        let id = pty_id.to_string();
        std::thread::Builder::new()
            .name(format!("shell-exit-{id}"))
            .spawn(move || {
                while !session.is_exited() {
                    std::thread::sleep(EXIT_POLL);
                }
                let code = session.exit_code().map(|code| code as i32);
                core.shell_exited(shell_id_of(&id), run, code);
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn output(&self, pty_id: &str, max_bytes: usize) -> Result<String, String> {
        self.session(pty_id)
            .map(|session| session.text_tail(max_bytes))
            .ok_or_else(|| format!("no terminal for {pty_id}"))
    }

    fn stop(&self, pty_id: &str) -> Result<(), String> {
        let Some(session) = self.sessions.lock().unwrap().remove(pty_id) else {
            // Restored after an app restart, or its start failed: nothing is running.
            return Ok(());
        };
        if session.is_exited() {
            return Ok(());
        }
        // Ctrl+C lets `docker compose up` bring its containers down; killing its CLI outright
        // would leave them running under the Docker daemon.
        session.write(b"\x03");
        if !wait_exit(&session, STOP_GRACE) {
            session.kill();
            wait_exit(&session, EXIT_DRAIN);
        }
        Ok(())
    }
}
