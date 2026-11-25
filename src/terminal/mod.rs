//! A terminal session: a real PTY (portable-pty, ConPTY on Windows) feeding an
//! `alacritty_terminal` emulator. Replaces `pty.rs` + xterm.js of the web build.

pub mod input;
pub mod monitor;
pub mod view;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Processor, Rgb, StdSyncHandler};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::theme::TermPalette;

/// `getTerminalScrollbackRows` of the web build at its economical tier: agents keep more history
/// than plain shells. alacritty grows the grid lazily, so unused history costs nothing.
pub fn scrollback_lines(agent: &str) -> usize {
    if matches!(agent, "shell" | "wsl") {
        3_000
    } else {
        6_000
    }
}

#[derive(Clone, Copy)]
pub struct TermSize {
    pub cols: usize,
    pub lines: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// Output in the last moments that the user did not cause by typing.
    Working,
    /// Alive and quiet.
    Waiting,
    /// The process ended.
    Exited,
}

/// Shared between the emulator (which raises events while parsing) and the UI.
struct Shared {
    writer: Mutex<Box<dyn Write + Send>>,
    title: Mutex<Option<String>>,
    palette: Mutex<[Rgb; 3]>,
    window: Mutex<WindowSize>,
    bell: AtomicBool,
    clipboard_store: Mutex<Option<String>>,
}

#[derive(Clone)]
pub struct Listener {
    shared: Arc<Shared>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => {
                if let Ok(mut w) = self.shared.writer.lock() {
                    let _ = w.write_all(text.as_bytes());
                    let _ = w.flush();
                }
            }
            Event::Title(title) => *self.shared.title.lock().unwrap() = Some(title),
            Event::ResetTitle => *self.shared.title.lock().unwrap() = None,
            Event::ColorRequest(index, format) => {
                // 256 = foreground, 257 = background, 258 = cursor; other slots answer with the
                // foreground, which is what the agents' theme probes care about.
                let [fg, bg, cursor] = *self.shared.palette.lock().unwrap();
                let color = match index {
                    257 => bg,
                    258 => cursor,
                    _ => fg,
                };
                let reply = format(color);
                if let Ok(mut w) = self.shared.writer.lock() {
                    let _ = w.write_all(reply.as_bytes());
                    let _ = w.flush();
                }
            }
            Event::TextAreaSizeRequest(format) => {
                let size = *self.shared.window.lock().unwrap();
                let reply = format(size);
                if let Ok(mut w) = self.shared.writer.lock() {
                    let _ = w.write_all(reply.as_bytes());
                    let _ = w.flush();
                }
            }
            Event::ClipboardStore(_, text) => {
                *self.shared.clipboard_store.lock().unwrap() = Some(text)
            }
            Event::Bell => self.shared.bell.store(true, Ordering::Relaxed),
            _ => {}
        }
    }
}

pub struct SpawnSpec {
    pub program_label: String,
    pub builder: CommandBuilder,
    pub cwd: String,
    pub size: TermSize,
    /// Typed into the agent once it settles (prompt from the Home composer).
    pub initial_input: Option<String>,
}

#[allow(dead_code)]
pub struct TerminalSession {
    pub term: Arc<Mutex<Term<Listener>>>,
    shared: Arc<Shared>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    exited: Arc<AtomicBool>,
    exit_code: Arc<AtomicI64>,
    /// Milliseconds since `epoch` of the last byte read from the PTY.
    last_output_ms: Arc<AtomicU64>,
    last_input: Mutex<Instant>,
    epoch: Instant,
    pub size: Mutex<TermSize>,
    pub label: String,
    pub started_at: Instant,
    pub pid: Option<u32>,
    /// Scroll position the view last rendered (lines above the live screen).
    pub had_output: Arc<AtomicBool>,
    /// Agents only: tells when an answer to a submitted prompt is complete.
    monitor: Option<Arc<Mutex<monitor::CompletionMonitor>>>,
}

fn rgb(c: crate::theme::Rgba8) -> Rgb {
    let [r, g, b, _] = c.0;
    Rgb { r, g, b }
}

impl TerminalSession {
    pub fn spawn(
        spec: SpawnSpec,
        palette: &TermPalette,
        repaint: egui::Context,
    ) -> Result<Arc<Self>, String> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: spec.size.lines.max(1) as u16,
                cols: spec.size.cols.max(1) as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;

        let mut builder = spec.builder;
        let cwd = if std::path::Path::new(&spec.cwd).is_dir() {
            spec.cwd.clone()
        } else {
            crate::storage::home_dir()
        };
        builder.cwd(&cwd);
        builder.env("ALETHE", "1");
        builder.env("TERM_PROGRAM", "Alethe");

