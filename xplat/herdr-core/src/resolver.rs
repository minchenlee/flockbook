//! Finds the log of an agent pane that herdr reports without `agent_session` (Swift: `SessionResolver.swift`).
//! Read-only: one process listing, the open files of the agent's process tree, and the first line of candidate logs.
//! It never sends anything to a pane. Synchronous (Swift: an actor).
//!
//! 1. The pane's process: TUI panes by `HERDR_PANE_ID` + `HERDR_SOCKET_PATH` in the env; hc panes by their pids.
//! 2. A log file that the agent's process tree holds open. Exactly one, or none.
//! 3. Grok Build only: the session `active_sessions.json` lists for the agent's pid.
//! 4. Codex only: the rollout whose `session_meta` cwd is the pane's cwd and that started at or after the agent.
//!    Exactly one unclaimed match, or none.
//!
//! Processes and open files come from a [`ProcessSource`]: macOS (and other unix) run `ps` and `lsof` as Swift does;
//! Linux reads `/proc` (with the environ, like `ps -E`); Windows lists processes with the `sysinfo` crate but cannot
//! list open files (see `resolver/windows.rs`). The pure logic takes process lists as arguments, so checks inject
//! them.

use crate::session_log::{self, deleting_last_path_component, home_dir, is_abs_path, last_path_component, standardize_path};
use crate::util::Date;
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

mod linux;
#[cfg(all(unix, not(target_os = "linux")))]
mod macos;
#[cfg(windows)]
mod windows;

pub use linux::{parse_linux_stat, LinuxProcSource};
#[cfg(all(unix, not(target_os = "linux")))]
pub use macos::MacProcessSource;
#[cfg(windows)]
pub use windows::WindowsProcessSource;

/// One process: as `ps -E -ww -o pid=,ppid=,lstart=,command=` prints it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    pub start: Date,
    /// Command line followed by the environment, as `ps -E` prints it.
    pub line: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Need {
    pub pane_id: String,
    pub kind: String,
    pub cwd: String,
    /// hc panes: process ids to start from (foreground first). Empty for TUI panes (found by `HERDR_PANE_ID`).
    pub pids: Vec<i32>,
}

impl Need {
    pub fn new(pane_id: &str, kind: &str, cwd: &str, pids: Vec<i32>) -> Need {
        Need { pane_id: pane_id.into(), kind: kind.into(), cwd: cwd.into(), pids }
    }
}

/// A Claude pane whose herdr session is not shown: `teammate` is herdr's session, `lead` the log shown instead.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LeadSwap {
    pub teammate: String,
    pub lead: Option<String>,
}

/// A Claude pane and the session herdr names for it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LeadAsk {
    pub pane_id: String,
    pub session_id: String,
}

impl LeadAsk {
    pub fn new(pane_id: &str, session_id: &str) -> LeadAsk {
        LeadAsk { pane_id: pane_id.into(), session_id: session_id.into() }
    }
}

/// One pass over the Claude panes: the panes whose log is not shown (paneId → swap), and the sessions judged to be a
/// lead's log, which the panes show as herdr names them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LeadPass {
    pub swaps: HashMap<String, LeadSwap>,
    pub leads: HashSet<String>,
}

/// The open files of a process tree (injected by checks).
pub type OpenFiles = Box<dyn Fn(&[i32]) -> Vec<String>>;

/// Where the process table and the open files come from. Read-only.
pub trait ProcessSource: Send + Sync {
    /// This user's processes, each with its command line and environment (`Proc::line`).
    fn processes(&self) -> Vec<Proc>;
    /// Absolute paths that the given processes hold open. Empty where the platform cannot list them.
    fn open_files(&self, pids: &[i32]) -> Vec<String>;
}

/// The source of the host platform: `/proc` on Linux, `ps` + `lsof` on macOS and other unix, `sysinfo` on Windows
/// (no open files there). Other platforms get [`NoProcessSource`] (nothing resolves).
pub fn default_source() -> Box<dyn ProcessSource> {
    #[cfg(target_os = "linux")]
    return Box::new(LinuxProcSource::new());
    #[cfg(all(unix, not(target_os = "linux")))]
    return Box::new(MacProcessSource);
    #[cfg(windows)]
    return Box::new(WindowsProcessSource);
    #[cfg(not(any(unix, windows)))]
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

/// This user's processes on the host ([`default_source`]).
pub fn processes() -> Vec<Proc> {
    default_source().processes()
}

/// The files the given processes hold open on the host ([`default_source`]); empty for no pids.
pub fn open_files(pids: &[i32]) -> Vec<String> {
    if pids.is_empty() {
        return vec![];
    }
    default_source().open_files(pids)
}

/// `stamp`: hc Claude panes only: the mtime of the pid file the entry came from (inner None: no file).
struct Entry {
    pid: Option<i32>,
    path: Option<String>,
    checked: Instant,
    stamp: Option<Option<SystemTime>>,
    stamp_pid: Option<i32>,
    need_pids: Vec<i32>,
}

/// `mate` None: undecided. `fin`: decided, or undecided after `verdict_cap` bytes; read again only if it shrinks.
#[derive(Clone, Copy)]
struct Verdict {
    file: u64,
    size: u64,
    mate: Option<bool>,
    fin: bool,
}

pub struct SessionResolver {
    socket_path: String,
    pub codex_dir: String,
    pub projects_dir: String,
    pub claude_dir: String,
    pub pi_agent_dir: String,
    pub agy_dir: String,
    /// The home folder the open-file route matches logs under (`~/.codex/sessions/`, `~/.claude/projects/`, ...).
    pub home: String,
    /// Unresolved panes are tried again after this (seconds); resolved ones are re-checked after `recheck`.
    pub retry: f64,
    pub recheck: f64,
    /// The most bytes read of one log to judge it.
    pub verdict_cap: usize,
    /// The session runs on another machine: the resolver is off.
    pub remote: bool,
    /// The open files of a process tree (the source's, unless a check injects them).
    pub open_files: OpenFiles,
    /// The process table (`resolve_with` without `procs`).
    source: Arc<dyn ProcessSource>,
    verdict_reads: usize,
    cache: HashMap<String, Entry>,
    verdicts: HashMap<String, Verdict>,
    last_lead: HashMap<String, String>,
    located: HashMap<String, String>,
}

/// Agent kinds whose session id herdr reports through its own integration (`herdr integration install <kind>`).
pub const HERDR_INTEGRATIONS: [&str; 3] = ["claude", "codex", "grok"];

/// On a host nothing guesses a session from its files, so an agent herdr reports with no session id has no
/// conversation until herdr's integration is installed there. The command to run on the host, or None.
pub fn host_setup(kind: &str, session_id: Option<&str>, remote: bool) -> Option<String> {
    if !remote || session_id.is_some() || !HERDR_INTEGRATIONS.contains(&kind) {
        return None;
    }
    Some(format!("herdr integration install {kind}"))
}

/// Swift's `max(by:)`: the first of the greatest elements.
fn max_by_first<T, I: IntoIterator<Item = T>>(it: I, less: impl Fn(&T, &T) -> bool) -> Option<T> {
    let mut best: Option<T> = None;
    for e in it {
        best = match best {
            Some(b) if !less(&b, &e) => Some(b),
            _ => Some(e),
        };
    }
    best
}

fn mtime(path: &str) -> Option<SystemTime> {
    std::fs::symlink_metadata(path).ok()?.modified().ok()
}

fn read_prefix(path: &str, n: usize) -> Option<Vec<u8>> {
    let f = std::fs::File::open(path).ok()?;
    let mut out = vec![];
    f.take(n as u64).read_to_end(&mut out).ok()?;
    Some(out)
}

#[cfg(unix)]
fn file_number(_path: &str, m: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    m.ino()
}

/// Windows: the volume serial and file index (`GetFileInformationByHandle`), as the inode on Unix. 0 when it cannot
/// be read: then a replaced log keeps its verdict until it shrinks.
#[cfg(windows)]
fn file_number(path: &str, _m: &std::fs::Metadata) -> u64 {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    let Ok(f) = std::fs::File::open(path) else { return 0 };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a valid open handle and a zeroed out-struct of the right type.
    if unsafe { GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) } == 0 {
        return 0;
    }
    let index = (info.nFileIndexHigh as u64) << 32 | info.nFileIndexLow as u64;
    index ^ ((info.dwVolumeSerialNumber as u64) << 48)
}

