//! Finds the log of an agent pane that herdr reports without `agent_session` (port of `SessionResolver.swift`).
//! Read-only: one process-table read, one open-file listing, and the first line of candidate logs. It never sends
//! anything to a pane.
//!
//! 1. The pane's process: the env holds `HERDR_PANE_ID` and `HERDR_SOCKET_PATH` (the session), so panes of other
//!    herdr sessions never match. The agent is the newest child of the pane's shell.
//! 2. A log file that the agent's process tree holds open. Exactly one, or none.
//! 3. Codex only: the rollout whose `session_meta` cwd is the pane's cwd and that started at or after the agent.
//!    Exactly one unclaimed match, or none. It never guesses between two.
//!
//! The pure logic (table parsing, matching, path rules) is platform-free. The process table and the open-file list
//! come from a [`ProcessSource`]: `ps`/`lsof` on macOS, `/proc` on Linux, the `sysinfo` crate on Windows.
//! Times are seconds since the Unix epoch (`f64`), like `util::parse_time`.

use chrono::{Duration as ChronoDuration, Local, NaiveDate, NaiveDateTime, TimeZone};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub mod lead;
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub use linux::{parse_linux_stat, LinuxProcSource};
#[cfg(target_os = "macos")]
pub use macos::MacProcessSource;
#[cfg(windows)]
pub use windows::WindowsProcessSource;

/// One process. `line` is the command line followed by the environment (what `ps -E` prints), so that
/// [`agent_process`] works the same on every platform.
#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    /// Start time, seconds since the Unix epoch.
    pub start: f64,
    pub line: String,
}

/// A pane that needs a log path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Need {
    pub pane_id: String,
    pub kind: String,
    pub cwd: String,
}

/// Where the process table and the open files come from. Read-only.
pub trait ProcessSource: Send + Sync {
    fn processes(&self) -> Vec<Proc>;
    /// Absolute paths that the given processes hold open. Empty where the platform cannot list them.
    fn open_files(&self, pids: &[i32]) -> Vec<String>;
}

/// The source of the host platform. Other platforms get an empty one (nothing resolves).
pub fn default_source() -> Box<dyn ProcessSource> {
    #[cfg(target_os = "macos")]
    return Box::new(MacProcessSource);
    #[cfg(target_os = "linux")]
    return Box::new(LinuxProcSource::new());
    #[cfg(windows)]
    return Box::new(WindowsProcessSource);
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    return Box::new(NoProcessSource);
}

/// A source that sees nothing.
pub struct NoProcessSource;
impl ProcessSource for NoProcessSource {
    fn processes(&self) -> Vec<Proc> {
        Vec::new()
    }
    fn open_files(&self, _: &[i32]) -> Vec<String> {
        Vec::new()
    }
}

// MARK: - Where each agent keeps its logs

/// Log roots per agent. Injected so tests never read a real home directory.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRoots {
    pub claude_projects: PathBuf,
    pub codex_sessions: PathBuf,
    pub cursor_projects: PathBuf,
    /// pi sessions: `PI_CODING_AGENT_SESSION_DIR`, else `<PI_CODING_AGENT_DIR>/sessions`, else `~/.pi/agent/sessions`.
    pub pi_sessions: PathBuf,
    pub grok_sessions: PathBuf,
}

impl AgentRoots {
    pub fn from_home(home: &Path) -> Self {
        Self::from_env(&HashMap::new(), home)
    }

    /// Honors `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `PI_CODING_AGENT_DIR` and `PI_CODING_AGENT_SESSION_DIR`.
    /// Empty values count as unset.
    pub fn from_env(env: &HashMap<String, String>, home: &Path) -> Self {
        let get = |k: &str| env.get(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let claude = get("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude"));
        let codex = get("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
        let pi = get("PI_CODING_AGENT_SESSION_DIR")
            .unwrap_or_else(|| get("PI_CODING_AGENT_DIR").unwrap_or_else(|| home.join(".pi").join("agent")).join("sessions"));
        AgentRoots {
            claude_projects: claude.join("projects"),
            codex_sessions: codex.join("sessions"),
            cursor_projects: home.join(".cursor").join("projects"),
            pi_sessions: pi,
            grok_sessions: get("GROK_HOME").unwrap_or_else(|| home.join(".grok")).join("sessions"),
        }
    }

    /// The search root for one agent kind (`"claude"` and any unknown kind use Claude's).
    pub fn dir_for(&self, agent: &str) -> PathBuf {
        match agent {
            "codex" => self.codex_sessions.clone(),
            "cursor" => self.cursor_projects.clone(),
            "pi" => self.pi_sessions.clone(),
            "grok" => self.grok_sessions.clone(),
            _ => self.claude_projects.clone(),
        }
    }

    pub fn from_process() -> Self {
        let env: HashMap<String, String> = std::env::vars().collect();
        Self::from_env(&env, &home_dir())
    }
}

/// `HOME`, or `USERPROFILE` on Windows.
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default()
}

