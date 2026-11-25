//! Memory monitor (`resources.rs` / `useResourceSupervisor` upstream, in its "only monitor" mode).
//!
//! Every 3 s one background thread samples the process table and the system's free memory. It
//! splits the app's process tree into the app itself and each terminal's process tree, and rates
//! the system's pressure. It never stops anything on its own: under critical pressure the app
//! warns, and suspending a group or project to free memory is the user's call.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pressure {
    #[default]
    Normal,
    Warning,
    Critical,
}

#[derive(Clone, Debug, Default)]
pub struct MemorySnapshot {
    /// The app's own process (and helpers that are not a terminal's, like the console hosts).
    pub app_mb: u64,
    /// Each terminal tab's process tree, by tab id, largest first.
    pub tabs: Vec<(String, u64)>,
    pub system_total_mb: u64,
    pub system_available_mb: u64,
    pub pressure: Pressure,
}

impl MemorySnapshot {
    pub fn terminals_mb(&self) -> u64 {
        self.tabs.iter().map(|(_, mb)| mb).sum()
    }
}

pub struct ResourceMonitor {
    /// (tab id, root pid) of every running terminal.
    roots: Arc<Mutex<Vec<(String, u32)>>>,
    snapshot: Arc<Mutex<MemorySnapshot>>,
}

impl ResourceMonitor {
    /// `total_mb` keeps the title bar pill's figure: the whole tree, app and terminals.
    pub fn start(total_mb: Arc<AtomicU64>, ctx: egui::Context) -> Self {
        let monitor = Self {
            roots: Arc::default(),
            snapshot: Arc::default(),
        };
        let roots = Arc::clone(&monitor.roots);
        let snapshot = Arc::clone(&monitor.snapshot);
        let _ = std::thread::Builder::new()
            .name("memory-probe".into())
            .spawn(move || {
                use sysinfo::{MemoryRefreshKind, ProcessRefreshKind, ProcessesToUpdate, System};
                let me = std::process::id();
                let mut system = System::new();
                let mut pressure = Pressure::Normal;
                loop {
                    system.refresh_processes_specifics(
                        ProcessesToUpdate::All,
                        true,
                        ProcessRefreshKind::nothing().with_memory(),
                    );
                    system.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
                    let tab_roots: HashMap<u32, String> = roots
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|(tab, pid)| (*pid, tab.clone()))
                        .collect();
                    let processes = system.processes();
                    let mut app = 0u64;
                    let mut tabs: HashMap<String, u64> = HashMap::new();
                    for (pid, process) in processes {
                        // Walk up to the first terminal root, or to the app itself.
                        let mut cursor = Some(*pid);
                        let mut owner: Option<Option<&String>> = None;
                        for _ in 0..32 {
                            let Some(current) = cursor else {
                                break;
                            };
                            let raw = current.as_u32();
                            if let Some(tab) = tab_roots.get(&raw) {
                                owner = Some(Some(tab));
                                break;
                            }
                            if raw == me {
                                owner = Some(None);
                                break;
                            }
                            cursor = processes.get(&current).and_then(|p| p.parent());
                        }
                        match owner {
                            Some(Some(tab)) => {
                                *tabs.entry(tab.clone()).or_default() += process.memory()
                            }
                            Some(None) => app += process.memory(),
                            None => {}
                        }
                    }
                    const MB: u64 = 1024 * 1024;
                    let mut tabs: Vec<(String, u64)> = tabs
                        .into_iter()
                        .map(|(tab, bytes)| (tab, bytes / MB))
                        .collect();
                    tabs.sort_by(|a, b| b.1.cmp(&a.1));
                    let total_mb_now = system.total_memory() / MB;
                    let available_mb = system.available_memory() / MB;
                    pressure = pressure_level(total_mb_now, available_mb, pressure);
                    let next = MemorySnapshot {
                        app_mb: app / MB,
                        tabs,
                        system_total_mb: total_mb_now,
                        system_available_mb: available_mb,
                        pressure,
                    };
                    total_mb.store(next.app_mb + next.terminals_mb(), Ordering::Relaxed);
                    *snapshot.lock().unwrap() = next;
                    ctx.request_repaint();
                    std::thread::sleep(Duration::from_secs(3));
                }
            });
        monitor
    }

    pub fn set_roots(&self, roots: Vec<(String, u32)>) {
        *self.roots.lock().unwrap() = roots;
    }

    pub fn snapshot(&self) -> MemorySnapshot {
        self.snapshot.lock().unwrap().clone()
    }
}

/// `pressure_level` upstream: critical under 5 % free (at least 512 MB), warning under 10 % (at
/// least 1 GB), with 25 % hysteresis so the level doesn't flap at the edge.
pub fn pressure_level(total_mb: u64, available_mb: u64, previous: Pressure) -> Pressure {
    let total = total_mb.max(1) as f64;
    let available = available_mb as f64;
    let critical_at = (total * 0.05).max(512.0);
    let warning_at = (total * 0.10).max(1024.0);
    if available <= critical_at
        || (previous == Pressure::Critical && available <= critical_at * 1.25)
    {
        Pressure::Critical
    } else if available <= warning_at
        || (previous == Pressure::Warning && available <= warning_at * 1.25)
    {
        Pressure::Warning
    } else {
        Pressure::Normal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_levels_with_hysteresis() {
        // 16 GB: critical at 819 MB free, warning at 1638 MB.
        let total = 16 * 1024;
        assert_eq!(
            pressure_level(total, 4000, Pressure::Normal),
            Pressure::Normal
        );
        assert_eq!(
            pressure_level(total, 1500, Pressure::Normal),
            Pressure::Warning
        );
        assert_eq!(
            pressure_level(total, 800, Pressure::Warning),
            Pressure::Critical
        );
        // Back above the line, but within 25 %: still critical.
        assert_eq!(
            pressure_level(total, 900, Pressure::Critical),
            Pressure::Critical
        );
        assert_eq!(
            pressure_level(total, 1900, Pressure::Warning),
            Pressure::Warning
        );
        assert_eq!(
            pressure_level(total, 2100, Pressure::Warning),
            Pressure::Normal
        );
        // Small machines: at least 512 MB / 1 GB.
        assert_eq!(
            pressure_level(4096, 600, Pressure::Normal),
            Pressure::Warning
        );
    }
}