#[cfg(not(any(unix, windows)))]
fn file_number(_path: &str, _m: &std::fs::Metadata) -> u64 {
    0
}

impl SessionResolver {
    /// Defaults as in Swift: `~/.codex/sessions`, `~/.claude/projects`, `~/.claude/sessions`, `~/.pi/agent`,
    /// `~/.gemini/antigravity-cli`; retry 5 s, recheck 30 s, verdict cap 8 MB; the host's processes and open files
    /// ([`default_source`]).
    pub fn new(socket_path: &str) -> SessionResolver {
        let home = home_dir();
        let source: Arc<dyn ProcessSource> = Arc::from(default_source());
        let files = source.clone();
        SessionResolver {
            socket_path: socket_path.into(),
            codex_dir: session_log::default_dir("codex").to_string_lossy().into_owned(),
            projects_dir: session_log::default_dir("claude").to_string_lossy().into_owned(),
            claude_dir: home.clone() + "/.claude/sessions",
            pi_agent_dir: home.clone() + "/.pi/agent",
            agy_dir: home.clone() + "/.gemini/antigravity-cli",
            home,
            retry: 5.0,
            recheck: 30.0,
            verdict_cap: 8 << 20,
            remote: false,
            open_files: Box::new(move |pids: &[i32]| if pids.is_empty() { vec![] } else { files.open_files(pids) }),
            source,
            verdict_reads: 0,
            cache: HashMap::new(),
            verdicts: HashMap::new(),
            last_lead: HashMap::new(),
            located: HashMap::new(),
        }
    }

    /// Processes and open files from `source` instead of the host's.
    pub fn with_source(mut self, source: Box<dyn ProcessSource>) -> SessionResolver {
        let source: Arc<dyn ProcessSource> = Arc::from(source);
        let files = source.clone();
        self.open_files = Box::new(move |pids: &[i32]| if pids.is_empty() { vec![] } else { files.open_files(pids) });
        self.source = source;
        self
    }

    /// Retry and recheck intervals in seconds.
    pub fn with_intervals(mut self, retry: f64, recheck: f64) -> SessionResolver {
        self.retry = retry;
        self.recheck = recheck;
        self
    }

    /// How many times a log was read to judge it.
    pub fn verdict_reads(&self) -> usize {
        self.verdict_reads
    }

    // MARK: - Teammates

    /// Judges the log herdr names for each Claude pane. A lead's log is shown as it is and becomes the pane's last
    /// lead. A teammate's log, an undecided one or one not found is never shown: the pane gets its last lead (judged
    /// again, and unless another pane shows that log), else no log.
    pub fn leads(&mut self, asks: &[LeadAsk], claimed: &HashSet<String>) -> LeadPass {
        if self.remote {
            return LeadPass { swaps: HashMap::new(), leads: asks.iter().map(|a| a.session_id.clone()).collect() };
        }
        let ids: HashSet<&str> = asks.iter().map(|a| a.pane_id.as_str()).collect();
        self.last_lead.retain(|k, _| ids.contains(k.as_str()));
        let mut pass = LeadPass::default();
        let mut asked: HashSet<String> = HashSet::new();
        for a in asks {
            let p = self.path(&a.session_id);
            if let Some(p) = p {
                asked.insert(p.clone());
                if self.judge(&p) == Some(false) {
                    self.last_lead.insert(a.pane_id.clone(), p);
                    pass.leads.insert(a.session_id.clone());
                }
            }
        }
        asked.extend(self.last_lead.values().cloned());
        self.verdicts.retain(|k, _| asked.contains(k));
        self.located.retain(|k, _| asks.iter().any(|a| &a.session_id == k));
        let shown: HashSet<String> =
            claimed.iter().map(|c| if is_abs_path(c) { standardize_path(c) } else { c.clone() }).collect();
        for a in asks {
            if pass.leads.contains(&a.session_id) {
                continue;
            }
            let lead = match self.last_lead.get(&a.pane_id).cloned() {
                Some(l) => {
                    let elsewhere = self.last_lead.iter().any(|(k, v)| k != &a.pane_id && v == &l);
                    if self.judge(&l) != Some(false) || is_claimed(&l, &shown) || elsewhere {
                        None
                    } else {
                        Some(l)
                    }
                }
                None => None,
            };
            pass.swaps.insert(a.pane_id.clone(), LeadSwap { teammate: a.session_id.clone(), lead });
        }
        pass
    }