// MARK: - The resolver

struct Entry {
    pid: Option<i32>,
    start: Option<f64>,
    need: Need,
    path: Option<PathBuf>,
    checked: f64,
}

/// Caches the result per pane and keeps it while the agent pid stays the same.
pub struct SessionResolver {
    cache: HashMap<String, Entry>,
    socket_path: String,
    codex_dir: PathBuf,
    roots: AgentRoots,
    source: Box<dyn ProcessSource>,
    /// Unresolved panes are tried again after this many seconds; resolved ones are re-checked after `recheck`.
    retry: f64,
    recheck: f64,
}

impl SessionResolver {
    pub fn new(socket_path: impl Into<String>, codex_dir: PathBuf, roots: AgentRoots, source: Box<dyn ProcessSource>) -> Self {
        SessionResolver { cache: HashMap::new(), socket_path: socket_path.into(), codex_dir, roots, source, retry: 5.0, recheck: 30.0 }
    }

    /// Host platform source, roots from the environment.
    pub fn for_host(socket_path: impl Into<String>) -> Self {
        let roots = AgentRoots::from_process();
        Self::new(socket_path, roots.codex_sessions.clone(), roots, default_source())
    }

    pub fn with_intervals(mut self, retry: f64, recheck: f64) -> Self {
        self.retry = retry;
        self.recheck = recheck;
        self
    }

    /// Log paths for the given panes (pane id to `.jsonl` path). `claimed`: session ids or paths that other panes
    /// already use; they are never handed out again. `now`: seconds since the epoch.
    pub fn resolve(&mut self, needs: &[Need], claimed: &HashSet<String>, now: f64) -> HashMap<String, PathBuf> {
        let ids: HashSet<&str> = needs.iter().map(|n| n.pane_id.as_str()).collect();
        self.cache.retain(|k, _| ids.contains(k.as_str()));
        let due: Vec<&Need> = needs
            .iter()
            .filter(|n| match self.cache.get(&n.pane_id) {
                None => true,
                Some(e) if e.need != **n => true,
                Some(e) => now - e.checked >= if e.path.is_none() { self.retry } else { self.recheck },
            })
            .collect();
        if !due.is_empty() {
            let procs = self.source.processes();
            for n in due {
                let agent = agent_process(&n.pane_id, &self.socket_path, &procs);
                if let Some(old) = self.cache.get_mut(&n.pane_id) {
                    if old.path.is_some() && old.pid.is_some() && old.pid == agent.as_ref().map(|a| a.pid) && old.start == agent.as_ref().map(|a| a.start) && old.need == *n {
                        old.checked = now;
                        continue;
                    }
                }
                let mut taken = claimed.clone();
                for (p, e) in &self.cache {
                    if p != &n.pane_id {
                        if let Some(path) = &e.path {
                            taken.insert(path.to_string_lossy().into_owned());
                        }
                    }
                }
                let path = agent.as_ref().and_then(|a| self.find(n, a, &procs, &taken, now));
                self.cache.insert(n.pane_id.clone(), Entry { pid: agent.as_ref().map(|a| a.pid), start: agent.as_ref().map(|a| a.start), need: n.clone(), path, checked: now });
            }
        }
        needs.iter().filter_map(|n| self.cache.get(&n.pane_id).and_then(|e| e.path.clone()).map(|p| (n.pane_id.clone(), p))).collect()
    }

