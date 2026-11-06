//! Linux process source: plain file reads under a proc root (`/proc` by default): `<pid>/stat`, `cmdline`,
//! `environ`, `cwd` and `fd/*`. It compiles on every platform (the uid filter and the clock tick rate need libc,
//! so they are unix only), and checks run it against a synthetic tree.

use super::{Proc, ProcessSource};
use chrono::{DateTime, Utc};
use std::path::PathBuf;

pub struct LinuxProcSource {
    root: PathBuf,
}

impl LinuxProcSource {
    pub fn new() -> Self {
        Self::with_root(PathBuf::from("/proc"))
    }

    pub fn with_root(root: PathBuf) -> Self {
        LinuxProcSource { root }
    }

    fn boot_time(&self) -> Option<i64> {
        let s = std::fs::read_to_string(self.root.join("stat")).ok()?;
        s.lines().find_map(|l| l.strip_prefix("btime ")).and_then(|v| v.trim().parse().ok())
    }
}

impl Default for LinuxProcSource {
    fn default() -> Self {
        Self::new()
    }
}

/// `(ppid, starttime in clock ticks after boot, comm)` from `/proc/<pid>/stat`. `comm` is in parentheses and may
/// hold spaces and parentheses itself: the fields follow the last `)`.
pub(crate) fn parse_stat(stat: &str) -> Option<(i32, u64, String)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let comm = stat.get(open + 1..close)?.to_string();
    let f: Vec<&str> = stat[close + 1..].split_whitespace().collect();
    // f[0] is field 3 (state); ppid is field 4, starttime field 22.
    Some((f.get(1)?.parse().ok()?, f.get(19)?.parse().ok()?, comm))
}

/// `(ppid, starttime in clock ticks since boot)` from `/proc/<pid>/stat`.
pub fn parse_linux_stat(text: &str) -> Option<(i32, u64)> {
    parse_stat(text).map(|(ppid, ticks, _)| (ppid, ticks))
}

fn nul_joined(b: &[u8]) -> String {
    b.split(|c| *c == 0).filter(|w| !w.is_empty()).map(String::from_utf8_lossy).collect::<Vec<_>>().join(" ")
}

/// Clock ticks per second (`USER_HZ`); 100 where it cannot be asked.
fn ticks_per_sec() -> u64 {
    #[cfg(unix)]
    {
        // SAFETY: a plain libc query without arguments that can fail only by returning -1.
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if hz > 0 {
            return hz as u64;
        }
    }
    100
}

/// Clock ticks after boot as a whole-second time, as [`LinuxProcSource::processes`] dates a process.
pub(crate) fn ticks_to_date(ticks: u64) -> Option<DateTime<Utc>> {
    let btime = LinuxProcSource::new().boot_time()?;
    DateTime::<Utc>::from_timestamp(btime + (ticks / ticks_per_sec()) as i64, 0)
}

/// `ps -x`: this user's processes only. Off unix every entry counts (the tree is synthetic there).
#[cfg(unix)]
fn mine(e: &std::fs::DirEntry) -> bool {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    e.metadata().map(|m| m.uid() == uid).unwrap_or(false)
}
#[cfg(not(unix))]
fn mine(_: &std::fs::DirEntry) -> bool {
    true
}

impl ProcessSource for LinuxProcSource {
    fn processes(&self) -> Vec<Proc> {
        let Some(btime) = self.boot_time() else { return vec![] };
        let hz = ticks_per_sec();
        let Ok(rd) = std::fs::read_dir(&self.root) else { return vec![] };
        let mut out = vec![];
        for e in rd.filter_map(|e| e.ok()) {
            let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
            if !mine(&e) {
                continue;
            }
            let base = e.path();
            let Some((ppid, ticks, comm)) = std::fs::read_to_string(base.join("stat")).ok().and_then(|s| parse_stat(&s)) else {
                continue;
            };
            // Whole seconds, as `ps` prints `lstart`.
            let Some(start) = DateTime::<Utc>::from_timestamp(btime + (ticks / hz) as i64, 0) else { continue };
            let mut line = std::fs::read(base.join("cmdline")).map(|b| nul_joined(&b)).unwrap_or_default();
            if line.is_empty() {
                line = format!("[{comm}]");
            }
            // Another user's environ is unreadable: the process is listed but carries no pane env.
            let env = std::fs::read(base.join("environ")).map(|b| nul_joined(&b)).unwrap_or_default();
            if !env.is_empty() {
                line = line + " " + &env;
            }
            out.push(Proc { pid, ppid, start, line });
        }
        out
    }

    fn open_files(&self, pids: &[i32]) -> Vec<String> {
        let mut out = vec![];
        for pid in pids {
            let base = self.root.join(pid.to_string());
            let mut links = vec![base.join("cwd")];
            if let Ok(rd) = std::fs::read_dir(base.join("fd")) {
                links.extend(rd.filter_map(|e| e.ok()).map(|e| e.path()));
            }
            for l in links {
                if let Ok(t) = std::fs::read_link(&l) {
                    let s = t.to_string_lossy().into_owned();
                    if s.starts_with('/') {
                        out.push(s);
                    }
                }
            }
        }
        out
    }
}