    /// Whether the log at `path` is a teammate's, None while undecided. A decided verdict is kept for the same file
    /// (file number) while it does not shrink.
    fn judge(&mut self, path: &str) -> Option<bool> {
        let m = std::fs::symlink_metadata(path).ok()?;
        let (file, size) = (file_number(path, &m), m.len());
        if let Some(v) = self.verdicts.get_mut(path) {
            if v.file == file && size >= v.size && (v.fin || size == v.size) {
                v.size = size;
                return v.mate;
            }
        }
        self.verdict_reads += 1;
        let (mate, capped) = teammate_verdict_path(path, self.verdict_cap);
        self.verdicts.insert(path.into(), Verdict { file, size, mate, fin: mate.is_some() || capped });
        mate
    }

    /// Session id → its log's standardized path, kept while the file is there.
    fn path(&mut self, id_or_path: &str) -> Option<String> {
        if is_abs_path(id_or_path) {
            return Some(standardize_path(id_or_path));
        }
        if let Some(p) = self.located.get(id_or_path) {
            if Path::new(p).exists() {
                let next = session_log::continued(Path::new(p), Some(Path::new(&self.projects_dir))).to_string_lossy().into_owned();
                self.located.insert(id_or_path.into(), next.clone());
                return Some(next);
            }
        }
        let p = session_log::locate("claude", id_or_path, &self.projects_dir)
            .map(|u| standardize_path(&u.to_string_lossy()));
        match &p {
            Some(p) => {
                self.located.insert(id_or_path.into(), p.clone());
            }
            None => {
                self.located.remove(id_or_path);
            }
        }
        p
    }

    // MARK: - Resolve

    /// Log paths for the given panes (paneId → absolute `.jsonl` path, or for hc Claude a session id). `claimed`:
    /// session ids or paths that other panes already use; they are never handed out again.
    pub fn resolve(&mut self, needs: &[Need], claimed: &HashSet<String>) -> HashMap<String, String> {
        self.resolve_with(needs, claimed, &HashMap::new(), None)
    }

    /// `resolve` with `claimed_cwd` (the cwd of the pane behind a claim, where known: a Codex claim from a pane in
    /// another cwd than the rollout's does not hide it) and `procs` (a process list to use instead of listing them).
    pub fn resolve_with(
        &mut self,
        needs: &[Need],
        claimed: &HashSet<String>,
        claimed_cwd: &HashMap<String, String>,
        procs: Option<&[Proc]>,
    ) -> HashMap<String, String> {
        if self.remote {
            return HashMap::new();
        }
        let ids: HashSet<&str> = needs.iter().map(|n| n.pane_id.as_str()).collect();
        self.cache.retain(|k, _| ids.contains(k.as_str()));
        let now = Instant::now();
        let due: Vec<&Need> = needs
            .iter()
            .filter(|n| match self.cache.get(&n.pane_id) {
                None => true,
                Some(e) => {
                    let wait = if e.path.is_none() { self.retry } else { self.recheck };
                    now.duration_since(e.checked).as_secs_f64() >= wait || self.pid_file_changed(e) || e.need_pids != n.pids
                }
            })
            .collect();
        if !due.is_empty() {
            let listed;
            let procs: &[Proc] = match procs {
                Some(p) => p,
                None => {
                    listed = self.source.processes();
                    &listed
                }
            };
            let socket = self.socket_path.clone();
            let agent_of = |n: &Need| -> Option<Proc> {
                if n.pids.is_empty() {
                    agent_process(&n.pane_id, &socket, procs)
                } else {
                    hc_agent_process(&n.pids, &n.kind, procs)
                }
            };
            // Two pi in one cwd: each one's start splits the folder's files between them.
            let pi_starts: Vec<(String, String, Date)> = needs
                .iter()
                .filter(|n| n.kind == "pi")
                .filter_map(|n| agent_of(n).map(|a| (n.pane_id.clone(), standardize_path(&n.cwd), a.start)))
                .collect();
            // Earliest agent first: it takes its file before a later one looks.
            let mut ordered: Vec<(&Need, Option<Proc>)> = due.iter().map(|n| (*n, agent_of(n))).collect();
            let far = DateTime::<Utc>::MAX_UTC;
            ordered.sort_by(|a, b| {
                let ka = (a.1.as_ref().map(|p| p.start).unwrap_or(far), &a.0.pane_id);
                let kb = (b.1.as_ref().map(|p| p.start).unwrap_or(far), &b.0.pane_id);
                ka.cmp(&kb)
            });
            for (n, agent) in ordered {
                // agy: /new or /resume swaps the presence lock in the same process, so every recheck reads it again.
                if let Some(old) = self.cache.get(&n.pane_id) {
                    if let Some(path) = &old.path {
                        if old.pid.is_some()
                            && old.pid == agent.as_ref().map(|a| a.pid)
                            && !self.pid_file_changed(old)
                            && n.kind != "agy"
                            && (n.kind != "pi" || Path::new(path).exists())
                        {
                            let e = self.cache.get_mut(&n.pane_id).unwrap();
                            e.checked = now;
                            e.need_pids = n.pids.clone();
                            continue;
                        }
                    }
                }
                let mut taken = claimed.clone();
                for (p, e) in &self.cache {
                    if p != &n.pane_id {
                        if let Some(path) = &e.path {
                            taken.insert(path.clone());
                        }
                    }
                }
                let cwd = standardize_path(&n.cwd);
                let rivals: Vec<Date> =
                    pi_starts.iter().filter(|(pane, c, _)| pane != &n.pane_id && c == &cwd).map(|(_, _, s)| *s).collect();
                let found = agent.as_ref().map(|a| self.find(n, a, procs, &taken, claimed_cwd, &rivals));
                let claude_hc = !n.pids.is_empty() && n.kind == "claude";
                // Unresolved: watch the agent's own file, where a new session id would most likely appear.
                let stamp_pid = found.as_ref().and_then(|f| f.1).or(agent.as_ref().map(|a| a.pid));
                let entry = Entry {
                    pid: agent.as_ref().map(|a| a.pid),
                    path: found.and_then(|f| f.0),
                    checked: now,
                    stamp: if claude_hc { Some(stamp_pid.and_then(|p| self.pid_file_stamp(p))) } else { None },
                    stamp_pid: if claude_hc { stamp_pid } else { None },
                    need_pids: n.pids.clone(),
                };
                self.cache.insert(n.pane_id.clone(), entry);
            }
        }
        let mut out = HashMap::new();
        for n in needs {
            if let Some(p) = self.cache.get(&n.pane_id).and_then(|e| e.path.clone()) {
                out.insert(n.pane_id.clone(), p);
            }
        }
        out
    }