    fn find(&self, n: &Need, agent: &Proc, procs: &[Proc], claimed: &HashSet<String>, now: f64) -> Option<PathBuf> {
        let tree = subtree(agent.pid, procs);
        let open = self.source.open_files(&tree);
        if n.kind == "grok" {
            let home = grok_home(&agent.line);
            if let Some(p) = grok_log_path(&open, &home).filter(|p| !is_claimed(&p.to_string_lossy(), claimed)) { return Some(p); }
            let data = std::fs::read(home.join("active_sessions.json")).ok()?;
            let id = grok_session(&data, &tree)?;
            let mut hits = Vec::new();
            for cwd in std::fs::read_dir(home.join("sessions")).ok()?.flatten() {
                let p = cwd.path().join(&id).join("updates.jsonl");
                if p.is_file() && !is_claimed(&p.to_string_lossy(), claimed) { hits.push(p); }
            }
            return if hits.len() == 1 { hits.pop() } else { None };
        }
        if let Some(p) = log_path(&open, &n.kind, &self.roots) {
            if !is_claimed(&p.to_string_lossy(), claimed) {
                return Some(p);
            }
        }
        if n.kind != "codex" {
            return None;
        }
        let mut c: Vec<PathBuf> =
            codex_candidates(&self.codex_dir, &n.cwd, agent.start, now).into_iter().filter(|p| !is_claimed(&p.to_string_lossy(), claimed)).collect();
        if c.len() == 1 {
            c.pop()
        } else {
            None
        }
    }
}

// MARK: - Pure parts

/// Splits off the first `n` whitespace-separated words; the rest keeps its inner spacing.
fn split_words(s: &str, n: usize) -> Option<(Vec<&str>, &str)> {
    let mut rest = s;
    let mut words = Vec::with_capacity(n);
    for _ in 0..n {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        words.push(&rest[..end]);
        rest = &rest[end..];
    }
    let rest = rest.trim_start();
    if rest.is_empty() {
        None
    } else {
        Some((words, rest))
    }
}

fn local_epoch(t: NaiveDateTime) -> Option<f64> {
    Local.from_local_datetime(&t).earliest().map(|d| d.timestamp() as f64)
}

/// `ps -E -ww -o pid=,ppid=,lstart=,command=` lines. `lstart` is five words in the C locale, in local time.
pub fn parse_ps(text: &str) -> Vec<Proc> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let Some((w, line)) = split_words(raw, 7) else { continue };
        let (Ok(pid), Ok(ppid)) = (w[0].parse::<i32>(), w[1].parse::<i32>()) else { continue };
        let Ok(t) = NaiveDateTime::parse_from_str(&w[2..7].join(" "), "%a %b %d %H:%M:%S %Y") else { continue };
        let Some(start) = local_epoch(t) else { continue };
        out.push(Proc { pid, ppid, start, line: line.to_string() });
    }
    out
}

/// `lsof -Fn` output: file name lines start with `n/`.
pub fn parse_lsof(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.strip_prefix("n/").map(|r| format!("/{r}"))).collect()
}

/// The agent process of a pane: the newest direct child of the pane's shell (the topmost process that carries the
/// pane's env). Without a child, the topmost process itself (a pane started with the agent as its command).
pub fn agent_process(pane_id: &str, socket_path: &str, procs: &[Proc]) -> Option<Proc> {
    let pane = format!(" HERDR_PANE_ID={pane_id} ");
    let sock = format!(" HERDR_SOCKET_PATH={socket_path} ");
    let mine: Vec<&Proc> = procs
        .iter()
        .filter(|p| {
            let l = format!("{} ", p.line);
            l.contains(&pane) && l.contains(&sock)
        })
        .collect();
    let pids: HashSet<i32> = mine.iter().map(|p| p.pid).collect();
    let roots: Vec<&Proc> = mine.iter().copied().filter(|p| !pids.contains(&p.ppid)).collect();
    let root_pids: HashSet<i32> = roots.iter().map(|p| p.pid).collect();
    let kids: Vec<&Proc> = mine.iter().copied().filter(|p| root_pids.contains(&p.ppid)).collect();
    let pool = if kids.is_empty() { roots } else { kids };
    // Newest start; the first one wins a tie (like Swift's `max(by:)`).
    pool.into_iter().fold(None::<&Proc>, |best, p| match best {
        Some(b) if b.start >= p.start => Some(b),
        _ => Some(p),
    }).cloned()
}

/// The pid and all its descendants, breadth first.
pub fn subtree(pid: i32, procs: &[Proc]) -> Vec<i32> {
    let mut out = vec![pid];
    let mut i = 0;
    while i < out.len() {
        let p = out[i];
        for q in procs {
            if q.ppid == p && !out.contains(&q.pid) {
                out.push(q.pid);
            }
        }
        i += 1;
    }
    out
}

