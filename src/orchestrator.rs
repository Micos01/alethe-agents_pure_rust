//! Hosts the delegation core (`backend::orchestrator_core`, vendored verbatim) the way the Tauri
//! build does in `orchestrator.rs` + `agent_events.rs`: an MCP endpoint over HTTP on 127.0.0.1,
//! guarded by a per-run token, that planner terminals (Claude via `--mcp-config`, Codex through a
//! stdio bridge) call to delegate work to Codex/Claude workers. The board reads `snapshot()`.
//!
//! Started lazily, on the first planner launch or the first board shown, so a session that never
//! uses the orchestrator spends neither the listener thread nor the PATH scan.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};

use crate::backend::cli_resolver;
use crate::backend::orchestrator_core::{
    self as core, Core, Launcher, OrchestrationSettings, Planner,
};

const HOST: &str = "127.0.0.1";
const BODY_LIMIT: u64 = 4 * 1024 * 1024;
const CODEX_MCP_MARK_START: &str = "# alethe-managed-mcp-start";
const CODEX_MCP_MARK_END: &str = "# alethe-managed-mcp-end";

pub struct Orchestrator {
    core: Core,
    token: String,
    port: OnceLock<u16>,
    store: PathBuf,
    /// Last board the core reported, and a counter the UI compares to know it changed.
    board: Mutex<Value>,
    revision: AtomicU64,
    ctx: egui::Context,
    /// Runs the shells planners open (`alethe_open_shell`); wired on start.
    shells: OnceLock<Arc<crate::orchestrator_shell_host::PtyShellHost>>,
    /// The last shell control that failed, for the board to show once.
    shell_error: Mutex<Option<String>>,
}