    /// The pid file changed since the entry was made (a stat, no process listing).
    fn pid_file_changed(&self, e: &Entry) -> bool {
        let Some(stamp) = e.stamp else { return false };
        let Some(pid) = e.stamp_pid.or(e.pid) else { return false };
        self.pid_file_stamp(pid) != stamp // no file: None both times, not a change
    }

    fn pid_file_stamp(&self, pid: i32) -> Option<SystemTime> {
        mtime(&format!("{}/{pid}.json", self.claude_dir))
    }

    /// The log path (or Claude session id) and, for hc Claude, the pid whose `<pid>.json` named it.
    fn find(
        &self,
        n: &Need,
        agent: &Proc,
        procs: &[Proc],
        claimed: &HashSet<String>,
        claimed_cwd: &HashMap<String, String>,
        rivals: &[Date],
    ) -> (Option<String>, Option<i32>) {
        if !n.pids.is_empty() && n.kind == "claude" {
            // hc: Claude's own pid file names the session id
            let mut pids = vec![agent.pid];
            pids.extend(&n.pids);
            let Some((sid, pid)) = claude_session_source(&pids, procs, &self.claude_dir) else { return (None, None) };
            return if is_claimed(&sid, claimed) { (None, Some(pid)) } else { (Some(sid), Some(pid)) };
        }
        let tree = subtree(agent.pid, procs);
        let mut open = (self.open_files)(&tree);
        if n.kind == "codex" {
            // Codex also keeps its subagents' rollouts open: only the pane's own one counts.
            open.retain(|p| !(last_path_component(p).starts_with("rollout-") && codex_subagent_rollout(p)));
        }
        if n.kind == "agy" {
            // agy keeps no log open: its presence lock names the conversation
            return match agy_log_path(&open, &self.agy_dir) {
                Some(p) if !is_claimed(&p, claimed) => (Some(p), None),
                _ => (None, None),
            };
        }
        let grok = if n.kind == "grok" { Some(grok_home(&agent.line)) } else { None };
        if let Some(p) = log_path(&open, &n.kind, grok.as_deref(), &self.home) {
            if !is_claimed(&p, claimed) {
                return (Some(p), None);
            }
        }
        if let Some(grok) = grok {
            // Grok Build lists its running sessions with their pids
            let sessions = grok.clone() + "/sessions";
            let Ok(data) = std::fs::read(grok + "/active_sessions.json") else { return (None, None) };
            let Some(id) = grok_session(&data, &tree) else { return (None, None) };
            return match session_log::locate_grok(&id, &sessions) {
                Some(p) if !is_claimed(&p.to_string_lossy(), claimed) => (Some(p.to_string_lossy().into_owned()), None),
                _ => (None, None),
            };
        }
        if n.kind == "pi" {
            // pi keeps no file open: its cwd's session folder
            let dir = pi_session_dir(&n.cwd, &self.pi_agent_dir);
            let mut files: Vec<(String, Date, Vec<u8>)> = vec![];
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for f in rd.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()) {
                    if !f.ends_with(".jsonl") {
                        continue;
                    }
                    let path = format!("{dir}/{f}");
                    let (Some(m), Some(head)) = (mtime(&path), read_prefix(&path, 1 << 16)) else { continue };
                    files.push((path, DateTime::<Utc>::from(m), head));
                }
            }
            return (pi_pick(&files, &n.cwd, agent.start, rivals, |p| is_claimed(p, claimed)), None);
        }
        if n.kind != "codex" {
            return (None, None);
        }
        // Every candidate's session_meta cwd is `n.cwd`: a claim by a pane elsewhere is a wrong report, not a rival.
        let c: Vec<String> = codex_candidates(&self.codex_dir, &n.cwd, agent.start, Utc::now())
            .into_iter()
            .filter(|p| !is_claimed_in_cwd(p, claimed, &n.cwd, claimed_cwd))
            .collect();
        if c.len() == 1 {
            (c.into_iter().next(), None)
        } else {
            (None, None)
        }
    }
}

// MARK: - Teammate verdicts

enum LineVerdict {
    Skip,
    Decided(bool),
    Undecided,
}

/// Whether a Claude log is a teammate's: its first message line (`user` or `assistant`) carries `teamName` and
/// `agentName`. None while no message line decides, such as a first line not yet whole.
pub fn teammate_verdict(data: &[u8]) -> Option<bool> {
    for line in data.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        match line_verdict(line) {
            LineVerdict::Skip => continue,
            LineVerdict::Decided(m) => return Some(m),
            LineVerdict::Undecided => return None,
        }
    }
    None
}