/// The one agent log among open files, or `None` when there is none or more than one.
pub fn log_path(open_files: &[String], kind: &str, roots: &AgentRoots) -> Option<PathBuf> {
    if kind == "grok" { return grok_log_path(open_files, roots.grok_sessions.parent()?); }
    let hits: BTreeSet<PathBuf> = open_files
        .iter()
        .map(PathBuf::from)
        .filter(|p| {
            if p.extension().map_or(true, |e| e != "jsonl") || p.components().any(|c| c == Component::ParentDir) {
                return false;
            }
            match kind {
                "codex" => {
                    starts_with_root(p, &roots.codex_sessions)
                        && p.file_name().map_or(false, |n| n.to_string_lossy().starts_with("rollout-"))
                }
                "cursor" => {
                    starts_with_root(p, &roots.cursor_projects) && p.components().any(|c| c.as_os_str() == "agent-transcripts")
                }
                "pi" => starts_with_root(p, &roots.pi_sessions),
                _ => starts_with_root(p, &roots.claude_projects),
            }
        })
        .collect();
    if hits.len() == 1 {
        hits.into_iter().next()
    } else {
        None
    }
}

/// A path counts as claimed when it, or its session id (the file name ends in `<id>.jsonl`), is in `claimed`.
pub fn is_claimed(path: &str, claimed: &HashSet<String>) -> bool {
    if claimed.contains(path) {
        return true;
    }
    if Path::new(path).file_name().is_some_and(|n| n == "updates.jsonl") && Path::new(path).parent().and_then(Path::file_name).is_some_and(|id| claimed.contains(id.to_string_lossy().as_ref())) { return true; }
    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    claimed.iter().any(|c| !c.is_empty() && !c.contains('/') && !c.contains('\\') && (name == format!("{c}.jsonl") || name.ends_with(&format!("-{c}.jsonl"))))
}

/// `starts_with` on whole path components; case-insensitive on Windows.
fn starts_with_root(p: &Path, root: &Path) -> bool {
    starts_with_root_with(p, root, cfg!(windows))
}

/// Pure form of [`starts_with_root`]: `ci` compares components ignoring case.
pub fn starts_with_root_with(p: &Path, root: &Path, ci: bool) -> bool {
    if !ci {
        return p.starts_with(root);
    }
    let lower = |x: &Path| -> Vec<String> { x.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect() };
    let (pc, rc) = (lower(p), lower(root));
    pc.len() >= rc.len() && pc[..rc.len()] == rc[..]
}

/// Lexical normalization like `standardizingPath`: expands a leading `~`, drops `.`, resolves `..`, ignores a trailing
/// separator, and strips `/private` from `/private/tmp`, `/private/var` and `/private/etc`. Lowercases on Windows.
fn standardize(p: &str) -> PathBuf {
    standardize_with(p, &home_dir(), cfg!(windows))
}

/// Pure form of [`standardize`]: `home` expands `~`, `ci` lowercases.
pub fn standardize_with(p: &str, home: &Path, ci: bool) -> PathBuf {
    let expanded: PathBuf = if p == "~" {
        home.to_path_buf()
    } else if let Some(rest) = p.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(p)
    };
    let mut out = PathBuf::new();
    for c in expanded.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    // `/private/tmp` etc. are the same places as `/tmp` etc. (macOS firmlinks); Foundation strips the prefix.
    if let Ok(rest) = out.strip_prefix("/private") {
        let first = rest.components().next().and_then(|c| c.as_os_str().to_str());
        if matches!(first, Some("tmp" | "var" | "etc")) {
            out = Path::new("/").join(rest);
        }
    }
    if ci {
        PathBuf::from(out.to_string_lossy().to_lowercase())
    } else {
        out
    }
}