impl Orchestrator {
    pub fn new(ctx: egui::Context, store: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            core: Core::default(),
            token: nanoid::nanoid!(32),
            port: OnceLock::new(),
            store,
            board: Mutex::new(Value::Null),
            revision: AtomicU64::new(0),
            ctx,
            shells: OnceLock::new(),
            shell_error: Mutex::new(None),
        })
    }

    /// Idempotent: restores history, wires the observer, resolves the worker CLIs and binds the
    /// listener. Returns the endpoint, or why it could not start.
    pub fn ensure_started(self: &Arc<Self>) -> Result<String, String> {
        if let Some(port) = self.port.get() {
            return Ok(endpoint(*port));
        }
        static STARTING: Mutex<()> = Mutex::new(());
        let _guard = STARTING.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(port) = self.port.get() {
            return Ok(endpoint(*port));
        }

        // History outlives the app: each worker's brief and report is kept, and Codex keeps the
        // thread, so a worker can be resumed with its context.
        self.core.set_store(self.store.clone());
        self.core.restore();
        let weak = Arc::downgrade(self);
        self.core.set_observer(Arc::new(move |snapshot: Value| {
            if let Some(this) = weak.upgrade() {
                *this.board.lock().unwrap() = snapshot;
                this.revision.fetch_add(1, Ordering::SeqCst);
                this.ctx.request_repaint();
            }
        }));
        let host = Arc::new(crate::orchestrator_shell_host::PtyShellHost::new(
            self.core.clone(),
            self.ctx.clone(),
        ));
        self.core.set_shell_host(host.clone());
        let _ = self.shells.set(host);
        // Smoke test hook: `ALETHE_NATIVE_SMOKE_SHELLS="<planner>|<cwd>|<command>;;..."` opens
        // those shells as if the planner had called `alethe_open_shell`.
        if let Ok(spec) = std::env::var("ALETHE_NATIVE_SMOKE_SHELLS") {
            for entry in spec.split(";;") {
                if let [planner, cwd, command] = entry.splitn(3, '|').collect::<Vec<_>>()[..] {
                    let planner = Some(planner).filter(|p| !p.is_empty());
                    if let Err(error) = self.core.open_shell(planner, command, None, cwd) {
                        eprintln!("[orchestrator] smoke shell: {error}");
                    }
                }
            }
        }
        self.publish();

        if let Some(program) = cli_resolver::find_windows_cli_launcher("codex") {
            #[allow(unused_mut)]
            let mut launcher = Launcher::codex_app_server(program);
            #[cfg(windows)]
            launcher.env.push((
                "Path".to_string(),
                core::path_without_store_aliases(&cli_resolver::rebuilt_path()),
            ));
            self.core.set_launcher(launcher);
        }
        if let Some(program) = cli_resolver::find_windows_cli_launcher("claude") {
            self.core.set_launcher(Launcher::claude_headless(program));
        }

        let server = tiny_http::Server::http(format!("{HOST}:0")).map_err(|e| e.to_string())?;
        let port = server
            .server_addr()
            .to_ip()
            .map(|addr| addr.port())
            .ok_or_else(|| "listener has no ip address".to_string())?;
        let this = Arc::clone(self);
        std::thread::Builder::new()
            .name("orchestrator-mcp".into())
            .spawn(move || this.serve(server))
            .map_err(|e| e.to_string())?;
        let _ = self.port.set(port);
        Ok(endpoint(port))
    }

    fn serve(self: Arc<Self>, server: tiny_http::Server) {
        for mut request in server.incoming_requests() {
            if request.method() != &tiny_http::Method::Post {
                let _ = request.respond(tiny_http::Response::empty(405));
                continue;
            }
            if request.url() != "/mcp" {
                let _ = request.respond(tiny_http::Response::empty(404));
                continue;
            }
            let header = |name: &'static str| {
                request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv(name))
                    .map(|h| h.value.as_str().to_string())
            };
            if header("X-Alethe-Token").as_deref() != Some(self.token.as_str()) {
                let _ = request.respond(tiny_http::Response::empty(401));
                continue;
            }
            let planner = header("X-Alethe-Planner").filter(|p| !p.is_empty());
            let mut body = String::new();
            if request
                .as_reader()
                .take(BODY_LIMIT)
                .read_to_string(&mut body)
                .is_err()
            {
                let _ = request.respond(tiny_http::Response::empty(400));
                continue;
            }
            // A tool call may wait on workers for minutes; each request gets its own thread.
            let this = Arc::clone(&self);
            std::thread::spawn(move || {
                match core::handle_mcp_body(&this.core, &body, planner.as_deref()) {
                    Some(payload) => {
                        let content_type =
                            tiny_http::Header::from_bytes("Content-Type", "application/json")
                                .expect("static header");
                        let _ = request.respond(
                            tiny_http::Response::from_string(payload).with_header(content_type),
                        );
                    }
                    None => {
                        let _ = request.respond(tiny_http::Response::empty(202));
                    }
                }
            });
        }
    }

    fn publish(&self) {
        *self.board.lock().unwrap() = self.core.snapshot();
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.ctx.request_repaint();
    }

    /// Re-reads the board so the elapsed clocks advance while workers run.
    pub fn refresh(&self) {
        self.publish();
    }

    pub fn is_started(&self) -> bool {
        self.port.get().is_some()
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn board(&self) -> Value {
        self.board.lock().unwrap().clone()
    }

    /// `orchestrator_mcp_config_path`: registers the terminal as a planner and writes the Claude
    /// `--mcp-config` file that points it at this app, tagged with its planner id.
    pub fn claude_config(self: &Arc<Self>, planner: &Planner) -> Result<PathBuf, String> {
        let endpoint = self.ensure_started()?;
        self.core.register_planner(planner.clone());
        let config = json!({
            "mcpServers": {
                "alethe": {
                    "type": "http",
                    "url": format!("{endpoint}/mcp"),
                    "headers": {
                        "X-Alethe-Token": self.token,
                        "X-Alethe-Planner": planner.id
                    }
                }
            }
        });
        let port = self.port.get().copied().unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "alethe-native-orchestrator-mcp-{port}-{}.json",
            safe_name(&planner.id)
        ));
        let body = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
        write_private(&path, body.as_bytes())?;
        self.publish();
        Ok(path)
    }

    /// `codex_mcp_config_write`: Codex only speaks stdio MCP, so a PowerShell bridge forwards its
    /// JSON-RPC lines to `/mcp`. The block in the repo's `.codex/config.toml` is ours and replaced
    /// in place; the terminal names itself through `ALETHE_PLANNER`.
    pub fn codex_config(self: &Arc<Self>, planner: &Planner, cwd: &str) -> Result<(), String> {
        let endpoint = self.ensure_started()?;
        self.core.register_planner(planner.clone());
        let script = format!(
            "$planner = $env:ALETHE_PLANNER\r\n\
             while ($line = [Console]::In.ReadLine()) {{\r\n\
             \x20\x20if ([string]::IsNullOrWhiteSpace($line)) {{ continue }}\r\n\
             \x20\x20try {{\r\n\
             \x20\x20\x20\x20$resp = Invoke-WebRequest -UseBasicParsing -ErrorAction Stop -Uri '{endpoint}/mcp' -Method Post -Body $line -ContentType 'application/json' -Headers @{{ 'X-Alethe-Token' = '{token}'; 'X-Alethe-Planner' = $planner }}\r\n\
             \x20\x20\x20\x20if ($resp.Content) {{\r\n\
             \x20\x20\x20\x20\x20\x20[Console]::Out.WriteLine($resp.Content)\r\n\
             \x20\x20\x20\x20\x20\x20[Console]::Out.Flush()\r\n\
             \x20\x20\x20\x20}}\r\n\
             \x20\x20}} catch {{\r\n\
             \x20\x20\x20\x20[Console]::Error.WriteLine('[alethe-mcp] request failed: ' + $_.Exception.Message)\r\n\
             \x20\x20}}\r\n\
             }}\r\n",
            token = self.token.replace('\'', "''"),
        );
        let script_path = std::env::temp_dir().join("alethe-native-codex-mcp-bridge.ps1");
        write_private(&script_path, script.as_bytes())?;

        let root = repository_root(cwd);
        let codex_dir = root.join(".codex");
        std::fs::create_dir_all(&codex_dir).map_err(|e| format!("mkdir_failed:{e}"))?;
        let path = codex_dir.join("config.toml");
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let mut body = strip_managed_block(&existing);
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&format!(
            "\n{CODEX_MCP_MARK_START}\n[mcp_servers.alethe]\ncommand = \"powershell.exe\"\nargs = [\"-NoProfile\", \"-ExecutionPolicy\", \"Bypass\", \"-File\", {}]\n{CODEX_MCP_MARK_END}\n",
            toml_string(&script_path.to_string_lossy()),
        ));
        std::fs::write(&path, body).map_err(|e| format!("write_failed:{e}"))?;
        self.publish();
        Ok(())
    }

    pub fn apply_settings(&self, settings: Option<&Value>) {
        if let Some(parsed) = settings
            .filter(|v| !v.is_null())
            .and_then(|v| serde_json::from_value::<OrchestrationSettings>(v.clone()).ok())
        {
            self.core.apply_settings(parsed);
        }
    }

    // ------------------------------------------------------------ board actions
    // Each may block (cancel waits on the worker, restart spawns one), so they run off the UI
    // thread and the board refreshes through the observer.

    pub fn answer(self: &Arc<Self>, job_id: String, decision: &'static str) {
        self.background(move |core| core.answer(&job_id, decision).map(|_| ()));
    }

    pub fn cancel(self: &Arc<Self>, job_id: String) {
        self.background(move |core| {
            core.cancel_jobs(&[job_id]);
            Ok(())
        });
    }

    pub fn restart(self: &Arc<Self>, job_id: String) {
        self.background(move |core| core::restart_job(core, &job_id).map(|_| ()));
    }

    pub fn message(self: &Arc<Self>, job_id: String, message: String, steer: bool) {
        self.background(move |core| {
            let mut arguments = serde_json::Map::new();
            arguments.insert("jobId".into(), Value::String(job_id));
            arguments.insert("message".into(), Value::String(message));
            let tool = if steer { "alethe_steer" } else { "alethe_send" };
            core::call_tool(core, tool, &arguments, None).map(|_| ())
        });
    }

    /// The worker's diff, fetched off the UI thread into `slot`.
    pub fn diff(
        self: &Arc<Self>,
        job_id: String,
        slot: Arc<Mutex<Option<Result<String, String>>>>,
    ) {
        let this = Arc::clone(self);
        std::thread::spawn(move || {
            let result = this.core.job_diff(&job_id);
            *slot.lock().unwrap() = Some(result);
            this.ctx.request_repaint();
        });
    }

    // ------------------------------------------------------------ shells

    /// The live terminal behind a shell, while its process (or its last screen) is around.
    pub fn shell_session(&self, pty_id: &str) -> Option<Arc<crate::terminal::TerminalSession>> {
        self.shells.get().and_then(|host| host.session(pty_id))
    }

    /// Stop waits up to five seconds for Ctrl+C to land, so every control runs off the UI thread.
    pub fn shell_control(self: &Arc<Self>, shell_id: String, control: ShellControl) {
        let this = Arc::clone(self);
        std::thread::spawn(move || {
            let result = match control {
                ShellControl::Stop => this.core.stop_shell(&shell_id),
                ShellControl::Restart => this.core.restart_shell(&shell_id),
                ShellControl::Remove => this.core.remove_shell(&shell_id),
            };
            if let Err(error) = result {
                *this.shell_error.lock().unwrap() = Some(error);
            }
            this.publish();
        });
    }

    pub fn take_shell_error(&self) -> Option<String> {
        self.shell_error.lock().unwrap().take()
    }

    /// The last lines a shell printed: live while it runs, the kept tail once it ended.
    pub fn shell_tail(&self, shell_id: &str, lines: usize) -> Option<String> {
        self.core
            .shell_output(shell_id, lines)
            .ok()
            .and_then(|value| {
                value
                    .get("output")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
    }

    pub fn kill_shells(&self) {
        if let Some(host) = self.shells.get() {
            host.kill_all();
        }
    }

    fn background<F>(self: &Arc<Self>, work: F)
    where
        F: FnOnce(&Core) -> Result<(), String> + Send + 'static,
    {
        let this = Arc::clone(self);
        std::thread::spawn(move || {
            if let Err(error) = work(&this.core) {
                eprintln!("[orchestrator] {error}");
            }
            this.publish();
        });
    }
}

/// `ShellControl` minus the two the UI handles itself (play is a restart of a finished shell;
/// opening the terminal is the inspector).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellControl {
    Stop,
    Restart,
    Remove,
}