/// `teammate_verdict` of the log at `path`, read line by line: at most 64 KB of each line and `cap` bytes in all.
/// The second value: the read stopped at `cap`.
pub fn teammate_verdict_path(path: &str, cap: usize) -> (Option<bool>, bool) {
    let Ok(mut f) = std::fs::File::open(path) else { return (None, false) };
    let keep = 64 << 10;
    let mut line: Vec<u8> = vec![];
    let mut cut = false;
    let mut read = 0usize;
    let mut buf = vec![0u8; 64 << 10];
    while read < cap {
        let want = (64 << 10).min(cap - read);
        let n = match f.read(&mut buf[..want]) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        read += n;
        let mut rest = &buf[..n];
        while let Some(nl) = rest.iter().position(|b| *b == b'\n') {
            if !cut {
                let room = keep - line.len();
                line.extend_from_slice(&rest[..nl.min(room)]);
            }
            match line_verdict(&line) {
                LineVerdict::Skip => {}
                LineVerdict::Decided(m) => return (Some(m), false),
                LineVerdict::Undecided => return (None, false),
            }
            line.clear();
            cut = false;
            rest = &rest[nl + 1..];
        }
        if !cut {
            let room = keep - line.len();
            line.extend_from_slice(&rest[..rest.len().min(room)]);
            cut = line.len() >= keep;
        }
    }
    (if line.is_empty() { None } else { teammate_verdict(&line) }, read >= cap)
}

/// One log line. A line cut off (by the 64 KB limit, or not yet whole) is judged by its keys: one that begins with a
/// `type` other than `user` or `assistant` is skipped; else the keys before `"message":` decide.
fn line_verdict(line: &[u8]) -> LineVerdict {
    if let Ok(Value::Object(o)) = serde_json::from_slice::<Value>(line) {
        let Some(t) = o.get("type").and_then(|v| v.as_str()) else { return LineVerdict::Skip };
        if t != "user" && t != "assistant" {
            return LineVerdict::Skip;
        }
        let named = |k: &str| o.get(k).and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false);
        return LineVerdict::Decided(named("teamName") && named("agentName"));
    }
    let s = String::from_utf8_lossy(line);
    if s.starts_with(r#"{"type":""#) && !s.starts_with(r#"{"type":"user""#) && !s.starts_with(r#"{"type":"assistant""#) {
        return LineVerdict::Skip;
    }
    let keys = match s.find(r#""message":"#) {
        Some(i) => &s[..i],
        None => &s[..],
    };
    if !keys.contains(r#""type":"user""#) && !keys.contains(r#""type":"assistant""#) {
        return LineVerdict::Undecided;
    }
    // `"<k>":"` followed by a character that is not `"`.
    let named = |k: &str| {
        let pat = format!("\"{k}\":\"");
        keys.match_indices(&pat).any(|(i, _)| keys[i + pat.len()..].chars().next().is_some_and(|c| c != '"'))
    };
    if !keys.contains(r#""teamName":"#) {
        return LineVerdict::Decided(false);
    }
    if named("teamName") && named("agentName") {
        LineVerdict::Decided(true)
    } else {
        LineVerdict::Undecided
    }
}

// MARK: - Pure parts

const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// `EEE MMM d HH:mm:ss yyyy` (`ps` `lstart` in the C locale), five words. The weekday is checked to be one, not
/// against the date (as `DateFormatter` does not).
fn parse_lstart(words: &[&str]) -> Option<NaiveDateTime> {
    if words.len() != 5 || !WEEKDAYS.contains(&words[0].to_ascii_lowercase().as_str()) {
        return None;
    }
    let month = MONTHS.iter().position(|m| *m == words[1].to_ascii_lowercase())? as u32 + 1;
    let day: u32 = words[2].parse().ok()?;
    let year: i32 = words[4].parse().ok()?;
    let t: Vec<&str> = words[3].split(':').collect();
    if t.len() != 3 || t.iter().any(|x| x.is_empty() || x.len() > 2 || !x.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let (h, m, s) = (t[0].parse().ok()?, t[1].parse().ok()?, t[2].parse().ok()?);
    NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(h, m, s)
}

/// A pid file's `procStart`: `ps lstart` text on macOS; on Linux Claude Code writes `/proc/<pid>/stat` starttime
/// (clock ticks after boot) instead.
fn proc_start(ps: &str) -> Option<Date> {
    if !ps.is_empty() && ps.bytes().all(|b| b.is_ascii_digit()) {
        return linux::ticks_to_date(ps.parse().ok()?);
    }
    let words: Vec<&str> = ps.split(' ').filter(|w| !w.is_empty()).collect();
    parse_lstart(&words).and_then(|n| local_to_utc(&Utc, &n))
}

fn local_to_utc<Tz: TimeZone>(tz: &Tz, n: &NaiveDateTime) -> Option<Date> {
    tz.from_local_datetime(n).earliest().map(|d| d.with_timezone(&Utc))
}

/// `ps -E -ww -o pid=,ppid=,lstart=,command=` lines, `lstart` in this machine's time zone.
pub fn parse_ps(text: &str) -> Vec<Proc> {
    parse_ps_in(text, &Local)
}

/// `parse_ps` with `lstart` in `tz` (another machine's run with `TZ=UTC0`).
pub fn parse_ps_in<Tz: TimeZone>(text: &str, tz: &Tz) -> Vec<Proc> {
    let mut out = vec![];
    for raw in text.split('\n').filter(|l| !l.is_empty()) {
        // Split on spaces, empties omitted, at most 7 splits: the eighth piece is the rest of the line.
        let mut w: Vec<&str> = vec![];
        let mut rest = raw;
        while w.len() < 7 {
            rest = rest.trim_start_matches(' ');
            if rest.is_empty() {
                break;
            }
            match rest.find(' ') {
                Some(i) => {
                    w.push(&rest[..i]);
                    rest = &rest[i..];
                }
                None => {
                    w.push(rest);
                    rest = "";
                }
            }
        }
        let rest = rest.trim_start_matches(' ');
        if w.len() != 7 || rest.is_empty() {
            continue;
        }
        let (Ok(pid), Ok(ppid)) = (w[0].parse::<i32>(), w[1].parse::<i32>()) else { continue };
        let Some(start) = parse_lstart(&w[2..7]).and_then(|n| local_to_utc(tz, &n)) else { continue };
        out.push(Proc { pid, ppid, start, line: rest.to_string() });
    }
    out
}

/// `lsof -Fn` output: the file name lines (`n/...`), as absolute paths.
pub fn parse_lsof(text: &str) -> Vec<String> {
    text.split('\n').filter(|l| l.starts_with("n/")).map(|l| l[1..].to_string()).collect()
}

/// The agent process of a pane: the newest direct child of the pane's shell (the topmost process that carries the
/// pane's env). Without a child, the topmost process itself.
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
    max_by_first(pool, |a, b| a.start < b.start).cloned()
}