/// Codex rollouts (`YYYY/MM/DD/rollout-<local time>-<id>.jsonl`) that started at or after `after` (2 s slack:
/// process start times have whole seconds) and whose `session_meta` cwd is `cwd`. Looks at the day folders from
/// `after` on (at most 8). Sorted by path.
pub fn codex_candidates(dir: &Path, cwd: &str, after: f64, now: f64) -> Vec<PathBuf> {
    let lower = after - 2.0;
    let want = standardize(cwd);
    let mut out = Vec::new();
    let Some(first) = Local.timestamp_opt(lower.floor() as i64, 0).earliest() else { return out };
    let mut date: NaiveDate = first.date_naive();
    for _ in 0..8 {
        let Some(day_start) = date.and_hms_opt(0, 0, 0).and_then(local_epoch) else { break };
        if day_start > now {
            break;
        }
        let folder = dir.join(date.format("%Y").to_string()).join(date.format("%m").to_string()).join(date.format("%d").to_string());
        if let Ok(rd) = std::fs::read_dir(&folder) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !(name.starts_with("rollout-") && name.ends_with(".jsonl")) {
                    continue;
                }
                let Some(stamp) = name.get(8..27) else { continue };
                let Ok(t) = NaiveDateTime::parse_from_str(stamp, "%Y-%m-%dT%H-%M-%S") else { continue };
                if local_epoch(t).map_or(true, |t| t < lower) {
                    continue;
                }
                let path = folder.join(&name);
                let Some((c, start)) = codex_meta(&path) else { continue };
                if standardize(&c) == want && start >= lower {
                    out.push(path);
                }
            }
        }
        date += ChronoDuration::days(1);
    }
    out.sort();
    out
}

/// cwd and start time (epoch seconds) from a rollout's first line (`session_meta`). Reads at most 1 MB.
pub fn codex_meta(path: &Path) -> Option<(String, f64)> {
    let mut data = Vec::new();
    std::fs::File::open(path).ok()?.take(1 << 20).read_to_end(&mut data).ok()?;
    if data.is_empty() {
        return None;
    }
    let end = data.iter().position(|b| *b == b'\n').unwrap_or(data.len());
    let o: serde_json::Value = serde_json::from_slice(&data[..end]).ok()?;
    if o.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let p = o.get("payload")?;
    let cwd = p.get("cwd")?.as_str()?;
    let ts = p.get("timestamp").and_then(|v| v.as_str()).or_else(|| o.get("timestamp").and_then(|v| v.as_str()))?;
    Some((cwd.to_string(), crate::util::parse_time(ts)?))
}

pub fn grok_home(process: &str) -> PathBuf {
    process.split_whitespace().find_map(|s| s.strip_prefix("GROK_HOME=").filter(|s| !s.is_empty()).map(PathBuf::from)).unwrap_or_else(|| home_dir().join(".grok"))
}
pub fn grok_log_path(open: &[String], home: &Path) -> Option<PathBuf> {
    let root = home.join("sessions");
    let hits: BTreeSet<_> = open.iter().map(PathBuf::from).filter_map(|p| {
        if p.components().any(|c| c == Component::ParentDir) { return None; }
        let rel = p.strip_prefix(&root).ok()?;
        if rel.components().count() != 3 || !matches!(p.file_name()?.to_str()?, "events.jsonl" | "updates.jsonl") { return None; }
        Some(p.parent()?.join("updates.jsonl"))
    }).collect();
    if hits.len() == 1 { hits.into_iter().next() } else { None }
}
pub fn grok_session(data: &[u8], pids: &[i32]) -> Option<String> {
    let list: serde_json::Value = serde_json::from_slice(data).ok()?;
    let ids: BTreeSet<_> = list.as_array()?.iter().filter(|v| v.get("pid").and_then(|v| v.as_i64()).is_some_and(|p| pids.contains(&(p as i32))))
        .filter_map(|v| v.get("session_id")?.as_str().filter(|s| !s.is_empty() && !s.contains(['/', '\\']) && *s != "." && *s != "..").map(str::to_owned)).collect();
    if ids.len() == 1 { ids.into_iter().next() } else { None }
}

#[cfg(test)]
mod grok_resolution_tests {
    use super::*;
    #[test]
    fn events_file_resolves_to_updates_and_ambiguous_sessions_are_hidden() {
        let root = Path::new("/synthetic/grok");
        assert_eq!(grok_log_path(&["/synthetic/grok/sessions/cwd/id/events.jsonl".into()], root), Some(root.join("sessions/cwd/id/updates.jsonl")));
        assert!(grok_log_path(&["/synthetic/grok/sessions/cwd/id/events.jsonl".into(), "/synthetic/grok/sessions/cwd/other/events.jsonl".into()], root).is_none());
        assert_eq!(grok_session(br#"[{"pid":4,"session_id":"one"}]"#, &[4]), Some("one".into()));
        assert!(grok_session(br#"[{"pid":4,"session_id":"one"},{"pid":5,"session_id":"two"}]"#, &[4,5]).is_none());
        assert!(grok_session(br#"[{"pid":4,"session_id":"../escape"}]"#, &[4]).is_none());
    }
}