fn endpoint(port: u16) -> String {
    format!("http://{HOST}:{port}")
}

fn safe_name(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// TOML basic string.
fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn strip_managed_block(existing: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut skipping = false;
    for line in existing.lines() {
        match line.trim() {
            CODEX_MCP_MARK_START => skipping = true,
            CODEX_MCP_MARK_END => skipping = false,
            _ if !skipping => kept.push(line),
            _ => {}
        }
    }
    kept.join("\n")
}

/// The git top level of `cwd`, or `cwd` itself outside a repository.
fn repository_root(cwd: &str) -> PathBuf {
    let mut command = std::process::Command::new("git");
    command
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from(cwd))
}

/// The config carries the listener token, so other local users must not read it.
fn write_private(path: &Path, body: &[u8]) -> Result<(), String> {
    std::fs::write(path, body).map_err(|e| format!("write_failed:{e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_codex_block_is_replaced_not_duplicated() {
        let existing = format!(
            "model = \"o3\"\n\n{CODEX_MCP_MARK_START}\n[mcp_servers.alethe]\ncommand = \"x\"\n{CODEX_MCP_MARK_END}\n[other]\nk = 1\n"
        );
        let stripped = strip_managed_block(&existing);
        assert!(!stripped.contains("mcp_servers.alethe"));
        assert!(stripped.contains("model = \"o3\""));
        assert!(stripped.contains("[other]"));
    }

    #[test]
    fn toml_strings_escape_windows_paths() {
        assert_eq!(
            toml_string(r#"C:\Temp\a "b".ps1"#),
            r#""C:\\Temp\\a \"b\".ps1""#
        );
    }
}