/// Executable names per agent kind, matched against the first two words of a command line (`node …/codex`).
fn executables(kind: &str) -> Vec<&str> {
    match kind {
        "claude" => vec!["claude"],
        "codex" => vec!["codex"],
        "cursor" => vec!["cursor-agent", "cursor"],
        "pi" => vec!["pi"],
        k => vec![k],
    }
}

fn runs(p: &Proc, kind: &str) -> bool {
    let names = executables(kind);
    p.line.split(' ').filter(|w| !w.is_empty()).take(2).any(|w| names.contains(&last_path_component(w)))
}

/// hc: the agent process of a terminal from its pids (foreground first, then the terminal's own process). A pid that
/// runs the agent itself wins; else the newest process in their trees that does.
pub fn hc_agent_process(pids: &[i32], kind: &str, procs: &[Proc]) -> Option<Proc> {
    let mut by_pid: HashMap<i32, &Proc> = HashMap::new();
    for p in procs {
        by_pid.entry(p.pid).or_insert(p);
    }
    for pid in pids {
        if let Some(p) = by_pid.get(pid) {
            if runs(p, kind) {
                return Some((*p).clone());
            }
        }
    }
    let tree: HashSet<i32> = pids.iter().flat_map(|p| subtree(*p, procs)).collect();
    max_by_first(procs.iter().filter(|p| tree.contains(&p.pid) && runs(p, kind)), |a, b| a.start < b.start).cloned()
}

/// Claude Code writes `<claude dir>/<pid>.json` with `pid`, `sessionId` and `procStart` (UTC, `ps lstart` format).
/// Exactly one session id among the given pids and their trees, or None.
pub fn claude_session(pids: &[i32], procs: &[Proc], dir: &str) -> Option<String> {
    claude_session_source(pids, procs, dir).map(|s| s.0)
}

/// `claude_session` plus the pid whose file named it (the first one in search order).
pub fn claude_session_source(pids: &[i32], procs: &[Proc], dir: &str) -> Option<(String, i32)> {
    claude_session_source_with(pids, procs, |pid| std::fs::read(format!("{dir}/{pid}.json")).ok())
}

/// The pids whose `<pid>.json` may name the session: the given ones, then their process trees.
pub fn claude_search_order(pids: &[i32], procs: &[Proc]) -> Vec<i32> {
    let mut order: Vec<i32> = vec![];
    let all = pids.iter().copied().chain(pids.iter().flat_map(|p| subtree(*p, procs)));
    for p in all {
        if !order.contains(&p) {
            order.push(p);
        }
    }
    order
}

/// NSNumber's `int32Value` of a JSON value (a bool counts as 0 / 1).
fn json_i32(v: &Value) -> Option<i32> {
    match v {
        Value::Number(n) => n.as_i64().map(|i| i as i32).or_else(|| n.as_f64().map(|f| f as i32)),
        Value::Bool(b) => Some(*b as i32),
        _ => None,
    }
}

/// `read`: the contents of `<pid>.json` (here, or fetched from another machine).
pub fn claude_session_source_with(
    pids: &[i32],
    procs: &[Proc],
    read: impl Fn(i32) -> Option<Vec<u8>>,
) -> Option<(String, i32)> {
    let mut by_pid: HashMap<i32, &Proc> = HashMap::new();
    for p in procs {
        by_pid.entry(p.pid).or_insert(p);
    }
    let mut found: HashSet<String> = HashSet::new();
    let mut source: Option<i32> = None;
    for pid in claude_search_order(pids, procs) {
        let Some(proc_) = by_pid.get(&pid) else { continue };
        let Some(data) = read(pid) else { continue };
        if data.len() >= 1 << 20 {
            continue;
        }
        let Ok(Value::Object(o)) = serde_json::from_slice::<Value>(&data) else { continue };
        if o.get("pid").and_then(json_i32) != Some(pid) {
            continue;
        }
        let Some(sid) = o.get("sessionId").and_then(|v| v.as_str()) else { continue };
        if sid.is_empty() || sid.contains('/') {
            continue;
        }
        let Some(ps) = o.get("procStart").and_then(|v| v.as_str()) else { continue };
        let Some(ps) = proc_start(ps) else { continue };
        if (ps - proc_.start).num_milliseconds().abs() > 2000 {
            continue;
        }
        found.insert(sid.to_string());
        if source.is_none() {
            source = Some(pid);
        }
    }
    if found.len() != 1 {
        return None;
    }
    Some((found.into_iter().next()?, source?))
}

/// `pid` and its descendants, breadth first.
pub fn subtree(pid: i32, procs: &[Proc]) -> Vec<i32> {
    let mut out = vec![pid];
    let mut i = 0;
    while i < out.len() {
        let p = out[i];
        let kids: Vec<i32> = procs.iter().filter(|q| q.ppid == p && !out.contains(&q.pid)).map(|q| q.pid).collect();
        out.extend(kids);
        i += 1;
    }
    out
}

/// The one agent log among open files, or None when there is none or more than one. Grok Build keeps its session's
/// `events.jsonl` open: the log is `updates.jsonl` in the same folder. `grok_home`: the agent's `GROK_HOME`
/// (default `~/.grok`). `home`: the home folder of the machine the files are on.
// TODO(windows): the prefixes are unix paths under `home`; open files are not listed on Windows (empty), so nothing
// matches there and the resolver falls back to the pid-file / cwd routes.
pub fn log_path(open_files: &[String], kind: &str, grok_home: Option<&str>, home: &str) -> Option<String> {
    if kind == "grok" {
        let sessions = grok_home.map(String::from).unwrap_or_else(|| home.to_string() + "/.grok") + "/sessions/";
        let dirs: HashSet<String> = open_files
            .iter()
            .filter_map(|p| {
                if !p.starts_with(&sessions) || p.contains("/../") {
                    return None;
                }
                let rest: Vec<&str> = p[sessions.len()..].split('/').filter(|s| !s.is_empty()).collect();
                if rest.len() != 3 || (rest[2] != "events.jsonl" && rest[2] != "updates.jsonl") {
                    return None;
                }
                Some(format!("{sessions}{}/{}", rest[0], rest[1]))
            })
            .collect();
        return if dirs.len() == 1 { dirs.into_iter().next().map(|d| d + "/updates.jsonl") } else { None };
    }
    let hits: HashSet<&String> = open_files
        .iter()
        .filter(|p| {
            if !p.ends_with(".jsonl") || p.contains("/../") {
                return false;
            }
            match kind {
                "codex" => p.starts_with(&(home.to_string() + "/.codex/sessions/")) && last_path_component(p).starts_with("rollout-"),
                "cursor" => p.starts_with(&(home.to_string() + "/.cursor/projects/")) && p.contains("/agent-transcripts/"),
                "pi" => p.starts_with(&(home.to_string() + "/.pi/")),
                _ => p.starts_with(&(home.to_string() + "/.claude/projects/")),
            }
        })
        .collect();
    if hits.len() == 1 {
        hits.into_iter().next().cloned()
    } else {
        None
    }
}

