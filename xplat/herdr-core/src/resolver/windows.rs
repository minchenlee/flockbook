//! Windows process source on the `sysinfo` crate: pid, parent, start time (whole seconds, as `ps lstart`), the
//! command line and the environment (sysinfo reads it from the process; empty where access is denied), in the shape
//! `ps -E` prints, so [`super::agent_process`] works the same as on unix.
//!
//! Checked with `cargo check --target x86_64-pc-windows-gnu`; not run on Windows yet.

use super::{Proc, ProcessSource};
use chrono::{DateTime, Utc};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

pub struct WindowsProcessSource;

impl ProcessSource for WindowsProcessSource {
    // TODO(windows): `ps -x` lists this user's processes only; sysinfo lists every process it can open. Another
    // user's environment cannot be read, so such a process carries no pane env and never matches a TUI pane.
    fn processes(&self) -> Vec<Proc> {
        let mut sys = System::new();
        let kind = ProcessRefreshKind::new().with_cmd(UpdateKind::Always).with_environ(UpdateKind::Always);
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        let join = |v: &[std::ffi::OsString]| v.iter().map(|s| s.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ");
        sys.processes()
            .values()
            .filter_map(|p| {
                let start = DateTime::<Utc>::from_timestamp(p.start_time() as i64, 0)?;
                let cmd = join(p.cmd());
                let env = join(p.environ());
                let line = if env.is_empty() { cmd } else { format!("{cmd} {env}") };
                Some(Proc { pid: p.pid().as_u32() as i32, ppid: p.parent().map_or(0, |q| q.as_u32() as i32), start, line })
            })
            .collect()
    }

    // TODO(windows): no open-file listing (it needs NtQuerySystemInformation handle enumeration or the Restart
    // Manager). Fails closed: empty, so the open-log route finds nothing and only the pid-file (hc Claude), Grok
    // `active_sessions.json`, pi folder and Codex cwd + start routes can resolve a pane.
    fn open_files(&self, _pids: &[i32]) -> Vec<String> {
        Vec::new()
    }
}