        let child = pair
            .slave
            .spawn_command(builder)
            .map_err(|e| e.to_string())?;
        drop(pair.slave);
        let pid = child.process_id();
        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

        let shared = Arc::new(Shared {
            writer: Mutex::new(writer),
            title: Mutex::new(None),
            palette: Mutex::new([
                rgb(palette.foreground),
                rgb(palette.background),
                rgb(palette.cursor),
            ]),
            window: Mutex::new(WindowSize {
                num_lines: spec.size.lines as u16,
                num_cols: spec.size.cols as u16,
                cell_width: 8,
                cell_height: 16,
            }),
            bell: AtomicBool::new(false),
            clipboard_store: Mutex::new(None),
        });
        let listener = Listener {
            shared: Arc::clone(&shared),
        };
        let config = Config {
            scrolling_history: scrollback_lines(&spec.program_label),
            ..Config::default()
        };
        let term = Arc::new(Mutex::new(Term::new(config, &spec.size, listener)));

        let epoch = Instant::now();
        let exited = Arc::new(AtomicBool::new(false));
        let exit_code = Arc::new(AtomicI64::new(i64::MIN));
        let last_output_ms = Arc::new(AtomicU64::new(0));
        let had_output = Arc::new(AtomicBool::new(false));
        let monitor = (!matches!(spec.program_label.as_str(), "shell" | "wsl"))
            .then(|| Arc::new(Mutex::new(monitor::CompletionMonitor::default())));