/// Foundation's `UUID(uuidString:)` shape: 8-4-4-4-12 hex digits.
fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { *c == b'-' } else { c.is_ascii_hexdigit() })
}

/// agy holds `<dataDir>/presence/<conversation id>.lock` open while a conversation is active. Exactly one such lock,
/// and its `brain/<id>/.system_generated/logs/transcript_full.jsonl` exists: that path.
pub fn agy_log_path(open_files: &[String], data_dir: &str) -> Option<String> {
    let dir = format!("{data_dir}/presence/");
    let ids: HashSet<String> = open_files
        .iter()
        .filter_map(|p| {
            let id = p.strip_prefix(&dir)?.strip_suffix(".lock")?;
            if is_uuid(id) {
                Some(id.to_lowercase())
            } else {
                None
            }
        })
        .collect();
    if ids.len() != 1 {
        return None;
    }
    let id = ids.into_iter().next()?;
    let path = format!("{data_dir}/brain/{id}/.system_generated/logs/transcript_full.jsonl");
    if Path::new(&path).exists() {
        Some(path)
    } else {
        None
    }
}

/// A path counts as claimed when it, or its session id (the file name ends in `<id>.jsonl`; Grok Build's
/// `<id>/updates.jsonl`), is in `claimed`.
pub fn is_claimed(path: &str, claimed: &HashSet<String>) -> bool {
    if claimed.contains(path) {
        return true;
    }
    claimed.iter().any(|c| claims(path, c))
}

fn claims(path: &str, c: &str) -> bool {
    if path == c {
        return true;
    }
    let name = last_path_component(path);
    let folder =
        if name == "updates.jsonl" { Some(last_path_component(&deleting_last_path_component(path)).to_string()) } else { None };
    !c.is_empty()
        && !c.contains('/')
        && (name == format!("{c}.jsonl") || name.ends_with(&format!("-{c}.jsonl")) || folder.as_deref() == Some(c))
}

/// `is_claimed`, ignoring claims whose pane is known to sit in another cwd than `cwd` (the log's own cwd).
pub fn is_claimed_in_cwd(path: &str, claimed: &HashSet<String>, cwd: &str, claimed_cwd: &HashMap<String, String>) -> bool {
    let want = standardize_path(cwd);
    claimed.iter().any(|c| {
        if !claims(path, c) {
            return false;
        }
        match claimed_cwd.get(c) {
            None => true,
            Some(other) => standardize_path(other) == want,
        }
    })
}

/// Grok Build's home for an agent process: `GROK_HOME` from its env, else `~/.grok`.
pub fn grok_home(proc_line: &str) -> String {
    let re = regex::Regex::new(r"\sGROK_HOME=(\S+)\s").expect("grok home pattern");
    if let Some(m) = re.captures(&format!(" {proc_line} ")) {
        return m[1].to_string();
    }
    home_dir() + "/.grok"
}

/// A JSON number that is a whole `Int` (`as? Int` on a JSONSerialization number).
fn json_int(v: &Value) -> Option<i64> {
    let n = v.as_number()?;
    n.as_i64().or_else(|| n.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 9.2e18).map(|f| f as i64))
}

/// The session of the one `active_sessions.json` entry whose pid is one of `pids`; None for none or several.
pub fn grok_session(active: &[u8], pids: &[i32]) -> Option<String> {
    let Ok(Value::Array(list)) = serde_json::from_slice::<Value>(active) else { return None };
    if !list.iter().all(|e| e.is_object()) {
        return None;
    }
    let want: HashSet<i64> = pids.iter().map(|p| *p as i64).collect();
    let ids: HashSet<String> = list
        .iter()
        .filter_map(|e| {
            let pid = json_int(e.get("pid")?)?;
            let id = e.get("session_id")?.as_str()?;
            if want.contains(&pid) && !id.is_empty() {
                Some(id.to_string())
            } else {
                None
            }
        })
        .collect();
    if ids.len() == 1 {
        ids.into_iter().next()
    } else {
        None
    }
}

/// ISO 8601 with or without fractional seconds (Swift tries both formatters).
pub(crate) fn parse_iso(s: &str) -> Option<Date> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

fn local_naive(d: Date) -> NaiveDateTime {
    d.with_timezone(&Local).naive_local()
}

fn day_folder(dir: &str, d: NaiveDate) -> String {
    use chrono::Datelike;
    format!("{dir}/{:04}/{:02}/{:02}", d.year(), d.month(), d.day())
}

/// Codex rollouts (`YYYY/MM/DD/rollout-<local time>-<id>.jsonl`) that started at or after `after` (2 s slack) and
/// whose `session_meta` cwd is `cwd`. Looks at the day folders (local time) from `after` on, at most 8.
pub fn codex_candidates(dir: &str, cwd: &str, after: Date, now: Date) -> Vec<String> {
    let lower = after - Duration::seconds(2);
    let want = standardize_path(cwd);
    let now_local = local_naive(now);
    let mut out = vec![];
    let mut day = local_naive(lower).date();
    let mut guard_days = 0;
    while day.and_hms_opt(0, 0, 0).is_some_and(|m| m <= now_local) && guard_days < 8 {
        let folder = day_folder(dir, day);
        if let Ok(rd) = std::fs::read_dir(&folder) {
            for f in rd.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()) {
                if !f.starts_with("rollout-") || !f.ends_with(".jsonl") {
                    continue;
                }
                let stamp: String = f.chars().skip(8).take(19).collect();
                let Some(t) = NaiveDateTime::parse_from_str(&stamp, "%Y-%m-%dT%H-%M-%S").ok().and_then(|n| local_to_utc(&Local, &n))
                else {
                    continue;
                };
                if t < lower {
                    continue;
                }
                let path = format!("{folder}/{f}");
                let Some((c, start)) = codex_meta(&path) else { continue };
                if standardize_path(&c) == want && start >= lower {
                    out.push(path);
                }
            }
        }
        let Some(next) = day.succ_opt() else { break };
        day = next;
        guard_days += 1;
    }
    out.sort();
    out
}

