//! Process-tree teardown, ported from `alethe-agents/src-tauri/src/pty.rs`
//! (`kill_process_tree`, `install_kill_on_close_guard`). AGPL-3.0-or-later.
//!
//! Agents run behind a shell (`pwsh -Command & 'claude.cmd'`), so killing the PTY child alone
//! would orphan node/claude/codex and their MCP servers.

use std::sync::OnceLock;

#[cfg(windows)]
pub fn kill_process_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("taskkill");
    command.args(["/F", "/T", "/PID", &pid.to_string()]);
    command.creation_flags(CREATE_NO_WINDOW);
    let _ = command.output();
}

/// The `kill(2)` target for the process group led by `pid`. 0 and 1 are refused: as group ids
/// they mean "our own group" and "every process we may signal".
#[cfg(not(windows))]
fn process_group_target(pid: u32) -> Option<i32> {
    i32::try_from(pid).ok().filter(|&pid| pid > 1).map(|pid| -pid)
}

#[cfg(not(windows))]
pub fn kill_process_tree(pid: u32) {
    // portable-pty calls setsid(), so the shell leads its own process group.
    let Some(group) = process_group_target(pid) else { return };
    unsafe { libc::kill(group, libc::SIGTERM) };
    std::thread::sleep(std::time::Duration::from_millis(200));
    unsafe { libc::kill(group, libc::SIGKILL) };
}

static JOB_GUARD_ACTIVE: OnceLock<bool> = OnceLock::new();

/// Puts the app in a Job Object with KILL_ON_JOB_CLOSE: every terminal and its descendants
/// inherit the job, so they die with the app even if it crashes.
#[cfg(windows)]
pub fn install_kill_on_close_guard() {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let active = unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            false
        } else {
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                let _ = CloseHandle(job);
                false
            } else if AssignProcessToJobObject(job, GetCurrentProcess()) != 0 {
                // The handle is leaked on purpose: closing it is what kills the tree.
                true
            } else {
                eprintln!("[process] AssignProcessToJobObject failed — orphan guard inactive");
                let _ = CloseHandle(job);
                false
            }
        }
    };
    let _ = JOB_GUARD_ACTIVE.set(active);
}

#[cfg(not(windows))]
pub fn install_kill_on_close_guard() {
    let _ = JOB_GUARD_ACTIVE.set(true);
}