        // Reader: PTY bytes -> emulator.
        {
            let term = Arc::clone(&term);
            let monitor = monitor.clone();
            let last_output_ms = Arc::clone(&last_output_ms);
            let had_output = Arc::clone(&had_output);
            let repaint = repaint.clone();
            std::thread::Builder::new()
                .name(format!("pty-read-{}", spec.program_label))
                .spawn(move || {
                    let mut processor: Processor<StdSyncHandler> = Processor::new();
                    let mut buf = vec![0u8; 64 * 1024];
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if let Ok(mut term) = term.lock() {
                                    processor.advance(&mut *term, &buf[..n]);
                                }
                                if let Some(monitor) = &monitor {
                                    if let Ok(mut monitor) = monitor.lock() {
                                        monitor.handle_output(&buf[..n]);
                                    }
                                }
                                last_output_ms
                                    .store(epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
                                had_output.store(true, Ordering::Relaxed);
                                repaint.request_repaint();
                            }
                        }
                    }
                })
                .map_err(|e| e.to_string())?;
        }

        // Waiter: owns the child, records how it ended.
        {
            let exited = Arc::clone(&exited);
            let exit_code = Arc::clone(&exit_code);
            let repaint = repaint.clone();
            let mut child = child;
            std::thread::Builder::new()
                .name(format!("pty-wait-{}", spec.program_label))
                .spawn(move || {
                    let status = child.wait();
                    if let Ok(status) = status {
                        exit_code.store(status.exit_code() as i64, Ordering::Relaxed);
                    }
                    exited.store(true, Ordering::Relaxed);
                    repaint.request_repaint();
                })
                .map_err(|e| e.to_string())?;
        }

        let session = Arc::new(Self {
            term,
            shared,
            master: Mutex::new(pair.master),
            killer: Mutex::new(killer),
            exited,
            exit_code,
            last_output_ms,
            last_input: Mutex::new(epoch),
            epoch,
            size: Mutex::new(spec.size),
            label: spec.program_label,
            started_at: Instant::now(),
            pid,
            had_output,
            monitor,
        });

        if let Some(text) = spec.initial_input.filter(|t| !t.trim().is_empty()) {
            Self::deliver_when_settled(Arc::downgrade(&session), text);
        }
        Ok(session)
    }

    /// Port of `agentPromptDelivery.ts`, simplified: wait for the agent's TUI to draw and go
    /// quiet, then paste the prompt and submit it.
    fn deliver_when_settled(session: std::sync::Weak<Self>, text: String) {
        std::thread::spawn(move || {
            let started = Instant::now();
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let Some(s) = session.upgrade() else { return };
                if s.is_exited() {
                    return;
                }
                let quiet = s.since_last_output() > Duration::from_millis(1200);
                let timed_out = started.elapsed() > Duration::from_secs(20);
                if (s.had_output.load(Ordering::Relaxed)
                    && quiet
                    && started.elapsed() > Duration::from_secs(2))
                    || timed_out
                {
                    s.paste(&text);
                    std::thread::sleep(Duration::from_millis(150));
                    s.write(b"\r");
                    return;
                }
            }
        });
    }

    /// True once per finished answer (see `monitor`). Polled by the app every frame.
    pub fn take_completion(&self) -> bool {
        let since_output = self.since_last_output();
        self.monitor
            .as_ref()
            .and_then(|m| m.lock().ok().map(|mut m| m.poll(since_output)))
            .unwrap_or(false)
    }

    /// A prompt was submitted and its answer has not been reported yet.
    pub fn completion_pending(&self) -> bool {
        !self.is_exited()
            && self
                .monitor
                .as_ref()
                .and_then(|m| m.lock().ok().map(|m| m.pending()))
                .unwrap_or(false)
    }

    pub fn write(&self, bytes: &[u8]) {
        *self.last_input.lock().unwrap() = Instant::now();
        if let Some(monitor) = &self.monitor {
            if let Ok(mut monitor) = monitor.lock() {
                monitor.handle_input(bytes);
            }
        }
        if let Ok(mut w) = self.shared.writer.lock() {
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
    }

    /// Pastes text, bracketed when the application asked for it.
    pub fn paste(&self, text: &str) {
        let bracketed = self
            .term
            .lock()
            .map(|t| {
                t.mode()
                    .contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE)
            })
            .unwrap_or(false);
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if bracketed {
            let clean = text.replace("\x1b[201~", "");
            self.write(format!("\x1b[200~{clean}\x1b[201~").as_bytes());
        } else {
            self.write(text.as_bytes());
        }
    }

    pub fn resize(&self, size: TermSize, cell_w: f32, cell_h: f32) {
        let mut current = self.size.lock().unwrap();
        if current.cols == size.cols && current.lines == size.lines {
            return;
        }
        *current = size;
        if let Ok(mut term) = self.term.lock() {
            term.resize(size);
        }
        *self.shared.window.lock().unwrap() = WindowSize {
            num_lines: size.lines as u16,
            num_cols: size.cols as u16,
            cell_width: cell_w.round() as u16,
            cell_height: cell_h.round() as u16,
        };
        if let Ok(master) = self.master.lock() {
            let _ = master.resize(PtySize {
                rows: size.lines as u16,
                cols: size.cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    pub fn set_palette(&self, palette: &TermPalette) {
        *self.shared.palette.lock().unwrap() = [
            rgb(palette.foreground),
            rgb(palette.background),
            rgb(palette.cursor),
        ];
    }

    /// Ends the whole process tree (shell wrapper, agent CLI, its MCP servers), off the UI thread.
    /// The tree goes first: once the wrapper shell is gone `taskkill /T` can no longer walk it.
    pub fn kill(&self) {
        if self.is_exited() {
            return;
        }
        let pid = self.pid;
        let mut killer = self.killer.lock().unwrap().clone_killer();
        std::thread::spawn(move || {
            if let Some(pid) = pid {
                crate::backend::process::kill_process_tree(pid);
            }
            let _ = killer.kill();
        });
    }

    /// Same as `kill`, but waits — used on app exit, when detached threads would not finish.
    pub fn kill_blocking(&self) {
        if self.is_exited() {
            return;
        }
        if let Some(pid) = self.pid {
            crate::backend::process::kill_process_tree(pid);
        }
        if let Ok(mut killer) = self.killer.lock() {
            let _ = killer.kill();
        }
    }

    /// The last line submitted from the keyboard (agents only), and when.
    pub fn last_submit(&self) -> Option<(Instant, String)> {
        self.monitor
            .as_ref()
            .and_then(|m| m.lock().ok().and_then(|m| m.last_submit()))
    }

    pub fn is_exited(&self) -> bool {
        self.exited.load(Ordering::Relaxed)
    }

    pub fn exit_code(&self) -> Option<i64> {
        let code = self.exit_code.load(Ordering::Relaxed);
        (code != i64::MIN).then_some(code)
    }

    #[allow(dead_code)]
    pub fn title(&self) -> Option<String> {
        self.shared.title.lock().unwrap().clone()
    }

    pub fn take_clipboard_store(&self) -> Option<String> {
        self.shared.clipboard_store.lock().unwrap().take()
    }

    #[allow(dead_code)]
    pub fn take_bell(&self) -> bool {
        self.shared.bell.swap(false, Ordering::Relaxed)
    }

    fn since_last_output(&self) -> Duration {
        let last = self.last_output_ms.load(Ordering::Relaxed);
        self.epoch
            .elapsed()
            .saturating_sub(Duration::from_millis(last))
    }

    pub fn status(&self) -> Status {
        if self.is_exited() {
            return Status::Exited;
        }
        let since_output = self.since_last_output();
        let since_input = self.last_input.lock().unwrap().elapsed();
        // Output that follows a keystroke within a moment is echo, not work.
        let echo = since_input < since_output + Duration::from_millis(400);
        if since_output < Duration::from_millis(1500)
            && !echo
            && self.had_output.load(Ordering::Relaxed)
        {
            Status::Working
        } else {
            Status::Waiting
        }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.kill();
    }
}