/// cwd and start time from a rollout's first line (`session_meta`). Reads at most 1 MB.
pub fn codex_meta(path: &str) -> Option<(String, Date)> {
    codex_meta_data(&read_prefix(path, 1 << 20)?)
}

/// cwd and start time from a rollout's first line.
pub fn codex_meta_data(data: &[u8]) -> Option<(String, Date)> {
    if data.is_empty() {
        return None;
    }
    let nl = data.iter().position(|b| *b == b'\n').unwrap_or(data.len());
    let Ok(Value::Object(o)) = serde_json::from_slice::<Value>(&data[..nl]) else { return None };
    if o.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let p = o.get("payload")?.as_object()?;
    if is_subagent_source(p) {
        return None;
    }
    let cwd = p.get("cwd")?.as_str()?;
    let ts = p.get("timestamp").and_then(|v| v.as_str()).or_else(|| o.get("timestamp").and_then(|v| v.as_str()))?;
    Some((cwd.to_string(), parse_iso(ts)?))
}

/// A subagent's rollout (the approvals reviewer "guardian", `spawn_agent` helpers): `source` is `{"subagent": ...}`.
fn is_subagent_source(payload: &serde_json::Map<String, Value>) -> bool {
    payload.get("source").and_then(|s| s.as_object()).is_some_and(|s| s.contains_key("subagent"))
}

/// Whether the rollout at `path` belongs to a subagent of the Codex that keeps it open, not to the pane.
fn codex_subagent_rollout(path: &str) -> bool {
    let Some(data) = read_prefix(path, 1 << 20) else { return false };
    let nl = data.iter().position(|b| *b == b'\n').unwrap_or(data.len());
    let Ok(Value::Object(o)) = serde_json::from_slice::<Value>(&data[..nl]) else { return false };
    o.get("payload").and_then(|p| p.as_object()).is_some_and(is_subagent_source)
}

/// The day folders (`<dir>/YYYY/MM/DD`, local time) a rollout started at or after `after` can be in, up to `now`
/// (at most 8). `slack_days`: days added before `after` and after `now`.
pub fn codex_day_folders(dir: &str, after: Date, now: Date, slack_days: i64) -> Vec<String> {
    let mut day = local_naive(after - Duration::seconds(2)).date() - Duration::days(slack_days);
    let last = local_naive(now) + Duration::days(slack_days);
    let mut out = vec![];
    while day.and_hms_opt(0, 0, 0).is_some_and(|m| m <= last) && (out.len() as i64) < 8 + 2 * slack_days {
        out.push(day_folder(dir, day));
        let Some(next) = day.succ_opt() else { break };
        day = next;
    }
    out
}

/// The rollouts among `(path, first line)` whose `session_meta` cwd is `cwd` and that started at or after `after`
/// (2 s slack), sorted.
pub fn codex_matches(first_lines: &[(String, Vec<u8>)], cwd: &str, after: Date) -> Vec<String> {
    let want = standardize_path(cwd);
    let lower = after - Duration::seconds(2);
    let mut out: Vec<String> = first_lines
        .iter()
        .filter_map(|(path, line)| {
            let (c, start) = codex_meta_data(line)?;
            if standardize_path(&c) == want && start >= lower {
                Some(path.clone())
            } else {
                None
            }
        })
        .collect();
    out.sort();
    out
}

/// pi's session folder for a cwd (pi session-manager.js getDefaultSessionDirPath).
pub fn pi_session_dir(cwd: &str, agent_dir: &str) -> String {
    let c = cwd.strip_prefix('/').or_else(|| cwd.strip_prefix('\\')).unwrap_or(cwd);
    let mapped: String = c.chars().map(|ch| if "/\\:".contains(ch) { '-' } else { ch }).collect();
    format!("{agent_dir}/sessions/--{mapped}--")
}

/// pi's file among `(path, mtime, first line)`: a `session` header with this cwd, started or written at or after
/// `after` (2 s slack), not claimed; the newest by mtime. `rivals`: start times of the other pi in this cwd; a header
/// at or after a later rival's start is that rival's file.
pub fn pi_pick(
    files: &[(String, Date, Vec<u8>)],
    cwd: &str,
    after: Date,
    rivals: &[Date],
    claimed: impl Fn(&str) -> bool,
) -> Option<String> {
    let later: Vec<Date> = rivals.iter().copied().filter(|r| *r > after).collect();
    let want = standardize_path(cwd);
    let lower = after - Duration::seconds(2);
    let ok = files.iter().filter(|(path, mtime, data)| {
        let nl = data.iter().position(|b| *b == b'\n').unwrap_or(data.len());
        let Ok(Value::Object(o)) = serde_json::from_slice::<Value>(&data[..nl]) else { return false };
        if o.get("type").and_then(|v| v.as_str()) != Some("session") {
            return false;
        }
        let Some(c) = o.get("cwd").and_then(|v| v.as_str()) else { return false };
        if standardize_path(c) != want {
            return false;
        }
        let started = o.get("timestamp").and_then(|v| v.as_str()).and_then(parse_iso);
        if let Some(t) = started {
            if later.iter().any(|r| t >= *r) {
                return false;
            }
        }
        (started.is_some_and(|t| t >= lower) || *mtime >= lower) && !claimed(path)
    });
    max_by_first(ok, |a, b| (a.1, &a.0) < (b.1, &b.0)).map(|f| f.0.clone())
}

#[cfg(test)]
mod tests;
